"""Runs as the host user in the VM; only CLI and host process observations."""
import json
import fcntl
import os
import signal
import socket
import selectors
import subprocess
import threading
import time

variant = ["org.example.Collection", "org.example.Fixture", "x86_64", "stable"]
base = ["flatpak-rs", "run", *variant]

def command(mode, options=()):
    return [*base, *options, "--", "/app/bin/probe", mode]

def instances():
    return json.loads(subprocess.check_output(["flatpak-rs", "instances"]))["instances"]

def wait_for(predicate):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.02)
    raise AssertionError("public instance state did not converge")

payload = bytes(range(256)) * 8192
for data in [b"", payload]:
    result = subprocess.run(command("echo"), input=data, capture_output=True, timeout=20)
    assert result.returncode == 0, (result.returncode, result.stderr[:1000])
    assert result.stdout == data and result.stderr == data

result = subprocess.run(command("exit"), input=payload, capture_output=True, timeout=10)
assert result.returncode == 23, (result.returncode, result.stderr[:1000])

# Healthy consumers may pause longer than the transport's retry interval.
# Resume without losing a partial JSON frame or any output bytes.
client = subprocess.Popen(command("burst"), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
time.sleep(2)
stdout, stderr = client.communicate(timeout=15)
assert client.returncode == 0, (client.returncode, stderr[:1000])
assert stdout == b"b" * (4096 * 512) and stderr == stdout

# A missing stdin is EOF, never a reused management descriptor.
result = subprocess.run(command("echo"), preexec_fn=lambda: os.close(0), capture_output=True, timeout=5)
assert result.returncode == 0 and result.stdout == result.stderr == b"", result

# Cancellation-aware output must not mutate the parent's descriptor flags.
read_fd, write_fd = os.pipe()
flags = fcntl.fcntl(write_fd, fcntl.F_GETFL)
try:
    result = subprocess.run(command("exit"), stdin=subprocess.DEVNULL, stdout=write_fd, stderr=subprocess.PIPE, timeout=5)
    assert result.returncode == 23
    assert fcntl.fcntl(write_fd, fcntl.F_GETFL) == flags
finally:
    os.close(read_fd)
    os.close(write_fd)

for mode in ["block", "descendant"]:
    client = subprocess.Popen(command(mode), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    wait_for(lambda: len(instances()) == 1)
    def flood():
        try:
            client.stdin.write(payload)
            client.stdin.flush()
        except (BrokenPipeError, ValueError):
            pass
    writer = threading.Thread(target=flood, daemon=True)
    writer.start()
    time.sleep(0.1)
    client.send_signal(signal.SIGINT)
    status = client.wait(timeout=5)
    assert status == 130, (mode, status, client.stdout.read(), client.stderr.read())
    writer.join(timeout=2)
    wait_for(lambda: not instances())
    client.stdin.close()
    client.stdout.close()
    client.stderr.close()

# A killed execution client must not leave its app or descendants behind.
client = subprocess.Popen(command("descendant"), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
wait_for(lambda: len(instances()) == 1)
client.kill()
client.wait(timeout=5)
client.stdin.close()
wait_for(lambda: not instances())

# Cancellation must remain responsive while the host output consumer is stalled.
for closed in [False, True]:
    client = subprocess.Popen(command("output"), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    with selectors.DefaultSelector() as selector:
        selector.register(client.stdout, selectors.EVENT_READ)
        assert selector.select(timeout=5), "fixture did not produce output"
        assert client.stdout.read(1) == b"x", client.stderr.read()
    time.sleep(0.1)
    try:
        if closed:
            client.stdout.close()
            client.stderr.close()
        else:
            client.send_signal(signal.SIGINT)
        status = client.wait(timeout=5)
        assert (status != 0) if closed else (status == 130), (closed, status)
    finally:
        if client.poll() is None:
            client.kill()
            client.wait(timeout=5)
        client.stdout.close()
        client.stderr.close()
    wait_for(lambda: not instances())

# Socket loss must cancel even when the connecting process remains alive.
with socket.socket(socket.AF_UNIX) as connection:
    connection.settimeout(5)
    connection.connect(os.environ["XDG_RUNTIME_DIR"] + "/flatpak-rs/management.sock")
    connection.sendall(json.dumps({"method": "org.flatpakrs.Manager.Run", "more": True,
        "parameters": {"variant": dict(zip(["collection", "id", "arch", "branch"], variant)),
        "command": ["/app/bin/probe", "descendant"]}}).encode() + b"\0")
    frame = bytearray()
    while True:
        byte = connection.recv(1)
        assert byte
        if byte == b"\0":
            assert json.loads(frame)["parameters"]["kind"] == "started"
            break
        frame.extend(byte)
wait_for(lambda: not instances())
