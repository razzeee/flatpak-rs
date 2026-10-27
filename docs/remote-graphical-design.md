# Remote graphical application milestone interview

Status: behavioral scope confirmed by the user; specification and ticket
breakdown follow from this interview record.

Published specification: https://github.com/razzeee/flatpak-rs/issues/31

## Confirmed outcome

- Download an existing, unmodified Flathub application and its published runtime.
- Show a visible window with working keyboard/mouse input and demonstrate one
  meaningful interaction. User-selected document access is a later milestone.
- Preserve confined execution, variant-private data, and denied host-home and
  application network access. Define and test the new display-access boundary.
- Provide automated evidence in a reproducible graphical VM and documented
  manual demonstration on the user's desktop.
- Target a small native-Wayland calculator; choose the exact existing Flathub
  application after checking its published artifacts. Demonstrate a calculation
  through its actual UI.
- Permit an explicitly selected host Wayland compositor socket. The compositor
  is part of the trusted boundary; exclude X11 from this milestone.
- Software rendering is sufficient. Do not expose GPU devices for the initial
  demonstration; verify the selected application's rendering requirements.
- Cover remote configuration and verified fresh installation, including
  interrupted-download retry and rejection of invalid signatures. Defer remote
  update planning until installation and graphical execution work end to end.
- Select GNOME Calculator (`org.gnome.Calculator`); demonstrate entering `2 + 3`
  and observing `5` in its visible window using unmodified published artifacts.
  Optional online features such as currency rates need not work.
- Bootstrap Flathub's signing key from its official HTTPS repository descriptor
  through explicit remote authorization. Record the authorized key and collection,
  verify repository metadata and downloaded content, and reject unexpected key
  changes. A matching collection name alone does not authorize a publisher.
- Keep the host session bus inaccessible. Test without a bus first; a
  sandbox-private bus is permitted if needed. A requirement for host services
  must return to the user as a design decision.
- Read the required runtime name and branch from verified application metadata;
  require an explicitly selected trusted runtime remote. For the demonstration,
  application and runtime providers are both Flathub.
- Support the extension subset needed by the demonstrated workload, determined
  through a launch experiment. Required extensions come from explicitly trusted
  providers; unsupported declarations must not silently yield an incomplete
  installation, and unnecessary extensions need not be downloaded.
- Launch by application variant using the command from verified application
  metadata, without manual executable paths or renderer environment setup in the
  normal demo command. Retain foreground lifetime semantics; detached launch
  and desktop-menu exports are deferred.
- Automated graphical acceptance must observe a window, enter the calculation,
  and verify the displayed result, retaining screenshots and logs. Use a
  deterministic signed repository for failure tests and a separate real-Flathub
  demonstration recording exact commits.
- Run a bounded compatibility experiment before completing the implementation
  specification, to establish the minimum runtime, rendering, and bus needs.
  Requirements for broader host access return to the user for a decision.
- Target x86_64 Linux with Wayland, with graphical VM acceptance and a manual
  demonstration on the user's GNOME Wayland desktop.
- Permit explicitly classified unnecessary extension declarations without their
  payloads for the validated Calculator/runtime workload. Unknown structural
  requirements fail clearly; a general extension resolver is deferred.
- Resolve and record exact application/runtime commits before content download,
  preserving selections across retries. Missing pinned content fails rather than
  silently following changed refs; newer selections require a new operation.
- Publish only after the complete selected pair is verified and prepared.
  Failed downloads may retain reusable cache but expose no partial installation.
  Existing installed variants remain unchanged; remote installation is not an
  implicit update.
- Use a saved remote installation plan containing exact app/runtime commits and
  trusted providers, reusable after CLI restart. Recheck trust when applying;
  the plan itself never authorizes a publisher.
- Accepted installation continues after CLI disconnect. Reconnect, inspect, and
  retry the same plan to resolve uncertain outcomes; timeout is not cancellation.
- Disabling a remote blocks future downloads and pending installations using it.
  Already-installed applications remain available; changing download trust does
  not implicitly uninstall or stop them. Signing-key replacement is explicit.
- Verify publisher authenticity and pinned selection without claiming the newest
  available version or comprehensive protection against replay of older signed
  metadata. Full rollback protection is deferred.

## Preparation findings

The current sandbox has no compositor endpoint or display environment, and its
syscall policy lacks operations needed for Wayland communication. Artifact
validation rejects extension declarations, including runtime extension points.
The existing VM exercises static command-line fixtures, not a graphical session.
Nix is not installed on the current host. These are preparation findings, not
decisions about how to extend the implementation.

## Candidate evidence

Read-only inspection of published Flathub metadata identified Calculator 51.1
with `org.gnome.Platform/x86_64/51`. Both app and runtime declare extension points;
which extensions are necessary for the selected workload remains unvalidated.
GTK documents a Cairo software renderer, but successful execution of these exact
artifacts under the proposed restrictions has not yet been demonstrated.

- Application manifest: https://github.com/flathub/org.gnome.Calculator/blob/master/org.gnome.Calculator.json
- Renderer documentation: https://docs.gtk.org/gtk4/running.html#gsk_renderer
- Trust bootstrap descriptor: https://dl.flathub.org/repo/flathub.flatpakrepo

These sources inform preparation; manifests and remote metadata are not proof
of a verified download or successful confined execution.

## Bounded compatibility experiment

The unmodified Calculator app was downloaded using reference Flatpak into an
isolated temporary installation with GPG verification enabled, without pulling
dependencies or related extensions. A bubblewrap probe then mounted that app
and an existing GNOME 51 runtime read-only, with private network/IPC/PID/mount
namespaces, temporary home/run directories, no GPU or host bus, and only the
selected host Wayland socket exposed. This is compatibility evidence, not
flatpak-rs sandbox acceptance.

- App commit: `00516a24d410ad62ed3e955fa5deb5af221ab6b181ed5549103f3670d10d2fca`.
- Existing runtime commit: `28e84823aa1311e85deb46cd564d589f3261c34be4e423fb3720fec61f52f8d4`.
- No app/runtime extensions were mounted. No `/.flatpak-info` was supplied.
- Environment included `GDK_BACKEND=wayland`, `GSK_RENDERER=cairo`,
  `XDG_DATA_DIRS=/app/share:/usr/share`, and a private `XDG_RUNTIME_DIR`.
- The first launch failed to find `libmpc.so.3`. Adding the app library path
  (`LD_LIBRARY_PATH=/app/lib`) allowed startup.
- Wayland client traces show a Calculator toplevel, compositor configuration,
  a 410-by-666 shared-memory buffer, surface attachment/commit, and buffer release.
- The process remained alive until the probe stopped it after 15 seconds;
  no probe application processes remained afterward.
- Missing session bus, host font configuration, and online currency-rate
  warnings did not prevent the observed surface submission.

This did not verify keyboard/mouse input, the displayed calculation, settings
persistence, accessibility, or a compositor presentation acknowledgement. It used
an already-installed runtime rather than demonstrating remote runtime retrieval.
It did not apply flatpak-rs's syscall filter or cgroup limits. These limitations
remain explicit acceptance obligations, not inferred successes.

Temporary probe source, command, and logs are at
`/tmp/opencode/calculator-probe.rU1Zku/`; this path is not durable test evidence.

## Implementation preparation obligations

- Specify bounded plan/protocol shapes and exact CLI spellings, including
  durable retry, inspection, and source-independent completed-install behavior.
- Validate libostree signature verification for repository metadata and pinned
  commits, binding the explicit collection/ref to the authorized provider.
- Define synchronization between remote disabling/key replacement and accepted
  installations so pending work cannot publish under withdrawn authorization.
- Validate the supported extension omission rules against the chosen artifacts;
  complete the keyboard/mouse and displayed-result experiment.
- Specify runtime environment, fonts, library lookup, and any compatibility
  metadata without treating that metadata as authenticated host-service identity.
- Specify safe compositor endpoint selection and lifetime, and the minimal syscall
  policy changes required for Wayland. Retain the established confinement and
  execution lifecycle guarantees.
- Provide the graphical VM tooling and test observations, including actual
  interaction, screenshots, and denied-access checks. Missing infrastructure
  remains a validation blocker, not a passing result.

These are preparation obligations within the agreed direction, not permission
to expand capabilities silently. Return genuine conflicts to the user.

## Interview closure

All recommendations through Q24 were accepted. The user confirmed the
consolidated milestone and requested specification and implementation tickets.

The seven-ticket breakdown was approved and published as native sub-issues of
the specification, with verified native blocking dependencies:

- [Configure trusted remotes (#32)](https://github.com/razzeee/flatpak-rs/issues/32): no blockers.
- [Plan remote installations (#33)](https://github.com/razzeee/flatpak-rs/issues/33): blocked by #32.
- [Apply remote installation plans (#34)](https://github.com/razzeee/flatpak-rs/issues/34): blocked by #33.
- [Launch a graphical fixture (#35)](https://github.com/razzeee/flatpak-rs/issues/35): no blockers.
- [Run unmodified Calculator (#36)](https://github.com/razzeee/flatpak-rs/issues/36): blocked by #35.
- [Validate recovery and trust withdrawal (#37)](https://github.com/razzeee/flatpak-rs/issues/37): blocked by #34.
- [Complete the Flathub demonstration (#38)](https://github.com/razzeee/flatpak-rs/issues/38): blocked by #36 and #37.

The initial milestone's restrictions remain in force until a follow-up contract
explicitly defines the changes. No general Flatpak 1 compatibility is implied.
