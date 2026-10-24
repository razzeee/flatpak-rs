# Building and testing

Linux only; Rust with edition 2024 support (1.85+), systemd 254+ (for
`DelegateSubgroup`), Linux 6.5+ (for `SO_PEERPIDFD`), unified cgroup v2 with
memory/pids controllers, and unprivileged user namespaces are required.
The pinned Nix environment supplies the tested compiler and operating system.
Native builds additionally require libostree and GLib/GIO development packages
and pkg-config (for example `ostree-devel glib2-devel pkgconf-pkg-config` on Fedora).

```sh
cargo build --locked
cargo check --locked --all-targets
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --test cli
cargo test --locked --test service
cargo test --locked
```

The service test uses the real binaries with an inherited listener supplied by a
local test harness. It checks protocol rejection and explicit missing-cgroup
reporting, not systemd activation. Run tests as a non-root user on Linux 6.5+.

## Primary acceptance suite

On an x86_64 Linux host with Nix (flakes enabled) and accessible `/dev/kvm`:

```sh
nix build -L .#checks.x86_64-linux.service
```

`flake.lock` pins nixpkgs; `Cargo.lock` pins Rust dependencies. The test builds
the actual binaries, boots a NixOS VM with its own kernel/systemd/user accounts,
and calls the CLI without first starting the daemon. It checks socket modes,
structured host prerequisites, malformed and fragmented Varlink messages,
unknown methods/parameters, UID and namespace denial, service survival, missing
user namespace support, and actionable connection failure. Missing KVM/Nix is
a test infrastructure failure, not a passing test or evidence of confinement.
VM output and test logs are in the Nix result directory. No upstream Flatpak
test code is reused; the original suite covers service/bootstrap behavior.

Issue #3 adds CLI-to-real-service installation and inspection tests using
payload-bearing app/runtime repositories prepared independently with reference
`flatpak build-finish` / `flatpak build-export`. Exact expected commits are
obtained with reference `ostree rev-parse`, not calculated by flatpak-rs.
See [the installation contract](installation-contract.md) for publication and
interruption semantics. Fixture preparation disables only the reference exporter's
icon-validator sandbox (there are no icons); this is not an execution mode in
flatpak-rs. The VM's immutable source also checks that installation need not
write into the input repository.

The installation suite checks collection/branch separation, mirror identity,
explicit runtime compatibility, missing refs, exact app/runtime commits,
idempotence, rejection of app/runtime updates, non-native architecture, structural
requirements, permission compatibility, corrupt objects, metadata/payload symlinks,
unpublished failed preparations, persistence across an ordinary service restart,
and unchanged reference installation inventory. These are project-specific
acceptance scenarios for the parent spec, not executions of the external
black-box suite or upstream Flatpak test scripts. No external test code is copied;
reference Flatpak/OSTree executables are used only for fixture preparation and
independent observations. This does not imply Flatpak CLI compatibility.

Still outside this ticket: crash-injected recovery/staging reclamation (#4),
launch and denied-access probes, runtime-removal protection, uninstall/data
retention, supervision, syscall/mount policy and cgroup enforcement on apps.
Those tests remain required in the dependent milestone tickets.

No apps are launched. Successful prerequisite reporting does **not** validate
namespace/mount setup, syscall policy, descriptor isolation, cgroup enforcement
on apps, permissions, or any other sandbox guarantee. Those require later
end-to-end acceptance tests and must fail closed when implemented.

## Manual experimental installation

Build with `cargo build --release --locked`. Install `flatpak-rs` on your PATH
and `flatpak-rs-service` at `/usr/libexec/flatpak-rs-service`, or change the
service unit's absolute ExecStart path to your installed binary. Install both
files in `systemd/` to `~/.config/systemd/user/`, then:

```sh
systemctl --user daemon-reload
systemctl --user enable --now flatpak-rs.socket
flatpak-rs status
journalctl --user -u flatpak-rs.service
```

Alternatively `nix build` produces binaries and units under `result/`; the unit
already references the immutable service binary. Enable the socket, not the
service. Run as a non-root user in a systemd login session with XDG_RUNTIME_DIR.
The socket lives at `$XDG_RUNTIME_DIR/flatpak-rs/management.sock`. The daemon
refuses manual startup without the systemd listener. Exit codes: 0 = prerequisites
available, 2 = missing prerequisites (JSON diagnostics), 1 = connection/protocol/
usage failure. All interfaces are experimental and deliberately local-only.

## Local artifact installation (experimental)

With the socket enabled, select every identity component explicitly:

```sh
flatpak-rs install /absolute/path/to/repo \
  org.example.Collection org.example.Fixture x86_64 stable \
  org.example.Collection org.example.Platform stable
flatpak-rs inspect org.example.Collection org.example.Fixture x86_64 stable
```

Both return JSON containing complete app/runtime selections, exact commits, and
ignored-permission diagnostics. Runtime architecture is the app architecture;
this single-source operation requires both selected collections to match the
local repository's collection ID. Neither a source path nor collection ID
authenticates a publisher. The service stores deployments under its own
`$XDG_DATA_HOME/flatpak-rs` (default `~/.local/share/flatpak-rs`), independently
of reference Flatpak installations. The CLI environment does not reconfigure an
already-running service's data directory. No app is executed by these commands.
