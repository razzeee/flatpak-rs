#!/usr/bin/env python3
"""#23 API/filesystem preparation probe, NOT update implementation/acceptance.

Requires Python PyGObject/OSTree, reference flatpak/ostree and Linux renameat2.
Creates an isolated directory below the supplied work root and leaves evidence.
Run on both tmpfs and a disk-backed filesystem; no existing installation is used.
"""

import ctypes
import errno
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import tempfile

import gi

gi.require_version("OSTree", "1.0")
from gi.repository import Gio, GLib, OSTree


def command(*args):
    return subprocess.check_output([str(arg) for arg in args], text=True, timeout=30).strip()


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def open_repo(path):
    repo = OSTree.Repo.new(Gio.File.new_for_path(str(path)))
    assert repo.open(None)
    return repo


def init(path, collection=None):
    command("ostree", f"--repo={path}", "init", "--mode=archive")
    command("ostree", f"--repo={path}", "config", "set", "core.min-free-space-percent", "0")
    if collection:
        command("ostree", f"--repo={path}", "config", "set", "core.collection-id", collection)


if sys.argv[1] == "--exchange-child":
    signal.alarm(30)
    boundary, published, slot = sys.argv[2:]
    if boundary != "before-exchange":
        libc = ctypes.CDLL(None, use_errno=True)
        libc.renameat2.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
        libc.renameat2.restype = ctypes.c_int
        if libc.renameat2(-100, os.fsencode(slot), -100, os.fsencode(published), 2) != 0:
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error))
        if boundary == "after-parent-sync":
            sync_dir(Path(published).parent)
            sync_dir(Path(slot).parent)
    print("ready", flush=True)
    signal.pause()
    raise SystemExit("checkpoint child unexpectedly resumed")


root = Path(tempfile.mkdtemp(prefix="local-update-primitives-", dir=sys.argv[1])).resolve()
collection = "org.example.Collection"
runtime_collection = "org.example.RuntimeCollection"
arch = platform.machine()
app_ref = f"app/org.example.Fixture/{arch}/stable"
runtime_ref = f"runtime/org.example.Platform/{arch}/stable"
source = root / "source"
runtime_source = root / "runtime-source"
app = root / "app"
runtime = root / "runtime"
(app / "files").mkdir(parents=True)
(runtime / "usr").mkdir(parents=True)
(runtime / "files").mkdir()
(app / "metadata").write_text(f"[Application]\nname=org.example.Fixture\nruntime=org.example.Platform/{arch}/stable\n")
(runtime / "metadata").write_text("[Runtime]\nname=org.example.Platform\n")
(app / "files/payload").write_text("old application\n")
(runtime / "usr/payload").write_text("runtime\n")
command("flatpak", "build-finish", app)
command("flatpak", "build-finish", runtime)
init(source, collection)
init(runtime_source, runtime_collection)
assert open_repo(source).get_collection_id() == collection
assert open_repo(runtime_source).get_collection_id() == runtime_collection


def export_app():
    command("flatpak", "build-export", "--disable-sandbox", f"--collection-id={collection}", source, app, "stable")
    return command("ostree", f"--repo={source}", "rev-parse", app_ref)


old = export_app()
(app / "files/payload").write_text("new application\n")
new = export_app()
assert old != new
command("flatpak", "build-export", "--disable-sandbox", "--runtime", f"--collection-id={runtime_collection}", runtime_source, runtime, "stable")
runtime_pin = command("ostree", f"--repo={runtime_source}", "rev-parse", runtime_ref)
results = []
counter = 0


def pull(src, col, ref, pin, expect_error=False):
    global counter
    counter += 1
    destination = root / f"import-{counter}"
    repo = OSTree.Repo.new(Gio.File.new_for_path(str(destination)))
    assert repo.create(OSTree.RepoMode.BARE_USER_ONLY, None)
    config = repo.copy_config()
    config.set_string("core", "min-free-space-percent", "0")
    assert repo.write_config_and_reload(config)
    options = GLib.Variant("a{sv}", {
        "collection-refs": GLib.Variant("a(sss)", [(col, ref, pin)]),
        "flags": GLib.Variant("i", int(OSTree.RepoPullFlags.UNTRUSTED)),
        "depth": GLib.Variant("i", 0),
        "gpg-verify": GLib.Variant("b", False),
        "gpg-verify-summary": GLib.Variant("b", False),
        "disable-static-deltas": GLib.Variant("b", True),
    })
    try:
        assert repo.pull_with_options(src.as_uri(), options, None, None)
        ok, commit, _state = repo.load_commit(pin)
        assert ok
        metadata = commit.get_child_value(0)
        bound_collection = metadata.lookup_value("ostree.collection-binding", GLib.VariantType("s"))
        bound_refs = metadata.lookup_value("ostree.ref-binding", GLib.VariantType("as"))
        if bound_collection is None or bound_collection.unpack() != col:
            raise ValueError("explicit post-import collection-binding check rejected requested collection")
        if bound_refs is None or ref not in bound_refs.unpack():
            raise ValueError("explicit post-import ref-binding check rejected requested ref")
    except (GLib.Error, ValueError) as error:
        if not expect_error:
            raise
        results.append({"ref": ref, "outcome": "rejected", "layer": "libostree" if isinstance(error, GLib.Error) else "explicit-binding-validation", "detail": str(error)})
        return
    assert not expect_error, "invalid selection was imported"
    results.append({"ref": ref, "outcome": "imported", "pin": pin})


# The source ref now names new, while the reviewed old pin remains available.
assert command("ostree", f"--repo={source}", "rev-parse", app_ref) == new
pull(source, collection, app_ref, old)
# Even removing the source ref must not force resolution of a replacement ref.
command("ostree", f"--repo={source}", "refs", "--delete", app_ref)
pull(source, collection, app_ref, old)
pull(runtime_source, runtime_collection, runtime_ref, runtime_pin)
pull(source, collection, app_ref.replace("/stable", "/beta"), old, True)
pull(source, collection, app_ref.replace("app/", "runtime/", 1), old, True)
pull(source, runtime_collection, app_ref, old, True)
pull(source, collection, app_ref, "0" * 64, True)
unbound = command("ostree", f"--repo={source}", "commit", "--no-bindings", "--orphan", "--subject=unbound", f"--tree=dir={app}")
pull(source, collection, app_ref, unbound, True)

# Primitive validation only: exchange populated deployment directories and kill
# a child on either side of the single visibility point. No recovery algorithm
# or production service is implemented by this probe.
observations = []
for boundary in ("before-exchange", "after-exchange", "after-parent-sync"):
    case = root / boundary
    published = case / "published/variant"
    slot = case / "active/transaction/slot"
    for directory, version in ((published, "old"), (slot, "new")):
        directory.mkdir(parents=True)
        for name in ("app", "runtime", "selection"):
            file = directory / name
            with file.open("w") as stream:
                stream.write(version)
                stream.flush()
                os.fsync(stream.fileno())
        sync_dir(directory)
        sync_dir(directory.parent)
    try:
        os.rename(slot, published)
    except OSError as error:
        assert error.errno in (errno.ENOTEMPTY, errno.EEXIST)
    else:
        raise AssertionError("plain rename unexpectedly replaced a populated deployment")
    child = subprocess.Popen([sys.executable, __file__, "--exchange-child", boundary, str(published), str(slot)], stdout=subprocess.PIPE, text=True)
    ready = child.stdout.readline()
    if ready != "ready\n":
        child.wait()
        raise AssertionError("exchange child failed before checkpoint")
    child.kill()
    assert child.wait() == -signal.SIGKILL
    child.stdout.close()
    expected = "old" if boundary == "before-exchange" else "new"
    actual = [(published / name).read_text() for name in ("app", "runtime", "selection")]
    assert actual == [expected] * 3
    assert (slot / "selection").read_text() == ("new" if expected == "old" else "old")
    observations.append({"killed_at": boundary, "published_pair": expected, "all_members_agree": True})

report = {
    "scope": "#23 API and atomic-exchange primitives only; not service acceptance or power-loss testing",
    "work_directory": str(root),
    "kernel": platform.release(),
    "filesystem": command("findmnt", "-n", "-o", "FSTYPE", "-T", root),
    "ostree_version": command("pkg-config", "--modversion", "ostree-1"),
    "flatpak_version": command("flatpak", "--version"),
    "pin_checks": results,
    "exchange_observations": observations,
}
(root / "report.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
