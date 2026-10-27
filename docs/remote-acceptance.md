# Remote installation failure acceptance

Follow-up to the Calculator demonstration, covering issues #34 and #37.

## Recorded validation: 2026-10-27

- Full real-service NixOS VM suite passed in **506.60 seconds**, including all
  existing local installation/update, graphical, confinement, foreground-lifetime,
  service-authentication and resource-limit regressions.
- Pinned Nix Rust check passed: build, formatting, all-target checks, Clippy,
  supplemental tests and workflow lint. Host `cargo test --locked --all-targets`
  and `cargo clippy --locked --all-targets -- -D warnings` also passed during the
  implementation.
- The actual production CLI socket read timeout fired after **326.40 seconds**
  (configured as 300 seconds; Linux may round long socket timeouts). It reported
  `remote installation outcome uncertain` with inspection/same-plan retry
  guidance and `Resource temporarily unavailable (os error 11)`. No shortened
  test timeout or substituted installation implementation was used.
- Filling a separate real ext4 filesystem produced `No space left on device
  (os error 28)` during installation, before publication. The test freed space,
  restarted twice and successfully retried the exact saved pair.

## Scenario map

`tests/remote-fixture.py` independently creates and signs OSTree repositories and
serves their original objects over loopback HTTP. In the VM it includes a static
BusyBox-backed application, deterministic incompressible payload, and a separate
runtime marker. Running the installed metadata command must print both
`remote-payload` and `verified-runtime`. This establishes payload execution from
the remote fixture independently of the live Flathub demonstration.

`tests/remote-acceptance.py` drives installed CLI binaries against the real service:

- Edited exact commits, collection/ref identity, traversal branch, authorization
  generation, plan version and incompatible runtime selections are rejected.
  A validly signed unknown structural requirement is also rejected.
- Missing and tampered commit signatures, tampered commit metadata, corrupt
  content objects, and unavailable exact commits fail without publication.
  Restoring original objects permits same-plan retry.
- The HTTP server deliberately sends truncated payload responses. Failure exposes
  no runnable partial installation; restoring delivery permits the same pins.
- Removing a signed source ref does not prevent applying an available valid pin.
  Moving the ref changes fresh planning but not the saved plan. Applying the new
  selection over an installed old pair is rejected rather than becoming an update.
- Competing CLI installations for one variant yield installed/unchanged for the
  same pair or installed/conflict for different pairs, preserving the first pair.
- Disabling or replacing runtime authorization during an observed application
  payload request blocks publication and subsequent stale-plan retry. Earlier
  real-service tests also cover app authorization withdrawal and client death.
- A transport-only proxy forwards the actual request to the service and receives
  its successful installed reply, then drops or withholds that reply. Public
  inspection sees the complete pair even while the timeout client waits. The CLI
  reports uncertainty; retry returns unchanged and executes the exact payload.
- An unrelated installed variant is inspected and run after rejected operations;
  its private data survives and runtime removal remains blocked by its dependency.

`tests/remote-extended-vm.py` adds service death during an observed payload request,
repeated restart with no published target, and exact-plan retry. Its isolated
96 MiB ext4 test pauses after both checkouts but before selection publication,
fills the filesystem synchronously in filesystem-sized blocks, and observes real
ENOSPC. Both the unrelated installed selection and retained runtime list remain
unchanged; runtime removal is refused. Private data for both variants survives
failure, recovery and retry. The normal installation remains usable after the
temporary filesystem is unmounted.

`tests/remote-vm.py` retains prepared/published crash recovery and injected
selection-write, selection-sync and publication-sync failures. Failures after
rename report uncertain publication; pre-publication failures expose no target.
Completed-plan retry works after authorization is disabled and, in the extended
suite, after the HTTP server is stopped.

## Reproduction and evidence

```sh
nix build --no-update-lock-file --no-link -L \
  .#checks.x86_64-linux.rust .#checks.x86_64-linux.service
```

Expect the full VM to take roughly nine minutes: the real CLI timeout dominates.
The VM requires KVM, user namespaces and systemd cgroup delegation; missing
capabilities fail the test. The signed fixtures require no live Flathub access.

Local passing log: `/tmp/opencode/remote-acceptance-5.log`. During test-only
iteration the VM imported `tests/vm.nix` using the already-built, identical Rust
package `/nix/store/bhdc2j5g2b3gbk5qck8fzfgyilsbm3br-flatpak-rs-0.1.0` and the
flake's pinned nixpkgs. This avoided recompiling unchanged Rust for Python test
edits. The initial pinned checks required fresh invocations after portable Nix's
known `/homeless-shelter` purity error, separately from test results.

The initial timeout harness allowance of 320 seconds was too short for kernel
rounding; the corrected test allows 400 seconds and requires an actual timeout
diagnostic. The initial buffered megabyte-sized disk-fill test did not establish
the expected failure status. The passing test uses synchronous filesystem-block-
sized filling with no reserved blocks and waits for a nonempty status file.

These are deterministic process/network/storage fault tests, not machine-power-loss
testing or production certification. Replay protection, remote updates and the
other exclusions in the remote contract remain deferred. Manual GNOME Calculator
result confirmation remains tracked separately in #38; it is not inferred from
the automated Sway evidence.
