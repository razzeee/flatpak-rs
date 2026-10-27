"""Adversarial signed-repository tests using the installed CLI and real service.

Run as the owning VM user. Repository mutations are independent publisher inputs;
the optional transport proxy only drops/delays replies from the real service.
"""
import copy
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import threading
import time

root = Path('/home/alice/remote-fixture')
info = json.loads(Path('/home/alice/remote-info.json').read_text())
base = json.loads(Path('/home/alice/remote-plan.json').read_text())
identity = ['org.example.Remote', 'org.example.RemoteApp', 'x86_64', 'stable']
keeper = identity[:-1] + ['keeper']
serial = 0

def cli(*args, ok=True, **kwargs):
    result = subprocess.run(['flatpak-rs', *args], capture_output=True, text=True, timeout=330, **kwargs)
    assert (result.returncode == 0) == ok, (args, result)
    return result.stdout if ok else result.stderr

def saved(plan):
    global serial
    serial += 1
    path = Path(f'/home/alice/remote-case-{serial}.json')
    path.write_text(json.dumps(plan))
    return str(path)

def fresh(name):
    remote = json.loads(cli('remote-add', name, identity[0], info['descriptor']))
    plan = copy.deepcopy(base)
    for key in ['app_provider', 'runtime_provider']:
        plan[key] = dict(name=name, generation=remote['generation'])
    return plan

def apply(plan, **kwargs):
    return cli('apply-install', saved(plan), **kwargs)

def absent():
    assert 'not installed' in cli('inspect', *identity, ok=False)
    assert 'not installed' in cli('run', *identity, ok=False)

def intact():
    assert json.loads(cli('inspect', *keeper)) == keep_selection
    assert cli('run', *keeper, '--', '/app/bin/fixture', 'read') == 'keep-data'
    assert cli('run', *keeper) == 'remote-payload\nverified-runtime\n'
    assert 'depend' in cli('remove-runtime', identity[0], 'org.example.Platform', 'x86_64', 'stable', ok=False)

def rejected(plan, diagnostic=None):
    error = apply(plan, ok=False)
    if diagnostic:
        assert diagnostic.lower() in error.lower(), error
    absent()
    intact()
    return error

def installed(plan):
    result = json.loads(apply(plan))
    assert result['installed']['app'] == plan['app']
    assert result['installed']['runtime'] == plan['runtime']
    assert cli('run', *identity) == 'remote-payload\nverified-runtime\n'
    return result

def wait_for(predicate):
    deadline = time.monotonic() + 20
    while not predicate():
        assert time.monotonic() < deadline, 'synchronization gate timed out'
        time.sleep(.02)

def gate():
    (root / 'content-waiting').unlink(missing_ok=True)
    (root / 'pause-content').touch()

def release():
    (root / 'pause-content').unlink(missing_ok=True)

def pending(plan):
    return subprocess.Popen(['flatpak-rs', 'apply-install', saved(plan)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

def finish(client, success):
    out, err = client.communicate(timeout=40)
    assert (client.returncode == 0) == success, (out, err)
    return json.loads(out) if success else err

cli('remote-replace', 'fixture', identity[0], info['descriptor'])
keep_plan = json.loads(cli('plan-install', 'fixture', identity[1], 'x86_64', 'keeper', '--runtime-remote', 'fixture'))
keep_selection = json.loads(apply(keep_plan))['installed']
cli('run', *keeper, '--', '/app/bin/fixture', 'write', 'keep-data')
cli('uninstall', *identity)

# Saved plans are untrusted: validly shaped edits cannot replace identity,
# runtime requirements, key authorization, or structural validation.
valid = fresh('edited')
for field, value in [('commit', '0' * 64), ('collection', 'org.example.Other'),
                     ('id', 'org.example.WrongBinding'), ('branch', '../escape')]:
    edited = copy.deepcopy(valid)
    edited['app'][field] = value
    rejected(edited)
edited = copy.deepcopy(valid)
edited['runtime']['commit'] = base['app']['commit']
rejected(edited, 'binding')
edited = copy.deepcopy(valid)
edited['app_provider']['generation'] = '0' * 32
rejected(edited, 'authorization changed')
edited = copy.deepcopy(valid)
edited['runtime']['id'] = 'org.example.OtherPlatform'
edited['runtime']['commit'] = info['other_runtime_commit']
rejected(edited, 'runtime')
edited = copy.deepcopy(valid)
edited['format_version'] = 99
rejected(edited)
edited = copy.deepcopy(valid)
edited['app']['branch'] = 'structural'
edited['app']['commit'] = info['structural_commit']
assert 'structural requirement' in apply(edited, ok=False)
assert 'not installed' in cli('inspect', *identity[:-1], 'structural', ok=False)
intact()

# Valid summary authorization cannot bless an unsigned or tampered commit.
for suffix, damage in [('commitmeta', None), ('commitmeta', b'bad signature'), ('commit', b'bad metadata')]:
    plan = fresh('signed-object-' + str(serial))
    object_path = root / 'repo/objects' / base['app']['commit'][:2] / (base['app']['commit'][2:] + '.' + suffix)
    original_object = object_path.read_bytes()
    try:
        object_path.unlink()
        if damage is not None:
            object_path.write_bytes(damage)
        rejected(plan)
    finally:
        object_path.unlink(missing_ok=True)
        object_path.write_bytes(original_object)

# Independently alter a signed payload object, then restore its exact bytes.
# A fresh provider cache for every scenario ensures the network is exercised.
payload = max((root / 'repo/objects').glob('*/*.filez'), key=lambda p: p.stat().st_size)
original = payload.read_bytes()
plan = fresh('corrupt')
try:
    payload.write_bytes(b'corrupted content')
    rejected(plan)
finally:
    payload.write_bytes(original)
assert installed(plan)['status'] == 'installed'
cli('uninstall', *identity)

plan = fresh('missing-pin')
commit_file = root / 'repo/objects' / base['app']['commit'][:2] / (base['app']['commit'][2:] + '.commit')
backup = commit_file.read_bytes()
try:
    commit_file.unlink()
    rejected(plan, '404')
finally:
    commit_file.write_bytes(backup)
installed(plan)
cli('uninstall', *identity)

plan = fresh('interrupted')
try:
    (root / 'interrupt-content').touch()
    rejected(plan)
    assert (root / 'content-interrupted').exists()
finally:
    (root / 'interrupt-content').unlink()
installed(plan)
cli('uninstall', *identity)

# Removing the ref does not remove the reviewed pin; apply does not resolve it.
reference = 'app/org.example.RemoteApp/x86_64/stable'
def ostree(*args):
    subprocess.run(['ostree', '--repo=' + str(root / 'repo'), *args], check=True, capture_output=True)
def summary():
    ostree('summary', '--update', '--gpg-sign=' + info['fingerprint'], '--gpg-homedir=' + str(root / 'keys'))
plan = fresh('missing-ref')
ostree('refs', '--delete', reference)
summary()
assert 'not' in cli('plan-install', 'missing-ref', identity[1], 'x86_64', 'stable', '--runtime-remote', 'missing-ref', ok=False).lower()
installed(plan)
cli('uninstall', *identity)
ostree('refs', '--create=' + reference, info['newer_commit'])
summary()
new_plan = json.loads(cli('plan-install', 'missing-ref', identity[1], 'x86_64', 'stable', '--runtime-remote', 'missing-ref'))
assert new_plan['app']['commit'] == info['newer_commit'] != plan['app']['commit']
installed(plan)
assert 'different selection' in apply(new_plan, ok=False)
assert json.loads(cli('inspect', *identity))['app'] == plan['app']
cli('uninstall', *identity)

# Actual simultaneous CLI submissions: a blocked first transfer holds installation
# ownership; the contender must return unchanged or conflict after publication.
for different in [False, True]:
    plan = fresh('concurrent-' + str(different).lower())
    contender = copy.deepcopy(plan)
    if different:
        contender['app']['commit'] = info['newer_commit']
    gate()
    first = pending(plan)
    try:
        wait_for(lambda: (root / 'content-waiting').exists())
        second = pending(contender)
        release()
        assert finish(first, True)['status'] == 'installed'
        outcome = finish(second, not different)
        if different:
            assert 'different selection' in outcome
        else:
            assert outcome['status'] == 'unchanged'
        assert json.loads(cli('inspect', *identity))['app'] == plan['app']
        intact()
    finally:
        release()
    cli('uninstall', *identity)

# Runtime authorization can change while application content is in flight.
# Retry must continue rejecting the saved authorization after the transfer ends.
for action in ['disable', 'replace']:
    plan = fresh('withdraw-app-' + action)
    provider = fresh('withdraw-runtime-' + action)['runtime_provider']
    plan['runtime_provider'] = provider
    gate()
    client = pending(plan)
    try:
        wait_for(lambda: (root / 'content-waiting').exists())
        if action == 'disable':
            cli('remote-disable', provider['name'])
        else:
            cli('remote-replace', provider['name'], identity[0], info['descriptor'])
        release()
        assert 'authorization' in finish(client, False)
        rejected(plan, 'authorization')
    finally:
        release()

# Drop a genuine accepted-work reply, then exercise the production CLI's actual
# 300-second read timeout. Only transport is intercepted: the real service
# verifies, downloads and publishes, and public inspection observes the result.
for mode in ['drop', 'timeout']:
    plan = fresh('reply-' + mode)
    runtime = Path('/home/alice/proxy-' + mode)
    (runtime / 'flatpak-rs').mkdir(parents=True, mode=0o700)
    runtime.chmod(0o700)
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(str(runtime / 'flatpak-rs/management.sock'))
    listener.listen(1)
    errors = []
    published = threading.Event()
    def proxy():
        try:
            with listener, listener.accept()[0] as downstream, socket.socket(socket.AF_UNIX) as upstream:
                upstream.settimeout(40)
                upstream.connect(os.environ['XDG_RUNTIME_DIR'] + '/flatpak-rs/management.sock')
                request = bytearray()
                while not request.endswith(b'\0'):
                    part = downstream.recv(65536)
                    assert part
                    request.extend(part)
                upstream.sendall(request)
                response = bytearray()
                while not response.endswith(b'\0'):
                    part = upstream.recv(65536)
                    assert part
                    response.extend(part)
                assert json.loads(response[:-1])['parameters']['status'] == 'installed'
                published.set()
                if mode == 'timeout':
                    # Long kernel socket timeouts can be rounded by the timer
                    # wheel. Leave headroom beyond the configured 300 seconds.
                    downstream.settimeout(390)
                    assert downstream.recv(1) == b''
        except BaseException as error:
            errors.append(error)
            published.set()
    thread = threading.Thread(target=proxy)
    thread.start()
    started = time.monotonic()
    client = subprocess.Popen(['flatpak-rs', 'apply-install', saved(plan)],
        env=dict(os.environ, XDG_RUNTIME_DIR=str(runtime)), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    assert published.wait(45)
    assert not errors, errors
    assert json.loads(cli('inspect', *identity))['app'] == plan['app']
    out, err = client.communicate(timeout=400)
    thread.join(timeout=10)
    assert not thread.is_alive() and not errors, errors
    assert client.returncode == 1 and not out and 'outcome uncertain' in err and 'same saved plan' in err, (out, err)
    if mode == 'timeout':
        assert time.monotonic() - started >= 290
        assert 'temporarily unavailable' in err.lower() or 'timed out' in err.lower(), err
        print('Production CLI socket timeout after', round(time.monotonic() - started, 2), 'seconds:', err.strip(), flush=True)
    assert installed(plan)['status'] == 'unchanged'
    intact()
    cli('uninstall', *identity)

print('REMOTE ADVERSARIAL, CONCURRENCY AND TRANSPORT ACCEPTANCE PASSED', flush=True)
