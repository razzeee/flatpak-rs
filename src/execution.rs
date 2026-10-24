//! Connection-owned foreground execution. No payload runs outside the helper.
use crate::{installation, sandbox, wire};
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Child(std::process::Child);
impl std::ops::Deref for Child {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for Child {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub variant: installation::Variant,
    pub command: Vec<String>,
    pub memory_mib: Option<u64>,
    pub pids: Option<u64>,
}

struct Group {
    path: PathBuf,
    root: PathBuf,
}
impl Group {
    fn new(memory: u64, pids: u64) -> io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        if fs::read_to_string("/proc/sys/vm/panic_on_oom")?.trim() == "2" {
            return Err(io::Error::other(
                "vm.panic_on_oom=2 panics even on cgroup OOM; require vm.panic_on_oom=0 or 1",
            ));
        }
        let parent = crate::host::delegation_root()?;
        let name = format!(
            "instance-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = parent.join(&name);
        let roots = crate::socket_path()
            .map_err(io::Error::other)?
            .parent()
            .unwrap()
            .join("roots");
        fs::create_dir_all(&roots)?;
        let root = roots.join(name);
        fs::create_dir(&root)?;
        if let Err(error) = fs::create_dir(&path) {
            let _ = fs::remove_dir(&root);
            return Err(error);
        }
        let group = Self { path, root };
        for (file, value) in [
            ("memory.max", (memory * 1024 * 1024).to_string()),
            ("pids.max", pids.to_string()),
            ("memory.swap.max", "0".into()),
            ("memory.oom.group", "1".into()),
        ] {
            fs::write(group.path.join(file), &value)?;
            if fs::read_to_string(group.path.join(file))?.trim() != value {
                return Err(io::Error::other("required cgroup limit readback mismatch"));
            }
        }
        fs::write(group.path.join("cgroup.kill"), "1")?;
        Ok(group)
    }
    fn empty(&self) -> bool {
        fs::read_to_string(self.path.join("cgroup.events"))
            .is_ok_and(|s| s.lines().any(|l| l == "populated 0"))
    }
    fn terminate(&self) -> io::Result<()> {
        let relative = format!(
            "/{}",
            self.path
                .strip_prefix("/sys/fs/cgroup")
                .map_err(io::Error::other)?
                .display()
        );
        for pid in fs::read_to_string(self.path.join("cgroup.procs"))?.lines() {
            let Ok(pid) = pid.parse::<i32>() else {
                continue;
            };
            // SAFETY: pidfd_open pins a process; membership is checked after opening.
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0u32) } as i32;
            if fd < 0 {
                continue;
            }
            // SAFETY: successful pidfd_open transferred ownership.
            let fd = unsafe { OwnedFd::from_raw_fd(fd) };
            if fs::read_to_string(format!("/proc/{pid}/cgroup"))
                .is_ok_and(|s| s.trim() == format!("0::{relative}"))
            {
                // SAFETY: the pidfd cannot be redirected by PID reuse.
                unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        fd.as_raw_fd(),
                        libc::SIGTERM,
                        std::ptr::null::<libc::siginfo_t>(),
                        0u32,
                    );
                }
            }
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while !self.empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.kill()
    }
    fn kill(&self) -> io::Result<()> {
        fs::write(self.path.join("cgroup.kill"), "1")?;
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if self.empty() {
            Ok(())
        } else {
            Err(io::Error::other(
                "instance cgroup did not empty; ownership remains reserved",
            ))
        }
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        let _ = self.kill();
        let _ = fs::remove_dir(&self.path);
        let _ = fs::remove_dir(&self.root);
    }
}

struct Output {
    stream: Mutex<UnixStream>,
    cancel: Arc<AtomicBool>,
}
type Sender = Arc<Output>;
fn event(sender: &Sender, parameters: Value, continues: bool) -> io::Result<()> {
    wire::send_until(
        &mut *sender
            .stream
            .lock()
            .map_err(|_| io::Error::other("writer poisoned"))?,
        &json!({"parameters": parameters, "continues": continues}),
        || sender.cancel.load(Ordering::Relaxed),
    )
}

pub(crate) fn serve(mut stream: UnixStream, request: Value) -> io::Result<()> {
    let result = (|| -> Result<()> {
        let object = request.as_object().ok_or("invalid Run request")?;
        if object
            .keys()
            .any(|key| !["method", "parameters", "more"].contains(&key.as_str()))
            || request["more"] != true
        {
            return Err("Run requires more:true and no oneway/upgrade/unknown fields".into());
        }
        let run: Run = serde_json::from_value(request["parameters"].clone())?;
        start(stream.try_clone()?, run)
    })();
    if let Err(error) = result {
        let detail: String = error.to_string().chars().take(2048).collect();
        wire::send(
            &mut stream,
            &json!({"error": "org.flatpakrs.Manager.OperationFailed", "parameters": {"detail": detail}}),
        )?;
    }
    Ok(())
}

fn start(stream: UnixStream, run: Run) -> Result<()> {
    let mut peer_fd: libc::c_int = -1;
    let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: the authenticated stream yields a newly owned, lifetime-pinned
    // peer pidfd. Process death cancels even if a socket copy remains elsewhere.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            77,
            (&mut peer_fd as *mut libc::c_int).cast(),
            &mut length,
        )
    } == -1
    {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: successful SO_PEERPIDFD transfers ownership of this descriptor.
    let peer_fd = unsafe { OwnedFd::from_raw_fd(peer_fd) };
    if run.command.is_empty() || run.command.iter().any(|arg| arg.contains('\0')) {
        return Err("missing/invalid command".into());
    }
    let memory = run.memory_mib.unwrap_or(256);
    let pids = run.pids.unwrap_or(64);
    if !(16..=1024).contains(&memory) || !(4..=256).contains(&pids) {
        return Err("limits require 16–1024 MiB and 4–256 processes".into());
    }
    let deployment = installation::acquire(run.variant)?;
    let group = Group::new(memory, pids).map_err(|e| format!("required cgroup setup: {e}; require Delegate=memory pids and DelegateSubgroup=supervisor"))?;
    let config = sandbox::Config {
        app: deployment.app,
        runtime: deployment.runtime,
        home: deployment.home,
        root: group.root.clone(),
        cgroup: group.path.clone(),
        command: run.command,
    };
    let (mut config_writer, config_reader) = UnixStream::pair()?;
    let config_fd = config_reader.as_raw_fd();
    let helper = std::env::current_exe()?.with_file_name("flatpak-rs-sandbox");
    let mut command = Command::new(helper);
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: only async-signal-safe syscalls execute between fork and exec.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(config_fd, 3) == -1
                || libc::fcntl(3, libc::F_SETFD, 0) == -1
                || libc::syscall(libc::SYS_close_range, 4u32, u32::MAX, 4u32) == -1
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = Child(command.spawn()?);
    drop(config_reader);
    // The helper is blocked reading configuration. Enroll and verify it before
    // releasing that barrier or publishing "started". Otherwise cancellation
    // could kill an empty cgroup, then wait forever as the helper joins later.
    let child_pid = child.id().to_string();
    fs::write(group.path.join("cgroup.procs"), &child_pid)?;
    if !fs::read_to_string(group.path.join("cgroup.procs"))?
        .lines()
        .any(|pid| pid == child_pid)
    {
        return Err("required instance cgroup enrollment failed".into());
    }
    config_writer.write_all(&serde_json::to_vec(&config)?)?;
    drop(config_writer);
    let input = child.stdin.take().ok_or("missing input pipe")?;
    // SAFETY: fcntl reads/modifies flags on our owned pipe.
    unsafe {
        let flags = libc::fcntl(input.as_raw_fd(), libc::F_GETFL);
        if flags < 0
            || libc::fcntl(input.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) == -1
        {
            return Err(io::Error::last_os_error().into());
        }
    }
    stream.set_write_timeout(Some(Duration::from_millis(100)))?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    let cancel = Arc::new(AtomicBool::new(false));
    let sender = Arc::new(Output {
        stream: Mutex::new(stream.try_clone()?),
        cancel: cancel.clone(),
    });
    let done = Arc::new(AtomicBool::new(false));
    event(&sender, json!({"kind": "started"}), true)?;
    let mut pumps = Vec::new();
    for (kind, mut pipe) in [
        (
            "stdout",
            Box::new(child.stdout.take().ok_or("missing stdout")?) as Box<dyn Read + Send>,
        ),
        (
            "stderr",
            Box::new(child.stderr.take().ok_or("missing stderr")?) as Box<dyn Read + Send>,
        ),
    ] {
        let sender = sender.clone();
        let cancel = cancel.clone();
        pumps.push(std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            loop {
                match pipe.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if event(
                            &sender,
                            json!({"kind": kind, "data": hex::encode(&buffer[..n])}),
                            true,
                        )
                        .is_err()
                        {
                            cancel.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        cancel.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }
        }));
    }
    let (input_queue, input_receiver) = std::sync::mpsc::sync_channel::<Option<Vec<u8>>>(2);
    let pending = Arc::new(AtomicBool::new(false));
    let pipe_pending = pending.clone();
    let pipe_done = done.clone();
    let pipe_cancel = cancel.clone();
    let pipe_sender = sender.clone();
    let input_writer = std::thread::spawn(move || {
        let mut pipe = Some(input);
        while !pipe_done.load(Ordering::Relaxed) && !pipe_cancel.load(Ordering::Relaxed) {
            let message = match input_receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(message) => message,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            };
            if let Some(bytes) = message {
                let mut offset = 0;
                while offset < bytes.len()
                    && !pipe_done.load(Ordering::Relaxed)
                    && !pipe_cancel.load(Ordering::Relaxed)
                {
                    let Some(writer) = pipe.as_mut() else { break };
                    match writer.write(&bytes[offset..]) {
                        Ok(0) => {
                            pipe.take();
                            break;
                        }
                        Ok(n) => offset += n,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        // Early app exit/closed stdin is normal, not cancellation.
                        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
                            pipe.take();
                            break;
                        }
                        Err(_) => {
                            pipe_cancel.store(true, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            } else {
                pipe.take();
            }
            pipe_pending.store(false, Ordering::Relaxed);
            if !pipe_done.load(Ordering::Relaxed)
                && event(&pipe_sender, json!({"kind": "input-ready"}), true).is_err()
            {
                pipe_cancel.store(true, Ordering::Relaxed);
            }
        }
    });
    let input_cancel = cancel.clone();
    let controls_done = Arc::new(AtomicBool::new(false));
    let input_done = controls_done.clone();
    let reader_stream = stream.try_clone()?;
    let input_pump = std::thread::spawn(move || {
        let mut reader = wire::Reader::new(reader_stream);
        let mut closed = false;
        while !input_done.load(Ordering::Relaxed) {
            let value = match reader.next() {
                Ok(Some(value)) => value,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    continue;
                }
                _ => {
                    input_cancel.store(true, Ordering::Relaxed);
                    break;
                }
            };
            let valid = (|| -> Result<bool> {
                if value["oneway"] != true
                    || value.as_object().is_none_or(|o| {
                        o.keys()
                            .any(|k| !["method", "parameters", "oneway"].contains(&k.as_str()))
                    })
                {
                    return Err("invalid stream request".into());
                }
                match value["method"].as_str() {
                    Some("org.flatpakrs.Manager.Cancel")
                        if value.get("parameters").is_none_or(|p| *p == json!({})) =>
                    {
                        return Ok(false);
                    }
                    Some("org.flatpakrs.Manager.CloseInput")
                        if value.get("parameters").is_none_or(|p| *p == json!({})) =>
                    {
                        if closed {
                            return Err("input already closed".into());
                        }
                        closed = true;
                        input_queue.try_send(None)?;
                    }
                    Some("org.flatpakrs.Manager.WriteInput") => {
                        let data = value["parameters"]
                            .as_object()
                            .filter(|o| o.len() == 1)
                            .and_then(|o| o.get("data"))
                            .and_then(Value::as_str)
                            .ok_or("invalid input")?;
                        if data.len() > 8192 {
                            return Err("input chunk too large".into());
                        }
                        let bytes = hex::decode(data)?;
                        if closed || pending.swap(true, Ordering::Relaxed) {
                            return Err("input credit exhausted/closed".into());
                        }
                        input_queue.try_send(Some(bytes))?;
                    }
                    _ => return Err("unsupported stream request".into()),
                }
                Ok(true)
            })();
            if !matches!(valid, Ok(true)) {
                input_cancel.store(true, Ordering::Relaxed);
                break;
            }
        }
    });
    let status = loop {
        let mut peer = libc::pollfd {
            fd: peer_fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid poll descriptor, no blocking.
        if unsafe { libc::poll(&mut peer, 1, 0) } != 0 {
            cancel.store(true, Ordering::Relaxed);
        }
        if cancel.load(Ordering::Relaxed) {
            if let Err(error) = group.terminate() {
                done.store(true, Ordering::Relaxed);
                let _ = stream.shutdown(Shutdown::Both);
                std::mem::forget(deployment.lease);
                return Err(error.into());
            }
            break child.wait()?;
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    done.store(true, Ordering::Relaxed);
    if let Err(error) = group.kill() {
        std::mem::forget(deployment.lease);
        let _ = stream.shutdown(Shutdown::Both);
        return Err(error.into());
    }
    let _ = input_writer.join();
    for pump in pumps {
        let _ = pump.join();
    }
    controls_done.store(true, Ordering::Relaxed);
    let _ = input_pump.join();
    let code = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(9));
    // Cleanup and release ownership before advertising completion, so an
    // immediate next run/uninstall cannot race a stale lease after the reply.
    drop(group);
    drop(deployment.lease);
    event(&sender, json!({"kind": "exit", "exit_status": code}), false)?;
    Ok(())
}
