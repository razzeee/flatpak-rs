# Transactional updates: primary CLI -> real service seam, independent fixtures.
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
