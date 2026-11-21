use std::process::Command;

#[test]
fn help_flags_print_usage_and_exit_zero() {
    let bin = env!("CARGO_BIN_EXE_native-host");
    for flag in ["--help", "-h"] {
        let output = Command::new(bin).arg(flag).output().expect("run native-host help");
        assert!(output.status.success(), "{flag} failed: {}", String::from_utf8_lossy(&output.stderr));
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Usage: native-host"), "missing usage header: {stdout}");
        assert!(stdout.contains("--width"), "missing --width in help: {stdout}");
        assert!(stdout.contains("--height"), "missing --height in help: {stdout}");
        assert!(stdout.contains("--title"), "missing --title in help: {stdout}");
        assert!(stdout.contains("--max-js-stack-bytes"), "missing --max-js-stack-bytes in help: {stdout}");
    }
}

#[test]
fn invalid_window_and_stack_cli_flags_fail_cleanly() {
    let bin = env!("CARGO_BIN_EXE_native-host");
    for args in [
        &["--width", "0"][..],
        &["--height", "-10"][..],
        &["--title", "   "][..],
        &["--max-js-stack-bytes", "0"][..],
    ] {
        let output = Command::new(bin).args(args).output().expect("run native-host with invalid arg");
        assert!(!output.status.success(), "expected failure for {args:?}");
    }
}
