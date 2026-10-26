# Required synchronization failures must not be called preserved-old failures.
for site in ["exchange-result", "sync-slot-parent", "mark-garbage", "sync-active-parent", "sync-garbage-parent"]:
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_FAIL_AT=" + site))
    restart_update_service()
    rejected("flatpak-rs apply-update /home/alice/update-plan.json", "outcome uncertain")
    assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == updated["selection"]
    machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_FAIL_AT"))
    restart_update_service()
    assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/restore-plan.json")))["selection"] == installed

for site in ["delete-obsolete", "sync-cleanup"]:
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_FAIL_AT=" + site))
    restart_update_service()
    warning = json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/update-plan.json")))
    assert warning["outcome"] == "updated" and warning["selection"] == updated["selection"]
    assert len(warning["warnings"]) == 1 and warning["warnings"][0]["code"] == "cleanup_pending"
    # Repeated recovery may fail cleanup without losing the committed selection.
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_TEST_PHASE=recovery FLATPAK_RS_UPDATE_FAIL_AT=delete-obsolete"))
    for attempt in range(2):
        restart_update_service()
        assert json.loads(machine.succeed(alice("flatpak-rs status")))["ready"]
    machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_TEST_PHASE FLATPAK_RS_UPDATE_FAIL_AT"))
    restart_update_service()
    assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == updated["selection"]
    assert machine.succeed(alice(update_run + "read")) == "update-data"
    assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/restore-plan.json")))["selection"] == installed

# Disconnect before publication and after commit; neither cancels accepted work.
for boundary in ["prepared", "committed"]:
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_STOP_AFTER=" + boundary))
    restart_update_service()
    machine.succeed(alice("flatpak-rs apply-update /home/alice/moved-plan.json >/home/alice/disconnected-update.log 2>&1 & echo $! > /home/alice/update-client.pid"))
    service_pid = machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip()
    machine.wait_until_succeeds("grep -q '^State:.*T' /proc/" + service_pid + "/status")
    machine.succeed(alice("kill -KILL $(cat /home/alice/update-client.pid)"))
    # Prepared checkouts no longer depend on subsequent source availability.
    machine.succeed(alice("mv /home/alice/update-source /home/alice/unavailable-update-source"))
    machine.succeed(alice("systemctl --user kill --kill-whom=main --signal=SIGCONT flatpak-rs.service"))
    assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == updated["selection"]
    retried = json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/moved-plan.json")))
    assert retried["outcome"] == "unchanged" and retried["selection"] == updated["selection"]
    machine.succeed(alice("mv /home/alice/unavailable-update-source /home/alice/update-source"))
    machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_STOP_AFTER"))
    restart_update_service()
    assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/restore-plan.json")))["selection"] == installed
