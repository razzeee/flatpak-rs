# Experimental remote installation contract

## Remote authorization

`remote-add NAME COLLECTION DESCRIPTOR` explicitly authorizes the OpenPGP keys
in a Flatpak repository descriptor. DESCRIPTOR is a bounded local UTF-8 file or
an HTTPS URL. `remote-replace` uses the same arguments and explicitly replaces
an existing authorization. `remotes` inspects configured providers;
`remote-disable NAME` withdraws future-use authorization without changing apps.
The service receives descriptor contents, not a caller-controlled config path.

Descriptors and requests are bounded by the existing 64 KiB message limit.
Names/collections use bounded identity components. Repository URLs must be HTTPS
(HTTP loopback is permitted for deterministic local test servers). TLS validation
is mandatory. Descriptor redirections must not downgrade HTTPS. Repository
redirects cannot change the trusted signing key or collection.

The service imports keys into a new isolated libostree repository for every
authorization generation, verifies the remote's signed summary, and records
key fingerprints, URL, collection, enabled state, and an opaque generation.
No collection-id option is set on the libostree remote: summary verification and
commit verification both remain enabled; collection/ref binding is checked
separately. A signed summary authenticates discovery, not freshness. Selected
commits must also have a valid signature under that generation's keyring.

Configuration publication is durable and atomic. Failed replacement leaves the
old authorization intact. Remote authorization has a separate mutex from long
installation work; disabling/replacement can complete during a download. Apply
captures provider generations, performs work, then holds the authorization mutex
while checking the currently enabled generations and publishing the selection.
Thus a completed disable prevents any still-unpublished work from publishing.
If publication wins the lock first, disable affects subsequent work. Old cache
and keyring directories are not authority; current configuration is.

Management methods are AddRemote(name, collection, descriptor, replace),
ListRemotes(), and DisableRemote(name). OperationFailed returns bounded diagnostic
text; malformed shapes use Varlink InvalidParameter. No request can disable
verification or automatically accept a new key. Duplicate AddRemote names fail
unless replacement was explicitly requested.

## Installation plans

`plan-install REMOTE APP ARCH BRANCH --runtime-remote REMOTE` produces a JSON
plan with format_version=1, complete app and runtime revisions, and app_provider
and runtime_provider objects containing remote name and authorization generation.
Collections are the explicitly configured provider collections; runtime ID/branch
come from the selected app's signed xa.metadata commit attribute. Signed summary
entries resolve refs to commits; COMMIT_ONLY pulls retrieve and verify pinned
commit metadata before full payload download. Verify collection/ref bindings
and signature even when objects are cached. Unsupported/missing signed artifact
metadata fails clearly rather than downloading unspecified content to guess.

`apply-install PLAN_FILE` accepts at most 32 KiB of strict, duplicate-free JSON.
It validates format, identities, pins, and provider generations first. If the
variant already has exactly that pair installed, it reports `unchanged` without
requiring source availability; this observes an existing installation rather than
authorizing new work, including after the remote was disabled. A different pair
fails rather than becoming an update. Otherwise both providers must still be
enabled at the planned generations, throughout the final publication boundary.

Apply imports exact pinned objects, verifies signatures/bindings and actual tree
metadata, prepares both checkouts, then publishes under the authorization mutex.
It returns `installed` or `unchanged` with the complete installed selection.
PlanInstall returns a `plan` wrapper on Varlink, unwrapped by the CLI. Methods
PlanInstall(remote, id, arch, branch, runtime_remote) and ApplyInstall(plan) use
the existing bounded framing. Malformed requests and operation failures are
explicit; failure to synchronize after publication uses RemoteOutcomeUncertain.
CLI transport/read/acknowledgement failures on apply direct inspection/retry of
the same plan. Accepted work is not cancelled by client loss. Read timeout is
300 seconds, write timeout 15 seconds, not whole-operation deadlines.

Download caches use archive repositories to preserve publisher file metadata
without requiring filesystem extended attributes. Preparing a deployment imports
from that cache with content verification before producing user-owned checkouts.
HTTP object requests identify the client as `flatpak-rs/VERSION`: Flathub returns
403 for the generic `libostree/VERSION` user agent. Signature, collection/ref and
content verification remain mandatory.

Host-controlled acceptance faults use `FLATPAK_RS_INSTALL_STOP_AFTER=prepared`
or `published` for process death at the publication boundary, and
`FLATPAK_RS_REMOTE_FAIL_AT=write-selection|sync-selection|sync-publication`
for ENOSPC/EIO at actual write/synchronization sites. Failure after the atomic
rename is reported as RemoteOutcomeUncertain. Artifacts and management requests
cannot enable these faults. The independent HTTP fixture can pause content
delivery to test client loss and concurrent authorization withdrawal.

`FLATPAK_RS_INSTALL_STOP_AFTER=remote-checkout` pauses after preparing both
checkouts but before writing the selected pair. The VM uses this point to fill a
separate ext4 installation filesystem, verify a real ENOSPC failure, and retry
after freeing space. This host-only checkpoint does not change request semantics.
The extended remote acceptance also exercises the CLI's actual 300-second read
timeout with a transport proxy withholding a successful real-service reply;
production timeouts are not shortened for the test.

## Bounded Calculator compatibility profile

Calculator locale/debug declarations and GNOME Platform 51's locale, GL/debug,
Vulkan layers, timezones, GStreamer, icon/GTK3 themes, VAAPI drivers and extra
codecs may be omitted at their explicitly recognized directories. Each omission
is reported. These supply localization, development assets, acceleration,
multimedia or optional presentation data outside the basic software-rendered
calculation workload. Unknown extension names, directories or structural keys
still fail. This does not install extensions or process their environment changes.
