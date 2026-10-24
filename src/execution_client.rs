//! Host-side foreground stream adapter; stdin never shares management authority.
use crate::{installation::Variant, wire};
use serde_json::json;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

static CANCEL: AtomicBool = AtomicBool::new(false);
extern "C" fn signal(_: libc::c_int) {
    CANCEL.store(true, Ordering::Relaxed);
}

fn duplicate(fd: i32) -> io::Result<File> {
    // SAFETY: fcntl duplicates an existing descriptor without changing its flags.
    let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if copy < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the successful call returned a newly owned descriptor.
    Ok(unsafe { File::from_raw_fd(copy) })
}

struct Output {
    requests: std::sync::mpsc::SyncSender<(bool, Vec<u8>)>,
    replies: std::sync::mpsc::Receiver<io::Result<()>>,
}
impl Output {
    fn new(mut stdout: File, mut stderr: File) -> Self {
        let (requests, work) = std::sync::mpsc::sync_channel::<(bool, Vec<u8>)>(1);
        let (completed, replies) = std::sync::mpsc::sync_channel(1);
        // CLI-process-only worker: on cancellation main exits without joining a
        // blocked host write. Never hold Rust's global stdout/stderr locks here,
        // which would deadlock process-exit flushing. At most one chunk is live.
        std::thread::spawn(move || {
            while let Ok((is_stdout, bytes)) = work.recv() {
                let file = if is_stdout { &mut stdout } else { &mut stderr };
                if completed.send(file.write_all(&bytes)).is_err() {
                    break;
                }
            }
        });
        Self { requests, replies }
    }

    fn write(&self, is_stdout: bool, bytes: Vec<u8>) -> io::Result<()> {
        self.requests
            .send((is_stdout, bytes))
            .map_err(|_| io::Error::other("output worker stopped"))?;
        loop {
            if CANCEL.load(Ordering::Relaxed) {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            match self.replies.recv_timeout(Duration::from_millis(50)) {
                Ok(result) => return result,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err(io::Error::other("output worker stopped")),
            }
        }
    }
}

pub fn run(args: &[String]) -> Result<i32, Box<dyn std::error::Error>> {
    if args.len() < 6 {
        return Err("usage: flatpak-rs run COLLECTION APP ARCH BRANCH [--memory-mib N] [--pids N] -- COMMAND [ARG...]".into());
    }
    let variant = Variant {
        collection: args[0].clone(),
        id: args[1].clone(),
        arch: args[2].clone(),
        branch: args[3].clone(),
    };
    let mut memory_mib = 256u64;
    let mut pids = 64u64;
    let mut index = 4;
    while args.get(index).is_some_and(|s| s != "--") {
        let value: u64 = args.get(index + 1).ok_or("missing limit value")?.parse()?;
        match args[index].as_str() {
            "--memory-mib" => memory_mib = value,
            "--pids" => pids = value,
            _ => return Err("unknown run option".into()),
        }
        index += 2;
    }
    if args.get(index).map(String::as_str) != Some("--") || args.len() <= index + 1 {
        return Err("run requires -- COMMAND".into());
    }
    let stdout = duplicate(1)?;
    let stderr = duplicate(2)?;
    // Pin stdin before socket creation: a closed fd 0 must never become the
    // management connection and be read by the stdin pump.
    let input = match duplicate(0) {
        Ok(file) => file,
        Err(error) if error.raw_os_error() == Some(libc::EBADF) => File::open("/dev/null")?,
        Err(error) => return Err(error.into()),
    };
    let mut stream = UnixStream::connect(crate::socket_path()?).map_err(|e| {
        format!("cannot connect to management socket: {e}; enable flatpak-rs.socket")
    })?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    wire::send(
        &mut stream,
        &json!({"method": "org.flatpakrs.Manager.Run", "more": true,
        "parameters": {"variant": variant, "command": args[index + 1..], "memory_mib": memory_mib, "pids": pids}}),
    )?;
    CANCEL.store(false, Ordering::Relaxed);
    // SAFETY: handlers perform only a lock-free atomic store, no I/O or allocation.
    unsafe {
        libc::signal(libc::SIGINT, signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, signal as *const () as libc::sighandler_t);
    }
    let output = Output::new(stdout, stderr);
    let sender = Arc::new(Mutex::new(stream.try_clone()?));
    let done = Arc::new(AtomicBool::new(false));
    let credit = Arc::new((Mutex::new(false), Condvar::new()));
    let input_credit = credit.clone();
    let input_sender = sender.clone();
    let input_done = done.clone();
    std::thread::spawn(move || {
        let mut buffer = [0; 4096];
        let mut input = input;
        while !input_done.load(Ordering::Relaxed) {
            let (lock, wake) = &*input_credit;
            let mut ready = match lock.lock() {
                Ok(ready) => ready,
                Err(_) => break,
            };
            while !*ready && !input_done.load(Ordering::Relaxed) && !CANCEL.load(Ordering::Relaxed)
            {
                ready = match wake.wait_timeout(ready, Duration::from_millis(100)) {
                    Ok((ready, _)) => ready,
                    Err(_) => return,
                };
            }
            if input_done.load(Ordering::Relaxed) || CANCEL.load(Ordering::Relaxed) {
                break;
            }
            *ready = false;
            drop(ready);
            let n = match input.read(&mut buffer) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => 0,
            };
            let message = if n == 0 {
                json!({"method": "org.flatpakrs.Manager.CloseInput", "parameters": {}, "oneway": true})
            } else {
                json!({"method": "org.flatpakrs.Manager.WriteInput", "parameters": {"data": hex::encode(&buffer[..n])}, "oneway": true})
            };
            let Ok(mut writer) = input_sender.lock() else {
                break;
            };
            if wire::send(&mut writer, &message).is_err() {
                // Preserve the read half: an early exit may already have a final
                // status queued, even though the server no longer wants stdin.
                let _ = writer.shutdown(Shutdown::Write);
                break;
            }
            if n == 0 {
                break;
            }
        }
    });
    let cancel_sender = sender.clone();
    let cancel_done = done.clone();
    std::thread::spawn(move || {
        while !cancel_done.load(Ordering::Relaxed) {
            if CANCEL.load(Ordering::Relaxed) {
                match cancel_sender.lock() {
                    Ok(mut writer) => {
                        if wire::send(&mut writer, &json!({"method": "org.flatpakrs.Manager.Cancel", "parameters": {}, "oneway": true})).is_err() { let _ = writer.shutdown(Shutdown::Both); }
                    }
                    Err(_) => break,
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let result = (|| -> Result<i32, Box<dyn std::error::Error>> {
        let mut reader = wire::Reader::new(stream.try_clone()?);
        loop {
            let value = match reader.next() {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                    ) =>
                {
                    continue;
                }
                Ok(Some(value)) => value,
                Ok(None) => return Err("execution service disconnected; instance canceled".into()),
                Err(e) => return Err(e.into()),
            };
            if value.get("error").is_some() {
                return Err(format!("execution rejected: {}", value["parameters"]).into());
            }
            match value["parameters"]["kind"].as_str() {
                Some("started" | "input-ready") => {
                    let (lock, wake) = &*credit;
                    *lock.lock().map_err(|_| "input credit poisoned")? = true;
                    wake.notify_one();
                }
                Some(kind @ ("stdout" | "stderr")) => {
                    let bytes = hex::decode(
                        value["parameters"]["data"]
                            .as_str()
                            .ok_or("invalid output chunk")?,
                    )?;
                    output.write(kind == "stdout", bytes)?;
                }
                Some("exit") => {
                    return Ok(if CANCEL.load(Ordering::Relaxed) {
                        130
                    } else {
                        value["parameters"]["exit_status"]
                            .as_i64()
                            .filter(|n| (0..=255).contains(n))
                            .ok_or("invalid exit status")? as i32
                    });
                }
                _ => return Err("invalid execution reply".into()),
            }
        }
    })();
    done.store(true, Ordering::Relaxed);
    credit.1.notify_all();
    let _ = stream.shutdown(Shutdown::Both);
    if CANCEL.load(Ordering::Relaxed) {
        Ok(130)
    } else {
        result
    }
}
