# Runtime-only, combined, older and explicit-provider replacements use one pair.
def pair_plan(command):
    machine.succeed(alice(command + " > /home/alice/pair-plan.json"))
    return json.loads(machine.succeed(alice("cat /home/alice/pair-plan.json")))

def apply_pair():
    result = json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/pair-plan.json")))
    assert result["outcome"] == "updated" and result["warnings"] == []
    assert result["selection"] == json.loads(machine.succeed(alice("flatpak-rs inspect " + selection)))
    return result["selection"]

runtime_plan = pair_plan(restore_command.replace("/repo ", "/update-runtime "))
runtime_only = apply_pair()
assert runtime_only["app"] == installed["app"]
assert runtime_only["runtime"]["commit"] == machine.succeed("cat /etc/install-fixtures/update-runtime-commit").strip()
runtime_output = "flatpak-rs run " + selection + " -- /usr/bin/sh -c 'while IFS= read -r value; do echo \"$value\"; done </usr/payload'"
assert machine.succeed(alice(runtime_output)) == "independently prepared runtime payload\nchanged runtime\n"

combined_plan = pair_plan(plan_command + " --runtime-source /etc/install-fixtures/update-runtime")
# Retaining this previously selected runtime is required even when only app changes.
machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_FAIL_AT=sync-retained"))
restart_update_service()
rejected("flatpak-rs apply-update /home/alice/pair-plan.json", "UpdateFailed")
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == runtime_only
machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_FAIL_AT"))
restart_update_service()
combined = apply_pair()
assert combined["app"] == updated["selection"]["app"] and combined["runtime"] == runtime_only["runtime"]
assert machine.succeed(alice("flatpak-rs run " + selection + " -- /app/bin/fixture")) == "fixture\nchanged-app\n"
assert machine.succeed(alice(runtime_output)) == "independently prepared runtime payload\nchanged runtime\n"
runtimes = json.loads(machine.succeed(alice("flatpak-rs runtimes")))["runtimes"]
assert installed["runtime"] in runtimes and combined["runtime"] in runtimes
rejected("flatpak-rs remove-runtime org.example.Collection org.example.Platform x86_64 stable", "depend")

machine.succeed(alice(update_run + "write newer-data"))
pair_plan(plan_command + " --app-commit " + installed["app"]["commit"] + " --runtime-commit " + installed["runtime"]["commit"])
assert apply_pair() == installed
assert machine.succeed(alice(update_run + "read")) == "newer-data"

pair_plan(standalone_plan)
provider_changed = apply_pair()
assert provider_changed["app"] == installed["app"]
assert provider_changed["runtime"]["collection"] == "org.example.Standalone"
pair_plan(restore_command)
assert apply_pair() == installed
assert provider_changed["runtime"] in json.loads(machine.succeed(alice("flatpak-rs runtimes")))["runtimes"]
assert json.loads(machine.succeed(alice("flatpak-rs remove-runtime org.example.Standalone org.example.Platform x86_64 stable")))["removed"]
assert provider_changed["runtime"] not in json.loads(machine.succeed(alice("flatpak-rs runtimes")))["runtimes"]
machine.succeed(alice(update_run + "write update-data"))
