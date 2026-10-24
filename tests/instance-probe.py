import glob
import json
import signal
import subprocess
import time

identity = ["org.example.Collection", "org.example.Fixture", "x86_64"]

def command(branch, mode, options=(), extra=()):
    return ["flatpak-rs", "run", *identity, branch, *options, "--", "/app/bin/probe", mode, *extra]

def instances():
    return json.loads(subprocess.check_output(["flatpak-rs", "instances"]))["instances"]

def wait_for(test):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        if test(): return
        time.sleep(0.02)
    raise AssertionError("instance/process state did not converge")

def start(branch, mode="block", extra=()):
    return subprocess.Popen(command(branch, mode, extra=extra), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

first = start("stable")
wait_for(lambda: len(instances()) == 1)
conflict = subprocess.run(command("stable", "exit"), capture_output=True, timeout=5)
assert conflict.returncode == 1 and b"already running" in conflict.stderr
second = start("beta")
wait_for(lambda: len(instances()) == 2)

for mode, options, status in [
    ("pids", ["--pids", "16"], 0),
    ("memory", ["--memory-mib", "32"], 137),
    ("kill-group", [], 0),
]:
    result = subprocess.run(command("permissions", mode, options), capture_output=True, timeout=12)
    assert result.returncode == status, (mode, result.returncode, result.stdout, result.stderr)
    wait_for(lambda: len(instances()) == 2)
    assert first.poll() is None and second.poll() is None

failed = subprocess.run(command("permissions", "exit")[:-2] + ["/app/bin/missing"], capture_output=True, timeout=5)
assert failed.returncode == 125 and b"required sandbox setup failed" in failed.stderr
retry = subprocess.run(command("permissions", "baseline"), capture_output=True, timeout=5)
assert retry.returncode == 17

contenders = [start("recovery-prepared") for _ in range(2)]
wait_for(lambda: sum(p.poll() is not None for p in contenders) == 1)
assert sorted(p.returncode for p in contenders if p.returncode is not None) == [1]
assert len(instances()) == 3
for process in [first, second, *contenders]:
    if process.poll() is None:
        process.send_signal(signal.SIGINT)
        assert process.wait(timeout=5) == 130
    process.stdout.close()
    process.stderr.close()
wait_for(lambda: not instances())

def fixture_processes():
    matches = []
    for path in glob.glob("/proc/[0-9]*/cmdline"):
        try:
            with open(path, "rb") as stream:
                args = stream.read().split(b"\0")
            if args[:3] == [b"/app/bin/probe", b"descendant", b"service-death-token"]: matches.append(path)
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            pass
    return matches

client = start("stable", "descendant", ["service-death-token"])
wait_for(lambda: len(fixture_processes()) >= 2)
subprocess.run(["systemctl", "--user", "kill", "--kill-whom=main", "--signal=SIGKILL", "flatpak-rs.service"], check=True)
client.wait(timeout=8)
wait_for(lambda: not fixture_processes())
subprocess.run(["systemctl", "--user", "reset-failed", "flatpak-rs.service", "flatpak-rs.socket"], check=True)
subprocess.run(["systemctl", "--user", "restart", "flatpak-rs.socket", "flatpak-rs.service"], check=True)
assert not instances()
assert subprocess.run(command("stable", "exit"), capture_output=True, timeout=5).returncode == 23
