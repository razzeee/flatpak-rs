"""Owner death must cancel post-exit draining despite an inherited socket copy."""
import glob
import json
import os
import select
import signal
import socket
import subprocess
import sys
import time

variant = dict(collection="org.example.Collection", id="org.example.Fixture", arch="x86_64", branch="stable")
fields = list(variant.values())


def instances():
    return json.loads(subprocess.check_output(["flatpak-rs", "instances"], timeout=5))["instances"]


def wait_for(predicate, detail):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    raise AssertionError(detail)


def payloads():
    matches = []
    for path in glob.glob("/proc/[0-9]*/cmdline"):
        try:
            with open(path, "rb") as stream:
                if stream.read().split(b"\0")[:2] == [b"/app/bin/probe", b"post-exit-output"]:
                    matches.append(path)
        except OSError:
            pass
    return matches


def receive(connection):
    frame = bytearray()
    while True:
        byte = connection.recv(1)
        assert byte, "unexpected stream EOF"
        if byte == b"\0":
            return json.loads(frame)["parameters"]
        frame.extend(byte)


ready_r, ready_w = os.pipe()
exit_r, exit_w = os.pipe()
owner = os.fork()
if owner == 0:
    os.close(ready_r)
    os.close(exit_w)
    try:
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(5)
            connection.connect(os.environ["XDG_RUNTIME_DIR"] + "/flatpak-rs/management.sock")
            connection.sendall(json.dumps(dict(method="org.flatpakrs.Manager.Run", more=True,
                parameters=dict(variant=variant, command=["/app/bin/probe", "post-exit-output"]))).encode() + b"\0")
            assert receive(connection)["kind"] == "started"
            # Prove payload execution, not merely helper startup, before pausing reads.
            output = receive(connection)
            assert output["kind"] == "stdout" and bytes.fromhex(output["data"]) == b"r" * 4096
            holder = os.fork()
            if holder == 0:
                os.close(ready_w)
                os.close(exit_r)
                time.sleep(30)  # Deliberately retains the socket, never dispatches requests.
                os._exit(0)
            os.write(ready_w, str(holder).encode())
            os.close(ready_w)
            assert os.read(exit_r, 1) == b"x"
        os._exit(0)
    except BaseException:
        os._exit(1)

os.close(ready_w)
os.close(exit_r)
holder = None
reaped = False
try:
    assert select.select([ready_r], [], [], 8)[0], "client did not launch payload"
    holder = int(os.read(ready_r, 64))
    wait_for(lambda: not payloads(), "finite-output payload did not exit")
    assert instances() == [variant], "probe did not establish post-exit output draining"
    # Ownership must still exclude lifecycle operations while the owner is alive.
    checks = [
        (["run", *fields, "--", "/app/bin/probe", "exit"], b"already running"),
        (["delete-data", *fields], b"running"),
        (["uninstall", *fields], b"running"),
    ]
    if len(sys.argv) > 1:
        checks.append((["apply-update", sys.argv[1]], b"UpdateBusy"))
    for args, diagnostic in checks:
        result = subprocess.run(["flatpak-rs", *args], capture_output=True, timeout=5)
        assert result.returncode == 1 and diagnostic in result.stderr, result
    os.write(exit_w, b"x")
    assert os.waitpid(owner, 0)[1] == 0
    reaped = True
    wait_for(lambda: not instances(), "lease retained after connecting owner died during output drain")
    if len(sys.argv) > 1:
        retried = subprocess.run(["flatpak-rs", "apply-update", sys.argv[1]], capture_output=True, timeout=5)
        assert retried.returncode == 0 and json.loads(retried.stdout)["outcome"] == "unchanged", retried
    os.kill(holder, 0)  # Cleanup must not depend on closing the inherited socket.
    assert not payloads()
    retry = subprocess.run(["flatpak-rs", "run", *fields, "--", "/app/bin/probe", "exit"], capture_output=True, timeout=5)
    assert retry.returncode == 23, retry
    deletion = subprocess.run(["flatpak-rs", "delete-data", *fields], capture_output=True, timeout=5)
    assert deletion.returncode == 0, deletion
finally:
    if not reaped:
        os.kill(owner, signal.SIGKILL)
        os.waitpid(owner, 0)
    if holder is not None:
        os.kill(holder, signal.SIGKILL)
    os.close(ready_r)
    os.close(exit_w)
