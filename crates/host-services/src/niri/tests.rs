use super::*;
use std::{
    fs,
    io::Read,
    os::unix::net::UnixListener,
    sync::{atomic::{AtomicU64, Ordering}, mpsc::{self, Receiver, TryRecvError}},
    time::Duration,
};

const DEADLINE: Duration = Duration::from_secs(5);

struct Fixture {
    directory: PathBuf,
    listener: UnixListener,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!("zgui-niri-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&directory).unwrap();
        let listener = UnixListener::bind(directory.join("socket")).unwrap();
        Self { directory, listener }
    }

    fn start(&self) -> (NiriSubscription, Receiver<()>, BufReader<UnixStream>) {
        let (send, wakes) = mpsc::channel();
        let subscription = NiriSubscription::start(self.directory.join("socket"), Arc::new(move || { let _ = send.send(()); })).unwrap();
        let mut fd = libc::pollfd { fd: self.listener.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid initialized poll record, borrowed only for this call.
        assert_eq!(unsafe { libc::poll(&mut fd, 1, 5000) }, 1, "worker did not connect");
        let (socket, _) = self.listener.accept().unwrap();
        socket.set_read_timeout(Some(DEADLINE)).unwrap();
        socket.set_write_timeout(Some(DEADLINE)).unwrap();
        let mut peer = BufReader::new(socket);
        let mut request = String::new();
        peer.read_line(&mut request).unwrap();
        assert_eq!(request, "\"EventStream\"\n");
        (subscription, wakes, peer)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn send(peer: &mut BufReader<UnixStream>, json: &str) {
    writeln!(peer.get_mut(), "{json}").unwrap();
}

fn window(id: u64) -> NiriWindow {
    NiriWindow { id, title: Some(format!("window {id}")), app_id: None, pid: Some(42), workspace_id: Some(1), is_focused: false, is_floating: false, is_urgent: false }
}

fn workspace(id: u64, output: &str) -> NiriWorkspace {
    NiriWorkspace { id, idx: id as u8, name: None, output: Some(output.into()), is_urgent: false, is_active: false, is_focused: false, active_window_id: None }
}

fn local_subscription() -> (NiriSubscription, Receiver<()>) {
    let (send, wakes) = mpsc::channel();
    let shared = Arc::new(Shared { inner: Mutex::new(Inner::default()), wake: Arc::new(move || { send.send(()).unwrap(); }) });
    (NiriSubscription { shared }, wakes)
}

fn apply(subscription: &NiriSubscription, event: Event) {
    subscription.shared.apply(event).unwrap();
}

#[test]
fn initial_aggregation_coalescing_and_shared_snapshot_ownership() {
    for windows_first in [true, false] {
        let (subscription, wakes) = local_subscription();
        let windows = Event::WindowsChanged { windows: vec![window(u64::MAX), window(2)] };
        let workspaces = Event::WorkspacesChanged { workspaces: vec![workspace(2, "B"), workspace(1, "A")] };
        let (first, second) = if windows_first { (windows, workspaces) } else { (workspaces, windows) };
        apply(&subscription, first);
        assert!(!subscription.has_pending());
        assert!(subscription.take().is_none());
        apply(&subscription, second);
        wakes.recv_timeout(DEADLINE).unwrap();
        apply(&subscription, Event::WindowUrgencyChanged { id: 2, urgent: true });
        apply(&subscription, Event::WindowFocusChanged { id: Some(u64::MAX) });
        assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
        let before = subscription.take().unwrap().unwrap();
        assert_eq!(before.windows.iter().map(|row| row.id).collect::<Vec<_>>(), [2, u64::MAX]);
        assert_eq!(before.workspaces.iter().map(|row| row.id).collect::<Vec<_>>(), [1, 2]);
        assert!(before.windows[0].is_urgent);
        assert!(before.windows[1].is_focused);
        assert!(!subscription.has_pending());
        assert!(subscription.take().is_none());
        apply(&subscription, Event::WindowFocusChanged { id: None });
        wakes.recv_timeout(DEADLINE).unwrap();
        let after = subscription.take().unwrap().unwrap();
        assert!(before.windows[1].is_focused, "delivered rows must remain immutable");
        assert!(!after.windows[1].is_focused);
        assert!(Arc::ptr_eq(&before.windows[0], &after.windows[0]));
        assert!(Arc::ptr_eq(&before.workspaces[0], &after.workspaces[0]));
        apply(&subscription, Event::WindowFocusChanged { id: None });
        assert!(!subscription.has_pending(), "unchanged normalized state must not wake");
    }
}

#[test]
fn updates_removals_activation_and_cross_references_follow_niri_semantics() {
    let mut state = State::default();
    let mut one = workspace(1, "A");
    one.is_active = true;
    one.is_focused = true;
    let mut three = workspace(3, "B");
    three.is_active = true;
    state.apply(Event::WorkspacesChanged { workspaces: vec![one, workspace(2, "A"), three] }).unwrap();
    state.apply(Event::WindowsChanged { windows: vec![window(10), window(20)] }).unwrap();
    state.apply(Event::WorkspaceActivated { id: 2, focused: false }).unwrap();
    assert!(state.workspaces[&1].is_focused);
    assert!(!state.workspaces[&1].is_active);
    assert!(state.workspaces[&2].is_active);
    assert!(state.workspaces[&3].is_active);
    state.apply(Event::WorkspaceActivated { id: 3, focused: true }).unwrap();
    assert!(!state.workspaces[&1].is_focused);
    assert!(state.workspaces[&2].is_active);
    assert!(state.workspaces[&3].is_focused);
    state.apply(Event::WorkspaceUrgencyChanged { id: 2, urgent: true }).unwrap();
    state.apply(Event::WorkspaceActiveWindowChanged { workspace_id: 2, active_window_id: Some(999) }).unwrap();
    assert!(state.workspaces[&2].is_urgent);
    assert_eq!(state.workspaces[&2].active_window_id, Some(999));
    state.apply(Event::WindowFocusChanged { id: Some(10) }).unwrap();
    let mut changed = window(20);
    changed.title = Some("changed".into());
    changed.is_focused = true;
    changed.is_floating = true;
    changed.is_urgent = true;
    state.apply(Event::WindowOpenedOrChanged { window: changed }).unwrap();
    assert!(!state.windows[&10].is_focused);
    assert!(state.windows[&20].is_focused && state.windows[&20].is_floating && state.windows[&20].is_urgent);
    assert_eq!(state.windows[&20].title.as_deref(), Some("changed"));
    state.apply(Event::WindowClosed { id: 10 }).unwrap();
    assert!(!state.windows.contains_key(&10));
    state.apply(Event::WindowsChanged { windows: vec![] }).unwrap();
    state.apply(Event::WorkspacesChanged { workspaces: vec![] }).unwrap();
    assert!(state.windows.is_empty() && state.workspaces.is_empty());
    assert_eq!(state.text_bytes, 0);
}

#[test]
fn malformed_relevant_events_fail_but_unrelated_events_are_ignored() {
    for json in [
        r#"{"TouchpadToggled":{"on":true}}"#,
        r#"{"Touchpad\u0054oggled":{"on":true}}"#,
        r#"{"WindowLayoutsChanged":{"changes":[]}}"#,
        r#"{"WindowFocusTimestampChanged":{"id":1,"focus_timestamp":null}}"#,
    ] {
        assert!(parse_event(json.as_bytes()).unwrap().is_none());
    }
    for json in [
        r#"{"WindowFocusChanged":{}}"#,
        r#"{"WindowFocusChanged":{"id":-1}}"#,
        r#"{"WindowUrgencyChanged":{"id":1,"urgent":"yes"}}"#,
        r#"{"WindowsChanged":{"windows":[{"id":1}]}}"#,
        r#"{"WorkspaceActivated":{"id":1}}"#,
        r#"{"WorkspacesChanged":{"workspaces":null}}"#,
        r#"{"TouchpadToggled":true,"WindowsChanged":{"windows":[]}}"#,
        r#"{"TouchpadToggled": [}"#,
    ] {
        assert_eq!(parse_event(json.as_bytes()).err().unwrap().code, "NiriProtocol", "{json}");
    }
    let mut state = State::default();
    assert_eq!(state.apply(Event::WindowsChanged { windows: vec![window(1), window(1)] }).unwrap_err().code, "NiriProtocol");
    assert_eq!(state.apply(Event::WorkspaceActivated { id: 99, focused: true }).unwrap_err().code, "NiriProtocol");
}

#[test]
fn retained_state_limits_cover_incremental_growth_and_replacement() {
    let mut state = State::default();
    assert_eq!(state.apply(Event::WindowsChanged { windows: (0..=MAX_ROWS as u64).map(window).collect() }).unwrap_err().code, "TooLarge");
    state.apply(Event::WindowsChanged { windows: (0..MAX_ROWS as u64).map(window).collect() }).unwrap();
    assert_eq!(state.apply(Event::WindowOpenedOrChanged { window: window(u64::MAX) }).unwrap_err().code, "TooLarge");
    state.apply(Event::WindowsChanged { windows: vec![] }).unwrap();
    let mut large = window(1);
    large.title = Some("x".repeat(MAX_STATE_TEXT_BYTES));
    state.apply(Event::WindowOpenedOrChanged { window: large }).unwrap();
    assert_eq!(state.apply(Event::WindowOpenedOrChanged { window: window(2) }).unwrap_err().code, "TooLarge");
}

#[test]
fn rejected_updates_cannot_escape_through_an_already_pending_snapshot() {
    let mut oversized_window = window(16);
    oversized_window.title = Some("x".repeat(64));
    oversized_window.is_focused = true;
    let oversized_workspace = workspace(1, &"x".repeat(64));
    for rejected in [
        Event::WindowOpenedOrChanged { window: oversized_window },
        Event::WorkspacesChanged { workspaces: vec![oversized_workspace] },
    ] {
        let (subscription, _wakes) = local_subscription();
        let windows = (0..16).map(|id| {
            let mut row = window(id);
            row.title = Some("x".repeat(MAX_STATE_TEXT_BYTES / 16 - 2));
            row.is_focused = id == 0;
            row
        }).collect();
        apply(&subscription, Event::WindowsChanged { windows });
        apply(&subscription, Event::WorkspacesChanged { workspaces: vec![workspace(1, "DP-1")] });
        let accepted = subscription.shared.lock().state.snapshot();
        let error = subscription.shared.apply(rejected).unwrap_err();
        assert_eq!(error.code, "TooLarge");
        // Deterministically consume in the gap before the worker records its
        // terminal error: only the accepted state, including focus, may escape.
        assert_eq!(subscription.take().unwrap().unwrap(), accepted);
        subscription.shared.finish(Err(error));
        assert_eq!(subscription.take().unwrap().unwrap_err().code, "TooLarge");
        assert!(subscription.take().is_none());
    }
}

#[test]
fn socket_handshake_fragmentation_initial_state_and_unknown_events() {
    let fixture = Fixture::new();
    let (subscription, wakes, mut peer) = fixture.start();
    peer.get_mut().write_all(b"{\"Ok\":\"Han").unwrap();
    peer.get_mut().write_all(b"dled\"}\n{\"TouchpadToggled\":{\"on\":true}}\n").unwrap();
    send(&mut peer, r#"{"WindowsChanged":{"windows":[]}}"#);
    send(&mut peer, r#"{"WorkspacesChanged":{"workspaces":[{"id":18446744073709551615,"idx":1,"name":null,"output":"A","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":null}]}}"#);
    wakes.recv_timeout(DEADLINE).unwrap();
    let snapshot = subscription.take().unwrap().unwrap();
    assert!(snapshot.windows.is_empty());
    assert_eq!(snapshot.workspaces[0].id, u64::MAX);
    send(&mut peer, r#"{"WorkspaceUrgencyChanged":{"id":18446744073709551615,"urgent":true}}"#);
    wakes.recv_timeout(DEADLINE).unwrap();
    assert!(subscription.take().unwrap().unwrap().workspaces[0].is_urgent);
    subscription.cancel();
    assert_eq!(peer.read(&mut [0]).unwrap(), 0, "cancel must shut down the installed socket");
}

#[test]
fn handshake_errors_eof_malformed_events_and_oversized_frames_are_terminal() {
    let cases: &[(&[u8], &str)] = &[
        (b"{\"Err\":\"not allowed\"}\n", "NiriProtocol"),
        (b"{\"Ok\":\"Version\"}\n", "NiriProtocol"),
        (b"{\"Ok\":\"Handled\"}\n{\"WindowClosed\":{\"id\":false}}\n", "NiriProtocol"),
        (b"{\"Ok\":\"Handled\"}\n{\"WindowClosed\":", "NiriProtocol"),
        (b"{\"Ok\":\"Handled\"}\n", "NiriProtocol"),
    ];
    for (bytes, code) in cases {
        let fixture = Fixture::new();
        let (subscription, wakes, mut peer) = fixture.start();
        peer.get_mut().write_all(bytes).unwrap();
        drop(peer);
        wakes.recv_timeout(DEADLINE).unwrap();
        assert_eq!(subscription.take().unwrap().unwrap_err().code, *code);
        assert!(!subscription.has_pending());
        assert!(subscription.take().is_none());
    }
    let fixture = Fixture::new();
    let (subscription, wakes, mut peer) = fixture.start();
    let _ = peer.get_mut().write_all(&vec![b'x'; MAX_LINE_BYTES + 1]);
    wakes.recv_timeout(DEADLINE).unwrap();
    assert_eq!(subscription.take().unwrap().unwrap_err().code, "TooLarge");
}

#[test]
fn cancellation_discards_pending_and_future_delivery_including_terminal_errors() {
    let (subscription, wakes) = local_subscription();
    apply(&subscription, Event::WindowsChanged { windows: vec![] });
    apply(&subscription, Event::WorkspacesChanged { workspaces: vec![] });
    wakes.recv_timeout(DEADLINE).unwrap();
    subscription.cancel();
    subscription.cancel();
    apply(&subscription, Event::WindowOpenedOrChanged { window: window(1) });
    subscription.shared.finish(Err(protocol("disconnected")));
    assert!(!subscription.has_pending());
    assert!(subscription.take().is_none());
    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
}

#[test]
fn cancel_before_connect_and_drop_during_blocked_read_release_transport() {
    let fixture = Fixture::new();
    let (subscription, _) = local_subscription();
    subscription.cancel();
    run(&fixture.directory.join("socket"), &subscription.shared).unwrap();
    let mut fd = libc::pollfd { fd: fixture.listener.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid initialized poll record; zero timeout observes without blocking.
    assert_eq!(unsafe { libc::poll(&mut fd, 1, 0) }, 0);
    let (subscription, _, mut peer) = fixture.start();
    // No handshake response: the worker is waiting for readable input.
    drop(subscription);
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
}

#[test]
fn connection_failure_is_asynchronous_and_delivered_once() {
    let fixture = Fixture::new();
    let (send, wakes) = mpsc::channel();
    let subscription = NiriSubscription::start(fixture.directory.join("missing"), Arc::new(move || { send.send(()).unwrap(); })).unwrap();
    wakes.recv_timeout(DEADLINE).unwrap();
    assert_eq!(subscription.take().unwrap().unwrap_err().code, "Io");
    assert!(subscription.take().is_none());
}

#[test]
fn terminal_failure_replaces_pending_snapshot_without_an_extra_wake() {
    let (subscription, wakes) = local_subscription();
    apply(&subscription, Event::WindowsChanged { windows: vec![] });
    apply(&subscription, Event::WorkspacesChanged { workspaces: vec![] });
    wakes.recv_timeout(DEADLINE).unwrap();
    subscription.shared.finish(Err(protocol("EOF")));
    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));
    assert_eq!(subscription.take().unwrap().unwrap_err().message, "EOF");
    assert!(subscription.take().is_none());
}

#[test]
fn cancellation_during_an_in_flight_wake_cannot_restore_pending_delivery() {
    let (entered_send, entered) = mpsc::channel();
    let (release_send, release) = mpsc::channel();
    let release = Mutex::new(release);
    let shared = Arc::new(Shared {
        inner: Mutex::new(Inner::default()),
        wake: Arc::new(move || {
            entered_send.send(()).unwrap();
            release.lock().unwrap().recv_timeout(DEADLINE).unwrap();
        }),
    });
    let subscription = NiriSubscription { shared: Arc::clone(&shared) };
    shared.apply(Event::WindowsChanged { windows: vec![] }).unwrap();
    let worker = thread::spawn(move || shared.apply(Event::WorkspacesChanged { workspaces: vec![] }));
    entered.recv_timeout(DEADLINE).unwrap();
    subscription.cancel();
    release_send.send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert!(!subscription.has_pending());
    assert!(subscription.take().is_none());
}
