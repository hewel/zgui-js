//! Live wire and ownership checks. All FD assertions count only this test's memfds.
#![cfg(target_os = "linux")]

use host_services::{
    frame_stream::FrameBudget,
    unix_socket::{Event, Framing, Socket},
};
use rustix::{
    fs::{MemfdFlags, memfd_create},
    net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg},
};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{IoSlice, Read, Write},
    mem::MaybeUninit,
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::Duration,
};

const NAME: &str = "zgui-frame-stream-wire-test";
const TIMEOUT: Duration = Duration::from_secs(2);

struct PeerPath(PathBuf);

impl Drop for PeerPath {
    fn drop(&mut self) {
        fs::remove_file(&self.0).expect("remove the private test socket");
    }
}

fn connect(
    opt_in: bool,
    path: &PeerPath,
    listener: &UnixListener,
) -> (Socket, UnixStream, Receiver<()>) {
    let (wake, notices) = mpsc::channel();
    let wake = Arc::new(move || {
        let _ = wake.send(());
    });
    let socket = if opt_in {
        Socket::connect_frames(path.0.clone(), FrameBudget::default(), None, wake)
    } else {
        Socket::connect(path.0.clone(), Framing::Length, wake)
    }
    .expect("connect to test producer");
    let (peer, _) = listener
        .accept()
        .expect("test producer accepts the endpoint");
    peer.set_read_timeout(Some(TIMEOUT)).unwrap();
    (socket, peer, notices)
}

fn next_event(socket: &Socket, notices: &Receiver<()>) -> Event {
    loop {
        if let Some(event) = socket.try_recv() {
            return event;
        }
        notices
            .recv_timeout(TIMEOUT)
            .expect("worker publishes the socket event");
    }
}

fn control(peer: &mut UnixStream) -> Value {
    let mut length = [0; 4];
    peer.read_exact(&mut length)
        .expect("read framed control size");
    let mut bytes = vec![0; u32::from_be_bytes(length) as usize];
    peer.read_exact(&mut bytes)
        .expect("read framed control payload");
    assert_eq!(bytes[0], 2);
    serde_json::from_slice(&bytes[1..]).unwrap()
}

fn submit(peer: &mut UnixStream, frame: u64, stride: u64, size: u64, rights_at: usize) {
    use std::os::fd::AsFd;
    let fd = File::from(memfd_create(NAME, MemfdFlags::CLOEXEC).unwrap());
    fd.set_len(size).unwrap();
    let body = serde_json::to_vec(&json!({
        "streamId": 3, "frameId": frame, "width": 4, "height": 2,
        "transport": "memfd", "pixelFormat": "rgba8",
        "planes": [{"fdIndex": 0, "offset": 0, "stride": stride}]
    }))
    .unwrap();
    let mut bytes = ((body.len() as u32) + 1).to_be_bytes().to_vec();
    bytes.push(2);
    bytes.extend(body);
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut ancillary = SendAncillaryBuffer::new(&mut space);
    let fds = [fd.as_fd()];
    assert!(ancillary.push(SendAncillaryMessage::ScmRights(&fds)));
    peer.write_all(&bytes[..rights_at]).unwrap();
    // Split both the framing prefix and payload independently of the ancillary barrier.
    assert_eq!(
        sendmsg(
            &*peer,
            &[IoSlice::new(&bytes[rights_at..rights_at + 1])],
            &mut ancillary,
            SendFlags::NOSIGNAL
        )
        .unwrap(),
        1
    );
    peer.write_all(&bytes[rights_at + 1..]).unwrap();
}

fn received_fds() -> usize {
    fs::read_dir("/proc/self/fd")
        .unwrap()
        .flatten()
        .filter(|entry| {
            fs::read_link(entry.path()).is_ok_and(|target| target.to_string_lossy().contains(NAME))
        })
        .count()
}

#[test]
fn live_frame_credits_keep_the_latest_pending_and_close_every_descriptor() {
    let path =
        PeerPath(std::env::temp_dir().join(format!("zgui-frame-{}.sock", std::process::id())));
    let listener = UnixListener::bind(&path.0).unwrap();
    let baseline = received_fds();
    let (socket, mut peer, notices) = connect(true, &path, &listener);
    assert_eq!(
        control(&mut peer),
        json!({"type":"hello","dmabufSupported":false,"supportedFormats":["rgba8","bgra8","nv12"],"supportedModifiers":{}})
    );
    submit(&mut peer, 0, 16, 32, 0);
    assert_eq!(next_event(&socket, &notices), Event::FrameStream(3));
    let stream = socket.frame_stream(3).expect("first frame opens a stream");
    let displayed = stream.take_pending().expect("first frame can be displayed");
    assert_eq!(displayed.header().frame_id, 0);

    for id in 1..=50 {
        submit(&mut peer, id, 16, 32, 0);
    }
    for id in 1..50 {
        assert_eq!(
            control(&mut peer),
            json!({"type":"release","streamId":3,"frameId":id})
        );
    }
    assert_eq!(
        received_fds(),
        baseline + 2,
        "one displayed and one pending memfd"
    );
    let pending = stream
        .take_pending()
        .expect("latest frame supersedes the whole burst");
    assert_eq!(pending.header().frame_id, 50);
    drop(displayed);
    assert_eq!(
        control(&mut peer),
        json!({"type":"release","streamId":3,"frameId":0})
    );
    assert_eq!(received_fds(), baseline + 1);
    drop(pending);
    assert_eq!(
        control(&mut peer),
        json!({"type":"release","streamId":3,"frameId":50})
    );
    submit(&mut peer, 51, 16, 32, 0);
    drop(peer);
    assert_eq!(next_event(&socket, &notices), Event::Close(None));
    assert_eq!(received_fds(), baseline, "EOF closes every pending memfd");
    socket.close().unwrap();

    for (opt_in, stride, size, offset) in [
        (false, 16, 32, 0),
        (true, 15, 32, 0),
        (true, 16, 31, 0),
        (true, 16, 32, 4),
    ] {
        let (socket, mut peer, notices) = connect(opt_in, &path, &listener);
        if opt_in {
            assert_eq!(control(&mut peer)["type"], "hello");
        }
        submit(&mut peer, 1, stride, size, offset);
        let Event::Close(Some(error)) = next_event(&socket, &notices) else {
            panic!("malformed submit must close the socket")
        };
        assert_eq!(error.code, "E_PROTOCOL");
        assert_eq!(
            received_fds(),
            baseline,
            "invalid submission must drop its received FD before publishing Close"
        );
        socket.close().unwrap();
    }

    let (socket, mut peer, notices) = connect(true, &path, &listener);
    assert_eq!(control(&mut peer)["type"], "hello");
    submit(&mut peer, 1, 16, 32, 0);
    assert_eq!(next_event(&socket, &notices), Event::FrameStream(3));
    let stream = socket.frame_stream(3).unwrap();
    let mut displayed = loop {
        if let Some(frame) = stream.take_presentable() {
            break frame;
        }
        notices
            .recv_timeout(TIMEOUT)
            .expect("worker prepares memfd pixels");
    };
    assert_eq!(displayed.take_rgba().expect("software pixels"), [0; 32]);
    assert_eq!(received_fds(), baseline, "upload does not retain the memfd");
    let credit = displayed.credit();
    drop(displayed);
    for id in 2..=3 {
        submit(&mut peer, id, 16, 32, 0);
    }
    assert_eq!(control(&mut peer)["frameId"], 2);
    socket.retire();
    assert_eq!(
        control(&mut peer)["frameId"],
        3,
        "close releases the pending frame"
    );
    drop(credit);
    assert_eq!(
        control(&mut peer)["frameId"],
        1,
        "close drains in-flight producer credits"
    );
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    assert_eq!(received_fds(), baseline);
}
