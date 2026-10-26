"""Supplement the VM driver with same-owner concurrent real CLI clients."""
import json
import signal
import subprocess
import sys
import time

with open(sys.argv[1]) as stream:
    plan = json.load(stream)
identity = [plan["target"][key] for key in ["collection", "id", "arch", "branch"]]
client = subprocess.Popen(["flatpak-rs", "run", *identity, "--", "/app/bin/probe", "block"],
                          stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
try:
    deadline = time.monotonic() + 8
    while True:
        instances = json.loads(subprocess.check_output(["flatpak-rs", "instances"]))["instances"]
        if plan["target"] in instances:
            break
        assert client.poll() is None and time.monotonic() < deadline
        time.sleep(0.02)
    applied = subprocess.run(["flatpak-rs", "apply-update", sys.argv[1]], capture_output=True, text=True, timeout=8)
    assert applied.returncode == 1 and "UpdateBusy" in applied.stderr, applied
    assert applied.stdout == ""
    runtime = plan["desired"]["runtime"]
    # Planning while the execution lease exists is allowed and reserves nothing.
    planned = subprocess.run(["flatpak-rs", "plan-update", "/etc/install-fixtures/update-app", *identity,
                              runtime["collection"], runtime["id"], runtime["branch"]], capture_output=True, text=True, timeout=8)
    assert planned.returncode == 0 and json.loads(planned.stdout)["target"] == plan["target"], planned
finally:
    if client.poll() is None:
        client.send_signal(signal.SIGINT)
        assert client.wait(timeout=8) == 130
    client.stderr.close()
