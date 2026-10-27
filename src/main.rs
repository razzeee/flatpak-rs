use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string().map_err(|_| "arguments must be UTF-8"))
        .collect::<Result<_, _>>()?;
    if args.first().map(String::as_str) == Some("run") {
        return flatpak_rs::execution_client::run(&args[1..]);
    }
    let variant = |fields: &[String]| {
        serde_json::json!({
            "collection": fields[0], "id": fields[1], "arch": fields[2], "branch": fields[3]
        })
    };
    let (method, parameters) = match args.first().map(String::as_str) {
        Some("apply-install") if args.len() == 2 => {
            let mut bytes = Vec::new();
            std::fs::File::open(&args[1])?.take(32 * 1024 + 1).read_to_end(&mut bytes)?;
            let plan = flatpak_rs::installation::remote::Plan::from_json(&bytes)?;
            ("ApplyInstall", serde_json::json!({"plan":plan}))
        }
        Some("plan-install") if args.len() == 7 && args[5] == "--runtime-remote" => ("PlanInstall", serde_json::json!({"remote":args[1], "id":args[2], "arch":args[3], "branch":args[4], "runtime_remote":args[6]})),
        Some("remote-add" | "remote-replace") if args.len() == 4 => {
            let mut bytes = Vec::new();
            if args[3].starts_with("https://") {
                let agent: ureq::Agent = ureq::Agent::config_builder().https_only(true).timeout_global(Some(Duration::from_secs(60))).build().into();
                agent.get(&args[3]).call()?.body_mut().as_reader().take(48 * 1024 + 1).read_to_end(&mut bytes)?;
            } else {
                std::fs::File::open(&args[3])?.take(48 * 1024 + 1).read_to_end(&mut bytes)?;
            }
            if bytes.len() > 48 * 1024 { return Err("repository descriptor exceeds 48 KiB".into()); }
            ("AddRemote", serde_json::json!({"name":args[1], "collection":args[2], "descriptor":String::from_utf8(bytes)?, "replace":args[0] == "remote-replace"}))
        }
        Some("remote-disable") if args.len() == 2 => ("DisableRemote", serde_json::json!({"name":args[1]})),
        Some("apply-update") if args.len() == 2 => {
            let mut bytes = Vec::new();
            std::fs::File::open(&args[1])?.take(flatpak_rs::installation::update::MAX_PLAN as u64 + 1).read_to_end(&mut bytes)?;
            let plan = flatpak_rs::installation::update::Plan::from_json(&bytes)?;
            ("ApplyUpdate", serde_json::json!({"plan":plan}))
        }
        Some("plan-update") if args.len() >= 9 => {
            let mut parameters = serde_json::json!({"source":args[1], "app":variant(&args[2..6]),
                "runtime":{"collection":args[6],"id":args[7],"arch":args[4],"branch":args[8]}});
            if !(args.len() - 9).is_multiple_of(2) { return Err("update options require values".into()); }
            for option in args[9..].as_chunks::<2>().0 {
                let key = match option[0].as_str() { "--runtime-source" => "runtime_source", "--app-commit" => "app_commit", "--runtime-commit" => "runtime_commit", _ => return Err("unknown update option".into()) };
                if parameters.get(key).is_some() { return Err("duplicate update option".into()); }
                parameters[key] = option[1].clone().into();
            }
            ("PlanUpdate", parameters)
        }
        Some("status") if args.len() == 1 => ("GetHostStatus", serde_json::json!({})),
        Some("recovery-status") if args.len() == 1 => ("GetRecoveryStatus", serde_json::json!({})),
        Some("instances") if args.len() == 1 => ("ListInstances", serde_json::json!({})),
        Some("runtimes") if args.len() == 1 => ("ListRuntimes", serde_json::json!({})),
        Some("remotes") if args.len() == 1 => ("ListRemotes", serde_json::json!({})),
        Some("uninstall" | "delete-data" | "remove-runtime") if args.len() == 5 => (
            match args[0].as_str() { "uninstall" => "Uninstall", "delete-data" => "DeleteData", _ => "RemoveRuntime" }, variant(&args[1..5])),
        Some("inspect") if args.len() == 5 => ("Inspect", variant(&args[1..5])),
        Some("install") if args.len() == 9 || (args.len() == 11 && args[9] == "--runtime-source") => ("Install", serde_json::json!({
            "source": args[1], "app": variant(&args[2..6]),
            "runtime_source": if args.len() == 11 { Some(&args[10]) } else { None },
            "runtime": {"collection": args[6], "id": args[7], "arch": args[4], "branch": args[8]}
        })),
        _ => return Err("usage: flatpak-rs status | recovery-status | instances | runtimes | remotes | remote-add/remote-replace NAME COLLECTION DESCRIPTOR | remote-disable NAME | plan-install REMOTE APP ARCH BRANCH --runtime-remote REMOTE | apply-install PLAN_FILE | inspect/uninstall/delete-data/remove-runtime COLLECTION ID ARCH BRANCH | install/plan-update SOURCE COLLECTION APP ARCH BRANCH RUNTIME_COLLECTION RUNTIME RUNTIME_BRANCH [--runtime-source PATH] [plan-update: --app-commit COMMIT --runtime-commit COMMIT] | apply-update PLAN_FILE | run COLLECTION APP ARCH BRANCH [--memory-mib N] [--pids N] [--wayland NAME] [-- COMMAND ARG...] (experimental; strict confinement only)".into()),
    };
    let path = flatpak_rs::socket_path()?;
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        format!("cannot connect to {}: {e}; install and start flatpak-rs.socket with systemctl --user enable --now flatpak-rs.socket; inspect journalctl --user -u flatpak-rs.service", path.display())
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(
        if [
            "Install",
            "PlanUpdate",
            "ApplyUpdate",
            "AddRemote",
            "PlanInstall",
            "ApplyInstall",
        ]
        .contains(&method)
        {
            300
        } else {
            15
        },
    )))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    let guidance = || {
        if method == "ApplyInstall" {
            return "remote installation outcome uncertain; inspect the application variant or retry the same saved plan".to_string();
        }
        let t = &parameters["plan"]["target"];
        format!(
            "update outcome uncertain; inspect {} {} {} {} before retrying the same saved plan",
            t["collection"].as_str().unwrap_or("COLLECTION"),
            t["id"].as_str().unwrap_or("ID"),
            t["arch"].as_str().unwrap_or("ARCH"),
            t["branch"].as_str().unwrap_or("BRANCH")
        )
    };
    let mut definitive = false;
    let result = (|| -> Result<i32, Box<dyn std::error::Error>> {
        serde_json::to_writer(
            &mut stream,
            &serde_json::json!({
                "method": format!("org.flatpakrs.Manager.{method}"), "parameters": parameters
            }),
        )?;
        stream.write_all(&[0])?;
        let response = read_reply(stream)?;
        if let Some(error) = response.get("error") {
            let error = error
                .as_str()
                .ok_or("invalid service error acknowledgement")?;
            if !error.contains('.')
                || !error.split('.').all(|part| {
                    !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                })
            {
                return Err("invalid service error name".into());
            }
            if !response["parameters"].is_object() {
                return Err("invalid service error parameters".into());
            }
            definitive = true;
            if error == "org.flatpakrs.Manager.UpdateOutcomeUncertain"
                || error == "org.flatpakrs.Manager.RemoteOutcomeUncertain"
            {
                return Err(format!("{}: {}", guidance(), response["parameters"]).into());
            }
            return Err(format!(
                "service rejected {method}: {error}: {}",
                response["parameters"]
            )
            .into());
        }
        let parameters = if ["PlanUpdate", "PlanInstall"].contains(&method) {
            &response["parameters"]["plan"]
        } else {
            &response["parameters"]
        };
        let code = if method == "GetHostStatus" {
            if serde_json::from_value::<flatpak_rs::HostStatus>(parameters.clone())?.ready {
                0
            } else {
                2
            }
        } else if method == "PlanInstall" {
            let plan: flatpak_rs::installation::remote::Plan =
                serde_json::from_value(parameters.clone())?;
            plan.validate()?;
            0
        } else if method == "PlanUpdate" {
            let plan: flatpak_rs::installation::update::Plan =
                serde_json::from_value(parameters.clone())?;
            plan.validate()?;
            0
        } else if method == "ApplyInstall" {
            let _: flatpak_rs::installation::remote::Applied =
                serde_json::from_value(parameters.clone())?;
            0
        } else if method == "ApplyUpdate" {
            let _: flatpak_rs::installation::update::Applied =
                serde_json::from_value(parameters.clone())?;
            0
        } else if [
            "GetRecoveryStatus",
            "ListInstances",
            "ListRuntimes",
            "ListRemotes",
            "AddRemote",
            "DisableRemote",
            "Uninstall",
            "DeleteData",
            "RemoveRuntime",
        ]
        .contains(&method)
        {
            0
        } else {
            let _: flatpak_rs::installation::Installed =
                serde_json::from_value(parameters.clone())?;
            0
        };
        println!("{}", serde_json::to_string_pretty(parameters)?);
        Ok(code)
    })();
    match result {
        Err(error) if ["ApplyUpdate", "ApplyInstall"].contains(&method) && !definitive => {
            Err(format!("{}: {error}", guidance()).into())
        }
        result => result,
    }
}

/// The same bounded socket-read path is exercised with a short timeout in tests.
fn read_reply(stream: UnixStream) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    for byte in BufReader::new(stream)
        .bytes()
        .take(flatpak_rs::MAX_MESSAGE + 1)
    {
        let byte = byte?;
        if byte == 0 {
            return Ok(serde_json::from_slice(&bytes)?);
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

#[cfg(test)]
mod transport_tests {
    #[test]
    fn stalled_management_reply_exercises_actual_socket_timeout() {
        let (client, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_millis(20)))
            .unwrap();
        let error = super::read_reply(client).unwrap_err();
        let error = error
            .downcast_ref::<std::io::Error>()
            .expect("actual socket read error");
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        ));
    }
}
