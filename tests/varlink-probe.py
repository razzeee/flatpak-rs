"""Direct Varlink acceptance client; runs in the VM, never starts the daemon."""
import json
import socket
import sys

PATH = "/run/user/1000/flatpak-rs/management.sock"


def connect():
    client = socket.socket(socket.AF_UNIX)
    client.settimeout(15)
    client.connect(PATH)
    return client


def receive(client):
    data = bytearray()
    while True:
        byte = client.recv(1)
        if not byte:
            raise EOFError("service closed connection")
        if byte == b"\0":
            return json.loads(data)
        data.extend(byte)


def call(request):
    with connect() as client:
        client.sendall(json.dumps(request).encode() + b"\0")
        return receive(client)


if sys.argv[1] == "denied":
    try:
        with connect() as client:
            client.sendall(b'{"method":"org.varlink.service.GetInfo"}\0')
            assert client.recv(1) == b"", "unauthorized peer received a response"
    except (PermissionError, ConnectionResetError, BrokenPipeError):
        pass
else:
    info = call({"method": "org.varlink.service.GetInfo"})["parameters"]
    assert "org.flatpakrs.Manager" in info["interfaces"]
    description = call({"method": "org.varlink.service.GetInterfaceDescription",
                        "parameters": {"interface": "org.flatpakrs.Manager"}})
    assert "method GetHostStatus()" in description["parameters"]["description"]
    for request, error in [
        ({"method": "org.flatpakrs.Manager.Missing"}, "MethodNotFound"),
        ({"method": "org.example.Missing.Call"}, "InterfaceNotFound"),
        ({"method": "org.flatpakrs.Manager.GetHostStatus", "parameters": {"unexpected": True}}, "InvalidParameter"),
        ({"method": "org.flatpakrs.Manager.GetHostStatus", "parameters": []}, "InvalidParameter"),
        ({"method": "org.flatpakrs.Manager.GetHostStatus", "more": True}, "InvalidParameter"),
        ({"method": "org.flatpakrs.Manager.GetHostStatus", "upgrade": True}, "InvalidParameter"),
        ({"method": "org.flatpakrs.Manager.Install", "parameters": {}}, "InvalidParameter"),
        ({"method": "org.flatpakrs.Manager.Install", "parameters": {"source": 1, "app": {}, "runtime": {}}}, "InvalidParameter"),
        ({"method": "org.flatpakrs.Manager.Inspect", "parameters": {"collection": "org.example.Collection", "id": "org.example.Fixture", "arch": "x86_64", "branch": "stable", "unexpected": True}}, "InvalidParameter"),
    ]:
        assert call(request)["error"] == "org.varlink.service." + error
    for payload in [b"not JSON\0", b"{\0", b"x" * 65537, b"[1,2,3]\0"]:
        with connect() as client:
            client.sendall(payload)
            client.shutdown(socket.SHUT_WR)
            try:
                assert client.recv(1) == b""
            except ConnectionResetError:
                pass
    # Fragmentation and multiple frames on a single connection.
    with connect() as client:
        client.sendall(b'{"method":"org.varlink.')
        client.sendall(b'service.GetInfo"}\0{"method":"org.varlink.service.GetInfo"}\0')
        assert receive(client)["parameters"]["product"] == info["product"]
        assert receive(client)["parameters"]["product"] == info["product"]
