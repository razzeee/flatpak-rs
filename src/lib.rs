use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod host;
pub mod installation;
pub mod service;

pub const MAX_MESSAGE: usize = 64 * 1024;

pub fn socket_path() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or("XDG_RUNTIME_DIR is missing; run in a systemd user login session")?;
    let runtime = PathBuf::from(runtime);
    if !runtime.is_absolute() {
        return Err("XDG_RUNTIME_DIR must be absolute".into());
    }
    Ok(runtime.join("flatpak-rs/management.sock"))
}

#[derive(Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub available: bool,
    pub detail: String,
}

#[derive(Serialize, Deserialize)]
pub struct HostStatus {
    pub ready: bool,
    pub checks: Vec<Check>,
}
