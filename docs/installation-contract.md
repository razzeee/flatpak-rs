# Local installation contract (#3)

This refines ADRs 0004, 0006, 0008, 0010 and 0011 before implementation.
It does not implement execution or authorize any weaker sandbox.

## Selection and storage boundary

`install SOURCE COLLECTION APP ARCH BRANCH RUNTIME_COLLECTION RUNTIME RUNTIME_BRANCH`
selects `app/APP/ARCH/BRANCH` and `runtime/RUNTIME/ARCH/RUNTIME_BRANCH`.
`inspect COLLECTION APP ARCH BRANCH` returns the completed selection, exact
commits and permission diagnostics as JSON. Runtime architecture is explicitly
the selected app architecture. Only the host-native architecture is accepted.
The source repository's collection ID must match both explicitly supplied
collections when using one repository. `--runtime-source PATH` explicitly selects
a second local runtime repository when the provider is in another collection;
its own collection ID must match the selected runtime. A provider is never
inferred from a runtime name. Mirrors with the same collection preserve identity.

libostree opens and resolves the local repository, imports the pinned objects
as untrusted input into private staging, verifies their checksums/structure,
reads metadata, and copies both commit trees. No production operation invokes a CLI.
Checkouts use user mode, no hardlinks and no source-side cache, so source files
cannot mutate installed content and a read-only source works. Selection is
pinned to resolved commits before preparation. Publisher authentication remains
deferred: a collection ID is a selection constraint, not proof of authenticity.

The service owns `$XDG_DATA_HOME/flatpak-rs` (default `$HOME/.local/share/flatpak-rs`).
It does not use reference Flatpak installation directories. Identity components
are validated before constructing paths. Service-owned directory components
must be real directories, not symlinks. Payload symlinks are not followed to read
metadata; libostree performs checkout into a fresh private destination. Payloads
are never executed, including hooks. Unsupported extensions, extra-data and
hook declarations are rejected separately from permission declarations.

## Publication and interruption

Serialize installation/inspection in the service. Prepare app and runtime under
a unique private `.staging` directory; validate regular-file metadata from the
OSTree tree without following symlinks. Write the complete selection record,
sync staged files/directories, then rename the whole deployment to its variant
path on the same filesystem and sync the parent. Inspect reads only published
records. The rename is the visibility point: an error before it exposes nothing;
a failure after it may have completed, so callers may retry. Identical app and
runtime selections/commits are idempotent; any replacement is an unsupported
update. The source path is not persisted as identity.

Ordinary errors attempt to remove this operation's staging; cleanup failures
leave only unpublished state. Abrupt termination can likewise leave
unpublished staging, which is never inspectable or runnable. On restart the
service removes abandoned staging before accepting requests; cleanup does not
follow symlinks and handles read-only directories. `recovery-status` reports
the number discarded at startup. The VM uses the host-only environment setting
`FLATPAK_RS_INSTALL_STOP_AFTER=prepared|published` to stop the daemon at a
boundary, then externally kills it. This is fault injection, never an
artifact/request-controlled execution mode.
There is no launch operation in #3; future launch must consume only published
deployments and independently establish the full confinement contract.

## Permissions and failures

Parse Flatpak key-file metadata. Validate app/runtime names and the app's exact
`NAME/ARCH/BRANCH` runtime requirement. Permission groups (Context, Environment,
bus policies and unfamiliar non-structural groups/keys) produce diagnostics,
never install blockers or grants. No required/optional classification exists.
Diagnostic text/count are bounded, with a summary for additional declarations,
so forward-compatible input cannot expand into an oversized public reply.
Malformed metadata and unsupported artifact structure remain errors.

Varlink exposes Install and Inspect with typed selections and structured
`OperationFailed(detail)` errors. Existing authentication and framing apply.
CLI errors exit 1. All interfaces remain experimental.

## Agreed test seams

Primary: CLI to socket-activated real service in the NixOS VM, with independent
payload-bearing reference `flatpak build-export` fixtures. Assert public JSON,
not private layout. Supplement: direct Varlink malformed/invalid requests.
No installation test establishes launch-time denied access or sandbox readiness.
