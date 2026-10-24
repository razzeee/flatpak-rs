fn main() {
    match flatpak_rs::sandbox::run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("required sandbox setup failed: {error}");
            std::process::exit(125);
        }
    }
}
