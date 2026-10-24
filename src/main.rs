use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let variant = |fields: &[String]| {
        serde_json::json!({
            "collection": fields[0], "id": fields[1], "arch": fields[2], "branch": fields[3]
        })
    };
    let (method, parameters) = match args.first().map(String::as_str) {
        Some("status") if args.len() == 1 => ("GetHostStatus", serde_json::json!({})),
        Some("inspect") if args.len() == 5 => ("Inspect", variant(&args[1..5])),
        Some("install") if args.len() == 9 => ("Install", serde_json::json!({
            "source": args[1], "app": variant(&args[2..6]),
            "runtime": {"collection": args[6], "id": args[7], "arch": args[4], "branch": args[8]}
        })),
        _ => return Err("usage: flatpak-rs status | inspect COLLECTION APP ARCH BRANCH | install SOURCE COLLECTION APP ARCH BRANCH RUNTIME_COLLECTION RUNTIME RUNTIME_BRANCH (experimental; no execution)".into()),
    };
    let path = flatpak_rs::socket_path()?;
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        format!("cannot connect to {}: {e}; install and start flatpak-rs.socket with systemctl --user enable --now flatpak-rs.socket; inspect journalctl --user -u flatpak-rs.service", path.display())
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(if method == "Install" {
        300
    } else {
        15
    })))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    serde_json::to_writer(
        &mut stream,
        &serde_json::json!({
            "method": format!("org.flatpakrs.Manager.{method}"), "parameters": parameters
        }),
    )?;
    stream.write_all(&[0])?;
    let mut bytes = Vec::new();
    for byte in BufReader::new(stream)
        .bytes()
        .take(flatpak_rs::MAX_MESSAGE + 1)
    {
        let byte = byte?;
        if byte == 0 {
            let response: serde_json::Value = serde_json::from_slice(&bytes)?;
            if let Some(error) = response.get("error") {
                return Err(format!(
                    "service rejected {method}: {error}: {}",
                    response["parameters"]
                )
                .into());
            }
            let parameters = &response["parameters"];
            let code = if method == "GetHostStatus" {
                if serde_json::from_value::<flatpak_rs::HostStatus>(parameters.clone())?.ready {
                    0
                } else {
                    2
                }
            } else {
                let _: flatpak_rs::installation::Installed =
                    serde_json::from_value(parameters.clone())?;
                0
            };
            println!("{}", serde_json::to_string_pretty(parameters)?);
            return Ok(code);
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
