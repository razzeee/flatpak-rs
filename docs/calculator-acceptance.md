# Calculator acceptance

## Validation recorded on 2026-10-27

- Official descriptor: `https://dl.flathub.org/repo/flathub.flatpakrepo`;
  collection `org.flathub.Stable`; signing fingerprint
  `6E5C05D979C76DAF93C081354184DD4D907A7CAE`.
- Calculator `app/org.gnome.Calculator/x86_64/stable` commit:
  `00516a24d410ad62ed3e955fa5deb5af221ab6b181ed5549103f3670d10d2fca`.
- Runtime `runtime/org.gnome.Platform/x86_64/51` commit:
  `36d7e84ff76b57bc8dd4ccf702bd154e3d15dcad2059d353d69edd10a6f3e3a3`.
- Both artifacts downloaded through the native service from Flathub. The opt-in
  VM independently verified and installed those same signed objects from the
  loopback mirror, then disabled authorization and stopped the mirror. Offline
  same-plan retry returned `unchanged`; metadata-based launch displayed `2+3 = 5`.
  Result-region OCR passed, screenshots were retained, and stopping the foreground
  client left no running instances. The Calculator VM passed in 94.91 seconds.
- The full deterministic service VM passed in 126.58 seconds, including graphical
  input/output, confinement assertions, remote publication crash recovery and
  injected write/synchronization failures. Rust tests, formatting, Clippy and
  workflow lint passed in the Nix Rust check.
- Manual GNOME launch succeeded, but user confirmation of its displayed result
  remains pending. Automated evidence uses Sway, not GNOME.

Local evidence for this run (not portable reproduction inputs):
`/tmp/opencode/calculator-result-check-2.log`,
`/tmp/opencode/acceptance-checks-6.log`, and Nix output
`/nix/store/f08f4x0awjrdir941r0bijckq1s6fllx-vm-test-run-flatpak-rs-service`.
The latter retains the screenshots, exact VM plan and application log. Portable
Nix required fresh invocations after its `/homeless-shelter` purity error;
these errors occurred between builds, separately from the passing VM tests.

The broader remote fault checklist is not fully covered by these results:
interrupted transfers, concurrent installs, remote-specific lost replies/timeouts,
and actual remote-install storage exhaustion still need dedicated acceptance.

## Manual reproduction

Build with `cargo build --locked`. Run the supplied per-user socket/service units
with their executable paths pointing to the build. The service requires its
documented cgroup delegation (`Delegate=memory pids`,
`DelegateSubgroup=supervisor`). Use a separate XDG_DATA_HOME for an isolated demo.

```sh
flatpak-rs remote-add flathub org.flathub.Stable https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak-rs plan-install flathub org.gnome.Calculator x86_64 stable --runtime-remote flathub > calculator-plan.json
flatpak-rs apply-install calculator-plan.json
flatpak-rs run org.flathub.Stable org.gnome.Calculator x86_64 stable --wayland wayland-0
```

Choose the actual GNOME Wayland socket explicitly. Enter `2 + 3 =` and verify
the displayed `5`. Keep the foreground CLI connected; closing it cancels the app.
The app runs without network, host bus, home, GPU or X11 access. Currency rate,
session-bus and host-font-configuration warnings are expected unavailable
features; they are not evidence of a completed graphical acceptance check.

## Automated acceptance

`nix build .#checks.x86_64-linux.service` runs deterministic signed repository,
crash/retry, local lifecycle and graphical fixture tests. The graphical fixture
requires keyboard and pointer input to change its rendered buffer to green;
the test checks screenshot pixels and retains images.

The separate opt-in Calculator VM consumes the downloaded, unmodified signed
objects. It re-verifies the pinned app/runtime through a local HTTP mirror and
the real service, launches Calculator, enters the calculation using virtual
keyboard input, and checks OCR of the result region (excluding keypad buttons).
It retains the screenshot, cropped result, exact plan and application log.
Network access is needed for preparation, not for running the VM.

```sh
python tests/prepare-calculator-acceptance.py \
  "$XDG_DATA_HOME/flatpak-rs/providers/AUTHORIZATION_GENERATION" \
  calculator-plan.json /absolute/path/calculator-acceptance
nix build --impure --no-link --expr '
  (builtins.getFlake "/absolute/path/flatpak-rs").lib.calculatorAcceptance {
    repo = /absolute/path/calculator-acceptance/repo;
    descriptor = /absolute/path/calculator-acceptance/fixture.flatpakrepo;
    plan = /absolute/path/calculator-acceptance/plan.json;
  }'
```

Find AUTHORIZATION_GENERATION in `flatpak-rs remotes`. The fixture descriptor
retains the official publisher key and changes only the source URL to the VM's
loopback HTTP mirror. The VM replaces the saved provider generation with its
newly authorized generation while preserving both exact revision pins. A plan
is not a credential; current publisher authorization is checked again on apply.
