# Apply edited plans as untrusted input. Assertions remain public inspection/CLI.
def save_input(value):
    machine.succeed(alice("printf %s " + shlex.quote(json.dumps(value)) + " > /home/alice/input-plan.json"))

def bad_input(change, expected):
    value = json.loads(json.dumps(plan))
    change(value)
    save_input(value)
    rejected("flatpak-rs apply-update /home/alice/input-plan.json", expected)
    assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed

bad_input(lambda v: v.update(format_version=2), "format version")
bad_input(lambda v: v.update(extra=True), "unknown field")
bad_input(lambda v: v["desired"]["app"].update(extra=True), "unknown field")
bad_input(lambda v: v["desired"]["app"].update(branch="beta"), "identity")
bad_input(lambda v: v["sources"].update(app="/tmp/../escape"), "dot components")
bad_input(lambda v: v["desired"]["app"].update(commit="0" * 64), "UpdateSourceInvalid")
bad_input(lambda v: v["desired"]["app"].update(commit="A" * 64), "lowercase")
bad_input(lambda v: v["sources"].update(app="/etc/install-fixtures/other"), "collection")
bad_input(lambda v: v["sources"].update(app="/etc/install-fixtures/corrupt-update"), "checksum")
for fixture in ["unbound", "mistyped", "wrong-binding"]:
    binding_pin = machine.succeed("cat /etc/install-fixtures/update-" + fixture + "-commit").strip()
    def invalid_binding(value):
        value["sources"]["app"] = "/etc/install-fixtures/update-bindings"
        value["desired"]["app"]["commit"] = binding_pin
    bad_input(invalid_binding, "UpdateSourceInvalid")
beta_pin = machine.succeed("ostree --repo=/etc/install-fixtures/repo rev-parse app/org.example.Fixture/x86_64/beta").strip()
bad_input(lambda v: v["desired"]["app"].update(commit=beta_pin), "ref binding")
runtime_beta_pin = machine.succeed("ostree --repo=/etc/install-fixtures/repo rev-parse runtime/org.example.Platform/x86_64/beta").strip()
bad_input(lambda v: v["desired"]["runtime"].update(branch="beta", commit=runtime_beta_pin), "incompatible")
hook_pin = machine.succeed("cat /etc/install-fixtures/update-hook-commit").strip()
def hook_input(value):
    value["sources"]["app"] = "/etc/install-fixtures/update-hook"
    value["desired"]["app"]["commit"] = hook_pin
bad_input(hook_input, "unsupported structural")

missing = json.loads(json.dumps(plan))
for identity in [missing["target"], missing["expected"]["app"], missing["desired"]["app"]]:
    identity["branch"] = "missing"
missing["desired"] = missing["expected"]
missing["sources"] = {"app":"/unavailable", "runtime":"/unavailable"}
save_input(missing)
rejected("flatpak-rs apply-update /home/alice/input-plan.json", "UpdateNotInstalled")

# Oversized/duplicate files fail locally without submitting an apply.
machine.succeed(alice("python - <<'PY'\nimport json\nv=json.load(open('/home/alice/update-plan.json'))\ns=json.dumps(v)\nopen('/home/alice/oversized-plan.json','w').write(s+' '*32769)\nopen('/home/alice/duplicate-plan.json','w').write(s.replace('\"format_version\": 1','\"format_version\": 1, \"format_version\": 1'))\nPY"))
rejected("flatpak-rs apply-update /home/alice/oversized-plan.json", "oversized")
rejected("flatpak-rs apply-update /home/alice/duplicate-plan.json", "duplicate")

# Updated permission declarations still grant no access; the full confinement
# fixture executes through the new deployment, not merely the restored baseline.
pair_plan(restore_command.replace("/repo ", "/update-permissions "))
permitted = apply_pair()
assert any("no access granted" in item for item in permitted["diagnostics"])
assert machine.succeed(alice(update_run + "read")) == "update-data"
code, output = machine.execute(alice(update_run + "baseline 2>/home/alice/update-permission-stderr"))
assert code == 17 and output == "confined\n", (code, output)
pair_plan(restore_command)
assert apply_pair() == installed
machine.succeed(alice(update_run + "write update-data"))

idle = json.loads(json.dumps(plan))
idle["desired"] = idle["expected"]
idle["sources"] = {"app":"/unavailable", "runtime":"/unavailable"}
machine.succeed(alice("printf %s " + shlex.quote(json.dumps(idle)) + " > /home/alice/draining-noop-plan.json"))
