# Local update preparation evidence and acceptance matrix (#23)

This records technical preparation for
[#23](https://github.com/razzeee/flatpak-rs/issues/23), not update implementation,
completed VM acceptance or machine-power-loss testing. Behavioral specification:
[#22](https://github.com/razzeee/flatpak-rs/issues/22). Concrete decisions are in
`local-update-protocol.md` and `local-update-storage.md`.

## Evidence obtained on 2026-10-26

Reproducible primitive probe: `validation/local-update-primitives.py`. It creates
isolated repositories/deployment directories under a supplied work root; it does
not connect to a service, modify an installation or implement update recovery.
Dependencies: reference Flatpak/OSTree CLI, Python PyGObject OSTree typelib,
pkg-config and a Linux libc exposing renameat2. Successful runs leave `report.json`
and their independent fixture/object trees for inspection.

Commands (relative to the repository):

```sh
python3 docs/validation/local-update-primitives.py /tmp/opencode
python3 docs/validation/local-update-primitives.py /home/razze/.cache
```

Both final runs exited 0, without the earlier fork warning. Environment: kernel
`7.2.11-200.fc44.x86_64`, libostree `2026.4`, Flatpak `1.18.4`.

| Work directory | Filesystem | Result |
| --- | --- | --- |
| `/tmp/opencode/local-update-primitives-zamf0ugc` | tmpfs | All 8 pin checks and 3 process-kill exchange observations passed |
| `/home/razze/.cache/local-update-primitives-0arf6tp8` | Btrfs | All 8 pin checks and 3 process-kill exchange observations passed |

The tmpfs run selected old app commit
`e74552214492aa434246f4e3204aaf034125b9c9fa517f05ee5d6e80f16606aa`;
the Btrfs run selected
`f1e2de2dbf20eb7f222c6d3eab768afd60526a49ce5f771783cbf65e7a529539`.
Both selected separate-collection runtime commit
`091dde20a83c50b622db85a47d16aac92c0cc95e3b5fef5ab3acf7101bc1a374`.
Commit values may differ on rerun because fixture export timestamps participate
in commit checksums; each probe compares its own captured exact pins.

| Probe | Observed behavior |
| --- | --- |
| App ref moved to a new commit, old object remains | `collection-refs` imported exactly the old pin |
| App ref removed entirely, old object remains | Same old pin imported without resolving a live ref |
| Runtime exported in a separate collection | Exact runtime pin imported and explicit binding checks passed |
| Same app pin submitted as another branch | libostree rejected missing requested ref in binding metadata |
| Same app pin submitted as runtime kind | libostree rejected the runtime ref binding |
| Requested collection differs from commit/source collection | libostree import returned success; explicit post-import collection-binding validation rejected it |
| All-zero missing commit pin | libostree rejected unavailable commit, without selecting replacement content |
| Commit deliberately written with no ref bindings | libostree rejected missing binding information |
| Plain rename over populated deployment | Rejected as nonempty; it is not a replacement mechanism |
| Child killed before exchange | Published app/runtime/selection all old; candidate all new |
| Child killed immediately after exchange | Published app/runtime/selection all new; slot all old |
| Child killed after both parent syncs | Same complete new published pair and old slot |

Exploratory corrections are not hidden: the first probe attempt used the wrong
PyGObject `write_config_and_reload` arity and stopped before importing. After
correcting that invocation, the collection mismatch test disproved the assumption
that a `collection-refs` tuple alone enforces the caller's collection. The final
probe records the explicit validation layer and the production contract requires
both source-config and commit-metadata checks. The original fork-based exchange
child emitted a multi-threaded-process warning after GIO imports; the final probe
uses a fresh subprocess instead.

### What this does not establish

- No planning/apply method, production exchange/recovery implementation or fault
  injection has been added to the service.
- The process-kill probe uses tiny populated directories and actual exchange, not
  the service's complete transactions. It does not prove the recovery algorithm,
  confinement after update, disk exhaustion handling or physical persistence.
- Runtime host probing used libostree 2026.4. The repository's pinned nixpkgs selects
  2026.2; its relevant source was separately checked below, but that library/VM
  combination was not executed for this preparation. The previous temporary
  `/tmp/opencode/nix-portable` wrapper is absent, so no new pinned Nix/VM run is
  claimed. Cached binaries/results are not fresh update acceptance evidence.
- Actual source checksums/corruption, file-copy behavior and sandbox enforcement
  must still be tested through the implemented service. Actual storage damage is
  not repaired merely by retrying synchronization.

## Primary-source/API validation

Retrieved/read for this preparation:

1. [Pinned nixpkgs OSTree package](https://github.com/NixOS/nixpkgs/blob/02c087f3fe673c9e317f206a627cb6b910336dbe/pkgs/by-name/os/ostree/package.nix)
   selects libostree **2026.2**. This ties API analysis to the actual repository
   dependency rather than only the desktop library.
2. [libostree 2026.2 pull options](https://github.com/ostreedev/ostree/blob/v2026.2/src/libostree/ostree-repo-pull.c#L3421-L3493):
   `collection-refs` is `a(sss)` with nonempty checksum to request an exact commit;
   it excludes `refs` and `override-commit-ids`. The option parser carries specified
   checksums directly instead of requiring the source ref to resolve.
3. [Binding verification in the 2026.2 pull path](https://github.com/ostreedev/ostree/blob/v2026.2/src/libostree/ostree-repo-pull.c#L1543-L1562)
   calls `_ostree_repo_verify_bindings` with source collection and requested ref,
   or no ref for checksum-only pulls.
   [Local collection lookup](https://github.com/ostreedev/ostree/blob/v2026.2/src/libostree/ostree-repo-pull.c#L1329-L1348)
   uses the file source repository's collection rather than proving equality with
   the caller's tuple. This explains the explicit-validation requirement.
4. [libostree 2026.2 binding verifier](https://github.com/ostreedev/ostree/blob/v2026.2/src/libostree/ostree-repo.c#L6624-L6698)
   reads `ostree.ref-binding` as a string array and collection binding as string;
   checks requested-ref membership and collection equality. Missing ref bindings
   are not universally rejected when no source collection is present, another
   reason for explicit required bindings in our source validation.
5. [Local UNTRUSTED import flags](https://github.com/ostreedev/ostree/blob/v2026.2/src/libostree/ostree-repo-pull.c#L3947-L3961)
   avoid the local trusted-import fast path. Keep the existing untrusted private
   import policy; local transport does not justify trusting checksums.
6. [libostree checkout file synchronization](https://github.com/ostreedev/ostree/blob/v2026.2/src/libostree/ostree-repo-checkout.c#L119-L183)
   uses the checkout `enable_fsync` option and syncs regular-file contents. The
   service contract additionally owns final selection records, permission order,
   directory entries and post-exchange parent synchronization.
7. [Linux rename(2)](https://man7.org/linux/man-pages/man2/rename.2.html): plain
   directory replacement needs an empty destination; `RENAME_EXCHANGE` atomically
   swaps two existing entries including nonempty directories; both must be on the
   same mounted filesystem. Unsupported flags and cross-mount errors need explicit
   handling. Its NFS failure caveat is not a supported local-transaction fallback.
8. [Linux fsync(2)](https://man7.org/linux/man-pages/man2/fsync.2.html): syncing a file
   does not necessarily sync its containing directory entry; explicit directory
   sync is required. Errors include EIO and ENOSPC and cannot be reported as commit
   success. These API guarantees are not a machine-power-loss test result.

## Host-controlled fault-injection contract

Future implementation snapshots these settings from the host service environment
at installation construction; they are never accepted in requests/plans/artifacts
or inherited from the execution client's application environment. No test settings
means normal production behavior. There is no public transaction-control API.

- `FLATPAK_RS_UPDATE_STOP_AFTER=BOUNDARY` logs a bounded checkpoint then SIGSTOPs
  the management process for the external harness to kill or resume. Boundaries:
  `imported`, `prepared`, `retained`, `intent`, `exchanged`, `parents-synced`,
  `committed`, `cleanup-started`, `cleanup-partial`.
- `FLATPAK_RS_UPDATE_FAIL_AT=SITE` injects one failure at that named site per service
  process. Sites: `write-selection`, `sync-selection`, `sync-payload`,
  `sync-retained`, `sync-intent`, `exchange`, `exchange-result`, `sync-installed-parent`,
  `sync-slot-parent`, `mark-garbage`, `sync-active-parent`, `sync-garbage-parent`,
  `delete-obsolete`, `sync-cleanup`. Apply these at the actual adapter/filesystem
  calls, not by pretending the entire installation method failed.
- `FLATPAK_RS_UPDATE_FAIL_KIND=EIO|ENOSPC` selects the injected errno (default EIO).
  At `write-selection`, include a deliberately short write followed by error to
  exercise incomplete private records. Recognized values form closed enums; invalid
  configurations fail update setup rather than silently picking an arbitrary site.
- `FLATPAK_RS_UPDATE_TEST_PHASE=apply|recovery` scopes both hooks (default apply).
  Recovery includes reconciliation triggered by a later request, not just startup.
  Empty journal initialization is not a checkpoint. Commit-parent sync site names
  refer only to the post-exchange bookkeeping calls, not initial directory creation.
  Thus startup/initialization cannot accidentally consume a fault meant to happen
  after publication. Planning is not subject to these apply/recovery hooks.
- Retry tests unset hooks before restarting, except explicit repeated-failure cases.
  Hooks also cover reconciliation calls at the matching sites, so persistent sync
  failure demonstrates recovery stays blocked, not falsely ready. Record which
  call fired, especially where first-use preparation and recovery share a name.

`exchange` fails before invoking the syscall; `exchange-result` performs the real
exchange then injects an error result. The latter verifies conservative outcome
classification and evidence preservation if an I/O failure obscures whether the
visibility point was crossed; it must never trigger deletion as unpublished state.

Use a small disposable ext4 filesystem in the VM for actual disk-exhaustion tests,
mount it as service-owned XDG data storage and keep local sources outside it.
Fill free space with a separate host-controlled file while stopped at `imported`
or `prepared`, then resume the real write/checkout/retention. Confirm an actual
ENOSPC/EDQUOT or documented libostree reserve refusal, distinguishing these from
the injected errno cases. To specifically exercise ENOSPC rather than just the
reserve check, fill after import/reserve checks and before subsequent checked
writes. Free the filler and retry/recover; never fill the host's real home or the
unrelated VM root. Keep the default service security policy intact.

## Required real-service acceptance matrix (not yet run)

Primary seam: CLI → socket-activated per-user service → real libostree/filesystem
and confined payload. Fixtures come from independent reference exports with distinct
app/runtime payload markers and collection IDs. Supplemental direct Varlink
clients test malformed requests and transport losses. Inspect/run/data/denied
access and public JSON are assertions; private paths are only fault synchronization
mechanics, never the public success criterion.

| ID | Scenario and external assertion | Owning ticket |
| --- | --- | --- |
| P1 | Plan/save/reload includes full expected/desired pins and two explicit sources; inspect/data unchanged | #24 |
| P2 | Planning while an execution lease exists does not reserve or publish; later valid operations can change selection | #24 |
| P3 | Fresh planning resolves moved refs; optional exact commits select available older objects | #24 |
| P4 | Malformed/extra/duplicate fields, versions, oversized/deep JSON and noncanonical commits fail without state mutation | #24, #25 |
| A1 | App-only apply returns updated complete pair; inspect and payload agree; other variant/data unchanged | #25 |
| A2 | Missing and active/draining targets reject even desired=no-op and absent sources | #25 |
| A3 | Eligible desired=current returns unchanged before stale-expected/source checks; stale real replacement conflicts before prep | #25 |
| A4 | Source refs moved/deleted but pins available apply exactly those pins; missing/corrupt/incompatible pins never fall back | #25 |
| A5 | Wrong ref/kind/architecture/name/collection, missing/mistyped bindings, edited identities, unsafe paths and hooks fail before publication | #25 |
| R1 | Runtime-only/combined/older pairs inspect and run correctly, including separate-provider sources | #26 |
| R2 | Retention failure preserves old pair; exact old runtime remains listed until explicit removal; installed dependents reject removal | #26 |
| R3 | Older content preserves newer data, without claiming data compatibility, migration or rollback | #26 |
| O1 | Disconnect after accepted apply and lose replies after commit; inspect and same-plan retry resolve updated/no-op | #27 |
| O2 | Actual client read-error path reports uncertain and inspect guidance; production timeout constants remain 300/15 seconds | #27 |
| O3 | Sync-installed/slot/commit-bookkeeping failures return uncertain, keep evidence and force same-process reconciliation before next storage op | #27 |
| O4 | Deletion/sync-cleanup failures after commitment return updated with bounded cleanup warning; repeated restart cleanup safe | #27 |
| C1 | Two plans with same expectation and different desires: one updates, next conflicts; same desired retry no-ops | #28 |
| C2 | Launch/uninstall/data deletion/runtime removal during preparation waits then observes completed outcome; leases/dependents remain protected | #28 |
| F1 | Kill at imported/prepared/retained/intent: old pair remains runnable; incomplete candidates cleaned, data unchanged | #25, #28 |
| F2 | Kill immediately after exchange, between parent syncs, during garbage transition: recover complete new pair, never exchange back | #25, #27, #28 |
| F3 | Kill during partial garbage deletion, then update/uninstall the variant and restart again: old garbage cleanup cannot touch current content | #27, #28 |
| F4 | Write/short-write/sync/ENOSPC before exchange preserve old selection; post-exchange required-sync failures uncertain; post-commit deletion failures warn | #28 |
| F5 | Actual small-filesystem exhaustion at checkout/record/retention; free space, recover and retry pinned plan; unrelated installation remains usable | #28 |
| F6 | Unsupported exchange, cross-mount and symlinked service-owned directories fail safely without two-step fallback or escaped cleanup | #25, #28 |
| F7 | Inconsistent/malformed sealed active state fails closed with evidence intact; repeated fault-free recovery is idempotent | #25, #28 |
| S1 | Updated unknown/known-unavailable permissions grant no access; required-isolation failures still deny launch | #25, #26, #28 |
| S2 | Existing full ownership/owner-death/draining, foreground, security, lifecycle and ordinary Rust/workflow checks pass | Every implementation slice; full recorded evidence #28 |

For O2, a transport-only CLI test can supply a short timeout to the private client
request helper and execute its real read-error/diagnostic path. Direct Varlink tests
can also disconnect at a service checkpoint. Neither is a substitute for real
storage acceptance; production timeout values remain fixed, with no new external
timeout-control knob or requirement to make every VM case wait five minutes.

## Completion boundary

Preparation specifies concrete mechanisms and validates the critical API/syscall
choices against sources and isolated probes. Each implementation slice must still
prove its behavior at the agreed seam before landing. No feature code, interface
file, service fault hook, installation layout or CI definition changes in #23.
No broad prefactor is needed: the configured installation handle already owns
serialization/recovery/leases. Escalate any implementation finding that invalidates
these choices instead of weakening the approved behavior silently.

Native `cargo test --locked` also passed during preparation (the existing CLI and
real-service protocol integration tests). Probe syntax, the protocol's JSON plan
example, local document pointers and `git diff --check` were checked. These do not
substitute for the unrun update VM matrix above.
