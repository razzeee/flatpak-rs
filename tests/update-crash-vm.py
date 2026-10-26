# Kill the real management process on either side of the visibility point.
for boundary in ["imported", "prepared", "retained", "intent", "exchanged", "parents-synced", "committed", "cleanup-started", "cleanup-partial"]:
    before_publication = boundary in ["imported", "prepared", "retained", "intent"]
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_STOP_AFTER=" + boundary))
    restart_update_service()
    machine.succeed(alice("flatpak-rs apply-update /home/alice/update-plan.json >/home/alice/interrupted-update.log 2>&1 &"))
    service_pid = machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip()
    machine.wait_until_succeeds("grep -q '^State:.*T' /proc/" + service_pid + "/status")
    machine.succeed(alice("systemctl --user kill --kill-whom=all --signal=SIGKILL flatpak-rs.service"))
    machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_STOP_AFTER"))
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    restart_update_service()
    observed = json.loads(machine.succeed(alice("flatpak-rs inspect " + selection)))
    assert observed == (installed if before_publication else updated["selection"])
    restart_update_service()
    assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == observed
    assert machine.succeed(alice(update_run + "read")) == "update-data"
    retried = json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/update-plan.json")))
    assert retried["outcome"] == ("updated" if before_publication else "unchanged")
    assert retried["selection"] == updated["selection"]
    assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/restore-plan.json")))["selection"] == installed
