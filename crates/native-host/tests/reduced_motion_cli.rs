use std::{fs, process::Command};

#[test]
fn reduced_motion_flag_is_accepted_without_opening_a_desktop() {
    let path = std::env::temp_dir().join(format!("zgui-reduced-motion-cli-{}.mjs", std::process::id()));
    fs::write(&path, "export function runSmoke() { return 'solid-smoke:ok'; }").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_native-host"))
        .args(["--reduced-motion", "--smoke-module"]).arg(&path).output();
    let _ = fs::remove_file(path);
    let result = result.unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "solid-smoke:ok");
}
