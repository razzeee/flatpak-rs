# The first public run must satisfy the entire baseline confinement contract.
run = "flatpak-rs run " + selection + " -- /app/bin/probe baseline"
machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
machine.succeed(alice("systemctl --user set-environment FLATPAK_RS_HOST_SECRET=host-secret"))
machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
code, output = machine.execute(alice(run + " 2>/home/alice/run-stderr"))
assert code == 17 and output == "confined\n", (code, output, machine.succeed(alice("cat /home/alice/run-stderr")))
assert machine.succeed(alice("cat /home/alice/run-stderr")) == "fixture stderr\n"
for branch in ["permissions", "recovery-prepared", "recovery-published"]:
    code, output = machine.execute(alice(run.replace("x86_64 stable", "x86_64 " + branch)))
    assert code == 17 and "confined" in output, (branch, code, output)
try:
    machine.succeed(alice("python /etc/foreground-probe.py"))
except Exception:
    machine.log(machine.succeed("ps -eo pid,ppid,stat,wchan:24,args"))
    raise
machine.succeed(alice("python /etc/lifecycle-probe.py"))
machine.succeed(alice("python /etc/instance-probe.py"))
