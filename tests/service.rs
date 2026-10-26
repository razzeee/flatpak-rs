//! Supplemental real-process Varlink tests. The VM alone tests systemd activation.
use std::io::{BufReader, Read, Write};
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
        let runtime = std::env::temp_dir().join(format!("flatpak-rs-test-{}", std::process::id()));
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
}
