use std::{
    fs,
    process::{Command, Output},
};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_native-host"))
        .args(args)
        .output()
        .expect("launch native-host CLI")
}

#[test]
fn accepts_repeated_bind_socket_flags_without_opening_a_desktop() {
    let path = std::env::temp_dir().join(format!("zgui-socket-binding-{}.mjs", std::process::id()));
    fs::write(
        &path,
        "export function runSmoke() { return 'solid-smoke:ok'; }",
    )
    .expect("write CLI smoke module");
    let result = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .args([
            "--bind-socket",
            "niri=/run/user/42/niri.sock",
            "--bind-socket",
            "status_2=/run/user/42/status=value.sock",
            "--smoke-module",
        ])
        .arg(&path)
        .output();
    fs::remove_file(path).expect("remove CLI smoke module");
    let result = result.expect("launch native-host with explicit socket bindings");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn rejects_invalid_or_duplicate_bind_socket_flags() {
    for (args, expected) in [
        (&["--bind-socket"][..], "requires name=path"),
        (&["--bind-socket", "niri"][..], "requires name=path"),
        (
            &["--bind-socket", "2niri=/run/niri.sock"][..],
            "requires a valid name",
        ),
        (
            &["--bind-socket", "niri=$XDG_RUNTIME_DIR/niri.sock"][..],
            "canonical absolute path",
        ),
        (
            &["--bind-socket", "niri=/run/../niri.sock"][..],
            "canonical absolute path",
        ),
        (
            &[
                "--bind-socket",
                "niri=/run/a",
                "--bind-socket",
                "niri=/run/b",
            ][..],
            "duplicate --bind-socket name",
        ),
    ] {
        let result = run(args);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            !result.status.success() && stderr.contains(expected),
            "{args:?}: {stderr}"
        );
    }
}
