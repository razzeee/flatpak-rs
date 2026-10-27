//! Supplemental real-process Varlink tests. The VM alone tests systemd activation.
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Service {
    child: Child,
    runtime: std::path::PathBuf,
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.runtime);
    }
}

impl Service {
    fn start() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let runtime = std::env::temp_dir().join(format!(
            "flatpak-rs-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&runtime).unwrap();
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        let directory = runtime.join("flatpak-rs");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.join("management.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "export LISTEN_PID=$$; exec \"$1\"",
                "activation-test",
                env!("CARGO_BIN_EXE_flatpak-rs-service"),
            ])
            .env("LISTEN_FDS", "1")
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("XDG_DATA_HOME", runtime.join("data"))
            .stdin(Stdio::null());
        let fd = listener.as_raw_fd();
        // SAFETY: only async-signal-safe descriptor operations run after fork.
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(fd, 3) == -1 || libc::fcntl(3, libc::F_SETFD, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Self {
            child: command.spawn().unwrap(),
            runtime,
        }
    }

    fn connect(&self) -> UnixStream {
        let stream = UnixStream::connect(self.runtime.join("flatpak-rs/management.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
    }
}

#[test]
fn fresh_service_has_no_authorized_remotes() {
    let service = Service::start();
    let output = Command::new(env!("CARGO_BIN_EXE_flatpak-rs"))
        .arg("remotes")
        .env("XDG_RUNTIME_DIR", &service.runtime)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply, serde_json::json!({"remotes": []}));
}

#[test]
fn graphical_run_rejects_paths_instead_of_display_names() {
    let service = Service::start();
    let result = Command::new(env!("CARGO_BIN_EXE_flatpak-rs"))
        .args([
            "run",
            "org.example.Collection",
            "org.example.Missing",
            "x86_64",
            "stable",
            "--wayland",
            "../bus",
            "--",
            "/app/bin/probe",
        ])
        .env("XDG_RUNTIME_DIR", &service.runtime)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("Wayland endpoint"),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn authorized_remote_rejects_tampering_and_can_be_disabled() {
    let service = Service::start();
    let mut fixture = Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/remote-fixture.py"
        ))
        .arg(service.runtime.join("fixture"))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    struct Stop(Child);
    impl Drop for Stop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut line = String::new();
    BufReader::new(fixture.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let _fixture = Stop(fixture);
    let info: serde_json::Value = serde_json::from_str(&line).unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_flatpak-rs"))
            .args(args)
            .env("XDG_RUNTIME_DIR", &service.runtime)
            .output()
            .unwrap()
    };
    let descriptor = info["descriptor"].as_str().unwrap();
    let result = cli(&["remote-add", "test", "org.example.Remote", descriptor]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let remote: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        remote["fingerprints"],
        serde_json::json!([info["fingerprint"]])
    );
    assert_eq!(remote["url"], info["url"]);
    assert_eq!(remote["enabled"], true);
    let wrong_binding = cli(&[
        "plan-install",
        "test",
        "org.example.WrongBinding",
        "x86_64",
        "stable",
        "--runtime-remote",
        "test",
    ]);
    assert!(!wrong_binding.status.success());
    assert!(String::from_utf8_lossy(&wrong_binding.stderr).contains("binding"));
    let planned = cli(&[
        "plan-install",
        "test",
        "org.example.RemoteApp",
        "x86_64",
        "stable",
        "--runtime-remote",
        "test",
    ]);
    assert!(
        planned.status.success(),
        "{}",
        String::from_utf8_lossy(&planned.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&planned.stdout).unwrap();
    assert_eq!(plan["app"]["commit"], info["commit"]);
    assert_eq!(plan["runtime"]["commit"], info["runtime_commit"]);
    assert_eq!(plan["runtime"]["id"], "org.example.Platform");
    assert_eq!(plan["app_provider"]["generation"], remote["generation"]);
    let plan_path = service.runtime.join("install-plan.json");
    std::fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let invalid_command = cli(&[
        "plan-install",
        "test",
        "org.example.RemoteApp",
        "x86_64",
        "invalid-command",
        "--runtime-remote",
        "test",
    ]);
    assert!(invalid_command.status.success());
    let invalid_path = service.runtime.join("invalid-command-plan.json");
    std::fs::write(&invalid_path, invalid_command.stdout).unwrap();
    assert!(
        cli(&["apply-install", invalid_path.to_str().unwrap()])
            .status
            .success()
    );
    let invalid_run = cli(&[
        "run",
        "org.example.Remote",
        "org.example.RemoteApp",
        "x86_64",
        "invalid-command",
    ]);
    assert!(!invalid_run.status.success());
    assert!(String::from_utf8_lossy(&invalid_run.stderr).contains("executable basename"));
    // Advance the signed app ref after saving the plan. Apply must still use
    // the saved commit, while a fresh plan observes the new signed selection.
    let repo_arg = format!("--repo={}", service.runtime.join("fixture/repo").display());
    let key_arg = format!("--gpg-sign={}", info["fingerprint"].as_str().unwrap());
    let home_arg = format!(
        "--gpg-homedir={}",
        service.runtime.join("fixture/keys").display()
    );
    let old_tree = format!("--tree=ref={}", info["commit"].as_str().unwrap());
    let metadata = "--add-metadata-string=xa.metadata=[Application]\nname=org.example.RemoteApp\nruntime=org.example.Platform/x86_64/stable\ncommand=fixture\n";
    let moved = Command::new("ostree")
        .args([
            repo_arg.as_str(),
            "commit",
            "--branch=app/org.example.RemoteApp/x86_64/stable",
            "--subject=moved",
            &old_tree,
            metadata,
            &key_arg,
            &home_arg,
        ])
        .output()
        .unwrap();
    assert!(
        moved.status.success(),
        "{}",
        String::from_utf8_lossy(&moved.stderr)
    );
    assert!(
        Command::new("ostree")
            .args([
                repo_arg.as_str(),
                "summary",
                "--update",
                &key_arg,
                &home_arg
            ])
            .status()
            .unwrap()
            .success()
    );
    let fresh = cli(&[
        "plan-install",
        "test",
        "org.example.RemoteApp",
        "x86_64",
        "stable",
        "--runtime-remote",
        "test",
    ]);
    assert!(
        fresh.status.success(),
        "{}",
        String::from_utf8_lossy(&fresh.stderr)
    );
    let fresh: serde_json::Value = serde_json::from_slice(&fresh.stdout).unwrap();
    assert_ne!(fresh["app"]["commit"], plan["app"]["commit"]);
    assert_eq!(fresh["runtime"], plan["runtime"]);
    assert!(
        !cli(&[
            "inspect",
            "org.example.Remote",
            "org.example.RemoteApp",
            "x86_64",
            "stable"
        ])
        .status
        .success()
    );
    assert!(
        !cli(&["remote-add", "test", "org.example.Remote", descriptor])
            .status
            .success()
    );
    assert!(
        !cli(&[
            "remote-add",
            "wrong-collection",
            "org.example.Other",
            descriptor
        ])
        .status
        .success()
    );
    assert!(
        !cli(&[
            "remote-add",
            "wrong-key",
            "org.example.Remote",
            info["wrong_descriptor"].as_str().unwrap()
        ])
        .status
        .success()
    );
    assert!(
        cli(&["remote-add", "runtime", "org.example.Remote", descriptor])
            .status
            .success()
    );
    let withdrawn = cli(&[
        "plan-install",
        "test",
        "org.example.RemoteApp",
        "x86_64",
        "stable",
        "--runtime-remote",
        "runtime",
    ]);
    assert!(withdrawn.status.success());
    let withdrawn_path = service.runtime.join("withdrawn-plan.json");
    std::fs::write(&withdrawn_path, &withdrawn.stdout).unwrap();
    assert!(cli(&["remote-disable", "runtime"]).status.success());
    assert!(
        !cli(&["apply-install", withdrawn_path.to_str().unwrap()])
            .status
            .success()
    );
    assert!(
        cli(&[
            "remote-replace",
            "runtime",
            "org.example.Remote",
            descriptor
        ])
        .status
        .success()
    );
    assert!(
        !cli(&["apply-install", withdrawn_path.to_str().unwrap()])
            .status
            .success()
    );
    // Withdraw trust while an accepted installation is blocked on payload I/O.
    // Remote authorization must remain available and publication must fail.
    assert!(
        cli(&["remote-add", "racing", "org.example.Remote", descriptor])
            .status
            .success()
    );
    let racing = cli(&[
        "plan-install",
        "racing",
        "org.example.RemoteApp",
        "x86_64",
        "stable",
        "--runtime-remote",
        "racing",
    ]);
    assert!(racing.status.success());
    let racing_path = service.runtime.join("racing-plan.json");
    std::fs::write(&racing_path, &racing.stdout).unwrap();
    let gate = service.runtime.join("fixture/pause-content");
    std::fs::write(&gate, b"").unwrap();
    let mut pending = Command::new(env!("CARGO_BIN_EXE_flatpak-rs"))
        .args(["apply-install", racing_path.to_str().unwrap()])
        .env("XDG_RUNTIME_DIR", &service.runtime)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !service.runtime.join("fixture/content-waiting").exists() {
        assert!(
            pending.try_wait().unwrap().is_none(),
            "installation exited before download gate"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "download gate never reached"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(cli(&["remote-disable", "racing"]).status.success());
    std::fs::remove_file(gate).unwrap();
    let pending = pending.wait_with_output().unwrap();
    assert!(!pending.status.success());
    assert!(String::from_utf8_lossy(&pending.stderr).contains("disabled"));
    assert!(
        !cli(&[
            "inspect",
            "org.example.Remote",
            "org.example.RemoteApp",
            "x86_64",
            "stable"
        ])
        .status
        .success()
    );
    // An accepted request outlives its client. A second independent cache makes
    // the same deterministic HTTP gate observable before disconnecting.
    assert!(
        cli(&[
            "remote-add",
            "client-loss",
            "org.example.Remote",
            descriptor
        ])
        .status
        .success()
    );
    let detached = cli(&[
        "plan-install",
        "client-loss",
        "org.example.RemoteApp",
        "x86_64",
        "stable",
        "--runtime-remote",
        "client-loss",
    ]);
    assert!(detached.status.success());
    let detached_plan: serde_json::Value = serde_json::from_slice(&detached.stdout).unwrap();
    let detached_path = service.runtime.join("detached-plan.json");
    std::fs::write(&detached_path, &detached.stdout).unwrap();
    std::fs::remove_file(service.runtime.join("fixture/content-waiting")).unwrap();
    let gate = service.runtime.join("fixture/pause-content");
    std::fs::write(&gate, b"").unwrap();
    let mut client = Command::new(env!("CARGO_BIN_EXE_flatpak-rs"))
        .args(["apply-install", detached_path.to_str().unwrap()])
        .env("XDG_RUNTIME_DIR", &service.runtime)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !service.runtime.join("fixture/content-waiting").exists() {
        assert!(client.try_wait().unwrap().is_none());
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    client.kill().unwrap();
    client.wait().unwrap();
    std::fs::remove_file(gate).unwrap();
    let completed = cli(&[
        "inspect",
        "org.example.Remote",
        "org.example.RemoteApp",
        "x86_64",
        "stable",
    ]);
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let completed: serde_json::Value = serde_json::from_slice(&completed.stdout).unwrap();
    assert_eq!(completed["app"], detached_plan["app"]);
    assert_eq!(completed["runtime"], detached_plan["runtime"]);
    assert!(
        cli(&[
            "uninstall",
            "org.example.Remote",
            "org.example.RemoteApp",
            "x86_64",
            "stable"
        ])
        .status
        .success()
    );
    let applied = cli(&["apply-install", plan_path.to_str().unwrap()]);
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let applied: serde_json::Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(applied["status"], "installed");
    assert_eq!(applied["installed"]["app"], plan["app"]);
    assert_eq!(applied["installed"]["runtime"], plan["runtime"]);
    let signature = service.runtime.join("fixture/repo/summary.sig");
    let original_signature = std::fs::read(&signature).unwrap();
    std::fs::remove_file(&signature).unwrap();
    assert!(
        !cli(&["remote-add", "unsigned", "org.example.Remote", descriptor])
            .status
            .success()
    );
    std::fs::write(&signature, &original_signature).unwrap();
    std::fs::write(
        service.runtime.join("fixture/repo/summary.sig"),
        b"tampered",
    )
    .unwrap();
    assert!(
        !cli(&["remote-replace", "test", "org.example.Remote", descriptor])
            .status
            .success()
    );
    let list = cli(&["remotes"]);
    let list: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(list["remotes"][0], remote);
    let disabled = cli(&["remote-disable", "test"]);
    assert!(
        disabled.status.success(),
        "{}",
        String::from_utf8_lossy(&disabled.stderr)
    );
    let disabled: serde_json::Value = serde_json::from_slice(&disabled.stdout).unwrap();
    assert_eq!(disabled["enabled"], false);
    let retry = cli(&["apply-install", plan_path.to_str().unwrap()]);
    assert!(
        retry.status.success(),
        "{}",
        String::from_utf8_lossy(&retry.stderr)
    );
    let retry: serde_json::Value = serde_json::from_slice(&retry.stdout).unwrap();
    assert_eq!(retry["status"], "unchanged");
}

#[test]
fn real_service_rejects_invalid_requests_and_remains_available() {
    let mut service = Service::start();
    let mut client = service.connect();
    client
        .write_all(b"{\"method\":\"org.varlink.service.GetInfo\"}\0")
        .unwrap();
    let mut reply = Vec::new();
    for byte in BufReader::new(&mut client).bytes() {
        let byte = byte.unwrap();
        if byte == 0 {
            break;
        }
        reply.push(byte);
    }
    let reply: serde_json::Value = serde_json::from_slice(&reply).unwrap();
    assert_eq!(
        reply["parameters"]["interfaces"][1],
        "org.flatpakrs.Manager"
    );

    client
        .write_all(b"{\"method\":\"org.flatpakrs.Manager.Install\",\"parameters\":{}}\0")
        .unwrap();
    let reply: Vec<u8> = BufReader::new(&mut client)
        .bytes()
        .map(Result::unwrap)
        .take_while(|b| *b != 0)
        .collect();
    let reply: serde_json::Value = serde_json::from_slice(&reply).unwrap();
    assert_eq!(reply["error"], "org.varlink.service.InvalidParameter");

    client.write_all(b"{\"method\":\"org.varlink.service.GetInfo\",\"parameters\":[],\"oneway\":true}\0{\"method\":\"org.varlink.service.GetInfo\"}\0").unwrap();
    let mut reply = Vec::new();
    for byte in BufReader::new(&mut client).bytes() {
        let byte = byte.unwrap();
        if byte == 0 {
            break;
        }
        reply.push(byte);
    }
    let reply: serde_json::Value = serde_json::from_slice(&reply).unwrap();
    assert!(
        reply.get("error").is_none(),
        "oneway must suppress error replies too"
    );

    let mut malformed = service.connect();
    malformed.write_all(b"{\"method\":7}\0").unwrap();
    // Invalid request shape is rejected immediately, not kept as an idle session.
    assert_eq!(malformed.read(&mut [0]).unwrap(), 0);
    assert!(service.child.try_wait().unwrap().is_none());

    let mut optional = service.connect();
    let request = serde_json::json!({"method": "org.flatpakrs.Manager.Run", "more": true,
        "parameters": {"variant": {"collection": "org.example.Collection", "id": "org.example.Missing",
        "arch": std::env::consts::ARCH, "branch": "stable"}, "command": ["/app/bin/missing"],
        "memory_mib": null, "pids": null}});
    optional
        .write_all(&serde_json::to_vec(&request).unwrap())
        .unwrap();
    optional.write_all(&[0]).unwrap();
    let reply: Vec<u8> = BufReader::new(&mut optional)
        .bytes()
        .map(Result::unwrap)
        .take_while(|b| *b != 0)
        .collect();
    let reply: serde_json::Value = serde_json::from_slice(&reply).unwrap();
    assert!(
        reply["parameters"]["detail"]
            .as_str()
            .unwrap()
            .contains("not installed"),
        "{reply}"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_flatpak-rs"))
        .arg("status")
        .env("XDG_RUNTIME_DIR", &service.runtime)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["ready"], false);
    assert!(
        status["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["name"] == "cgroup_control" && check["available"] == false)
    );

    let mut planning = service.connect();
    let target = serde_json::json!({"collection":"org.example.Collection", "id":"org.example.Missing",
        "arch":std::env::consts::ARCH, "branch":"stable"});
    let request = serde_json::json!({"method":"org.flatpakrs.Manager.PlanUpdate", "parameters":{
        "source":"/missing/source", "app":target, "runtime":{
            "collection":"org.example.Collection", "id":"org.example.Platform",
            "arch":std::env::consts::ARCH, "branch":"stable"}}});
    planning
        .write_all(&serde_json::to_vec(&request).unwrap())
        .unwrap();
    planning.write_all(&[0]).unwrap();
    let bytes: Vec<_> = BufReader::new(planning)
        .bytes()
        .map(Result::unwrap)
        .take_while(|b| *b != 0)
        .collect();
    let reply: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reply["error"], "org.flatpakrs.Manager.UpdateNotInstalled");
    assert_eq!(reply["parameters"]["target"], target);

    let mut duplicate = service.connect();
    duplicate.write_all(b"{\"method\":\"org.flatpakrs.Manager.PlanUpdate\",\"parameters\":{},\"parameters\":{}}\0").unwrap();
    assert_eq!(
        duplicate.read(&mut [0]).unwrap(),
        0,
        "duplicate members must be rejected before dispatch"
    );

    let app = serde_json::json!({"collection":"org.example.Collection","id":"org.example.Missing",
        "arch":std::env::consts::ARCH,"branch":"stable","commit":"a".repeat(64)});
    let runtime = serde_json::json!({"collection":"org.example.Collection","id":"org.example.Platform",
        "arch":std::env::consts::ARCH,"branch":"stable","commit":"b".repeat(64)});
    let pair = serde_json::json!({"app":app,"runtime":runtime});
    let plan = serde_json::json!({"format_version":1,"target":target,"expected":pair,"desired":pair,
        "sources":{"app":"/missing/source","runtime":"/missing/source"}});
    let mut apply = service.connect();
    let request = serde_json::json!({"method":"org.flatpakrs.Manager.ApplyUpdate","parameters":{"plan":plan}});
    apply
        .write_all(&serde_json::to_vec(&request).unwrap())
        .unwrap();
    apply.write_all(&[0]).unwrap();
    let bytes: Vec<_> = BufReader::new(apply)
        .bytes()
        .map(Result::unwrap)
        .take_while(|b| *b != 0)
        .collect();
    let reply: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reply["error"], "org.flatpakrs.Manager.UpdateNotInstalled");
    let file = service.runtime.join("missing-plan.json");
    std::fs::write(
        &file,
        serde_json::to_vec(&request["parameters"]["plan"]).unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_flatpak-rs"))
        .args(["apply-update", file.to_str().unwrap()])
        .env("XDG_RUNTIME_DIR", &service.runtime)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let message = String::from_utf8(output.stderr).unwrap();
    assert!(
        message.contains("UpdateNotInstalled") && !message.contains("outcome uncertain"),
        "{message}"
    );
}
