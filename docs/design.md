# Initial milestone design

This document captures the initial milestone design confirmed by the user.
The [post-milestone design](post-milestone-design.md) outlines the tentative longer-term
direction; its unresolved proposals are not initial milestone requirements.

## First milestone

- Install a prepared app and runtime from a local source.
- Run a command-line fixture app.
- Uninstall it.
- Use a CLI communicating with a per-user service that owns installation and execution.
- Support confined execution with explicit, tested isolation guarantees.
- Target modern Linux with systemd and unprivileged user namespaces.
- Provide a reproducible test environment and clear missing-capability errors.
- Consume prepared app and runtime artifacts from a local OSTree repository,
  using reference Flatpak fixture tooling and an existing OSTree implementation.
- Keep storage behind a narrow boundary for future backends.
- Use Varlink for the experimental CLI/service interface.
- Include collection identity in application identity from the outset.
- Implement namespace and mount setup directly in Rust.
- Operate without appd initially; defer host-service access requiring trustworthy
  application identity. Internal instance tracking is not appd authentication.

Remote discovery, graphical integration, updates, unconfined execution, and a
public libflatpak-compatible library are deferred beyond this milestone.

## Installation and artifact selection

- Use project-owned per-user installation storage, separate from existing Flatpak
  installations. Do not modify the reference installation.
- Select applications by collection, app ID, architecture, and branch; record the
  exact OSTree commit installed. Initially permit one installed revision per variant.
- Require an explicitly selected runtime from the local source, validate it against
  the app's declared runtime requirement, and record its exact commit. Missing or
  incompatible runtimes are errors; do not silently choose a collection by runtime name.
- Accept explicitly supplied local test repositories for the first milestone.
  Publisher authentication and signed remote distribution are deferred. Repository
  contents remain untrusted input; collection IDs are not proof of authenticity.

## Initial execution contract

- App and runtime mounts are read-only.
- Temporary storage and persistent app data are private to the sandbox/application
  respectively. Persistent data is isolated by application variant (collection,
  app ID, architecture, and branch).
- No host home access, host-service sockets, network access, or GPU/audio/input devices.
- Expose only minimal devices and process interfaces needed for execution and
  deliberately connected standard input/output.
- Verify denied access as well as successful execution.
- Preserve `/app` for application files and `/usr` for runtime files. Provide a
  private home backed by variant-specific persistent data, controlled environment
  variables, private temporary storage, and minimal `/proc` and `/dev`. Do not
  inherit arbitrary host environment variables.
- Support only host-native architecture and self-contained app/runtime fixtures
  initially. Extensions, extra-data downloads, and installation hooks are outside
  this milestone; reject artifacts requiring these unsupported structural features.

## Permission compatibility

- Unknown permission declarations do not block installation or launch. Ignore them,
  grant no corresponding access, and emit a diagnostic.
- Known permissions that cannot be provided likewise do not block installation or
  launch; the corresponding access remains denied.
- Do not introduce required/optional permission categories.
- Apps must handle unavailable capabilities; launch compatibility does not guarantee
  every application feature works on older implementations.
- Permission declarations never authorize sandbox holes. Failure to establish the
  required sandbox itself still blocks execution. Unsupported artifact structures
  are distinct from unsupported permission declarations.

## CLI and test reuse

Use a small project-specific CLI and adapt selected existing test scenarios to it.
Preserve useful lifecycle behavior without promising Flatpak 1 CLI compatibility;
report adapted coverage explicitly rather than implying full black-box suite coverage.

## Lifecycle and failure semantics

- Uninstall removes the installed app but retains persistent data; deleting data
  requires a separate explicit action.
- Runtimes remain until explicitly removed. Reject removal while any installed
  app depends on that runtime.
- Allow one running instance per application variant; reject uninstall while it
  runs. Different variants may run independently.
- Stage installation and expose it only after app and runtime preparation succeeds.
  Publish the completed installation atomically and recover abandoned staging on
  restart. Interrupted installation must not produce a runnable partial installation.
- Installing the same selected app/runtime commits is idempotent; selecting
  different commits for an installed variant requires a future update operation.
- Use systemd socket activation for the per-user service. Stop running apps if the
  service fails or restarts, and verify cleanup of their process trees.
- Provide foreground non-interactive execution with stdin, stdout, stderr, exit
  status, and cancellation across the service boundary. Terminal allocation and
  interactive job control are deferred.

## Security and resource boundaries

- Treat app processes and artifact contents as potentially malicious. Trust the
  host kernel, systemd, and the account running the service. Protection from a
  compromised host or other unsandboxed processes running as that user is outside
  this milestone.
- Cover path traversal, symlink escapes, and unauthorized service access in the
  threat model.
- Restrict the management socket to host-side clients of the owning user, authenticate
  peers, and keep it unavailable inside app sandboxes. Do not pass management sockets
  or unintended file descriptors into apps. App-facing portals are a separate interface.
- Fail execution if required isolation cannot be established. Require namespace
  isolation, restricted mounts and devices, dropped capabilities, `no_new_privs`,
  and an explicit syscall policy. Validate the detailed policy before implementation.
- Require per-instance cgroup control on the test host and configurable process-count
  and memory limits, with documented defaults chosen during implementation.
  Disk quotas and comprehensive denial-of-service protection are deferred.
- Losing the execution client stops its app. Cancellation terminates the complete
  sandbox process tree using bounded graceful shutdown followed by forced cleanup.

## Storage integration

Use libostree through Rust bindings or a narrow FFI wrapper behind the storage
boundary, rather than invoking the OSTree CLI for production operations. Reference
CLI tooling remains available for independent fixture preparation.

## Acceptance criteria

Automated tests must cover the successful lifecycle, isolation failures,
collection/variant separation, dependency protection, interrupted installation
recovery, and process cleanup. Run these in a reproducible Linux VM providing the
required kernel/systemd capabilities, alongside ordinary Rust checks. Record
exactly which upstream or black-box scenarios were reused and any unsupported cases.
Also verify that unknown and known-but-unavailable permission declarations permit
installation and launch without granting the denied access.

## References

- [Flatpak Next talk, LAS 2026](https://www.youtube.com/watch?v=NsVhkz2Xl0E):
  user-supplied transcript; proposals rather than a finalized specification.
- [Flatpak black-box tests](https://github.com/razzeee/flatpak-blackbox-tests):
  public-interface compatibility tests with target adapters and reusable fixtures.
- [Flatpak upstream tests](https://github.com/flatpak/flatpak/tree/main/tests):
  integration tests and internal tests with differing reuse costs.

Compatibility is chosen per feature, as described in
[ADR 0001](adr/0001-target-flatpak-next.md).

## Implementation preparation

The user has confirmed this scope and approved implementation preparation followed
by implementation. Concrete syscall
policy, namespace/mount setup, peer authentication, descriptor handling, atomic
publication and recovery, and cgroup enforcement must be specified and validated
against this contract before implementing the affected components. These technical
details do not authorize weakening the agreed confinement contract.

Publisher authentication is deferred beyond the local-test milestone.
