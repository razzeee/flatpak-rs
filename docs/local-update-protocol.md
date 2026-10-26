# Local update protocol contract (#23)

Technical preparation for [#22](https://github.com/razzeee/flatpak-rs/issues/22).
Implemented by the separately authorized #24–#28 work following preparation #23.
The behavioral scope remains the approved specification. Read
`local-update-storage.md` for publication/recovery and
`local-update-validation.md` for evidence and the acceptance matrix.

## Commands and methods

```text
flatpak-rs plan-update SOURCE COLLECTION APP ARCH BRANCH RUNTIME_COLLECTION RUNTIME RUNTIME_BRANCH
    [--runtime-source PATH] [--app-commit COMMIT] [--runtime-commit COMMIT]
flatpak-rs apply-update PLAN_FILE
```

The first command mirrors explicit install selection: runtime architecture is
the selected app architecture. Each option occurs at most once, in any order
after positional arguments. Sources are local absolute UTF-8 paths. If omitted,
runtime source is the application source; the generated plan always expands both
paths explicitly. Optional commit overrides select exact commits for the given
identities (including deliberately older ones); otherwise planning resolves each
specified ref once. Neither applying nor retrying resolves refs.

`PLAN_FILE` is a UTF-8 JSON file, not inline JSON or an automatic plan-and-apply
request. No stdin shorthand, output-file option or convenience update command is
added. Save stdout with shell redirection; CLI output is the plan itself, not a
Varlink envelope. An incomplete file from a failed command is not a valid plan.
The CLI reads the bounded file completely before connecting and submits the
validated object. The service independently validates it again.

The planned Varlink additions, reusing existing `Variant` and `Revision`, are:

```varlink
type UpdateSelection (app: Revision, runtime: Revision)
type UpdateSources (app: string, runtime: string)
type UpdatePlan (format_version: int, target: Variant, expected: UpdateSelection, desired: UpdateSelection, sources: UpdateSources)
type UpdateInstalled (app: Revision, runtime: Revision, diagnostics: []string)
type UpdateWarning (code: string, detail: string)

method PlanUpdate(source: string, app: Variant, runtime: Variant, runtime_source: ?string, app_commit: ?string, runtime_commit: ?string) -> (plan: UpdatePlan)
method ApplyUpdate(plan: UpdatePlan) -> (outcome: string, selection: UpdateInstalled, warnings: []UpdateWarning)

error UpdateInvalidPlan(detail: string)
error UpdateNotInstalled(target: Variant)
error UpdateBusy(target: Variant)
error UpdateConflict(current: UpdateSelection)
error UpdateSourceInvalid(detail: string)
error UpdateFailed(detail: string)
error UpdateOutcomeUncertain(detail: string)
error UpdateRecoveryRequired(detail: string)
```

Methods use the existing `org.flatpakrs.Manager` interface; error names are
qualified by it. These are additive experimental methods, not a version bump or
rewrite of Install/Inspect/Run. Installed selection comparisons use only the two
revisions, never diagnostics, warning text or source path equality.

`PlanUpdate` returns `parameters.plan`; the CLI deliberately unwraps that one field.
`ApplyUpdate` prints its full reply parameters, consistent with existing management
result printing. Both return a single final JSON reply; no streaming/upgrade.
Existing host-only socket ownership, peer UID/pidfd/namespace authentication and
NUL framing remain mandatory. Normal calls omit `oneway`; as with existing
management dispatch a well-formed `oneway:true` request can execute without a
reply, but is not used by the CLI. `more:true` and `upgrade:true` are rejected.

## Version 1 plan

Example (the repeated hash characters illustrate shape, not real artifacts):

```json
{
  "format_version": 1,
  "target": {"collection":"org.example.Collection","id":"org.example.App","arch":"x86_64","branch":"stable"},
  "expected": {
    "app": {"collection":"org.example.Collection","id":"org.example.App","arch":"x86_64","branch":"stable","commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
    "runtime": {"collection":"org.example.Platforms","id":"org.example.Platform","arch":"x86_64","branch":"stable","commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
  },
  "desired": {
    "app": {"collection":"org.example.Collection","id":"org.example.App","arch":"x86_64","branch":"stable","commit":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"},
    "runtime": {"collection":"org.example.Platforms","id":"org.example.Platform","arch":"x86_64","branch":"stable","commit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}
  },
  "sources": {"app":"/srv/fixtures/apps","runtime":"/srv/fixtures/platforms"}
}
```

- All shown plan fields are required, non-null, and typed exactly. Version is the
  integer `1`; other versions fail before storage work. Unknown fields, duplicate
  member names, trailing non-whitespace and invalid JSON/UTF-8 are errors at the
  new methods' input boundary. Do not pass a duplicate-losing generic JSON value
  through to a supposedly strict typed validator. This requires preserving or
  detecting duplicate information before the current generic dispatch parse.
- Identity components retain the existing grammar: 1–200 ASCII bytes, first
  alphanumeric, remaining bytes alphanumeric or `.`, `_`, `-`; architecture must
  equal the native architecture. Both app revision identities equal `target`.
  Both runtime architectures equal target architecture. Runtime identities may
  otherwise differ between expected and desired; each provider is explicit.
- Commits are exactly 64 lowercase hexadecimal characters, never refs,
  abbreviations, version labels or parent expressions. Expected/desired revisions
  have exactly five fields; validate nested fields explicitly rather than relying
  on the existing flattened revision deserializer to reject extras.
- Each source is an absolute UTF-8 filesystem path of at most 4096 bytes, without
  NUL, `.` or `..` components. It names a local repository, not a URI or remote
  nickname. Filesystem opening/availability is checked only after apply eligibility,
  unchanged and conflict decisions. No-op still requires syntactically valid paths.
  Sources do not name destinations or become cleanup targets.
- A plan has no job ID, reservation token, signature, creation timestamp, captured
  diagnostics or filesystem paths into the service's private installation. A valid
  edited plan is still ordinary input: applying it is an explicit selection subject
  to the same checks, not evidence of its authenticity or a bypass.
- CLI plan files and generated pretty-printed plans are at most 32 KiB including
  whitespace; reject oversized input using a bounded read. New-method nesting is
  at most 16 containers. Request/reply frames remain at most 64 KiB excluding NUL.
  Check the actual encoded size, not only field lengths, including JSON escaping.
  Reject a plan that cannot fit these limits, rather than truncating identities.
- Installed diagnostics retain the existing bounded permission diagnostics
  (32 previews of up to 192 characters plus summary). Error/warning detail is at
  most 2048 Unicode scalar values and the full encoded frame must still fit.
  Success has zero or one warning: `code: "cleanup_pending"`. Extra queued garbage
  is logged during recovery rather than expanded into an unbounded reply list.
  Check that the candidate success reply, including maximum warning space, fits
  before publication; never discover an oversized success record only after commit.

## Planning

Under the installation operation scope, read the installed selection and resolve
or validate the explicitly requested desired revisions. Use private libostree
imports to verify both objects and metadata before issuing a plan. Discard the
planning stage; it does not become a durable pin cache. Planning can fail if
objects change or disappear during preparation. A subsequent apply always imports
from the explicit sources anew unless it is an eligible no-op.

Planning does not acquire an execution lease or require inactivity; it does not
alter the current selection or write persistent app data. It may run while the
app runs, and reserves nothing after its operation lock is released. Verify source
collection, kind/ref bindings, names, native architecture, runtime requirement,
supported structure and permission diagnostics as described in the storage
contract. Report absent target as `UpdateNotInstalled`; invalid selection/source
as `UpdateSourceInvalid`; local preparation I/O errors as `UpdateFailed`.

## Applying: precedence and replies

Run all installation-state observations and preparation/publication under the
existing installation-wide operation lock. Order is normative:

1. Decode/validate format, identities, commit syntax, sources' syntax and limits.
2. Reconcile any unresolved active update before inspecting installation state.
   Reconciliation uses only service-owned state, never the submitted sources.
3. Require the target to be installed, then inactive (including draining lease).
4. If current app/runtime revisions equal desired, return `outcome: "unchanged"`,
   the current complete installed record in `selection`, and `warnings: []`.
   Do not open sources, compare expected or allocate a transaction.
5. Otherwise if current differs from expected, return `UpdateConflict(current)`.
   No source import or replacement preparation has occurred.
6. Validate/import exact desired pins, prepare complete app/runtime checkouts,
   retain the prior runtime, then publish/sync/reclaim per the storage contract.
7. Return `outcome: "updated"` with the complete new installed record and either
   `warnings: []` or one `cleanup_pending` warning. Never report unchanged after
   actually replacing the selection.

This is selection-based compare-and-replace, not generation-based ABA protection:
if other operations change away and back to the exact expected pair, the
precondition matches. No transaction ledger or duplicate-operation identifier is
promised. A later retry can conflict if another client selected different content.

Invalid outer parameters/types/flags use existing Varlink `InvalidParameter`;
invalid contents of an otherwise decodable plan use `UpdateInvalidPlan`.
Syntactically malformed framing/JSON may close the connection as today, before
dispatch. Duplicate-member detection for new methods must precede dispatch and
must not permit one interpretation to execute a different interpretation.

For real replacements, missing/corrupt/incompatible/unbound/wrongly identified
source content is `UpdateSourceInvalid`. I/O or unsupported atomic-exchange
capability before the visibility point is `UpdateFailed`: this apply did not
change the published selection. A pre-publication error can leave a safely
retained runtime or unpublished cleanup pending; it does not promise zero disk
side effects. Post-visibility errors before required synchronization completes are
`UpdateOutcomeUncertain`; never downgrade them to `UpdateFailed`. If reconciliation
cannot establish a safe state, return `UpdateRecoveryRequired` without guessing or
publishing; existing non-update operations can retain their `OperationFailed`
wrapper but must not bypass unresolved recovery.

## CLI lifetime and exit behavior

Both new operations use 300-second socket reads and 15-second writes. Service
connection input-loop budgets do not interrupt dispatched work, and reply write
failure does not cancel a dispatched apply. No client-cancellable flag is passed
into libostree or the installation transaction. This does not change Run's
different client-loss lifetime.

Successful updated, unchanged and updated-with-warning replies exit 0. Failures
exit 1, print actionable errors on stderr, and do not print a success object on
stdout. Preserve structured Varlink errors for direct clients. Cleanup warnings
are machine-readable in the successful JSON and should not be mistaken for an
unsuccessful update requiring automatic retry.

Before connecting/writing, local file/format/connect errors can state that no apply
was submitted. Once any apply bytes may have been sent, write/read timeout, EOF,
truncated/oversized/invalid reply or lost acknowledgement is conservatively an
uncertain outcome unless a definitive service error was decoded. Use wording:
`update outcome uncertain; inspect COLLECTION ID ARCH BRANCH before retrying the
same saved plan`. Never automatically re-plan, retry, cancel or roll back. Even
inspection can fail until service-owned recovery succeeds; keep that error
actionable rather than claiming a particular selection survived.
