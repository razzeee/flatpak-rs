# Independently compare every required namespace, not merely denied syscalls.
namespaces = ["user", "mnt", "pid", "net", "ipc", "uts", "cgroup"]
host_namespaces = [machine.succeed("readlink /proc/self/ns/" + n).strip() for n in namespaces]
namespace_probe = "flatpak-rs run " + selection + " -- /app/bin/probe namespaces " + " ".join(shlex.quote(n) for n in host_namespaces)
assert machine.succeed(alice(namespace_probe)) == "private namespaces\n"
assert machine.succeed(alice("flatpak-rs run " + selection + " -- /app/bin/fixture")) == "fixture\n"
machine.succeed(alice("python /etc/execution-ipc-probe.py"))

machine.succeed(alice("flatpak-rs run " + selection + " -- /app/bin/probe write security-marker"))
machine.succeed(alice("mkdir -p /home/alice/.config/systemd/user/flatpak-rs.service.d"))
for mechanism in ["mount", "seccomp"]:
    machine.succeed("install -D -m644 -o alice /etc/deny-" + mechanism + ".conf /home/alice/.config/systemd/user/flatpak-rs.service.d/fault.conf")
    machine.succeed(alice("systemctl --user daemon-reload"))
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    refused_run(mechanism)
    machine.succeed(alice("rm /home/alice/.config/systemd/user/flatpak-rs.service.d/fault.conf"))
    machine.succeed(alice("systemctl --user daemon-reload"))
    machine.succeed(alice("systemctl --user reset-failed flatpak-rs.service flatpak-rs.socket"))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    assert machine.succeed(alice("flatpak-rs run " + selection + " -- /app/bin/probe read")) == "security-marker"

machine.succeed("sysctl -w vm.panic_on_oom=2")
refused_run("panic_on_oom")
machine.succeed("sysctl -w vm.panic_on_oom=0")
assert machine.succeed(alice("flatpak-rs run " + selection + " -- /app/bin/probe read")) == "security-marker"
