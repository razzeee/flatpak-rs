import json
import signal
import subprocess
import time

selection = ["org.example.Collection", "org.example.Fixture", "x86_64", "stable"]
beta = [*selection[:3], "beta"]
runtime = ["org.example.Collection", "org.example.Platform", "x86_64", "stable"]

def cli(*args, success=True):
    result = subprocess.run(["flatpak-rs", *args], capture_output=True, text=True, timeout=12)
    assert (result.returncode == 0) == success, (args, result.returncode, result.stdout, result.stderr)
    return json.loads(result.stdout) if success else result.stderr

def app(variant, mode, *args):
    result = subprocess.run(["flatpak-rs", "run", *variant, "--", "/app/bin/probe", mode, *args], capture_output=True, text=True, timeout=8)
    assert result.returncode == 0, (variant, mode, result.returncode, result.stdout, result.stderr)
    return result.stdout

original = cli("inspect", *selection)
app(selection, "write", "retained")
app(beta, "write", "independent-branch")
other = ["org.example.OtherCollection", *selection[1:]]
app(other, "write", "independent-collection")
assert app(selection, "read") == "retained"
assert app(beta, "read") == "independent-branch"
assert app(other, "read") == "independent-collection"

active = subprocess.Popen(["flatpak-rs", "run", *selection, "--", "/app/bin/probe", "block"], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
deadline = time.monotonic() + 5
while not cli("instances")["instances"]:
    assert time.monotonic() < deadline
    time.sleep(0.02)
assert "running" in cli("uninstall", *selection, success=False)
assert "running" in cli("delete-data", *selection, success=False)
assert "depend" in cli("remove-runtime", *runtime, success=False)
active.send_signal(signal.SIGINT)
assert active.wait(timeout=5) == 130

assert cli("uninstall", *selection)["removed"]
assert "not installed" in cli("inspect", *selection, success=False)
assert "not installed" in cli("run", *selection, "--", "/app/bin/probe", "read", success=False)
assert app(beta, "read") == "independent-branch"
cli("install", "/etc/install-fixtures/repo", *selection, "org.example.Collection", "org.example.Platform", "stable")
assert app(selection, "read") == "retained"
assert cli("inspect", *selection) == original
app(selection, "symlink-data")
assert cli("delete-data", *selection)["removed"]
assert not cli("delete-data", *selection)["removed"]
assert app(selection, "read") == "missing\n"
assert app(beta, "read") == "independent-branch"
assert app(other, "read") == "independent-collection"
with open("/home/alice/untouched") as stream: assert stream.read() == "untouched"
assert "invalid identity" in cli("delete-data", *selection[:3], "../escape", success=False)

# A differently collected app explicitly depends on this provider, and no other
# installation uses it. Removing the app retains its runtime until requested.
shared = [*other[:3], "shared-runtime"]
provider = ["org.example.Standalone", "org.example.Platform", "x86_64", "stable"]
cli("install", "/etc/install-fixtures/other", *shared, "org.example.Standalone", "org.example.Platform", "stable", "--runtime-source", "/etc/install-fixtures/standalone")
assert cli("inspect", *shared)["runtime"]["collection"] == provider[0]
assert "depend" in cli("remove-runtime", *provider, success=False)
cli("uninstall", *shared)
assert any(r["collection"] == provider[0] for r in cli("runtimes")["runtimes"])
assert cli("remove-runtime", *provider)["removed"]
assert not any(r["collection"] == provider[0] for r in cli("runtimes")["runtimes"])
assert "depend" in cli("remove-runtime", *runtime, success=False)
assert app(selection, "read") == "missing\n"
