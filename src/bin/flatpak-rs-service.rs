fn main() {
    if std::env::args().nth(1).as_deref() == Some("--probe-userns") {
        // SAFETY: unshare takes flags only, in this single-threaded probe child.
        if unsafe { libc::unshare(libc::CLONE_NEWUSER) } != 0 {
            eprintln!("{}", std::io::Error::last_os_error());
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = flatpak_rs::service::run() {
        eprintln!("flatpak-rs-service: {error}");
        std::process::exit(1);
    }
}
