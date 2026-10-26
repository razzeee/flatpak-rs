# Local update storage and recovery contract (#23)

Implemented storage/recovery contract, following technical preparation. Behavioral authority:
[#22](https://github.com/razzeee/flatpak-rs/issues/22). Public shapes and error
precedence: `local-update-protocol.md`. Probe results, primary references and
real-service acceptance: `local-update-validation.md`.

## Pinned objects: bind kind and full identity

The current storage adapter resolves a ref, then pulls checksum strings with
`refs: [COMMIT]`. That is enough to select bytes but does not carry the requested
ref name into libostree's branch-binding check. Metadata `name` and the app's
runtime requirement alone do not establish the app's branch or architecture.
Reusing that path without an additional binding check would trust an edited plan.

For both planning and replacement apply:

1. Open each explicit source as a local repository and require its configured
   collection ID to match the respective selected collection. Runtime providers
   are explicit even when metadata gives only a runtime name/arch/branch.
2. Resolve refs only for planning inputs without commit overrides. For exact pins,
   validate the 64-character commit syntax and do not look up the live ref.
3. Import into a fresh private `BareUserOnly` repo using `pull_with_options` with
   `collection-refs: [(COLLECTION, FULL_REF, COMMIT)]`, GVariant `a(sss)`. The pin
   is always nonempty; full ref is `app/ID/ARCH/BRANCH` or
   `runtime/ID/ARCH/BRANCH`. Do not mix this option with `refs` or
   `override-commit-ids`.
4. Keep signed-int32 `UNTRUSTED`, depth 0, static deltas disabled, no GPG/summary
   authentication claim and binding verification enabled. Use a file URI, never
   a remote nickname. Private imports verify checksums/structure; an unavailable
   pinned object is an error, not permission to follow a newer ref or an ancestor.
5. Explicitly inspect the imported commit's checksum-covered metadata for both
   artifacts, regardless of whether libostree already checked it:
   `ostree.collection-binding` must be a string equal to the selected collection;
   `ostree.ref-binding` must be a string array containing the selected full ref.
   Missing/mistyped bindings, a missing matching ref, or a different collection
   fail. Multiple bound refs are acceptable when the requested one is present.
6. Read regular-file metadata without following symlinks, keep the existing
   64-KiB metadata limit, verify kind/name, real payload directory and the desired
   application's exact runtime requirement, and reject unsupported structures.
   Permission diagnostics retain existing non-blocking/no-grant semantics.
7. Checkout the two exact commits as independent copies: user mode, forced copies,
   fsync enabled, no hardlinks or source-side cache. Remove the temporary object
   repo before sealing a deployment, preserving only the two trees and complete
   installed record. Source mutation cannot mutate a completed deployment.

The explicit collection checks in steps 1 and 5 are essential: the probe found
that libostree accepted a `collection-refs` tuple with a different requested
collection for a file source whose own collection matched the commit. The tuple
is not a substitute for validating the requested collection. Also inspect both
bindings even when app and runtime share an imported object store, avoiding
reliance on whether an already-cached object's pull path repeats all checks.

**Compatibility boundary:** an unbound commit cannot establish its branch after
refs move and is rejected as desired content for this update milestone. Reference
Flatpak exports in our fixtures contain both bindings. Existing install behavior
and installed records are not retroactively invalidated: an eligible no-op uses
the installed record and does not require source bindings. A genuine replacement
requires verifiable desired objects even for its unchanged runtime revision.
Bindings prove object/selection consistency, not publisher authenticity; a local
attacker able to supply a whole new repository can forge its collection claims.

## Concrete atomic replacement choice

Keep the existing real deployment directory and installed index. Exchange the
entire prepared deployment with the installed variant using Linux
`renameat2(..., RENAME_EXCHANGE)`. This is one visibility point for the app tree,
runtime tree and selection record. Plain rename cannot replace a nonempty
deployment directory; moving old away then moving new in has a missing-selection
window and is forbidden. Do not migrate the index to symlink pointers or introduce
a global content-addressed deployment layout for this feature.

Both directory parents must be on the same mounted filesystem, under the
validated service-owned root. New syscall use stays inside the storage boundary.
Use validated directory descriptors and fixed relative names for the exchange,
checking real directory types with no-follow operations. Linux/kernel/filesystem
unsupported flags, cross-mount placement and read-only storage produce an
actionable pre-publication failure; no copy/unpublish/two-rename fallback.
The contract targets local filesystems with successful directory fsync and atomic
exchange; remote/NFS rename failure ambiguity is not a supported transaction
backend. The VM's actual target filesystem remains an implementation acceptance
gate, not inferred from these host probes.

All preparation/publication/recovery occurs under the existing installation
operation lock. Target absence and busy leases are checked there before creating
transaction state. Acquiring a lease also uses that scope, so no instance can
start on an about-to-be-reclaimed deployment. Lease drop still uses only its
issuing registry lock. Different installations/backends and per-variant parallel
transactions are not introduced. Only the existing activated service is a writer.

## Service-owned transaction states

Allocate new update state outside `.staging`, which current startup recovery
blindly discards. Use `.updates/active/ID` and `.updates/garbage/ID` beneath the
installation root. ID is service-generated, collision-resistant, validated and
created exclusively; clients/artifacts never choose it. Create these real private
directories with mode 0700, validate every ancestor and sync newly created parent
entries. Use the existing installed selection/data/runtime-pool layout unchanged.

An active directory contains:

- `slot/`: the complete candidate deployment before exchange; the complete
  superseded deployment after exchange. It contains `app/`, `runtime/` and the
  installed `selection.json` record, as an existing deployment does.
- `intent.json`: an immutable version-1 record with exactly `format_version`,
  `target`, `expected` and `desired` using the protocol's pair-only selection
  types. No source, arbitrary path, executable action or rollback instruction.
  Read regular files no-follow with a 16-KiB bound and validate all identities.
  It is sealed via a temporary file + fsync + rename + directory fsync before
  exchange is allowed. Expected and desired must differ in an active intent.

State is encoded by the directory's parent, the sealed intent and the two complete
selection records. No persisted mutable phase flag is raced against exchange.
An absent sealed intent means preparation never reached the visibility point;
only this private directory can be discarded. Malformed sealed intent is a
recovery error, not an invitation to guess a target or delete published content.

`garbage` is a monotonic **safe-to-delete** class for either discarded candidates
or superseded deployments after commitment. Moving the whole transaction into
this class before deleting any of it makes partially deleted intent/selection
records harmless. Garbage recovery never needs the current published selection,
so a later update/uninstall cannot make old cleanup misidentify its target.

## Normal replacement and required sync order

1. Validate/reconcile/inspect/eligibility/no-op/conflict per the public contract.
   Create an active private directory and prepare both checkouts in its slot.
2. Finish all file writes and final permission changes before synchronization.
   Check every regular-file sync and sync all real directories bottom-up; never
   follow payload symlinks to fsync their targets. Directory fsync covers symlink
   entries. Reject unexpected node types. libostree checkout's `enable_fsync` is
   useful but not a substitute for auditing final directory/record synchronization.
3. Write and fsync complete `selection.json`, fsync slot, remove import scratch and
   fsync the active transaction directory. No fallible metadata write remains
   inside the candidate when it becomes visible.
4. Ensure retention of the exact previous runtime in the existing runtime pool,
   including when the desired runtime is unchanged. Copy without source hardlinks,
   sync final content/permissions and revision record, publish retention atomically,
   and sync its parents before exchange. If an exact retained revision already
   exists, validate and re-sync it and parents: a previous retention attempt might
   have renamed successfully but failed the final parent sync. Retention failure
   leaves the old app published. A successfully retained runtime can remain after
   an aborted apply; it is not rollback history and is removed only explicitly.
5. Seal intent with the full observed expected/desired pairs. Sync the intent,
   transaction directory and active directory entry before exchange. Earlier
   preparation failures safely move only their private state to garbage.
6. Exchange `slot` with the installed variant directory. This successful syscall
   is the visibility point. Set in-memory published/uncertain state immediately;
   no subsequent error is reported as preserving the previous selection. Never
   exchange back in an error handler or repeat exchange during recovery.
   An attempted exchange returning an I/O error must not automatically enter an
   unpublished-stage deletion handler: inspect the two pair records under the
   lock. Only expected-at-installed/desired-in-slot establishes unchanged state;
   desired-at-installed/expected-in-slot or unreadable/ambiguous evidence is an
   uncertain outcome requiring reconciliation. Keep both directories intact in
   the ambiguous case rather than assuming every syscall error means no mutation.
7. Fsync **both changed parents**: the installed variant's parent and the active
   transaction directory holding slot. Interrupted fsync is retried; other failures
   are uncertain. Keep intent/slot intact and mark the owning installation as
   requiring reconciliation before another storage operation.
8. Move the entire active transaction directory to its unique garbage directory
   using a non-overwriting rename. Fsync the active and garbage parents. This
   transition and both syncs are part of required commit bookkeeping, not obsolete
   payload deletion. Failure here is also uncertain, even if publication's first
   syncs succeeded. A durable safe-to-delete class avoids a separate commit marker
   whose partial deletion could lose recovery discrimination.
9. Publication and required sync are now confirmed. Recursively remove only the
   transaction in garbage with the service-owned no-follow deletion helper,
   tolerating read-only payload directories. Sync the garbage parent after removal.
   Any failure here returns updated plus `cleanup_pending`; it cannot undo commit.
   Leave garbage for restart retry. Later storage operations may proceed once
   active transactions are resolved, even if garbage cleanup still fails.

No-op allocates none of this state and requires no retention/source availability.
The existing app-private data tree is never within either exchanged directory.
Reclaiming the old deployment also removes its bundled runtime copy, but only
after the exact runtime has a synchronized retained copy in the separate pool;
this does not remove the retained runtime revision.
Current runtime-removal API removes a runtime variant and refuses installed
dependents of that variant; preserve that policy rather than introducing a new
per-commit removal command in this milestone.

## Recovery and same-process continuation

Recover active updates before ordinary storage observations/mutations and before
ordinary `.staging` cleanup can affect installation readiness. Startup recovery
failure keeps management error reporting available but fails affected storage
operations closed. Replace the current once-per-handle readiness assumption for
unresolved updates: after a post-visibility error, the *next* storage operation
must reconcile before it can inspect, run, install, apply, remove or delete data.
Implementation may use a private dirty flag or scan active entries each time;
callers never sequence locks/recovery themselves.

For each sealed active intent, compare complete **pair** identities/commits from
the installed directory and slot, ignoring diagnostics:

Read installed/slot selection records as regular no-follow files with a 64-KiB
bound, validate their shape/identities and require each record's app variant to
equal the intent target before comparing pairs. Records are service-owned input,
not arbitrary cleanup path instructions.

| Installed pair | Slot pair | Recovery action |
| --- | --- | --- |
| Expected | Desired | Exchange did not happen. Keep old published. Move the uncommitted transaction to garbage and clean it. Never resume applying automatically. |
| Desired | Expected | Exchange happened. Re-sync changed parents, move transaction to garbage and sync both parents, then clean obsolete content. Keep new published; never exchange back. |
| Missing, malformed or any other combination | Any | Fail closed with recovery required; retain evidence and delete neither published directory nor active slot. |

An unsealed preparation directory is unpublished and can be classified as garbage
without reading sources. On process-kill/restart, the sealed record cannot disappear
after exchange because its writes were finished before that syscall. Unsupported
media corruption/power-loss anomalies are not normalized into guessed states.
Validate safe types/names before using any recovery record; the account and kernel
remain trusted as in the existing threat model.

If the active-to-garbage rename succeeded but its sync failed, same-process
reconciliation or startup must sync active/garbage parents before deleting anything
in garbage. Always perform that barrier before garbage traversal. If the barrier
continues to fail, the transition is unresolved: block affected storage operations
and preserve evidence. Ordinary garbage deletion errors after this barrier only
warn/log and retain cleanup work. Partial garbage contents can be safely removed
without a record; the namespace classification is the authority.

Recovery must resolve active state before allowing a later operation to change
the installed pair. Thus an unresolved transaction cannot observe an unrelated
third selection caused by a later accepted update. Multiple sealed active intents
for one variant are an invariant violation; do not guess an order or exchange any.
Garbage from committed or aborted operations is independent of current selections.
Crash during abort cleanup is safe for the same reason as committed cleanup.

`GetRecoveryStatus.discarded_stages` keeps its existing meaning for ordinary
staging; log update reconciliation/garbage retry separately, with bounded messages.
No general repair/rollback command, transaction-query API or retained app history
is introduced. Recovery required after genuine inconsistent/corrupt state is an
actionable error, not a promise to repair arbitrary storage damage.

## Boundary of the guarantee

The intended guarantee is process crash/restart plus correctly classified injected
storage failures, with proper synchronization in the implementation. A successful
fsync is not evidence of tested machine-power-loss behavior. An EIO can reflect
real media damage; retrying directory sync does not prove damaged data repaired
itself. Keep malformed state intact and fail closed. The atomic-exchange probes
validate the syscall/API choice on the reported hosts, not the complete service
algorithm or physical persistence on every filesystem.
