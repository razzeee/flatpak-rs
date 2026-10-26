use std::process::Command;

#[test]
fn malformed_apply_acknowledgement_reports_uncertain_outcome() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;
    // A transport-only peer: no storage or transaction behavior is substituted.
    let directory = std::env::temp_dir().join(format!("flatpak-rs-cli-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    std::fs::create_dir(directory.join("flatpak-rs")).unwrap();
    let app = serde_json::json!({"collection":"org.example.Collection","id":"org.example.App",
        "arch":std::env::consts::ARCH,"branch":"stable","commit":"a".repeat(64)});
    let pair = serde_json::json!({"app":app,"runtime":app});
    let plan = serde_json::json!({"format_version":1,"target":{"collection":"org.example.Collection",
        "id":"org.example.App","arch":std::env::consts::ARCH,"branch":"stable"},
        "expected":pair,"desired":pair,"sources":{"app":"/unavailable","runtime":"/unavailable"}});
    let file = directory.join("plan.json");
    std::fs::write(&file, serde_json::to_vec(&plan).unwrap()).unwrap();
    let socket = directory.join("flatpak-rs/management.sock");
    let replies: [&'static [u8]; 6] = [
        b"{\"error\":null,\"parameters\":{}}\0",
        b"{broken\0",
        b"{\"parameters\":{}",
        b"",
        b"{\"error\":\"org.flatpakrs.Manager.UpdateFailed\",\"parameters\":null}\0",
        b"{\"error\":\"\",\"parameters\":{}}\0",
    ];
    for response in replies {
        let listener = UnixListener::bind(&socket).unwrap();
        let peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut byte = [0];
            loop {
                stream.read_exact(&mut byte).unwrap();
                if byte == [0] {
                    break;
                }
            }
            stream.write_all(response).unwrap();
        });
        let output = Command::new(env!("CARGO_BIN_EXE_flatpak-rs"))
            .args(["apply-update", file.to_str().unwrap()])
            .env("XDG_RUNTIME_DIR", &directory)
            .output()
            .unwrap();
        peer.join().unwrap();
        std::fs::remove_file(&socket).unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let message = String::from_utf8(output.stderr).unwrap();
        assert!(
            message.contains("outcome uncertain")
                && message.contains("inspect org.example.Collection org.example.App"),
            "{message}"
        );
    }
    std::fs::remove_dir_all(&directory).unwrap();
}

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

#[test]
fn non_utf8_plan_argument_is_rejected_before_submission() {
    use std::os::unix::ffi::OsStringExt;
    let output = Command::new(env!("CARGO_BIN_EXE_flatpak-rs"))
        .arg("apply-update")
        .arg(std::ffi::OsString::from_vec(vec![0xff]))
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let message = String::from_utf8(output.stderr).unwrap();
    assert!(
        message.contains("UTF-8") && !message.contains("outcome uncertain"),
        "{message}"
    );
}
