# Failure classification through the real CLI, not a mocked transaction boundary.
for site, expected_error, expected_selection in [
    ("sync-retained", "UpdateFailed", installed),
    ("sync-installed-parent", "outcome uncertain", updated["selection"]),
]:
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_FAIL_AT=" + site))
    restart_update_service()
    rejected("flatpak-rs apply-update /home/alice/update-plan.json", expected_error)
    # Inspection also forces unresolved same-process recovery before reading state.
    assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == expected_selection
    machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_FAIL_AT"))
    restart_update_service()
    if expected_selection != installed:
        assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/restore-plan.json")))["selection"] == installed
