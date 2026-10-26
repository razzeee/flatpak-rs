# Transactional updates: primary CLI -> real service seam, independent fixtures.
def restart_update_service():
    # Frequent deliberate restarts must reset the test's start-limit counter,
    # not weaken the production unit's restart protection.
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("systemctl --user start flatpak-rs.socket"))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))

plan_command = install.replace(" install ", " plan-update ").replace("/repo ", "/update-app ")
machine.succeed(alice(plan_command + " > /home/alice/update-plan.json"))
plan = json.loads(machine.succeed(alice("cat /home/alice/update-plan.json")))
assert plan["format_version"] == 1
assert plan["target"] == {k: installed["app"][k] for k in ["collection", "id", "arch", "branch"]}
assert plan["expected"] == {k: installed[k] for k in ["app", "runtime"]}
assert plan["desired"]["app"]["commit"] == machine.succeed("cat /etc/install-fixtures/update-app-commit").strip()
assert plan["desired"]["runtime"] == installed["runtime"]
assert plan["sources"] == {"app":"/etc/install-fixtures/update-app", "runtime":"/etc/install-fixtures/update-app"}
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed
older = json.loads(machine.succeed(alice(plan_command + " --app-commit " + installed["app"]["commit"] + " --runtime-commit " + installed["runtime"]["commit"])))
assert older["desired"] == older["expected"]
standalone_plan = "flatpak-rs plan-update /etc/install-fixtures/repo " + selection + " org.example.Standalone org.example.Platform stable --runtime-source /etc/install-fixtures/standalone"
separate = json.loads(machine.succeed(alice(standalone_plan)))
assert separate["desired"]["runtime"]["collection"] == "org.example.Standalone"
assert separate["sources"]["runtime"] == "/etc/install-fixtures/standalone"
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed

# A saved plan is not a reservation. Another planning request may choose new refs.
machine.succeed(alice("cp -a /etc/install-fixtures/update-app /home/alice/update-source; chmod -R u+w /home/alice/update-source"))
mutable_plan_command = plan_command.replace("/etc/install-fixtures/update-app", "/home/alice/update-source")
first = json.loads(machine.succeed(alice(mutable_plan_command)))
machine.succeed(alice("ostree --repo=/home/alice/update-source refs --create=app/org.example.Fixture/x86_64/stable --force " + installed["app"]["commit"]))
second = json.loads(machine.succeed(alice(mutable_plan_command)))
assert first["desired"]["app"] != second["desired"]["app"]
assert second["desired"] == second["expected"]
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed

update_run = "flatpak-rs run " + selection + " -- /app/bin/probe "
machine.succeed(alice(update_run + "write update-data"))
updated = json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/update-plan.json")))
assert updated["outcome"] == "updated" and updated["warnings"] == []
assert updated["selection"]["app"] == plan["desired"]["app"]
assert updated["selection"]["runtime"] == installed["runtime"]
assert updated["selection"] == json.loads(machine.succeed(alice("flatpak-rs inspect " + selection)))
assert machine.succeed(alice("flatpak-rs run " + selection + " -- /app/bin/fixture")) == "fixture\nchanged-app\n"
assert machine.succeed(alice(update_run + "read")) == "update-data"

# An eligible pinned retry is unchanged before expected/source checks.
machine.succeed(alice("python - <<'PY'\nimport json\np='/home/alice/update-plan.json'\nv=json.load(open(p)); v['sources']={'app':'/unavailable','runtime':'/unavailable'}\njson.dump(v,open('/home/alice/noop-plan.json','w'))\nPY"))
unchanged = json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/noop-plan.json")))
assert unchanged["outcome"] == "unchanged" and unchanged["selection"] == updated["selection"]
machine.succeed(alice("python /etc/update-probe.py /home/alice/noop-plan.json"))

# Return to baseline for the existing, unchanged execution/lifecycle suite.
restore_command = install.replace(" install ", " plan-update ")
machine.succeed(alice(restore_command + " > /home/alice/restore-plan.json"))
restored = json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/restore-plan.json")))
assert restored["outcome"] == "updated" and restored["selection"] == installed

machine.succeed(alice("printf %s " + shlex.quote(json.dumps(first)) + " > /home/alice/moved-plan.json"))
for remove_ref in [False, True]:
    if remove_ref:
        machine.succeed(alice("ostree --repo=/home/alice/update-source refs --delete app/org.example.Fixture/x86_64/stable"))
    moved = json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/moved-plan.json")))
    assert moved["selection"]["app"] == first["desired"]["app"]
    assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/restore-plan.json")))["selection"] == installed

stale = json.loads(json.dumps(plan))
stale["expected"]["app"]["commit"] = "0" * 64
stale["sources"] = {"app":"/unavailable", "runtime":"/unavailable"}
machine.succeed(alice("printf %s " + shlex.quote(json.dumps(stale)) + " > /home/alice/stale-plan.json"))
rejected("flatpak-rs apply-update /home/alice/stale-plan.json", "UpdateConflict")
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed
