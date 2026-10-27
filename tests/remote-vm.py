"""Remote authorization through the socket-activated service, with real signatures."""
machine.succeed(alice("systemd-run --user --unit=remote-fixture sh -c 'python /etc/remote-fixture.py /home/alice/remote-fixture > /home/alice/remote-info.json'"))
machine.wait_until_succeeds("test -s /home/alice/remote-info.json")
remote_info = json.loads(machine.succeed("cat /home/alice/remote-info.json"))
remote = json.loads(machine.succeed(alice("flatpak-rs remote-add fixture org.example.Remote " + remote_info["descriptor"])))
assert remote["fingerprints"] == [remote_info["fingerprint"]]
assert remote["enabled"] is True
machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
assert json.loads(machine.succeed(alice("flatpak-rs remotes")))["remotes"] == [remote]
machine.succeed(alice("flatpak-rs plan-install fixture org.example.RemoteApp x86_64 stable --runtime-remote fixture > /home/alice/remote-plan.json"))
plan = json.loads(machine.succeed("cat /home/alice/remote-plan.json"))
assert plan["app"]["commit"] == remote_info["commit"]
assert plan["runtime"]["commit"] == remote_info["runtime_commit"]
applied = json.loads(machine.succeed(alice("flatpak-rs apply-install /home/alice/remote-plan.json")))
assert applied["status"] == "installed"
assert applied["installed"]["app"] == plan["app"]
assert applied["installed"]["runtime"] == plan["runtime"]
for boundary in ["prepared", "published"]:
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("flatpak-rs uninstall org.example.Remote org.example.RemoteApp x86_64 stable"))
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_INSTALL_STOP_AFTER=" + boundary))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    machine.succeed(alice("flatpak-rs apply-install /home/alice/remote-plan.json </dev/null >/home/alice/remote-interrupted.log 2>&1 &"))
    service_pid = machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip()
    machine.wait_until_succeeds("grep -q '^State:.*T' /proc/" + service_pid + "/status", timeout=30)
    machine.succeed(alice("systemctl --user kill --kill-whom=all --signal=SIGKILL flatpak-rs.service"))
    machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_INSTALL_STOP_AFTER"))
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    assert json.loads(machine.succeed(alice("flatpak-rs recovery-status")))["discarded_stages"] == (1 if boundary == "prepared" else 0)
    if boundary == "prepared":
        machine.fail(alice("flatpak-rs inspect org.example.Remote org.example.RemoteApp x86_64 stable"))
    retried = json.loads(machine.succeed(alice("flatpak-rs apply-install /home/alice/remote-plan.json")))
    assert retried["installed"] == applied["installed"]
    assert retried["status"] == ("installed" if boundary == "prepared" else "unchanged")
for site in ["write-selection", "sync-selection", "sync-publication"]:
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("flatpak-rs uninstall org.example.Remote org.example.RemoteApp x86_64 stable"))
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_REMOTE_FAIL_AT=" + site))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    code, output = machine.execute(alice("flatpak-rs apply-install /home/alice/remote-plan.json 2>&1"))
    assert code != 0, output
    if site == "sync-publication":
        assert "outcome uncertain" in output, output
        assert json.loads(machine.succeed(alice("flatpak-rs inspect org.example.Remote org.example.RemoteApp x86_64 stable"))) == applied["installed"]
    else:
        machine.fail(alice("flatpak-rs inspect org.example.Remote org.example.RemoteApp x86_64 stable"))
    machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_REMOTE_FAIL_AT"))
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    assert json.loads(machine.succeed(alice("flatpak-rs apply-install /home/alice/remote-plan.json")))["installed"] == applied["installed"]
machine.fail(alice("flatpak-rs remote-add wrong org.example.Remote " + remote_info["wrong_descriptor"]))
machine.fail(alice("flatpak-rs remote-add wrong-collection org.example.Other " + remote_info["descriptor"]))
machine.succeed("printf tampered > /home/alice/remote-fixture/repo/summary.sig")
machine.fail(alice("flatpak-rs remote-replace fixture org.example.Remote " + remote_info["descriptor"]))
assert json.loads(machine.succeed(alice("flatpak-rs remotes")))["remotes"] == [remote]
disabled = json.loads(machine.succeed(alice("flatpak-rs remote-disable fixture")))
assert disabled["enabled"] is False
machine.succeed(alice("systemctl --user stop remote-fixture"))
assert json.loads(machine.succeed(alice("flatpak-rs apply-install /home/alice/remote-plan.json")))["status"] == "unchanged"
