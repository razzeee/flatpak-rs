{ pkgs, package }:
pkgs.testers.runNixOSTest {
  name = "flatpak-rs-service";
  nodes.machine = { ... }: {
    virtualisation.memorySize = 2048;
    virtualisation.cores = 2;
    users.users.alice = { isNormalUser = true; uid = 1000; linger = true; };
    users.users.bob = { isNormalUser = true; uid = 1001; };
    environment.systemPackages = [ package pkgs.python3 pkgs.util-linux ];
    systemd.packages = [ package ];
    systemd.user.sockets.flatpak-rs.wantedBy = [ "sockets.target" ];
    environment.etc."varlink-probe.py".source = ./varlink-probe.py;
    environment.etc."no-cgroup.conf".text = ''
      [Service]
      Delegate=no
      DelegateSubgroup=
    '';
  };
  testScript = ''
    import json
    import shlex

    machine.start()
    machine.wait_for_unit("user@1000.service")

    def alice(command):
        return "su - alice -c " + shlex.quote("export XDG_RUNTIME_DIR=/run/user/1000; " + command)

    machine.wait_until_succeeds(alice("systemctl --user is-active flatpak-rs.socket"))
    machine.fail(alice("systemctl --user is-active flatpak-rs.service"))
    # First operation must activate the service without manual daemon startup.
    status = json.loads(machine.succeed(alice("flatpak-rs status")))
    assert status["ready"], status
    assert {c["name"] for c in status["checks"]} == {"user_namespaces", "systemd_user_manager", "cgroup_control"}
    machine.succeed(alice("systemctl --user is-active flatpak-rs.service"))
    pid = machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip()
    assert machine.succeed("stat -c '%a:%u' /run/user/1000/flatpak-rs/management.sock").strip() == "600:1000"
    assert machine.succeed("stat -c '%a:%u' /run/user/1000/flatpak-rs").strip() == "700:1000"
    machine.succeed(alice("python /etc/varlink-probe.py protocol"))
    # Same owner but outside the service's host mount/user namespaces is denied.
    machine.succeed(alice("unshare --user --map-current-user --mount python /etc/varlink-probe.py denied"))
    machine.succeed(alice("unshare --user --map-current-user python /etc/varlink-probe.py denied"))
    # Isolate each remaining namespace check without also changing user namespace.
    for namespace in ["--mount", "--pid --fork", "--net"]:
        machine.succeed("unshare " + namespace + " su - alice -c 'python /etc/varlink-probe.py denied'")
    # The private runtime directory normally rejects another UID at connect().
    machine.succeed("su - bob -c 'python /etc/varlink-probe.py denied'")
    # Bypass filesystem DAC as root to verify service-side UID authentication too.
    machine.succeed("python /etc/varlink-probe.py denied")
    machine.succeed("chmod 711 /run/user/1000 /run/user/1000/flatpak-rs")
    machine.succeed("chmod 666 /run/user/1000/flatpak-rs/management.sock")
    machine.succeed("su - bob -c 'python /etc/varlink-probe.py denied'")
    machine.succeed("chmod 700 /run/user/1000 /run/user/1000/flatpak-rs")
    machine.succeed("chmod 600 /run/user/1000/flatpak-rs/management.sock")
    assert machine.succeed(alice("systemctl --user show flatpak-rs.service -p MainPID --value")).strip() == pid
    machine.succeed(alice("flatpak-rs status"))

    # Required capability missing: actionable structured failure, never success.
    machine.succeed("sysctl -w user.max_user_namespaces=0")
    code, output = machine.execute(alice("flatpak-rs status"))
    assert code == 2, (code, output)
    missing = json.loads(output)
    assert not missing["ready"]
    assert any(c["name"] == "user_namespaces" and not c["available"] and "enable" in c["detail"] for c in missing["checks"])
    machine.succeed("sysctl -w user.max_user_namespaces=10000")

    machine.succeed("install -D -o alice /etc/no-cgroup.conf /home/alice/.config/systemd/user/flatpak-rs.service.d/no-cgroup.conf")
    machine.succeed(alice("systemctl --user daemon-reload"))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    code, output = machine.execute(alice("flatpak-rs status"))
    assert code == 2, (code, output)
    missing = json.loads(output)
    assert any(c["name"] == "cgroup_control" and not c["available"] and "Delegate=" in c["detail"] for c in missing["checks"])

    machine.succeed(alice("systemctl --user stop flatpak-rs.service flatpak-rs.socket"))
    code, output = machine.execute(alice("flatpak-rs status 2>&1"))
    assert code == 1 and "enable --now flatpak-rs.socket" in output, output
  '';
}
