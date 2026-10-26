# Transactional local updates: design interview

Status: the user approved the consolidated behavioral summary, specification
publication and six-ticket breakdown. The specification and implementation
tickets are published with native parent/blocking relationships. Implementation
has not been authorized or started.
Published specification: https://github.com/razzeee/flatpak-rs/issues/22.
See `local-update-specification.md` for the local publication snapshot.

This is a follow-up to the completed initial milestone, not a retroactive change
to its acceptance contract. The longer-term lifecycle direction is recorded in
`post-milestone-design.md`; only the bounded scope below is being designed here.

## Confirmed scope and semantics

- Add an explicit local-artifact update of an existing application variant.
  Prepare and validate replacement content before switching the complete installed
  selection atomically. Preserve the variant's identity and persistent data.
- Reject updates while the target variant holds an execution lease, including
  output draining. Do not queue an update, stop the app automatically, or replace
  content underneath an active instance. Live updates would require a separate
  design for retaining revisions used by existing instances.
- Each update explicitly selects both the application and its compatible runtime,
  following the current explicit installation selection model even when the
  runtime is unchanged. Record exact commits; do not infer runtime providers.
- Permit any valid explicitly selected replacement, including previously
  installed commits. Commit hashes alone do not establish version ordering;
  there is no downgrade-protection claim. Replacing content with older revisions
  does not revert persistent data.
- Carry the exact current application/runtime commits the caller expects to
  replace. Reject a different current selection as a conflict rather than silently
  overwriting another client's change. If the desired selection is already
  installed, report an unchanged result after the eligibility checks below.
- Pin the desired application/runtime commits when forming a logical update;
  retries of that update reuse those pins rather than silently resolving moved
  source refs to different content. A fresh update request may resolve newer refs.
  Expected and desired revisions retain their full collection/ID/architecture/
  branch identities as well as commits, preserving the existing identity model.
- If source refs move after planning, apply the pinned objects when they remain
  available and valid for their stated identities. Missing, corrupt, incompatible
  or wrongly identified pinned content fails before publication; do not resolve
  newer refs as a fallback.
- Permit app-only, runtime-only and combined revision changes, while always
  validating and publishing one complete compatible installed selection. An
  identical selection is unchanged; there is no partial app/runtime switch.
- Retain installation-wide serialization for preparation and publication in this
  milestone. Launches and other installation operations may wait and then observe
  the completed outcome. Do not introduce per-variant concurrent transactions;
  reduced responsiveness during large preparations is an accepted trade-off.
- Before publication, failure preserves the old selection. Once publication has
  happened, the new selection may already be installed even if the caller loses
  its reply. Do not automatically roll back because acknowledgement was lost;
  inspection after restart recovery must expose one complete selection, never
  a mixture.
- Once accepted, an update is owned by the management process and continues
  toward a safe outcome if the CLI disconnects. Do not add an explicit transaction
  cancellation feature in this milestone. This does not change foreground app
  execution's cancellation-on-client-loss contract. Service failure still invokes
  restart recovery.
- Successful updates return the complete final installed selection, with exact
  application/runtime identities and commits, and distinguish updated from
  unchanged outcomes.
- Applying requires an installed, inactive target variant, even for an unchanged
  desired selection. Missing variants are errors, not implicit installs, and
  active leases are busy errors. Validate plan format and identities first; after
  eligibility checks, an already-installed desired selection returns unchanged
  without requiring source repositories to remain available. Otherwise, reject
  a mismatched expected selection before preparing a real replacement.
- Expose a non-publishing planning operation and a separate apply operation.
  Produce a saveable, reusable JSON update plan containing the observed installed
  selection, desired pinned revisions and explicit local sources. Planning
  reserves nothing. Applying revalidates the plan. A one-command convenience
  flow is deferred; the saved plan preserves retry inputs across lost replies.
- Preserve previous runtimes until explicit runtime removal. Ensure retention
  before publishing the replacement; failure to retain a required previous
  runtime aborts before publication. Reclaim superseded application content
  after committed publication or restart recovery, without retaining application
  rollback history or introducing a general garbage collector.
- Distinguish failures after publication: if publication happened but required
  synchronization fails, report an uncertain outcome and direct the caller to
  inspect. If publication and required synchronization succeeded but obsolete
  application-content deletion fails, report updated with a cleanup warning;
  restart recovery retries cleanup. Neither case silently rolls back.
- Planning and applying use final JSON replies, not progress streaming. Both
  receive a 300-second CLI socket read timeout, matching Install, and retain
  15-second write timeouts. An apply timeout explicitly reports an uncertain
  outcome and directs the caller to inspect; it does not imply cancellation or
  automatically retry with newly resolved source refs.
- Require service-kill/restart recovery tests plus injected storage-write,
  synchronization and disk-exhaustion failures. Still require correct filesystem
  synchronization, but defer actual machine-power-loss testing and do not claim
  untested power-loss guarantees.
- Continue the existing confinement, permission-forward-compatibility, collection
  identity, and variant-private data guarantees. Unsupported permissions do not
  grant access or block launch; required isolation must still fail closed.
- Network distribution, publisher authentication, a rollback command, and repair
  remain outside this first update milestone. This remains experimental, not a
  production-readiness claim.

## Acceptance outline for the specification

Use the established CLI-to-real-management-process NixOS VM seam with independently
prepared OSTree fixtures; supplement it with direct Varlink misuse/failure tests.
Do not replace enforcement or storage behavior with mocks.

- Planning preserves installed selections/data, captures full expected/desired
  revisions and explicit sources, and does not reserve the variant.
- App-only, runtime-only, combined and explicit older-revision updates expose one
  complete compatible selection. App execution subsequently uses that selection.
- Same-selection requests are unchanged; missing/busy targets and stale expected
  selections obey the confirmed eligibility and conflict ordering.
- A saved plan never drifts when refs move. Missing/corrupt/incompatible pinned
  objects and malformed or wrongly identified plans fail safely. An eligible
  unchanged request does not require the original source to remain available.
- Concurrent plans/applies, launch, uninstall, data deletion and runtime removal
  cannot race publication or bypass instance ownership/dependency protection.
- Persistent data, other variants and required/retained runtimes survive updates
  and failures. Runtime retention failure prevents publication.
- Client loss does not cancel accepted application of a plan; a lost reply is
  resolved by inspection and a pinned retry, not automatic rollback or re-resolution.
- Service termination at meaningful preparation/publication/cleanup boundaries
  yields a complete old or new selection after recovery. Previously completed
  installations remain usable.
- Storage-write, synchronization and disk-exhaustion faults distinguish unchanged
  pre-publication state, uncertain publication, and committed updates with cleanup
  warnings. Recovery safely retries obsolete-content cleanup.
- The existing full confinement, permission-forward-compatibility, foreground,
  ownership and lifecycle suite still passes, alongside formatting, type checks,
  Clippy, workflow linting and Rust tests. No machine-power-loss or production
  readiness claim is made.

## Specification and implementation preparation still required

- Write the exact bounded/versioned plan and reply/error shapes and CLI/Varlink
  spellings. A submitted plan is input to validate, not publisher authentication,
  a reservation, or permission to execute artifact hooks.
- Validate pinned-object identity checks when source refs have moved, preserving
  explicit collection/runtime-provider selection without trusting edited plans.
- Specify and validate atomic replacement, required synchronization, recovery
  discrimination, previous-runtime retention and safe old-content reclamation
  before implementing those mechanisms. Escalate any genuine contract conflict.
- Define concrete fault injection and before/after publication observations at
  the agreed test seam. Publish the specification and blocker-linked tickets
  through GitHub Issues before implementation.

These are engineering/specification tasks, not approval to start coding or new
product scope. Shared-understanding confirmation and specification/ticket
publication were approved; implementation requires separate approval.

## Published implementation graph

All tickets are native sub-issues of [specification #22](https://github.com/razzeee/flatpak-rs/issues/22)
and carry `ready-for-agent`. That classification does not authorize work before
the separate implementation approval. Blocking relationships are native GitHub
dependencies; only the preparation ticket has no blockers.

| Ticket | Deliverable | Blocked by |
| --- | --- | --- |
| [#23](https://github.com/razzeee/flatpak-rs/issues/23) | Validate protocol, pinned identity and replacement recovery contract | None |
| [#24](https://github.com/razzeee/flatpak-rs/issues/24) | Saveable non-publishing plans through CLI/service | #23 |
| [#25](https://github.com/razzeee/flatpak-rs/issues/25) | Atomic app-revision replacement, eligibility and basic recovery | #24 |
| [#26](https://github.com/razzeee/flatpak-rs/issues/26) | Runtime-only, combined and older-revision replacements | #25 |
| [#27](https://github.com/razzeee/flatpak-rs/issues/27) | Lost replies, uncertain sync and retryable cleanup | #25 |
| [#28](https://github.com/razzeee/flatpak-rs/issues/28) | Concurrent-update and storage-fault milestone acceptance | #26, #27 |

The first apply slice already owes safe publication, outcome classification and
recovery; later fault coverage does not permit an unsafe intermediate transaction.

## Existing behavior checked during the interview

- `src/main.rs` gives Install a 300-second socket read timeout; other management
  methods use 15 seconds, and writes use 15 seconds. These are socket-I/O timeouts,
  not an overall transaction deadline.
- Dispatched installation has no client-cancellation signal. The management
  process writes the reply only after the operation returns, so an install may
  finish and publish after its CLI has timed out or disconnected. The connection
  input-loop budget does not interrupt dispatched storage work.
- Uninstall retains the exact runtime before unpublishing the application.
  Retention failure leaves the application published; sync/deletion failures can
  occur after unpublication. Data is retained, and runtime removal rejects
  installed dependents.
- Missing app variants are errors for inspect, run and uninstall. Missing retained
  data/runtime removal returns `removed: false` after eligibility checks. Install
  validates source content even when its selected revisions are already installed.

These are facts about the current implementation. The confirmed update policies
above describe the proposed milestone, not update behavior already implemented.
