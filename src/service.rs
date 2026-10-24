use crate::MAX_MESSAGE;
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read, Write};
use std::mem::{size_of, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

const INTERFACE: &str = include_str!("../interfaces/org.flatpakrs.Manager.varlink");
const DISCOVERY: &str = include_str!("../interfaces/org.varlink.service.varlink");

fn denied(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, detail)
}

fn authenticate(stream: &UnixStream) -> io::Result<()> {
    // SAFETY: getsockopt writes into correctly sized, initialized storage.
    let mut cred: libc::ucred = unsafe { zeroed() };
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut length,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: getuid has no preconditions.
    if cred.uid != unsafe { libc::getuid() } || cred.pid <= 0 {
        return Err(denied("peer UID does not own this service"));
    }
    let mut fd: libc::c_int = -1;
    length = size_of::<libc::c_int>() as libc::socklen_t;
    // SO_PEERPIDFD was introduced in Linux 6.5; no unsafe PID-only fallback.
    // SAFETY: output and length point to correctly sized writable storage.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            77,
            (&mut fd as *mut libc::c_int).cast(),
            &mut length,
        )
    } != 0
    {
        return Err(io::Error::other(format!(
            "SO_PEERPIDFD requires Linux >= 6.5: {}",
            io::Error::last_os_error()
        )));
    }
    // SAFETY: successful SO_PEERPIDFD returned a new owned file descriptor.
    let pidfd = unsafe { OwnedFd::from_raw_fd(fd) };
    let alive = || -> io::Result<()> {
        let mut pollfd = libc::pollfd {
            fd: pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pollfd points to one initialized entry.
        match unsafe { libc::poll(&mut pollfd, 1, 0) } {
            0 => Ok(()),
            -1 => Err(io::Error::last_os_error()),
            _ => Err(denied("connecting process exited")),
        }
    };
    alive()?;
    let proc = fs::File::open(format!("/proc/{}", cred.pid))?;
    for namespace in ["user", "mnt", "pid", "net"] {
        let peer = fs::metadata(format!("/proc/self/fd/{}/ns/{namespace}", proc.as_raw_fd()))?;
        let own = fs::metadata(format!("/proc/self/ns/{namespace}"))?;
        if (peer.dev(), peer.ino()) != (own.dev(), own.ino()) {
            return Err(denied("peer is not in the host service namespaces"));
        }
    }
    alive()
}

fn error(name: &str, parameters: Value) -> Value {
    json!({"error": format!("org.varlink.service.{name}"), "parameters": parameters})
}

fn dispatch(request: Value) -> Option<Value> {
    let object = request.as_object()?;
    let method = object.get("method")?.as_str()?;
    if object
        .keys()
        .any(|key| !["method", "parameters", "oneway", "more", "upgrade"].contains(&key.as_str()))
    {
        return Some(error("InvalidParameter", json!({"parameter": "request"})));
    }
    for flag in ["oneway", "more", "upgrade"] {
        if object
            .get(flag)
            .is_some_and(|value| !value.is_boolean() || (flag != "oneway" && value == true))
        {
            return Some(error("InvalidParameter", json!({"parameter": flag})));
        }
    }
    let parameters = object.get("parameters").cloned().unwrap_or(json!({}));
    let Some(parameters) = parameters.as_object() else {
        return Some(error(
            "InvalidParameter",
            json!({"parameter": "parameters"}),
        ));
    };
    let response = match method {
        "org.flatpakrs.Manager.GetHostStatus"
        | "org.flatpakrs.Manager.GetRecoveryStatus"
        | "org.flatpakrs.Manager.ListInstances"
        | "org.flatpakrs.Manager.ListRuntimes"
        | "org.varlink.service.GetInfo"
            if !parameters.is_empty() =>
        {
            error(
                "InvalidParameter",
                json!({"parameter": parameters.keys().next().unwrap()}),
            )
        }
        "org.flatpakrs.Manager.GetHostStatus" => json!({"parameters": crate::host::status()}),
        "org.flatpakrs.Manager.ListInstances" => {
            json!({"parameters": {"instances": crate::installation::instances()}})
        }
        "org.flatpakrs.Manager.ListRuntimes" => match crate::installation::list_runtimes() {
            Ok(runtimes) => json!({"parameters": runtimes}),
            Err(e) => {
                let detail: String = e.to_string().chars().take(2048).collect();
                json!({"error": "org.flatpakrs.Manager.OperationFailed", "parameters": {"detail": detail}})
            }
        },
        "org.flatpakrs.Manager.Uninstall" => operation(
            Value::Object(parameters.clone()),
            crate::installation::uninstall,
        ),
        "org.flatpakrs.Manager.DeleteData" => operation(
            Value::Object(parameters.clone()),
            crate::installation::delete_data,
        ),
        "org.flatpakrs.Manager.RemoveRuntime" => operation(
            Value::Object(parameters.clone()),
            crate::installation::remove_runtime,
        ),
        "org.flatpakrs.Manager.GetRecoveryStatus" if parameters.is_empty() => {
            json!({"parameters": {"discarded_stages": crate::installation::recovered_stages()}})
        }
        "org.flatpakrs.Manager.Install" => operation::<crate::installation::Install, _>(
            Value::Object(parameters.clone()),
            crate::installation::install,
        ),
        "org.flatpakrs.Manager.Inspect" => operation::<crate::installation::Variant, _>(
            Value::Object(parameters.clone()),
            crate::installation::inspect,
        ),
        "org.varlink.service.GetInfo" => json!({"parameters": {
            "vendor": "flatpak-rs contributors", "product": "flatpak-rs experimental management service",
            "version": env!("CARGO_PKG_VERSION"), "url": "https://github.com/razzeee/flatpak-rs",
            "interfaces": ["org.varlink.service", "org.flatpakrs.Manager"]
        }}),
        "org.varlink.service.GetInterfaceDescription" => {
            if parameters.len() != 1 || !parameters.get("interface").is_some_and(Value::is_string) {
                error("InvalidParameter", json!({"parameter": "interface"}))
            } else {
                match parameters["interface"].as_str().unwrap() {
                    "org.flatpakrs.Manager" => json!({"parameters": {"description": INTERFACE}}),
                    "org.varlink.service" => json!({"parameters": {"description": DISCOVERY}}),
                    interface => error("InterfaceNotFound", json!({"interface": interface})),
                }
            }
        }
        _ => {
            let (interface, method) = method.rsplit_once('.')?;
            if ["org.flatpakrs.Manager", "org.varlink.service"].contains(&interface) {
                error("MethodNotFound", json!({"method": method}))
            } else {
                error("InterfaceNotFound", json!({"interface": interface}))
            }
        }
    };
    Some(response)
}

fn operation<T: serde::de::DeserializeOwned, R: serde::Serialize>(
    parameters: Value,
    run: impl FnOnce(T) -> Result<R, Box<dyn std::error::Error>>,
) -> Value {
    match serde_json::from_value(parameters) {
        Err(_) => error("InvalidParameter", json!({"parameter": "parameters"})),
        Ok(request) => match run(request) {
            Ok(result) => json!({"parameters": result}),
            Err(e) => {
                let detail: String = e.to_string().chars().take(2048).collect();
                json!({"error": "org.flatpakrs.Manager.OperationFailed", "parameters": {"detail": detail}})
            }
        },
    }
}

fn serve(mut stream: UnixStream) -> io::Result<()> {
    authenticate(&stream)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    // Absolute connection budget prevents a slow byte stream holding a slot forever.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut frame = Vec::new();
    loop {
        if std::time::Instant::now() >= deadline {
            return Ok(());
        }
        let mut byte = [0];
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        if byte[0] == 0 {
            let Ok(request) = serde_json::from_slice::<Value>(&frame) else {
                return Ok(());
            };
            if !request
                .as_object()
                .and_then(|o| o.get("method"))
                .and_then(Value::as_str)
                .is_some_and(|method| {
                    method
                        .rsplit_once('.')
                        .is_some_and(|(interface, name)| !interface.is_empty() && !name.is_empty())
                })
            {
                return Ok(());
            }
            let oneway = request.get("oneway") == Some(&Value::Bool(true));
            if request["method"] == "org.flatpakrs.Manager.Run" {
                if oneway {
                    return Ok(());
                }
                return crate::execution::serve(stream, request);
            }
            if let (false, Some(reply)) = (oneway, dispatch(request)) {
                serde_json::to_writer(&mut stream, &reply)?;
                stream.write_all(&[0])?;
            }
            frame.clear();
        } else {
            if frame.len() == MAX_MESSAGE {
                return Ok(());
            }
            frame.push(byte[0]);
        }
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: getuid has no preconditions.
    if unsafe { libc::getuid() } == 0 {
        return Err("must run as a non-root user".into());
    }
    if std::env::var("LISTEN_PID").ok().as_deref() != Some(&std::process::id().to_string())
        || std::env::var("LISTEN_FDS").ok().as_deref() != Some("1")
    {
        return Err("expected exactly one systemd-activated listener".into());
    }
    // SAFETY: fcntl validates fd 3 before ownership is assumed.
    if unsafe { libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: systemd activation transfers ownership of fd 3 to this process.
    let listener = unsafe { UnixListener::from_raw_fd(3) };
    let path = crate::socket_path()?;
    if listener.local_addr()?.as_pathname() != Some(path.as_path()) {
        return Err("unexpected activation socket address".into());
    }
    let meta = fs::symlink_metadata(&path)?;
    let parent = fs::symlink_metadata(path.parent().unwrap())?;
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    if meta.uid() != uid
        || meta.mode() & 0o777 != 0o600
        || parent.uid() != uid
        || parent.mode() & 0o777 != 0o700
        || !parent.is_dir()
    {
        return Err(
            "management socket requires owner UID, mode 0600, and a private mode-0700 directory"
                .into(),
        );
    }
    let mut accepting = 0;
    let mut len = size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: correctly sized output storage for SO_ACCEPTCONN.
    if unsafe {
        libc::getsockopt(
            listener.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_ACCEPTCONN,
            (&mut accepting as *mut libc::c_int).cast(),
            &mut len,
        )
    } != 0
        || accepting != 1
    {
        return Err("activation descriptor is not a listening socket".into());
    }
    listener.set_nonblocking(false)?;
    if let Err(error) = crate::installation::recover() {
        // Keep management available for actionable errors; storage operations
        // retry recovery under their lock and cannot bypass a recovery failure.
        eprintln!("installation recovery unavailable: {error}");
    }
    let active = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let stream = stream?;
        if active.load(Ordering::Relaxed) >= 32 {
            continue;
        }
        active.fetch_add(1, Ordering::Relaxed);
        let active = Arc::clone(&active);
        if let Err(error) = std::thread::Builder::new().spawn(move || {
            if let Err(error) = serve(stream) {
                eprintln!("client rejected/closed: {error}");
            }
            active.fetch_sub(1, Ordering::Relaxed);
        }) {
            return Err(error.into());
        }
    }
    Ok(())
}
