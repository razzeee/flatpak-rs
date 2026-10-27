"""Independent signed HTTP OSTree fixture; stdout reports readiness as JSON."""
import base64
import functools
import http.server
import json
import pathlib
import subprocess
import sys
import time

root = pathlib.Path(sys.argv[1])
root.mkdir()
home = root / "keys"
home.mkdir(mode=0o700)

def run(*args):
    return subprocess.check_output(args, stderr=subprocess.PIPE).decode().strip()

run("gpg", "--homedir", str(home), "--batch", "--pinentry-mode", "loopback",
    "--passphrase", "", "--quick-generate-key", "Fixture <fixture@example.test>", "ed25519", "sign", "0")
keys = run("gpg", "--homedir", str(home), "--with-colons", "--list-keys")
fingerprint = next(line.split(":")[9] for line in keys.splitlines() if line.startswith("fpr:"))
key = subprocess.check_output(["gpg", "--homedir", str(home), "--export", fingerprint])
repo = root / "repo"
run("ostree", f"--repo={repo}", "init", "--mode=archive", "--collection-id=org.example.Remote")
tree = root / "tree"
(tree / "files").mkdir(parents=True)
(tree / "metadata").write_text("[Application]\nname=org.example.RemoteApp\nruntime=org.example.Platform/x86_64/stable\ncommand=fixture\n")
commit = run("ostree", f"--repo={repo}", "commit", "--branch=app/org.example.RemoteApp/x86_64/stable",
             f"--tree=dir={tree}", "--add-metadata-string=xa.metadata=" + (tree / "metadata").read_text(), f"--gpg-sign={fingerprint}", f"--gpg-homedir={home}")
(tree / "metadata").write_text("[Runtime]\nname=org.example.Platform\n")
runtime_commit = run("ostree", f"--repo={repo}", "commit", "--branch=runtime/org.example.Platform/x86_64/stable",
             f"--tree=dir={tree}", "--add-metadata-string=xa.metadata=" + (tree / "metadata").read_text(), f"--gpg-sign={fingerprint}", f"--gpg-homedir={home}")
(tree / "metadata").write_text("[Application]\nname=org.example.RemoteApp\nruntime=org.example.Platform/x86_64/stable\ncommand=fixture --shell-syntax\n")
run("ostree", f"--repo={repo}", "commit", "--branch=app/org.example.RemoteApp/x86_64/invalid-command",
    f"--tree=dir={tree}", "--add-metadata-string=xa.metadata=" + (tree / "metadata").read_text(),
    f"--gpg-sign={fingerprint}", f"--gpg-homedir={home}")
run("ostree", f"--repo={repo}", "refs", "--create=app/org.example.WrongBinding/x86_64/stable", commit)
run("ostree", f"--repo={repo}", "summary", "--update", f"--gpg-sign={fingerprint}", f"--gpg-homedir={home}")

class Quiet(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path.endswith(".filez") and (root / "pause-content").exists():
            (root / "content-waiting").touch()
            deadline = time.monotonic() + 30
            while (root / "pause-content").exists():
                if time.monotonic() > deadline:
                    self.send_error(503, "test gate timed out")
                    return
                time.sleep(0.01)
        # Test mutations can share a one-second filesystem timestamp. Always
        # serve current bytes instead of returning a stale conditional 304.
        if "If-Modified-Since" in self.headers:
            del self.headers["If-Modified-Since"]
        super().do_GET()

    def log_message(self, *args):
        pass

server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(root)))
url = f"http://127.0.0.1:{server.server_port}/repo"
descriptor = root / "fixture.flatpakrepo"
descriptor.write_text(f"[Flatpak Repo]\nUrl={url}\nGPGKey={base64.b64encode(key).decode()}\n")
run("gpg", "--homedir", str(home), "--batch", "--pinentry-mode", "loopback",
    "--passphrase", "", "--quick-generate-key", "Other <other@example.test>", "ed25519", "sign", "0")
wrong_key = subprocess.check_output(["gpg", "--homedir", str(home), "--export", "other@example.test"])
wrong_descriptor = root / "wrong-key.flatpakrepo"
wrong_descriptor.write_text(f"[Flatpak Repo]\nUrl={url}\nGPGKey={base64.b64encode(wrong_key).decode()}\n")
print(json.dumps({"url": url, "descriptor": str(descriptor), "wrong_descriptor": str(wrong_descriptor), "fingerprint": fingerprint, "commit": commit, "runtime_commit":runtime_commit}), flush=True)
server.serve_forever()
