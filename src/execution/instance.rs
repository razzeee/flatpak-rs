//! Own a confined process tree and its installation lease as one lifetime.
//!
//! Transport receives only pipes. Successful waiting verifies process cleanup
//! but retains ownership until transport draining finishes. Every early return
//! uses the same cleanup path; unverified cleanup must never release the lease.
use super::{Result, Run};
use crate::{installation, sandbox};
use std::fs;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(super) struct Pipes {
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
    pub stderr: ChildStderr,
}

pub(super) struct Instance {
    child: Option<Child>,
    group: Option<Group>,
    lease: Option<installation::Lease>,
}

/// A stopped process tree still owns its variant while transport drains output.
pub(super) struct Completion {
    instance: Instance,
    code: i32,
}

impl Instance {
    /// Validate and claim a variant, enforce limits, then enroll the helper
    /// behind its configuration barrier before allowing any payload execution.
    pub fn start(run: Run, installation: &installation::Installation) -> Result<(Self, Pipes)> {
        if run.command.is_empty() || run.command.iter().any(|arg| arg.contains('\0')) {
            return Err("missing/invalid command".into());
        }
        let memory = run.memory_mib.unwrap_or(256);
        let pids = run.pids.unwrap_or(64);
        if !(16..=1024).contains(&memory) || !(4..=256).contains(&pids) {
            return Err("limits require 16–1024 MiB and 4–256 processes".into());
        }
        let deployment = installation.acquire(run.variant)?;
        let group = Group::new(memory, pids).map_err(|e| format!("required cgroup setup: {e}; require Delegate=memory pids and DelegateSubgroup=supervisor"))?;
        let config = sandbox::Config {
            app: deployment.app,
            runtime: deployment.runtime,
            home: deployment.home,
            root: group.root.clone(),
            cgroup: group.path.clone(),
            command: run.command,
        };
        let mut instance = Self {
            child: None,
            group: Some(group),
            lease: Some(deployment.lease),
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
        instance.child = Some(Child(command.spawn()?));
        drop(config_reader);
        let child = instance.child.as_mut().unwrap();
        let group = instance.group.as_ref().unwrap();
        // The helper cannot execute the payload until this barrier is released.
        // Cancellation must never race a helper joining a previously empty group.
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
        let stdin = child.stdin.take().ok_or("missing input pipe")?;
        // SAFETY: fcntl reads/modifies flags on our owned pipe.
        unsafe {
            let flags = libc::fcntl(stdin.as_raw_fd(), libc::F_GETFL);
            if flags < 0
                || libc::fcntl(stdin.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) == -1
            {
                return Err(io::Error::last_os_error().into());
            }
        }
        let pipes = Pipes {
            stdin,
            stdout: child.stdout.take().ok_or("missing stdout")?,
            stderr: child.stderr.take().ok_or("missing stderr")?,
        };
        Ok((instance, pipes))
    }

    /// Stop the whole tree on cancellation, or wait for normal helper exit.
    /// Success guarantees an empty cgroup, not release of the variant lease.
    pub fn wait(mut self, cancel: &AtomicBool) -> Result<Completion> {
        let child = self.child.as_mut().unwrap();
        let group = self.group.as_ref().unwrap();
        let status = loop {
            if cancel.load(Ordering::Relaxed) {
                group.terminate()?;
                break child.wait()?;
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        group.kill()?;
        let code = status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(9));
        Ok(Completion {
            instance: self,
            code,
        })
    }

    fn cleanup(&mut self) -> io::Result<()> {
        if let Some(group) = &self.group {
            group.kill()?;
        }
        // Also handles a helper that failed before enrollment. Reap the helper
        // and remove its group/root before relinquishing installation ownership.
        drop(self.child.take());
        drop(self.group.take());
        drop(self.lease.take());
        Ok(())
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            // Keep the variant reserved if the complete process tree could not
            // be proved empty, even when the caller exits through an error path.
            if let Some(lease) = self.lease.take() {
                std::mem::forget(lease);
            }
            eprintln!("instance cleanup failed; ownership remains reserved: {error}");
        }
    }
}

impl Completion {
    /// Call after transport draining, before advertising the final exit reply.
    /// The returned status implies cleanup and ownership release have completed.
    pub fn finish(mut self) -> Result<i32> {
        self.instance.cleanup()?;
        Ok(self.code)
    }
}

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
