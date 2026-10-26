{ pkgs, package }:
let fixtures = pkgs.runCommand "flatpak-rs-install-fixtures" {
  nativeBuildInputs = [ pkgs.flatpak pkgs.ostree pkgs.pkgsStatic.stdenv.cc ];
  FIXTURE_SHELL = "${pkgs.pkgsStatic.busybox}/bin/busybox";
  FIXTURE_PROBE_SOURCE = ./fixture-probe.c;
  FIXTURE_CC = "${pkgs.pkgsStatic.stdenv.cc}/bin/${pkgs.pkgsStatic.stdenv.cc.targetPrefix}cc";
} ''
  export HOME="$TMPDIR"
  sh ${./prepare-install-fixtures.sh} "$out"
''; in pkgs.testers.runNixOSTest {
  name = "flatpak-rs-service";
  nodes.machine = { ... }: {
    virtualisation.memorySize = 2048;
    virtualisation.cores = 2;
    # The test harness defaults to compulsory panic-on-OOM, including memcg OOM.
    # Exercise normal cgroup-scoped OOM killing instead, without removing limits.
    boot.kernel.sysctl."vm.panic_on_oom" = pkgs.lib.mkForce 0;
    boot.kernelModules = [ "loop" ];
    users.users.alice = { isNormalUser = true; uid = 1000; linger = true; };
    users.users.bob = { isNormalUser = true; uid = 1001; };
    environment.systemPackages = [ package pkgs.python3 pkgs.util-linux pkgs.flatpak pkgs.ostree pkgs.e2fsprogs ];
    systemd.packages = [ package ];
    systemd.user.sockets.flatpak-rs.wantedBy = [ "sockets.target" ];
    environment.etc."varlink-probe.py".source = ./varlink-probe.py;
    environment.etc."install-fixtures".source = fixtures;
    environment.etc."foreground-probe.py".source = ./foreground-probe.py;
    environment.etc."owner-death-probe.py".source = ./owner-death-probe.py;
    environment.etc."instance-probe.py".source = ./instance-probe.py;
    environment.etc."update-probe.py".source = ./update-probe.py;
    environment.etc."update-concurrency.py".source = ./update-concurrency.py;
    environment.etc."lifecycle-probe.py".source = ./lifecycle-probe.py;
    environment.etc."execution-ipc-probe.py".source = ./execution-ipc-probe.py;
    environment.etc."deny-mount.conf".text = ''
      [Service]
      SystemCallFilter=~mount
      SystemCallErrorNumber=EPERM
    '';
    environment.etc."deny-seccomp.conf".text = ''
      [Service]
      SystemCallFilter=~seccomp
      SystemCallErrorNumber=EPERM
    '';
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

    def refused_run(expected):
        command = "flatpak-rs run org.example.Collection org.example.Fixture x86_64 stable -- /app/bin/probe baseline 2>&1"
        code, output = machine.execute(alice(command))
        assert code != 0 and expected in output and "confined" not in output, (code, output)
        assert json.loads(machine.succeed(alice("flatpak-rs instances")))["instances"] == []

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

    exec(${builtins.toJSON (builtins.readFile ./install-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./recovery-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./update-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./update-crash-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./update-fault-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./update-pairs-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./update-outcome-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./update-extended-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./update-input-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./update-concurrency-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./update-disk-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./execution-vm.py)})
    exec(${builtins.toJSON (builtins.readFile ./security-vm.py)})

    # Required capability missing: actionable structured failure, never success.
    machine.succeed("sysctl -w user.max_user_namespaces=0")
    code, output = machine.execute(alice("flatpak-rs status"))
    assert code == 2, (code, output)
    missing = json.loads(output)
    assert not missing["ready"]
    assert any(c["name"] == "user_namespaces" and not c["available"] and "enable" in c["detail"] for c in missing["checks"])
    refused_run("user namespace")
    machine.succeed("sysctl -w user.max_user_namespaces=10000")

    machine.succeed("install -D -o alice /etc/no-cgroup.conf /home/alice/.config/systemd/user/flatpak-rs.service.d/no-cgroup.conf")
    machine.succeed(alice("systemctl --user daemon-reload"))
    machine.succeed(alice("systemctl --user restart flatpak-rs.service"))
    code, output = machine.execute(alice("flatpak-rs status"))
    assert code == 2, (code, output)
    missing = json.loads(output)
    assert any(c["name"] == "cgroup_control" and not c["available"] and "Delegate=" in c["detail"] for c in missing["checks"])
    refused_run("cgroup")

    machine.succeed(alice("systemctl --user stop flatpak-rs.service flatpak-rs.socket"))
    code, output = machine.execute(alice("flatpak-rs status 2>&1"))
    assert code == 1 and "enable --now flatpak-rs.socket" in output, output
  '';
}
