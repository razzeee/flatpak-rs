//! Connection-owned foreground execution. No payload runs outside the helper.
use crate::{installation, wire};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
mod instance;
use instance::Instance;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

// Process ownership outlives helper waiting: output and even the final reply
// can still be backpressured after the app exits. A scope-owned monitor keeps
// cancellation live throughout teardown, even if another process holds a
// socket copy and prevents the control reader from observing EOF.
struct PeerMonitor {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl PeerMonitor {
    fn new(peer_fd: OwnedFd, cancel: Arc<AtomicBool>) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = std::thread::Builder::new()
            .name("run-peer-monitor".into())
            .spawn(move || {
                let mut peer = libc::pollfd {
                    fd: peer_fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                while !stopped.load(Ordering::Relaxed) {
                    // SAFETY: the owned pidfd stays alive through this bounded poll.
                    if unsafe { libc::poll(&mut peer, 1, 100) } != 0 {
                        cancel.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for PeerMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub variant: installation::Variant,
    pub command: Vec<String>,
    pub memory_mib: Option<u64>,
    pub pids: Option<u64>,
    #[serde(default)]
    pub wayland: Option<String>,
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

pub(crate) fn serve(
    mut stream: UnixStream,
    request: Value,
    installation: &installation::Installation,
) -> io::Result<()> {
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
        start(stream.try_clone()?, run, installation)
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

fn start(stream: UnixStream, run: Run, installation: &installation::Installation) -> Result<()> {
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
    let (instance, pipes) = Instance::start(run, installation)?;
    let input = pipes.stdin;
    stream.set_write_timeout(Some(Duration::from_millis(100)))?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    let cancel = Arc::new(AtomicBool::new(false));
    let _peer_monitor = PeerMonitor::new(peer_fd, cancel.clone())?;
    let sender = Arc::new(Output {
        stream: Mutex::new(stream.try_clone()?),
        cancel: cancel.clone(),
    });
    let done = Arc::new(AtomicBool::new(false));
    event(&sender, json!({"kind": "started"}), true)?;
    let mut pumps = Vec::new();
    for (kind, mut pipe) in [
        ("stdout", Box::new(pipes.stdout) as Box<dyn Read + Send>),
        ("stderr", Box::new(pipes.stderr) as Box<dyn Read + Send>),
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
    let completion = instance.wait(&cancel);
    done.store(true, Ordering::Relaxed);
    let completion = match completion {
        Ok(completion) => completion,
        Err(error) => {
            let _ = stream.shutdown(Shutdown::Both);
            return Err(error);
        }
    };
    let _ = input_writer.join();
    for pump in pumps {
        let _ = pump.join();
    }
    controls_done.store(true, Ordering::Relaxed);
    let _ = input_pump.join();
    let code = completion.finish()?;
    event(&sender, json!({"kind": "exit", "exit_status": code}), false)?;
    Ok(())
}
