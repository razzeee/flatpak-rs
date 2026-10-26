# Kill the real management process on either side of the visibility point.
for boundary in ["prepared", "exchanged"]:
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_STOP_AFTER=" + boundary))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    machine.succeed(alice("flatpak-rs apply-update /home/alice/update-plan.json >/home/alice/interrupted-update.log 2>&1 &"))
    service_pid = machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip()
    machine.wait_until_succeeds("grep -q '^State:.*T' /proc/" + service_pid + "/status")
    machine.succeed(alice("systemctl --user kill --kill-whom=all --signal=SIGKILL flatpak-rs.service"))
    machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_STOP_AFTER"))
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    observed = json.loads(machine.succeed(alice("flatpak-rs inspect " + selection)))
    assert observed == (installed if boundary == "prepared" else updated["selection"])
    assert machine.succeed(alice(update_run + "read")) == "update-data"
    retried = json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/update-plan.json")))
    assert retried["outcome"] == ("updated" if boundary == "prepared" else "unchanged")
    assert retried["selection"] == updated["selection"]
    assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/restore-plan.json")))["selection"] == installed
