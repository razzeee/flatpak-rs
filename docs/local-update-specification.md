# Specification: transactional local updates

Published as https://github.com/razzeee/flatpak-rs/issues/22. This local document
is the publication snapshot; GitHub Issues is the specification tracker.
The approved six-ticket breakdown is published as native sub-issues #23–#28;
the current graph is also recorded in `local-update-design.md`.

Status: approved behavioral scope, synthesized for GitHub publication. Publishing
this specification or marking tickets ready-for-agent does not authorize
implementation. Separate implementation approval is required. Exact protocol and
storage mechanisms remain gated by the preparation obligations below.

## Problem Statement

The experimental local installation demonstrator can install and run an exact
application/runtime pair, but cannot replace that pair for an installed
application variant. Users need to update from explicitly selected local
artifacts without uninstalling, losing variant-private data, overwriting a
concurrent client's selection, or exposing half of an app/runtime replacement.
Moving source refs, interrupted service work and lost replies must not silently
change what an update means or leave the installation unusable.

## Solution

Provide two host-side management operations through the existing CLI and real
per-user Varlink service: non-publishing planning and application of a saved JSON
update plan. A plan records the target application variant, the observed expected
installed selection, explicit local application/runtime sources and the desired
exact revisions. Planning reserves nothing; applying revalidates untrusted input.

An eligible apply publishes one complete compatible selection atomically,
preserving the variant's identity and data. It can change the app, runtime, or
both, including explicitly older revisions. Retries reuse exact pins. Failures
before publication preserve the prior selection; lost acknowledgements and
uncertain synchronization are resolved by inspection, never automatic rollback.

## User Stories

1. As an operator, I want to plan an update from explicit local artifacts so that I can choose content without configuring remotes.
2. As an operator, I want planning to leave the installed selection and data unchanged so that reviewing a proposal does not deploy it.
3. As an operator, I want to save a JSON plan so that review and application can happen separately.
4. As a client author, I want a plan to capture the observed installed selection so that another client's change is not silently overwritten.
5. As an operator, I want exact desired app/runtime commits pinned so that a saved plan does not drift with source refs.
6. As an operator, I want retries to reuse the same pins so that a lost reply does not become a different update.
7. As an operator, I want a fresh planning operation to be able to resolve newer refs so that deliberately choosing new content remains possible.
8. As an operator, I want full collection, ID, architecture and branch identities preserved so that equal names in distinct collections do not collide.
9. As an operator, I want to select the runtime provider explicitly so that runtime names never select an unintended collection.
10. As an operator, I want app and runtime sources to be explicit, including separate local repositories, so that provider identity is validated independently.
11. As an operator, I want to change only the application revision so that an unchanged compatible runtime can remain selected.
12. As an operator, I want to change only the runtime revision so that an application can use a replacement compatible runtime.
13. As an operator, I want to change both revisions atomically so that a mismatched intermediate pair is never runnable.
14. As an operator, I want to select older valid revisions explicitly so that local replacement does not invent a version-ordering policy.
15. As an operator, I want persistent data preserved so that replacing artifacts does not delete or roll back application state.
16. As an operator, I want missing targets rejected so that applying an update cannot implicitly install an app.
17. As an operator, I want active targets rejected, including during output draining, so that content is never replaced under an execution lease.
18. As a client author, I want an eligible already-installed desired selection reported unchanged even if its original source is unavailable so that retries can be resolved safely.
19. As a client author, I want stale expected selections rejected before preparation so that conflicts do not perform replacement work.
20. As an operator, I want moved refs to leave available valid pinned objects usable so that a reviewed plan remains meaningful.
21. As an operator, I want unavailable, corrupt, incompatible or wrongly identified pins rejected so that failures cannot silently choose replacement refs.
22. As an operator, I want malformed or edited plans revalidated so that plans do not bypass artifact, identity or path checks.
23. As an operator, I want other installation operations serialized with preparation and publication so that launch and lifecycle operations observe a completed outcome.
24. As an operator, I want prior runtimes retained until explicit removal so that updating does not implicitly remove installed runtime content.
25. As an operator, I want retention failure to prevent publication so that an update cannot violate runtime-retention promises.
26. As an operator, I want obsolete application payloads reclaimed after committed publication so that updates do not accumulate a rollback history.
27. As an operator, I want accepted applies to survive client disconnect so that a communication failure does not cause transaction cancellation races.
28. As a client author, I want success to distinguish updated from unchanged and return the complete final selection so that outcomes are machine-readable.
29. As an operator, I want an apply timeout to say the outcome is uncertain and direct me to inspect so that I do not mistake a timeout for cancellation.
30. As an operator, I want pre-publication failures to preserve the old selection so that preparation errors leave the installed app usable.
31. As an operator, I want recovery to expose a complete old or new selection so that service crashes cannot expose a mixed deployment.
32. As an operator, I want publication synchronization failures distinguished from cleanup warnings so that committed updates are not misreported as rolled back.
33. As an operator, I want restart recovery to retry obsolete-content cleanup so that a cleanup failure is recoverable without a repair command.
34. As an application user, I want confinement and unavailable-permission behavior preserved after updating so that new artifacts do not acquire host access.
35. As a maintainer, I want real-service crash and storage-fault acceptance evidence so that guarantees are tied to tested behavior rather than private implementation assumptions.

## Implementation Decisions

### Scope and invariants

- An update replaces an existing application variant's installed selection. Its
  collection/application ID/architecture/branch identity and variant-private data
  do not change. Runtime selection remains explicit and must satisfy the desired
  application's declared runtime requirement; never infer a provider by name.
- Both revisions are explicitly selected even when only one changes. Every
  expected and desired revision includes full identity and an exact commit.
- Allow application-only, runtime-only, combined and explicitly older revisions.
  Commit hashes do not establish version order. No downgrade protection, data
  migration, or data rollback is promised.
- Preserve the existing isolated per-user installation, native-architecture and
  supported-artifact restrictions. Leave reference Flatpak installations alone.
- Unknown or unavailable permissions grant no access and do not block installation
  or launch. Required isolation still fails closed. Unsupported structural
  features remain distinct from permission declarations; never execute hooks.

### Planning and source validation

- Planning returns a saveable JSON plan with explicit local sources, target,
  expected installed selection and desired pinned selection. It does not publish,
  reserve the variant or authorize a later apply. Do not infer a prohibition on
  private staging/cache work from non-publication; the observable selection and
  persistent data must remain unchanged.
- Planning may resolve current source refs. Applying and retrying a saved plan
  must use its exact pins, not resolve refs again. Fresh planning may choose newer
  refs. Source paths are locations, not collection identity or publisher proof.
- After refs move, use pinned objects if they remain available and valid for their
  stated identities. Missing, corrupt, incompatible or wrongly identified objects
  fail before publication; never resolve replacement refs as a fallback.
- Treat plans and repository contents as untrusted input. Revalidate identities,
  compatibility, artifact structure, checksums, source collection constraints and
  safe paths. A plan is neither authentication nor permission to execute payloads.
  Collection identity does not authenticate a publisher.

### Apply ordering and concurrency

1. Validate the submitted plan's format and identities.
2. Require an installed target with no active execution lease, including draining.
   Missing and busy errors apply even to desired selections already installed.
3. If the complete desired selection is already installed, return unchanged with
   that complete final selection, without requiring source availability. This
   precedes the expected-selection conflict check.
4. Otherwise require the complete current selection to match the expected
   selection; report conflict before preparing a replacement when it differs.
5. Validate and prepare the pinned content, ensure previous-runtime retention,
   and atomically publish the complete desired selection with required sync.
6. Reclaim superseded app content only after committed publication, or in recovery.

- Preparation and publication remain serialized installation-wide by the owning
  installation handle. Launch and other installation operations may wait, then
  observe the completed outcome. Preserve lease ownership, dependency protection
  and lock ordering; do not introduce per-variant concurrent transactions.
- Busy means reject, not automatically stop the app, queue a future update, or
  replace content while an instance runs. Waiting for the installation operation
  lock is distinct from scheduling an update for a running instance.

### Publication, lifetime and results

- The visibility point publishes the entire app/runtime selection atomically.
  Failures before it preserve the previous published selection. After recovery,
  inspection and execution see one complete old or new selection, never a mixture.
- Accepted applies continue if the client disconnects. Service failure invokes
  restart recovery. Foreground execution's client-loss cancellation is unchanged.
- Losing a reply does not roll back publication or trigger automatic re-resolution.
  Inspect after reconnecting/recovery and reuse the saved plan for a pinned retry.
- Successful final JSON replies distinguish updated and unchanged and include
  the complete final installed app/runtime identities and commits.
- Publication followed by required-sync failure reports outcome uncertain and
  directs the caller to inspect; it must not claim the previous selection survived.
- Confirmed publication and synchronization followed by obsolete-app deletion
  failure reports updated with a cleanup warning. Recovery retries cleanup;
  neither error class silently rolls back.
- Retain previous runtimes until explicit removal, preparing retention before
  replacement publication. Retention errors prevent publication. Runtime removal
  continues to reject installed dependents. Reclaim only superseded app content;
  do not delete persistent data, other variants, retained runtimes or in-use content.
- Planning and applying return final JSON rather than progress streams. Each uses
  a 300-second CLI socket read timeout and 15-second write timeout. These are I/O
  timeouts, not whole-transaction deadlines. Apply timeouts explicitly report
  uncertain outcome, direct inspection and do not imply cancellation or auto-retry.

### Required preparation before affected implementation

These are technical obligations, not new behavior or authorization to code:

- Specify bounded/versioned plan, result and error shapes, CLI/Varlink spellings,
  compatibility handling and malformed-input behavior. Preserve existing framing,
  peer authentication and bounded public diagnostics.
- Validate how libostree verifies pinned objects' stated identities after refs
  move, including separate collections and edited plans, without treating ref
  names or collection IDs as publisher authentication.
- Specify and validate atomic replacement on the supported filesystem, sync
  ordering, recovery discrimination, runtime retention and safe reclamation.
  Existing install-only rename/recovery behavior is not evidence that replacement
  works. The mechanism is deliberately not selected by this specification.
- Define deterministic host-controlled crash/write/sync/disk-exhaustion injection
  at meaningful boundaries; artifacts and management requests cannot enable it.
- Escalate genuine conflicts with the approved contract instead of silently
  weakening it. No broad prefactor or backend abstraction is currently justified;
  installation ownership has already been deepened for this work.

## Testing Decisions

Use the existing CLI-to-socket-activated real-service NixOS VM seam with
independently prepared, payload-bearing reference OSTree/Flatpak fixtures. The
user approved this seam during the design interview. Supplemental direct Varlink
tests cover invalid requests and transport misuse. Keep libostree behind its
existing adapter; do not replace storage or enforcement with mocks.

Good tests assert public plans, JSON outcomes, inspection, launched payload
behavior, retained data and denied access. They do not assert private struct
layout or staging-directory names. Host-controlled fault hooks may synchronize
the test without becoming a public transaction API.

Required acceptance coverage:

- Planning is non-publishing and non-reserving; plans survive JSON save/reload and
  contain full expected/desired pins and explicit sources. Later changes invalidate
  stale replacement expectations rather than being silently overwritten.
- App-only, runtime-only, combined and older selections launch the intended payload
  and compatible runtime. Permission diagnostics and denied access remain correct.
- Apply validation order: malformed identities first; missing/busy targets even
  for no-ops; source-independent unchanged before conflict; conflict before prep.
- Moved refs with available valid pins succeed with those pins; missing, corrupt,
  incompatible, wrong-collection and wrong-identity content fails without fallback.
  Cover edited plans, unsafe paths and unsupported artifact structures.
- Competing plans/applies and launch/uninstall/data deletion/runtime removal cannot
  race publication or bypass lease/dependency protection, including output draining.
- Data and other variants survive. Previous runtimes remain removable only by the
  existing explicit operation, subject to dependents. Retention failure preserves
  the prior published selection.
- Client disconnect and lost replies do not cancel accepted applies. Inspect and
  saved-plan retry resolve outcomes without re-resolution; success/no-op replies
  contain complete final selections. Verify timeout diagnostics without requiring
  every test to wait five minutes or changing production timeout semantics.
- Kill/restart around preparation, retention, publication, required sync and cleanup;
  after recovery, inspect and run one complete old or new pair. Repeat recovery to
  demonstrate safe cleanup retry, not just one successful restart.
- Inject write failures, sync failures and disk exhaustion. Distinguish preserved
  pre-publication selection, uncertain publication and committed cleanup warnings.
  Verify unrelated completed installations remain usable and cleanup never deletes
  data or retained/required content.
- Run the existing full confinement, foreground execution, ownership, lifecycle
  and permission-forward-compatibility VM suite plus Rust tests, formatting,
  all-target type checks, Clippy and workflow linting. Record scenario provenance
  and limitations, not blanket Flatpak compatibility.

Require correct synchronization, but do not claim machine-power-loss guarantees:
actual machine-power-loss testing is deferred. This remains an experimental
demonstrator, not production certification or an independent security audit.

## Out of Scope

- Remote configuration/discovery/download and publisher authentication.
- Automatic provider selection, implicit upgrades or downgrade protection.
- Rollback commands/history, data rollback/migration and repair.
- Live updates, stopping apps automatically, per-variant parallel transactions.
- General garbage collection, automatic runtime removal and disk quotas.
- Progress streaming, transaction cancellation and one-command convenience updates.
- Additional storage backends, general protocol cleanup, app stores, portals,
  new sandbox permissions and blanket Flatpak 1 API/CLI compatibility.
- Actual machine-power-loss testing and production-readiness claims.

## Further Notes

This is a follow-up to the completed initial milestone, not a retroactive change
to its acceptance evidence. It preserves ADRs 0006, 0008, 0009, 0010 and 0011.
The existing install-only refusal of replacements remains correct until this
separate update feature is implemented. Broader lifecycle proposals are not
requirements of this bounded milestone.

The approved interview record is `docs/local-update-design.md`; terminology is
recorded in `GLOSSARY.md`. Both are currently local documentation, not evidence of
implemented update support. Implementation tickets will be native sub-issues
with native blocking dependencies after breakdown review. Readiness labels do
not remove the separate implementation-approval gate.
