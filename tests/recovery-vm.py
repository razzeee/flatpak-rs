# Crash through the real service at each publication boundary; no layout probes.
machine.succeed(alice("printf recovery-safe > /home/alice/outside/marker"))
for boundary in ["prepared", "published"]:
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_INSTALL_STOP_AFTER=" + boundary))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    command = install.replace("x86_64 stable", "x86_64 recovery-" + boundary)
    machine.succeed(alice(command + " </dev/null >/home/alice/interrupted-install.log 2>&1 &"))
    service_pid = machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip()
    machine.wait_until_succeeds("grep -q '^State:.*T' /proc/" + service_pid + "/status")
    machine.succeed(alice("systemctl --user kill --kill-whom=all --signal=SIGKILL flatpak-rs.service"))
    machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_INSTALL_STOP_AFTER"))
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    recovery = json.loads(machine.succeed(alice("flatpak-rs recovery-status")))
    assert recovery["discarded_stages"] == (1 if boundary == "prepared" else 0), recovery
    query = "flatpak-rs inspect " + selection.replace("stable", "recovery-" + boundary)
    if boundary == "prepared":
        rejected(query, "not installed")
    else:
        assert json.loads(machine.succeed(alice(query)))["runtime"] == installed["runtime"]
    retried = json.loads(machine.succeed(alice(command)))
    assert retried == json.loads(machine.succeed(alice(query)))
    assert installed == json.loads(machine.succeed(alice("flatpak-rs inspect " + selection)))
    assert machine.succeed(alice("cat /home/alice/outside/marker")) == "recovery-safe"
