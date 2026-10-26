# Initial milestone acceptance and compatibility

This is an experimental local-artifact demonstrator, not a production-ready
Flatpak replacement. The contract is issue #1 and `docs/design.md`. No public
libflatpak-compatible library, publisher authentication, network remotes,
rollback, appd/portals, desktop integration, foreign architectures,
extensions, extra-data, installation hooks or interactive terminals are promised.

The separately authorized follow-up #22 adds transactional local updates, not
rollback or remote distribution. Its evidence is recorded in
[local-update-validation.md](local-update-validation.md); the initial-milestone
results below retain their original provenance.

## Reproduce

Use a non-root Linux account, the pinned flake, and accessible `/dev/kvm`:

```sh
cargo build --locked
cargo check --locked --all-targets
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
nix build -L .#checks.x86_64-linux.service
```

The VM boots its own kernel/systemd and uses reference Flatpak exporters and a
static independent C payload. It does not mock libostree, IPC, sandbox syscalls,
cgroups or supervision. Missing KVM/Nix/kernel mechanisms fail the run, not skip
security assertions. Rust's supplemental process tests alone are not milestone
acceptance. The Nix result retains the driver/VM logs.

## Evidence map

| Parent requirement / ticket | Real acceptance observations |
| --- | --- |
| Build, activation, restricted management (#2) | `tests/vm.nix`, `varlink-probe.py`: socket activation, modes, UID and all host-namespace peer checks, malformed/fragmented requests and service survival |
| Identities, selection and prepared payloads (#3) | `install-vm.py`: independently obtained commits, explicit compatible runtime, collection/branch separation, mirrors, isolated reference installation, idempotence and update rejection |
| Atomic publication/restart (#4) | `recovery-vm.py`: SIGKILL before and after publication, public recovery count, retry/inspection, preservation of existing installations; later execution of both recovered variants |
| Complete baseline confinement (#5) | `execution-vm.py`, `fixture-probe.c`: positive fixture execution, independent PID/user/mount/net/IPC/UTS/cgroup namespace checks, read-only content, private data/temp, missing host paths/devices/sockets, fixed environment, zero capabilities, no_new_privs and actual syscall denials |
| Foreground streams and cancellation (#6) | `foreground-probe.py`: binary multi-megabyte simultaneous streams, EOF/closed stdin, early exit, blocked stdin, paused/resumed and closed host output, parent descriptor-flag preservation, SIGINT, ignored graceful signals, client death and socket loss while the owning process remains alive |
| Ownership, limits and service failure (#7) | `instance-probe.py`: same-variant races, independent variants, descendant memory/pids limits, process-group signalling containment, failed launch retry, killing only the service MainPID and independently checking descendant disappearance. `owner-death-probe.py` (#16): owner death during post-exit output draining with an inherited socket still open, lease exclusion before death and release/retry/data deletion afterwards |
| Uninstall/runtime protection (#8) | `lifecycle-probe.py`: running-app rejection, unpublish/launch failure, independent variants, retained runtimes and explicit removal, including an explicitly selected provider from a different collection |
| Persistent data lifetime (#9) | `lifecycle-probe.py`: run/write/uninstall/reinstall/read, independent collection/branch contents, explicit/idempotent deletion, running-app exclusion and hostile data symlinks/read-only directories |
| Cross-feature security failures (#10) | `security-vm.py`, `execution-ipc-probe.py`: descriptor-transfer misuse and fd census, high clone-flag rejection, malformed streaming controls and standard optional/zero-argument Varlink calls, actual denied mount/seccomp syscalls, disabled user namespaces/cgroup delegation, dangerous host OOM policy refusal, no positive payload output on setup failure, instance cleanup |
| Complete run and precise reporting (#11) | All of the above in one reproducible check, alongside Rust checks, with these scope/provenance notes |

The table describes required assertions, not a substitute for running them. A
passing denial probe must also prove the application executed: baseline returns
its distinctive status/output only after its internal denial assertions pass.
A setup failure is never accepted as proof that a resource was denied inside
a successfully running sandbox.

## Adapted scenario provenance

No external test code is copied or a complete upstream suite claimed. These
public-interface scenarios are reimplemented for the approved project CLI:

* `razzeee/flatpak-blackbox-tests` at
  `e24a06a33d07e205a7242069fac4120ce58d7e0f`, `sandbox_scenarios.py`:
  `_denied`'s requirement to distinguish a fixture denial from launcher failure;
  `sandbox-clear-env`/environment absence; filesystem/private persistence;
  default network denial. Internal C assertions preserve the meaningful
  negative checks while a distinctive successful fixture result proves launch.
  Network sharing, filesystem grants, host sockets and overrides are intentionally
  not adapted: declarations never grant those capabilities in this milestone.
* `flatpak/flatpak` at `13b1eeb0a19b91660e82fe4162ba3b413a13e72d`,
  `tests/test-run.sh`: XDG home/environment observations and execution against
  independently exported artifacts. Here XDG locations are fixed under the
  variant-private home, and host-prefixed environment values are absent rather
  than forwarded. Flatpak 1's private installation symlinks and CLI spellings
  are deliberately not compatibility assertions.

The rest of the lifecycle, interruption and cgroup scenarios are new acceptance
tests for this project's approved specification. They are not representations
that external library, graphical, portal, remote-trust or update suites passed.

## Validation state

Validated on 2026-10-25 from `implement-3-local-installation`:

* `cargo build --locked`, `cargo check --locked --all-targets`, formatting,
  Clippy with `-D warnings`, and `cargo test --locked` passed.
* The complete pinned NixOS VM check passed every assertion in one run; the
  test script completed in 41.87 seconds. Result:
  `/nix/store/2jww2gxkqswh1z1z3p7mm4aay3hjxhvz-vm-test-run-flatpak-rs-service`.
* Implementation review covered startup/cleanup ownership, partial stream frames,
  host-output backpressure, syscall flags, descriptor inheritance, storage
  publication and independent data/runtime deletion. Regression tests cover the
  confirmed findings; this was not an independent security audit.

This completes the approved experimental milestone's automated acceptance,
not production certification or full Flatpak compatibility. Production hardening,
broader application compatibility, and the deferred features above remain outside
the validated scope.
