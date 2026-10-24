//! Single-threaded setup helper. Never called from the management process.
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

mod seccomp;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Serialize, Deserialize)]
pub(crate) struct Config {
    pub app: PathBuf,
    pub runtime: PathBuf,
    pub home: PathBuf,
    pub root: PathBuf,
    pub cgroup: PathBuf,
    pub command: Vec<String>,
}

fn syscall(result: libc::c_int, operation: &str) -> Result<()> {
    if result == -1 {
        Err(format!("{operation}: {}", io::Error::last_os_error()).into())
    } else {
        Ok(())
    }
}

fn mount(
    source: Option<&Path>,
    target: &Path,
    kind: Option<&str>,
    flags: libc::c_ulong,
    data: Option<&str>,
) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let source = source
        .map(|p| CString::new(p.as_os_str().as_bytes()))
        .transpose()?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    let kind = kind.map(CString::new).transpose()?;
    let data = data.map(CString::new).transpose()?;
    // SAFETY: all optional pointers refer to live NUL-terminated strings.
    syscall(
        unsafe {
            libc::mount(
                source.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                target.as_ptr(),
                kind.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                flags,
                data.as_ref()
                    .map_or(std::ptr::null(), |s| s.as_ptr().cast()),
            )
        },
        &format!(
            "mount {} (type {kind:?}, flags {flags:#x})",
            target.to_string_lossy()
        ),
    )
}

fn bind(source: &Path, target: &Path, flags: libc::c_ulong) -> Result<()> {
    mount(Some(source), target, None, libc::MS_BIND, None)?;
    mount(
        None,
        target,
        None,
        libc::MS_REMOUNT | libc::MS_BIND | flags,
        None,
    )
}

fn prctl(option: libc::c_int, value: libc::c_ulong) -> Result<()> {
    // SAFETY: these prctl operations accept scalar arguments only.
    syscall(
        unsafe {
            libc::prctl(
                option,
                value,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            )
        },
        "prctl",
    )
}

fn drop_capabilities() -> Result<()> {
    prctl(libc::PR_SET_SECUREBITS, 0xf)?;
    prctl(
        libc::PR_CAP_AMBIENT,
        libc::PR_CAP_AMBIENT_CLEAR_ALL as libc::c_ulong,
    )?;
    for capability in 0..64 {
        // SAFETY: capability is an integer; EINVAL denotes unsupported bits.
        if unsafe { libc::prctl(libc::PR_CAPBSET_DROP, capability, 0, 0, 0) } == -1
            && io::Error::last_os_error().raw_os_error() != Some(libc::EINVAL)
        {
            return Err(io::Error::last_os_error().into());
        }
    }
    #[repr(C)]
    struct Header {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    struct Data {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    let header = Header {
        version: 0x20080522,
        pid: 0,
    };
    let data = [
        Data {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
        Data {
            effective: 0,
            permitted: 0,
            inheritable: 0,
        },
    ];
    // SAFETY: the kernel capset v3 ABI requires one header and two data entries.
    syscall(
        unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) } as i32,
        "capset",
    )?;
    prctl(libc::PR_SET_NO_NEW_PRIVS, 1)
}

pub fn run() -> Result<i32> {
    let config: Config = {
        // SAFETY: the service deliberately transfers its configuration pipe as fd 3.
        let mut pipe = unsafe { File::from_raw_fd(3) };
        let mut bytes = Vec::new();
        (&mut pipe)
            .take(crate::MAX_MESSAGE as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > crate::MAX_MESSAGE {
            return Err("oversized helper configuration".into());
        }
        serde_json::from_slice(&bytes)?
    };
    if config.command.is_empty() {
        return Err("missing command".into());
    }
    fs::write(config.cgroup.join("cgroup.procs"), "0")?;
    // SAFETY: querying the current IDs has no preconditions.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    // SAFETY: the re-executed helper is single-threaded, with no shared service state.
    syscall(
        unsafe { libc::unshare(libc::CLONE_NEWUSER) },
        "user namespace (enable unprivileged user namespaces)",
    )?;
    fs::write("/proc/self/setgroups", "deny")?;
    fs::write("/proc/self/uid_map", format!("0 {uid} 1\n"))?;
    fs::write("/proc/self/gid_map", format!("0 {gid} 1\n"))?;
    // SAFETY: all namespaces are private; PID namespace takes effect in the child.
    syscall(
        unsafe {
            libc::unshare(
                libc::CLONE_NEWNS
                    | libc::CLONE_NEWPID
                    | libc::CLONE_NEWNET
                    | libc::CLONE_NEWIPC
                    | libc::CLONE_NEWUTS
                    | libc::CLONE_NEWCGROUP,
            )
        },
        "required namespaces",
    )?;
    mount(
        None,
        Path::new("/"),
        None,
        libc::MS_REC | libc::MS_PRIVATE,
        None,
    )?;
    mount(
        None,
        &config.root,
        Some("tmpfs"),
        libc::MS_NOSUID | libc::MS_NODEV,
        Some("size=64m,mode=0755"),
    )?;
    for directory in [
        "app",
        "usr",
        "home/user",
        "tmp",
        "dev",
        "proc",
        "run",
        "etc",
        ".old",
    ] {
        fs::create_dir_all(config.root.join(directory))?;
    }
    bind(
        &config.app,
        &config.root.join("app"),
        libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
    )?;
    bind(
        &config.runtime,
        &config.root.join("usr"),
        libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
    )?;
    bind(
        &config.home,
        &config.root.join("home/user"),
        libc::MS_NOSUID | libc::MS_NODEV,
    )?;
    mount(
        None,
        &config.root.join("tmp"),
        Some("tmpfs"),
        libc::MS_NOSUID | libc::MS_NODEV,
        Some("size=64m,mode=1777"),
    )?;
    mount(
        None,
        &config.root.join("dev"),
        Some("tmpfs"),
        libc::MS_NOSUID | libc::MS_NOEXEC,
        Some("size=1m,mode=0755"),
    )?;
    for device in ["null", "zero", "random", "urandom"] {
        let target = config.root.join("dev").join(device);
        File::create(&target)?;
        bind(
            &Path::new("/dev").join(device),
            &target,
            libc::MS_NOSUID | libc::MS_NOEXEC,
        )?;
    }
    std::os::unix::fs::symlink("/proc/self/fd", config.root.join("dev/fd"))?;
    for (name, target) in [("stdin", "0"), ("stdout", "1"), ("stderr", "2")] {
        std::os::unix::fs::symlink(
            format!("/proc/self/fd/{target}"),
            config.root.join("dev").join(name),
        )?;
    }
    mount(
        None,
        &config.root.join("dev"),
        None,
        libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NOEXEC,
        None,
    )?;
    for (name, target) in [
        ("bin", "usr/bin"),
        ("sbin", "usr/sbin"),
        ("lib", "usr/lib"),
        ("lib64", "usr/lib64"),
    ] {
        std::os::unix::fs::symlink(target, config.root.join(name))?;
    }
    fs::write(
        config.root.join("etc/passwd"),
        "user:x:0:0::/home/user:/usr/bin/sh\n",
    )?;
    fs::write(config.root.join("etc/group"), "user:x:0:\n")?;
    // SAFETY: single-threaded helper; child becomes PID 1 of the new PID namespace.
    let pid = unsafe { libc::fork() };
    if pid == -1 {
        return Err(io::Error::last_os_error().into());
    }
    if pid > 0 {
        let mut status = 0;
        loop {
            // SAFETY: wait for the exact owned child with writable status storage.
            if unsafe { libc::waitpid(pid, &mut status, 0) } == pid {
                break;
            }
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return Err(io::Error::last_os_error().into());
            }
        }
        return Ok(if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            128 + libc::WTERMSIG(status)
        });
    }
    // SAFETY: PID 1 is not the inherited process-group leader. Give it a new
    // session so kill(0, ...) cannot target host/service process groups.
    syscall(unsafe { libc::setsid() }, "private session")?;
    mount(
        None,
        &config.root.join("proc"),
        Some("proc"),
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
        None,
    )?;
    // Mount proc while the original proc is still visible: the kernel's
    // unprivileged procfs visibility checks must pass before detaching it.
    std::env::set_current_dir(&config.root)?;
    let dot = CString::new(".")?;
    let old = CString::new(".old")?;
    // SAFETY: both paths are real directories under the newly mounted root.
    syscall(
        unsafe { libc::syscall(libc::SYS_pivot_root, dot.as_ptr(), old.as_ptr()) } as i32,
        "pivot_root",
    )?;
    std::env::set_current_dir("/")?;
    let old = CString::new("/.old")?;
    // SAFETY: detach the host root before any untrusted code or exec.
    syscall(
        unsafe { libc::umount2(old.as_ptr(), libc::MNT_DETACH) },
        "detach host root",
    )?;
    fs::remove_dir("/.old")?;
    mount(
        None,
        Path::new("/proc"),
        None,
        libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
        None,
    )?;
    mount(
        None,
        Path::new("/"),
        None,
        libc::MS_REMOUNT | libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV,
        None,
    )?;
    std::env::set_current_dir("/home/user")?;
    drop_capabilities()?;
    // SAFETY: close every descriptor except deliberately connected standard streams.
    syscall(
        unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) } as i32,
        "close_range",
    )?;
    seccomp::install().map_err(|error| format!("required seccomp filter: {error}"))?;
    let error = Command::new(&config.command[0])
        .args(&config.command[1..])
        .env_clear()
        .env("HOME", "/home/user")
        .env("USER", "user")
        .env("LOGNAME", "user")
        .env("PATH", "/app/bin:/usr/bin")
        .env("LANG", "C.UTF-8")
        .env("TMPDIR", "/tmp")
        .env("XDG_DATA_HOME", "/home/user/.local/share")
        .env("XDG_CONFIG_HOME", "/home/user/.config")
        .env("XDG_CACHE_HOME", "/home/user/.cache")
        .exec();
    Err(error.into())
}
