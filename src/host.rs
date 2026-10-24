use crate::{Check, HostStatus};
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

fn bounded_output(command: &mut Command) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(result.err().unwrap_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, "host capability probe timed out")
                }));
            }
        }
    }
}

fn check(name: &str, result: Result<String, String>) -> Check {
    match result {
        Ok(detail) => Check {
            name: name.into(),
            available: true,
            detail,
        },
        Err(detail) => Check {
            name: name.into(),
            available: false,
            detail,
        },
    }
}

pub fn status() -> HostStatus {
    let checks = vec![
        check("user_namespaces", user_namespaces()),
        check("systemd_user_manager", user_manager()),
        check(
            "cgroup_control",
            cgroup_control().map_err(|e| {
                format!("{e}; require cgroup v2 and Delegate=memory pids on flatpak-rs.service")
            }),
        ),
    ];
    HostStatus {
        ready: checks.iter().all(|c| c.available),
        checks,
    }
}

fn user_manager() -> Result<String, String> {
    // is-system-running returns nonzero for a degraded but reachable manager.
    let output = bounded_output(
        Command::new("systemctl")
            .args(["--user", "show", "--property=Version", "--value"])
            .env("SYSTEMD_PAGER", "cat"),
    )
    .map_err(|e| {
        format!("systemctl unavailable: {e}; install systemd and check the user session")
    })?;
    if output.status.success() && !output.stdout.is_empty() {
        Ok("systemd user manager reachable".into())
    } else {
        Err("systemd user manager unavailable; log in through a systemd user session".into())
    }
}

fn user_namespaces() -> Result<String, String> {
    // Re-exec rather than fork from a multithreaded service. The child does not
    // touch storage or change the service's namespaces.
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let output = bounded_output(Command::new(executable).arg("--probe-userns"))
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok("unprivileged user namespace creation succeeded (not a sandbox test)".into())
    } else {
        Err(format!(
            "cannot create an unprivileged user namespace: {}; enable kernel/user policy support",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub(crate) fn delegation_root() -> io::Result<std::path::PathBuf> {
    let membership = fs::read_to_string("/proc/self/cgroup")?;
    let relative = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or_else(|| io::Error::other("unified cgroup v2 membership is missing"))?;
    if relative.split('/').any(|part| part == "..") {
        return Err(io::Error::other("unexpected cgroup path"));
    }
    let group = Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/'));
    // DelegateSubgroup=supervisor keeps the service out of the delegation root.
    // The root can then enable controllers without the no-internal-process rule.
    let parent = group
        .parent()
        .ok_or_else(|| io::Error::other("no delegation root"))?;
    if group.file_name().and_then(|n| n.to_str()) != Some("supervisor") {
        return Err(io::Error::other(
            "service needs DelegateSubgroup=supervisor",
        ));
    }
    let controllers = fs::read_to_string(parent.join("cgroup.controllers"))?;
    for required in ["memory", "pids"] {
        if !controllers.split_whitespace().any(|c| c == required) {
            return Err(io::Error::other(format!(
                "{required} controller not delegated"
            )));
        }
    }
    let enabled = fs::read_to_string(parent.join("cgroup.subtree_control"))?;
    if !["memory", "pids"]
        .iter()
        .all(|required| enabled.split_whitespace().any(|c| c == *required))
    {
        fs::write(parent.join("cgroup.subtree_control"), "+memory +pids")?;
    }
    Ok(parent.to_path_buf())
}

fn cgroup_control() -> io::Result<String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = delegation_root()?;
    let probe = parent.join(format!(
        "probe-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&probe)?;
    let result = (|| {
        fs::write(probe.join("memory.max"), "67108864")?;
        fs::write(probe.join("pids.max"), "16")?;
        if fs::read_to_string(probe.join("memory.max"))?.trim() != "67108864"
            || fs::read_to_string(probe.join("pids.max"))?.trim() != "16"
        {
            return Err(io::Error::other("cgroup limit readback mismatch"));
        }
        Ok("disposable cgroup memory/pids limits writable; no sandbox execution attempted".into())
    })();
    let cleanup = fs::remove_dir(probe);
    cleanup?;
    result
}
