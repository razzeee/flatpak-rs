import array
import json
import os
import socket
import subprocess
import time

variant = dict(collection="org.example.Collection", id="org.example.Fixture", arch="x86_64", branch="stable")

def receive(connection):
    frame = bytearray()
    while True:
        byte = connection.recv(1)
        assert byte, "unexpected EOF"
        if byte == b"\0": return json.loads(frame)
        frame.extend(byte)

def connect():
    connection = socket.socket(socket.AF_UNIX)
    connection.settimeout(5)
    connection.connect(os.environ["XDG_RUNTIME_DIR"] + "/flatpak-rs/management.sock")
    return connection

# Optional Varlink integers accept both omission and explicit null.
with connect() as connection:
    connection.sendall(json.dumps(dict(method="org.flatpakrs.Manager.Run", more=True,
        parameters=dict(variant=variant, command=["/app/bin/probe", "exit"], memory_mib=None, pids=None))).encode() + b"\0")
    while True:
        reply = receive(connection)
        assert "error" not in reply, reply
        if reply["parameters"]["kind"] == "exit":
            assert reply["parameters"]["exit_status"] == 23
            break

with connect() as connection, open("/home/alice/untouched", "rb") as host_file:
    request = dict(method="org.flatpakrs.Manager.Run", more=True,
                   parameters=dict(variant=variant, command=["/app/bin/probe", "baseline"]))
    connection.sendmsg([json.dumps(request).encode() + b"\0"], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", [host_file.fileno()]))])
    stdout = bytearray()
    while True:
        response = receive(connection)
        assert "error" not in response, response
        parameters = response["parameters"]
        if parameters["kind"] == "stdout": stdout.extend(bytes.fromhex(parameters["data"]))
        if parameters["kind"] == "exit":
            assert parameters["exit_status"] == 17 and stdout == b"confined\n"
            break

for parameters in [dict(data="not-hex"), dict(data="00", extra=True)]:
    with connect() as connection:
        connection.sendall(json.dumps(dict(method="org.flatpakrs.Manager.Run", more=True,
            parameters=dict(variant=variant, command=["/app/bin/probe", "block"]))).encode() + b"\0")
        assert receive(connection)["parameters"]["kind"] == "started"
        connection.sendall(json.dumps(dict(method="org.flatpakrs.Manager.WriteInput", oneway=True, parameters=parameters)).encode() + b"\0")
        while True:
            reply = receive(connection)
            if reply.get("parameters", {}).get("kind") == "exit":
                assert reply["parameters"]["exit_status"] != 0
                break

# Zero-argument Varlink calls may omit the parameters object entirely.
with connect() as connection:
    connection.sendall(json.dumps(dict(method="org.flatpakrs.Manager.Run", more=True,
        parameters=dict(variant=variant, command=["/app/bin/probe", "block"]))).encode() + b"\0")
    assert receive(connection)["parameters"]["kind"] == "started"
    connection.sendall(b'{"method":"org.flatpakrs.Manager.CloseInput","oneway":true}\0')
    assert receive(connection)["parameters"]["kind"] == "input-ready"
    connection.sendall(b'{"method":"org.flatpakrs.Manager.Cancel","oneway":true}\0')
    assert receive(connection)["parameters"]["kind"] == "exit"

deadline = time.monotonic() + 5
while json.loads(subprocess.check_output(["flatpak-rs", "instances"]))["instances"]:
    assert time.monotonic() < deadline
    time.sleep(0.02)

# An input connection is not a second management dispatcher. It must not delete
# another variant's data when a caller sends a management method as stream input.
with connect() as connection:
    connection.sendall(json.dumps(dict(method="org.flatpakrs.Manager.Run", more=True,
        parameters=dict(variant=variant, command=["/app/bin/probe", "block"]))).encode() + b"\0")
    assert receive(connection)["parameters"]["kind"] == "started"
    connection.sendall(json.dumps(dict(method="org.flatpakrs.Manager.DeleteData", oneway=True,
        parameters={**variant, "branch": "beta"})).encode() + b"\0")
    while receive(connection).get("parameters", {}).get("kind") != "exit": pass
assert subprocess.check_output(["flatpak-rs", "run", variant["collection"], variant["id"], variant["arch"], "beta", "--", "/app/bin/probe", "read"]) == b"independent-branch"
