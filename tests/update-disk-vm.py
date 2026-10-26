# A disposable, real ext4 target: filling it cannot exhaust the VM's ordinary
# installation or its root filesystem. Sources and CLI output stay outside it.
machine.succeed("mkdir -p /tmp/opencode /home/alice/update-disk")
machine.succeed("truncate -s 96M /tmp/opencode/update-data.img")
machine.succeed("mkfs.ext4 -q -F /tmp/opencode/update-data.img")
machine.succeed("mount -o loop /tmp/opencode/update-data.img /home/alice/update-disk")
machine.succeed("chown alice /home/alice/update-disk")
assert machine.succeed("findmnt -n -o FSTYPE -T /home/alice/update-disk").strip() == "ext4"
machine.succeed(alice("systemctl --user set-environment XDG_DATA_HOME=/home/alice/update-disk"))
restart_update_service()
assert json.loads(machine.succeed(alice(install))) == installed
machine.succeed(alice(update_run + "write disk-data"))
machine.succeed(alice(plan_command + " > /home/alice/disk-plan.json"))

machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_STOP_AFTER=imported"))
restart_update_service()
machine.succeed(alice("(flatpak-rs apply-update /home/alice/disk-plan.json >/home/alice/disk-apply.json 2>/home/alice/disk-apply.err; echo $? >/home/alice/disk-apply.status) >/dev/null 2>&1 &"))
service_pid = machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip()
machine.wait_until_succeeds("grep -q '^State:.*T' /proc/" + service_pid + "/status")
code, fill_error = machine.execute(alice("dd if=/dev/zero of=/home/alice/update-disk/filler bs=1M status=none 2>&1"))
assert code != 0 and "No space left" in fill_error, (code, fill_error)
machine.succeed(alice("systemctl --user kill --kill-whom=main --signal=SIGCONT flatpak-rs.service"))
machine.wait_until_succeeds(alice("test -f /home/alice/disk-apply.status"))
assert machine.succeed(alice("cat /home/alice/disk-apply.status")).strip() == "1"
disk_error = machine.succeed(alice("cat /home/alice/disk-apply.err"))
assert "UpdateFailed" in disk_error and "space" in disk_error.lower(), disk_error
machine.log("Actual ext4 exhaustion result: " + disk_error)
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed
assert machine.succeed(alice(update_run + "read")) == "disk-data"
machine.succeed(alice("rm /home/alice/update-disk/filler"))
machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_STOP_AFTER"))
restart_update_service()
assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/disk-plan.json")))["selection"] == updated["selection"]
assert machine.succeed(alice("flatpak-rs run " + selection + " -- /app/bin/fixture")) == "fixture\nchanged-app\n"
machine.succeed(alice(restore_command + " > /home/alice/disk-restore.json"))
assert json.loads(machine.succeed(alice("flatpak-rs apply-update /home/alice/disk-restore.json")))["selection"] == installed

# A separate transaction mount must fail the real exchange syscall, with no
# remove-then-rename fallback. A symlinked journal must not be traversed either.
disk_updates = "/home/alice/update-disk/flatpak-rs/.updates"
machine.succeed("mount -t tmpfs -o size=48M,uid=1000,mode=700 tmpfs " + disk_updates)
restart_update_service()
cross_code, cross_error = machine.execute(alice("flatpak-rs apply-update /home/alice/disk-plan.json 2>&1"))
assert cross_code == 1 and "UpdateFailed" in cross_error and "cross-device" in cross_error.lower(), cross_error
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed
assert machine.succeed(alice(update_run + "read")) == "disk-data"
machine.succeed("umount " + disk_updates)
machine.succeed(alice("mv " + disk_updates + " " + disk_updates + "-saved; ln -s " + disk_updates + "-saved " + disk_updates))
restart_update_service()
rejected("flatpak-rs apply-update /home/alice/disk-plan.json", "UpdateRecoveryRequired")
machine.succeed(alice("unlink " + disk_updates + "; mv " + disk_updates + "-saved " + disk_updates))
restart_update_service()
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed

# Deliberately damage sealed evidence in this disposable installation. Recovering
# must fail closed and preserve both directories, not guess a rollback or publish.
for damage in ["malformed", "mismatched-pair"]:
    machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_UPDATE_STOP_AFTER=intent"))
    restart_update_service()
    machine.succeed(alice("flatpak-rs apply-update /home/alice/disk-plan.json >/home/alice/damaged-update.log 2>&1 &"))
    service_pid = machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip()
    machine.wait_until_succeeds("grep -q '^State:.*T' /proc/" + service_pid + "/status")
    machine.succeed(alice("systemctl --user kill --kill-whom=all --signal=SIGKILL flatpak-rs.service"))
    machine.succeed(alice("systemctl --user unset-environment FLATPAK_RS_UPDATE_STOP_AFTER"))
    mutation = """
import glob,json,os
paths=glob.glob('/home/alice/update-disk/flatpak-rs/.updates/active/*/intent.json')
assert len(paths)==1,paths
p=paths[0]; data=open(p).read()
json.dump({'path':p,'data':data},open('/home/alice/intent-backup.json','w'))
if DAMAGE=='malformed': data='not JSON'
else:
    v=json.loads(data); v['desired']['app']['commit']='0'*64; data=json.dumps(v)
with open(p,'r+') as f: f.seek(0); f.truncate(); f.write(data); f.flush(); os.fsync(f.fileno())
""".replace("DAMAGE", repr(damage))
    machine.succeed(alice("python - <<'PY'\n" + mutation + "\nPY"))
    restart_update_service()
    rejected("flatpak-rs apply-update /home/alice/disk-plan.json", "UpdateRecoveryRequired")
    rejected("flatpak-rs inspect " + selection, "OperationFailed")
    # Restoring the exact test fixture proves evidence survived the failure; this
    # is not a product repair API. An absent/deleted record cannot be restored r+.
    machine.succeed(alice("python - <<'PY'\nimport json,os\nv=json.load(open('/home/alice/intent-backup.json'))\nwith open(v['path'],'r+') as f: f.seek(0); f.truncate(); f.write(v['data']); f.flush(); os.fsync(f.fileno())\nPY"))
    restart_update_service()
    assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed
    assert machine.succeed(alice(update_run + "read")) == "disk-data"

machine.succeed(alice("systemctl --user unset-environment XDG_DATA_HOME"))
restart_update_service()
machine.succeed("umount /home/alice/update-disk")
machine.succeed("rm /tmp/opencode/update-data.img")
assert json.loads(machine.succeed(alice("flatpak-rs inspect " + selection))) == installed
assert machine.succeed(alice(update_run + "read")) == "update-data"
