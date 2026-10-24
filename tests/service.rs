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
}
