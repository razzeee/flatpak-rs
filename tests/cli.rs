use std::process::Command;

#[test]
fn connection_failure_is_actionable() {
    let result = Command::new(env!("CARGO_BIN_EXE_flatpak-rs"))
        .arg("status")
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("XDG_RUNTIME_DIR"));
}
