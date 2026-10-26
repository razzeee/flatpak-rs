# Pre-publication partial writes and synchronization errors preserve the old pair.
for kind in ["EIO", "ENOSPC"]:
    for site in ["write-selection", "sync-selection", "sync-payload", "sync-intent", "exchange"]:
        machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_FAIL_KIND=" + kind + " FLATPAK_RS_UPDATE_FAIL_AT=" + site))
        restart_update_service()
        rejected("flatpak-rs apply-update /home/alice/update-plan.json", "UpdateFailed")
        assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed
        assert machine.succeed(alice(update_run + "read")) == "update-data"
        machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_FAIL_KIND FLATPAK_RS_UPDATE_FAIL_AT"))

# Keep committed garbage pending across later update/uninstall operations. Recovery
# must not infer its meaning from a now different/missing published selection.
machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_FAIL_AT=delete-obsolete"))
restart_update_service()
pending = json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/update-plan.json")))
assert pending["outcome"] == "updated" and pending["warnings"][0]["code"] == "cleanup_pending"
machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_TEST_PHASE=recovery FLATPAK_RS_UPDATE_FAIL_COUNT=100"))
restart_update_service()
cleanup_pid = machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip()
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == updated["selection"]
assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/restore-plan.json")))["selection"] == installed
assert json.loads(machine.succeed(alice("flatpak-rs uninstall " + selection)))["removed"]
rejected("flatpak-rs inspect " + selection, "not installed")
# Confirm the host fault actually repeated; public outcomes alone would also pass
# if obsolete content had silently been reclaimed before the competing operations.
count_command = "journalctl --user -b -u flatpak-rs.service _PID=" + cleanup_pid + " --no-pager -o cat | grep -c 'update injected recovery failure at delete-obsolete'"
machine.wait_until_succeeds(alice("test $(" + count_command + ") -ge 3"), timeout=5)
machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_TEST_PHASE FLATPAK_RS_UPDATE_FAIL_AT FLATPAK_RS_UPDATE_FAIL_COUNT"))
restart_update_service()
assert json.loads(machine.succeed(alice(install))) == installed
assert machine.succeed(alice(update_run + "read")) == "update-data"
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection.replace("stable", "beta")))) == beta
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection.replace("org.example.Collection", "org.example.OtherCollection")))) == other
