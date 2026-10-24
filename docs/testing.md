# Building and testing

Linux only; Rust with edition 2024 support (1.85+), systemd 254+ (for
`DelegateSubgroup`), Linux 6.5+ (for `SO_PEERPIDFD`), unified cgroup v2 with
memory/pids controllers, and unprivileged user namespaces are required.
The pinned Nix environment supplies the tested compiler and operating system.

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
tests have been adapted yet; this is only service/bootstrap coverage.

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
