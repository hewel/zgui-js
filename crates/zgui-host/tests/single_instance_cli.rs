use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, DirBuilder},
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{ffi::OsStringExt, fs::{DirBuilderExt, FileTypeExt, PermissionsExt, symlink}, net::{UnixListener, UnixStream}},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{atomic::{AtomicUsize, Ordering}, mpsc},
    thread,
    time::{Duration, Instant},
};

const APP_ID: &str = "dev.zgui.activation-test";
const HOST: &str = env!("CARGO_BIN_EXE_zgui-host");
const DEADLINE: Duration = Duration::from_secs(5);

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!("zgui-si-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }

    fn bundle(&self, control: bool) -> PathBuf {
        let directory = self.0.join(if control { "with-control" } else { "activation-only" });
        fs::create_dir(&directory).unwrap();
        let style = json!({"name":"lightningcss","profile":"lightningcss-v1","configDigest":digest(b"test")});
        let build_info = serde_json::to_vec(&json!({"styleSystem":style})).unwrap();
        let files = [
            ("app.mjs", br#"
                export const hostApiVersion = 1;
                export function mount() {
                    const services = globalThis.__zguiServices;
                    const stop = services.onActivation(info => console.info('live ' + JSON.stringify(info)));
                    console.info('cold ' + JSON.stringify(services.getInitialActivation()));
                    const job = services.spawnCommand('env', []);
                    job.promise.then(({ stdout }) => {
                        console.info('child-token ' + stdout.split('\n').some(line => line.startsWith('XDG_ACTIVATION_TOKEN=')));
                    });
                    return () => { stop(); job.cancel(); };
                }
            "#.as_slice()),
            ("app.mjs.map", br#"{"version":3,"sources":[],"names":[],"mappings":""}"#),
            ("build-info.json", &build_info),
        ];
        let artifacts: serde_json::Map<String, Value> = files.into_iter().map(|(name, bytes)| {
            fs::write(directory.join(name), bytes).unwrap();
            (name.to_owned(), json!(digest(bytes)))
        }).collect();
        let mut capabilities = json!({"singleInstance":true,"shellSurfaces":true,"commands":["env"]});
        if control { capabilities["controlChannel"] = json!(true); }
        let mut manifest = json!({"schemaVersion":2,"id":APP_ID,"hostApiVersion":1,
            "entry":"app.mjs","sourceMap":"app.mjs.map","styles":[],"assets":[],"fonts":[],
            "styleSystem":style,"artifactDigests":artifacts,"capabilities":capabilities});
        manifest["buildId"] = json!(digest(&serde_json::to_vec(&manifest).unwrap()));
        fs::write(directory.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        directory
    }

    fn command(&self, bundle: &Path) -> Command {
        let mut command = Command::new(HOST);
        command.args(["--pure-shell", "--bundle"]).arg(bundle)
            .current_dir(&self.0)
            .env("ZGUI_PLATFORM", "wayland")
            .env("XDG_RUNTIME_DIR", &self.0)
            .env("WAYLAND_DISPLAY", self.0.join("missing-display"))
            .env_remove("WAYLAND_SOCKET")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env_remove("XDG_ACTIVATION_TOKEN");
        command
    }
}
impl Drop for Workspace {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn digest(bytes: &[u8]) -> String { format!("sha256:{:x}", Sha256::digest(bytes)) }

struct Primary {
    child: Child,
    stderr: mpsc::Receiver<String>,
}
impl Primary {
    fn spawn(command: &mut Command, display: &Path) -> Self {
        let mut child = command.env("WAYLAND_DISPLAY", display).stderr(Stdio::piped()).spawn().unwrap();
        let stderr = BufReader::new(child.stderr.take().unwrap());
        let (send, recv) = mpsc::channel();
        thread::spawn(move || {
            for line in stderr.lines().map_while(Result::ok) {
                if send.send(line).is_err() { break; }
            }
        });
        Self { child, stderr: recv }
    }

    fn expect(&self, prefix: &str) -> String {
        let end = Instant::now() + DEADLINE;
        let mut seen = Vec::new();
        loop {
            let line = self.stderr.recv_timeout(end.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("waiting for {prefix:?}: {error}; {seen:?}"));
            if let Some(value) = line.strip_prefix(prefix) { return value.into(); }
            seen.push(line);
        }
    }
}
impl Drop for Primary {
    fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}

// Windowless primary: advertise only the mandatory globals and answer display
// sync. No renderer, desktop session, synthetic app hooks, or external compositor.
fn compositor(listener: UnixListener) -> thread::JoinHandle<()> {
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let end = Instant::now() + DEADLINE;
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < end => thread::sleep(Duration::from_millis(2)),
                Err(error) => panic!("accept Wayland client: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream.set_read_timeout(Some(DEADLINE)).unwrap();
        let mut header = [0; 8];
        loop {
            match stream.read_exact(&mut header) {
                Ok(()) => {},
                Err(error) if matches!(error.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset) => break,
                Err(error) => panic!("read Wayland request: {error}"),
            }
            let id = word(&header);
            let descriptor = word(&header[4..]);
            let mut body = vec![0; (descriptor >> 16) as usize - 8];
            stream.read_exact(&mut body).unwrap();
            if id != 1 { continue; }
            let object = word(&body);
            match descriptor & 0xffff {
                0 => {
                    event(&mut stream, object, 0, &0_u32.to_ne_bytes());
                    event(&mut stream, 1, 1, &object.to_ne_bytes());
                }
                1 => for (name, interface, version) in [(1_u32, "wl_compositor", 6_u32), (2, "wl_shm", 1), (3, "xdg_wm_base", 6)] {
                    let mut global = Vec::from(name.to_ne_bytes());
                    let len = u32::try_from(interface.len() + 1).unwrap();
                    global.extend(len.to_ne_bytes());
                    global.extend(interface.as_bytes());
                    global.resize(8 + (len as usize).next_multiple_of(4), 0);
                    global.extend(version.to_ne_bytes());
                    event(&mut stream, object, 0, &global);
                },
                opcode => panic!("unexpected wl_display request: {opcode}"),
            }
        }
    })
}

fn word(bytes: &[u8]) -> u32 { u32::from_ne_bytes(bytes[..4].try_into().unwrap()) }

fn event(socket: &mut UnixStream, id: u32, opcode: u32, body: &[u8]) {
    let length = u32::try_from(8 + body.len()).unwrap();
    socket.write_all(&id.to_ne_bytes()).unwrap();
    socket.write_all(&((length << 16) | opcode).to_ne_bytes()).unwrap();
    socket.write_all(body).unwrap();
}

fn success(output: Output) {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn activation_survives_crash_and_forwards_between_live_host_processes() {
    let workspace = Workspace::new();
    let dir = &workspace.0;
    let instance = dir.join("zgui-js").join(format!("{APP_ID}.instance.sock"));
    let control = dir.join("zgui-js").join(format!("{APP_ID}.sock"));
    let display = dir.join("display");
    let listener = UnixListener::bind(&display).unwrap();
    let other_cwd = dir.join("other");
    fs::create_dir(&other_cwd).unwrap();
    let cwd_alias = dir.join("cwd-alias");
    symlink(&other_cwd, &cwd_alias).unwrap();
    let with_control_bundle = workspace.bundle(true);
    let activation_only_bundle = workspace.bundle(false);
    for (bundle, with_control, crash) in [
        (&with_control_bundle, true, true),
        (&with_control_bundle, true, false),
        (&activation_only_bundle, false, false),
    ] {
        let compositor = compositor(listener.try_clone().unwrap());
        let mut command = workspace.command(bundle);
        command.env("XDG_ACTIVATION_TOKEN", "startup-token");
        if !crash { command.args(["--exit-after-ms", "1600"]); }
        command.args(["--", "--boot", "-1", "two words", "", "--"]);
        let mut primary = Primary::spawn(&mut command, &display);
        let initial = primary.expect("js[info]: cold ");
        assert_eq!(serde_json::from_str::<Value>(&initial).unwrap(), json!({"args":["--boot","-1","two words","","--"],"cwd":dir}));
        assert_eq!(primary.expect("js[info]: child-token "), "false");
        let metadata = fs::symlink_metadata(&instance).expect("instance socket published");
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(control.exists(), with_control);

        // This process cannot connect to Wayland. Exit 0 proves it never had to.
        success(workspace.command(bundle).current_dir(&cwd_alias).env("XDG_ACTIVATION_TOKEN", "forwarded-token")
            .args(["--", "--unknown-to-host", "日 α", ""])
            .output().unwrap());
        let live = primary.expect("js[info]: live ");
        assert_eq!(serde_json::from_str::<Value>(&live).unwrap(), json!({"args":["--unknown-to-host","日 α",""],"cwd":other_cwd}));
        if crash {
            primary.child.kill().unwrap();
            assert!(!primary.child.wait().unwrap().success());
            assert!(fs::symlink_metadata(&instance).unwrap().file_type().is_socket(), "SIGKILL leaves a stale instance socket");
            assert!(fs::symlink_metadata(&control).unwrap().file_type().is_socket(), "SIGKILL also leaves a stale control socket");
        } else {
            primary.expect("zgui-host: clean shutdown");
            assert!(primary.child.wait().unwrap().success());
            assert!(!instance.exists(), "clean shutdown removes the instance socket");
            assert!(!control.exists(), "clean shutdown removes the control socket");
        }
        compositor.join().unwrap();
    }

    symlink(&display, &instance).unwrap();
    let result = workspace.command(&activation_only_bundle).output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("no symlinks"), "{:?}", result.stderr);
    assert_eq!(fs::read_link(&instance).unwrap(), display);
}

#[test]
fn secondary_forwards_only_a_valid_activation_token_to_the_primary() {
    let workspace = Workspace::new();
    let bundle = workspace.bundle(false);
    let runtime = workspace.0.join("zgui-js");
    DirBuilder::new().mode(0o700).create(&runtime).unwrap();
    let path = runtime.join(format!("{APP_ID}.instance.sock"));
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    listener.set_nonblocking(true).unwrap();
    for (supplied, expected) in [("launch-α", Some("launch-α")), ("bad\nlaunch", None)] {
        let mut child = workspace.command(&bundle).env("XDG_ACTIVATION_TOKEN", supplied)
            .args(["--", "a file.txt"]).spawn().unwrap();
        let end = Instant::now() + DEADLINE;
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < end => {
                    assert!(child.try_wait().unwrap().is_none(), "secondary exited before forwarding");
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("accept activation request: {error}"),
            }
        };
        stream.set_read_timeout(Some(DEADLINE)).unwrap();
        let mut size = [0; 4];
        stream.read_exact(&mut size).unwrap();
        let mut body = vec![0; u32::from_be_bytes(size) as usize];
        stream.read_exact(&mut body).unwrap();
        stream.write_all(b"zgui-activation-v1:ok\n").unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"appId":APP_ID,"args":["a file.txt"],"cwd":workspace.0,"activationToken":expected}),
        );
    }
}

#[test]
fn application_arguments_are_bounded_utf8_and_host_flags_stay_strict() {
    let oversized = "é".repeat(32769);
    let invalid = std::ffi::OsString::from_vec(vec![0xff]);
    for (args, message) in [
        (vec!["--".into(); 66], "at most 64 application arguments"),
        (vec!["--".into(), oversized.into()], "64 KiB"),
        (vec!["--".into(), invalid], "UTF-8"),
        (vec!["--not-a-host-flag".into(), "--".into()], "unknown argument: --not-a-host-flag"),
    ] {
        let result = Command::new(HOST).args(args).output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains(message), "{}", String::from_utf8_lossy(&result.stderr));
    }
}
