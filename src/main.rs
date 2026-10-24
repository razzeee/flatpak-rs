use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["status"] {
        return Err(
            "usage: flatpak-rs status (experimental host prerequisites, not sandbox validation)"
                .into(),
        );
    }
    let path = flatpak_rs::socket_path()?;
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        format!("cannot connect to {}: {e}; install and start flatpak-rs.socket with systemctl --user enable --now flatpak-rs.socket; inspect journalctl --user -u flatpak-rs.service", path.display())
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    stream
        .write_all(b"{\"method\":\"org.flatpakrs.Manager.GetHostStatus\",\"parameters\":{}}\0")?;
    let mut bytes = Vec::new();
    for byte in BufReader::new(stream)
        .bytes()
        .take(flatpak_rs::MAX_MESSAGE + 1)
    {
        let byte = byte?;
        if byte == 0 {
            let response: serde_json::Value = serde_json::from_slice(&bytes)?;
            if let Some(error) = response.get("error") {
                return Err(format!("service rejected status request: {error}").into());
            }
            let status: flatpak_rs::HostStatus =
                serde_json::from_value(response["parameters"].clone())?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            return Ok(if status.ready { 0 } else { 2 });
        }
        bytes.push(byte);
    }
    Err("service returned a truncated or oversized Varlink reply; inspect journalctl --user -u flatpak-rs.service".into())
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("flatpak-rs: {error}");
            1
        }
    };
    std::process::exit(code);
}
