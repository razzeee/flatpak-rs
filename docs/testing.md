# Building and testing

Linux only; the pinned Rust toolchain, systemd 254+ (for
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

## Continuous integration

GitHub Actions runs **Rust checks** and **VM acceptance** on pull requests and
pushes to `main`; manual runs are also supported. Both jobs use the compiler,
dependencies, and OS pinned by `flake.lock`, without updating either lockfile:

```sh
nix build --no-update-lock-file -L .#checks.x86_64-linux.rust
nix build --no-update-lock-file -L .#checks.x86_64-linux.service
```

The Rust derivation builds the package, checks formatting, type-checks every
target, runs Clippy with warnings denied, runs the supplemental tests, and validates
the workflow with actionlint.
The VM job requires working KVM and executes the entire acceptance suite below.
Missing KVM is a failed job, never a skipped security test. Jobs run independently,
have 45-minute timeouts, and retain build logs for 14 days, including on failure.
Actions are pinned to immutable commits; PR jobs receive only read permissions
and do not require repository secrets or privileged `pull_request_target` events.

Maintainers should require **Rust checks** and **VM acceptance** in the `main`
branch ruleset before merging. The workflow reports these checks but does not
configure repository-level branch protection itself. A green run is regression
evidence for the experimental contract, not production certification.

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
VM output and test logs are in the Nix result directory. See the
[acceptance map and provenance](milestone-acceptance.md) for exact scenario reuse.

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

The same VM check also runs crash-injected recovery, confined fixture execution,
foreground streams/cancellation, instance limits and cleanup, uninstall/runtime
dependency protection, and persistent-data deletion. Actual mount/seccomp denial,
disabled namespaces/delegation, descriptor misuse and permission-forward-compatibility
cases verify fail-closed behavior. Successful prerequisite reporting alone is
**not** evidence of sandbox enforcement. The VM uses `vm.panic_on_oom=0` so a
deliberate per-instance OOM exercises scoped killing rather than the harness's
compulsory panic policy. Launch rejects host `vm.panic_on_oom=2`.

## Manual experimental installation

Build with `cargo build --release --locked`. Install `flatpak-rs` on your PATH
and `flatpak-rs-service` at `/usr/libexec/flatpak-rs-service`, with
`flatpak-rs-sandbox` alongside the service, or change the
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
the default single-source operation requires both selected collections to match
the local repository's collection ID. Supply `--runtime-source /absolute/path`
for an explicitly selected provider in another collection. Neither a source path nor collection ID
authenticates a publisher. The service stores deployments under its own
`$XDG_DATA_HOME/flatpak-rs` (default `~/.local/share/flatpak-rs`), independently
of reference Flatpak installations. The CLI environment does not reconfigure an
already-running service's data directory. No app is executed by these commands.

## Transactional local updates (experimental)

Plan explicitly, save the JSON, then apply that exact file:

```sh
flatpak-rs plan-update /absolute/path/to/repo \
  org.example.Collection org.example.Fixture x86_64 stable \
  org.example.Collection org.example.Platform stable > update-plan.json
flatpak-rs apply-update update-plan.json
flatpak-rs inspect org.example.Collection org.example.Fixture x86_64 stable
```

Planning reserves nothing. Optional `--runtime-source PATH`, `--app-commit COMMIT`
and `--runtime-commit COMMIT` select explicit providers and exact older objects.
Apply requires an installed inactive variant, including no output-draining lease.
It compares complete expected selections, imports exact pins without following
moved refs, and atomically publishes the complete app/runtime pair. An eligible
already-installed desired pair returns `unchanged` even without the sources.
Data is preserved, not rolled back or guaranteed compatible with older content.
Previous runtimes remain until explicit removal.

Apply returns `updated` or `unchanged`, the complete selection, and warnings.
`cleanup_pending` means committed publication succeeded but obsolete content still
needs cleanup. Uncertain synchronization or lost acknowledgements require inspection
and retry of the **same saved plan**, not fresh ref resolution. Accepted work is
not cancelled by disconnect. Planning/apply read timeouts are 300 seconds; writes
use 15 seconds. No repair, rollback/history, authenticated remotes, live update,
progress stream or one-command update is provided. See the
[protocol](local-update-protocol.md), [storage contract](local-update-storage.md)
and [acceptance evidence](local-update-validation.md).

## Confined foreground execution and lifecycle

```sh
flatpak-rs run org.example.Collection org.example.Fixture x86_64 stable \
  --memory-mib 256 --pids 64 -- /app/bin/fixture
flatpak-rs instances
flatpak-rs uninstall org.example.Collection org.example.Fixture x86_64 stable
flatpak-rs delete-data org.example.Collection org.example.Fixture x86_64 stable
flatpak-rs runtimes
flatpak-rs remove-runtime org.example.Collection org.example.Platform x86_64 stable
```

Run returns the command's status (130 on client cancellation); stdout/stderr carry
payload bytes, not JSON. No TTY is allocated. Ctrl-C, disconnect and client death
stop the complete instance. Defaults are 256 MiB/64 processes; accepted ranges
are 16–1024 MiB/4–256 processes, with swap disabled. Only one instance per variant
may run. Uninstall retains data and runtimes; deleting data is separate and
runtime removal rejects installed dependents. Management commands return JSON.

These interfaces implement an experimental demonstrator, not production readiness.
