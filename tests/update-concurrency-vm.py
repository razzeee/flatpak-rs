machine.succeed(alice(restore_command.replace("/repo ", "/update-runtime ") + " > /home/alice/contender-plan.json"))
machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_DELAY_MS=5000"))
restart_update_service()
machine.succeed(alice("python /etc/update-concurrency.py /home/alice/update-plan.json /home/alice/contender-plan.json"))
machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_DELAY_MS"))
restart_update_service()
assert machine.succeed(alice(update_run + "read")) == "update-data"
assert machine.succeed(alice("flatpak-rs run " + selection + " -- /app/bin/fixture")) == "fixture\nchanged-app\n"
assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/restore-plan.json")))["selection"] == installed
