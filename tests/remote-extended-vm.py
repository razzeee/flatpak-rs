"""Finish remote acceptance with public outcomes and real storage exhaustion."""
machine.log(machine.succeed(alice("python /etc/remote-acceptance.py"), timeout=600))

def restart_remote_service():
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service flatpak-rs.socket"))

# Kill the real service during an observed HTTP payload request, then recover
# repeatedly. No private cache/staging path is used to decide the outcome.
remote = json.loads(machine.succeed(alice("flatpak-rs remote-add transfer-crash org.example.Remote " + remote_info["descriptor"])))
crash_plan = json.loads(json.dumps(plan))
for key in ["app_provider", "runtime_provider"]:
    crash_plan[key] = {"name": "transfer-crash", "generation": remote["generation"]}
machine.succeed("printf %s " + shlex.quote(json.dumps(crash_plan)) + " > /home/alice/remote-crash-plan.json")
machine.succeed(alice("rm -f /home/alice/remote-fixture/content-waiting; touch /home/alice/remote-fixture/pause-content"))
machine.succeed(alice("flatpak-rs apply-install /home/alice/remote-crash-plan.json >/home/alice/transfer-crash.log 2>&1 &"))
machine.wait_for_file("/home/alice/remote-fixture/content-waiting")
machine.succeed(alice("systemctl --user kill --kill-whom=all --signal=SIGKILL flatpak-rs.service"))
machine.succeed(alice("rm /home/alice/remote-fixture/pause-content"))
for attempt in range(2):
    restart_remote_service()
    machine.fail(alice("flatpak-rs inspect org.example.Remote org.example.RemoteApp x86_64 stable"))
    assert machine.succeed(alice("flatpak-rs run org.example.Remote org.example.RemoteApp x86_64 keeper -- /app/bin/fixture read")) == "keep-data"
retried = json.loads(machine.succeed(alice("flatpak-rs apply-install /home/alice/remote-crash-plan.json")))
assert retried["installed"]["app"] == crash_plan["app"]
assert retried["installed"]["runtime"] == crash_plan["runtime"]

# A separate ext4 filesystem confines exhaustion to this disposable installation.
machine.succeed("mkdir -p /tmp/opencode /home/alice/remote-disk")
machine.succeed("truncate -s 96M /tmp/opencode/remote-data.img")
machine.succeed("mkfs.ext4 -q -F -b 4096 -m 0 /tmp/opencode/remote-data.img")
machine.succeed("mount -o loop /tmp/opencode/remote-data.img /home/alice/remote-disk")
machine.succeed("chown alice /home/alice/remote-disk")
assert machine.succeed("findmnt -n -o FSTYPE -T /home/alice/remote-disk").strip() == "ext4"
machine.succeed(alice("systemctl --user set-environment XDG_DATA_HOME=/home/alice/remote-disk"))
restart_remote_service()
machine.succeed(alice("flatpak-rs remote-add disk org.example.Remote " + remote_info["descriptor"]))
for branch in ["keeper", "stable"]:
    machine.succeed(alice("flatpak-rs plan-install disk org.example.RemoteApp x86_64 " + branch + " --runtime-remote disk > /home/alice/disk-remote-" + branch + ".json"))
    machine.succeed(alice("flatpak-rs apply-install /home/alice/disk-remote-" + branch + ".json"))
    machine.succeed(alice("flatpak-rs run org.example.Remote org.example.RemoteApp x86_64 " + branch + " -- /app/bin/fixture write disk-" + branch))
disk_keeper = json.loads(machine.succeed(alice("flatpak-rs inspect org.example.Remote org.example.RemoteApp x86_64 keeper")))
disk_selected = json.loads(machine.succeed(alice("flatpak-rs inspect org.example.Remote org.example.RemoteApp x86_64 stable")))
machine.succeed(alice("flatpak-rs uninstall org.example.Remote org.example.RemoteApp x86_64 stable"))
retained = json.loads(machine.succeed(alice("flatpak-rs runtimes")))
machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_INSTALL_STOP_AFTER=remote-checkout"))
restart_remote_service()
machine.succeed(alice("(flatpak-rs apply-install /home/alice/disk-remote-stable.json >/home/alice/remote-disk.json 2>/home/alice/remote-disk.err; echo $? >/home/alice/remote-disk.status) >/dev/null 2>&1 &"))
service_pid = machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip()
machine.wait_until_succeeds("grep -q '^State:.*T' /proc/" + service_pid + "/status", timeout=30)
code, error = machine.execute(alice("dd if=/dev/zero of=/home/alice/remote-disk/filler bs=4096 oflag=sync status=none 2>&1"))
assert code != 0 and "No space left" in error, error
machine.succeed(alice("systemctl --user kill --kill-whom=main --signal=SIGCONT flatpak-rs.service"))
machine.wait_until_succeeds("test -s /home/alice/remote-disk.status")
status = machine.succeed("cat /home/alice/remote-disk.status").strip()
error = machine.succeed("cat /home/alice/remote-disk.err")
assert status == "1", (status, error, machine.succeed("cat /home/alice/remote-disk.json; df /home/alice/remote-disk"))
assert "space" in error.lower() and "outcome uncertain" not in error, error
machine.log("Actual remote ext4 exhaustion: " + error)
machine.fail(alice("flatpak-rs inspect org.example.Remote org.example.RemoteApp x86_64 stable"))
assert json.loads(machine.succeed(alice("flatpak-rs inspect org.example.Remote org.example.RemoteApp x86_64 keeper"))) == disk_keeper
assert json.loads(machine.succeed(alice("flatpak-rs runtimes"))) == retained
machine.fail(alice("flatpak-rs remove-runtime org.example.Remote org.example.Platform x86_64 stable"))
assert machine.succeed(alice("flatpak-rs run org.example.Remote org.example.RemoteApp x86_64 keeper -- /app/bin/fixture read")) == "disk-keeper"
machine.succeed(alice("rm /home/alice/remote-disk/filler"))
machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_INSTALL_STOP_AFTER"))
for attempt in range(2):
    restart_remote_service()
    machine.fail(alice("flatpak-rs inspect org.example.Remote org.example.RemoteApp x86_64 stable"))
assert json.loads(machine.succeed(alice("flatpak-rs apply-install /home/alice/disk-remote-stable.json")))["installed"] == disk_selected
for branch in ["keeper", "stable"]:
    assert machine.succeed(alice("flatpak-rs run org.example.Remote org.example.RemoteApp x86_64 " + branch + " -- /app/bin/fixture read")) == "disk-" + branch
machine.succeed(alice("systemctl --user unset-environment XDG_DATA_HOME"))
restart_remote_service()
machine.succeed("umount /home/alice/remote-disk")
machine.succeed("rm /tmp/opencode/remote-data.img")
assert machine.succeed(alice("flatpak-rs run org.example.Remote org.example.RemoteApp x86_64 keeper -- /app/bin/fixture read")) == "keep-data"
machine.succeed(alice("systemctl --user stop remote-fixture"))
assert json.loads(machine.succeed(alice("flatpak-rs apply-install /home/alice/remote-crash-plan.json")))["status"] == "unchanged"
