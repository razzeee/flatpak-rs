"""Real concurrent management clients; the daemon's other threads stay runnable."""
import json
import subprocess
import sys
import time

with open(sys.argv[1]) as stream:
    plan = json.load(stream)
fields = [plan["target"][key] for key in ["collection", "id", "arch", "branch"]]
unused = [*fields[:3], "unpublished"]
subprocess.run(["flatpak-rs", "run", *unused, "--", "/app/bin/probe", "write", "disposable"], check=True, timeout=8)
pid = subprocess.check_output(["systemctl", "--user", "show", "flatpak-rs.service", "-p", "MainPID", "--value"], text=True).strip()
primary = subprocess.Popen(["flatpak-rs", "apply-update", sys.argv[1]], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
peers = []
try:
    deadline = time.monotonic() + 8
    while True:
        log = subprocess.check_output(["journalctl", "--user", "-b", "-u", "flatpak-rs.service", "_PID=" + pid, "--no-pager", "-o", "cat"], text=True)
        if "update delay apply prepared" in log:
            break
        assert primary.poll() is None and time.monotonic() < deadline, (primary.poll(), log)
        time.sleep(0.02)
    commands = [
        ["apply-update", sys.argv[2]],
        ["inspect", *fields],
        ["run", *fields[:3], "beta", "--", "/app/bin/fixture"],
        ["uninstall", *unused],
        ["delete-data", *unused],
        ["remove-runtime", "org.example.Collection", "org.example.Platform", "x86_64", "stable"],
        ["plan-update", plan["sources"]["app"], *fields,
         plan["desired"]["runtime"]["collection"], plan["desired"]["runtime"]["id"], plan["desired"]["runtime"]["branch"],
         "--runtime-source", plan["sources"]["runtime"]],
    ]
    peers = [subprocess.Popen(["flatpak-rs", *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for args in commands]
    # This query can answer on another daemon thread while storage peers wait.
    status = subprocess.run(["flatpak-rs", "status"], capture_output=True, text=True, timeout=3)
    assert status.returncode == 0 and json.loads(status.stdout)["ready"], status
    assert primary.poll() is None, "preparation delay did not hold the primary operation"
    assert all(peer.poll() is None for peer in peers), "storage operation bypassed installation serialization"
    output, errors = primary.communicate(timeout=12)
    assert primary.returncode == 0, errors
    result = json.loads(output)
    assert result["outcome"] == "updated"
    assert {key: result["selection"][key] for key in ["app", "runtime"]} == plan["desired"]
    answers = []
    for peer in peers:
        output, errors = peer.communicate(timeout=12)
        answers.append((peer.returncode, output, errors))
    assert answers[0][0] == 1 and "UpdateConflict" in answers[0][2], answers
    assert answers[1][0] == 0 and json.loads(answers[1][1]) == result["selection"], answers
    assert answers[2][:2] == (0, "fixture\n"), answers
    assert answers[3][0] == answers[4][0] == 0, answers
    assert json.loads(answers[3][1])["removed"] and json.loads(answers[4][1])["removed"], answers
    assert answers[5][0] == 1 and "depend" in answers[5][2], answers
    assert answers[6][0] == 0 and json.loads(answers[6][1])["expected"] == plan["desired"], answers
finally:
    for process in [primary, *peers]:
        if process.poll() is None:
            process.kill()
        process.wait(timeout=12)
        process.stdout.close()
        process.stderr.close()
