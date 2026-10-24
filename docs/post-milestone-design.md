# Beyond the initial milestone

This is a rough design direction, not a committed release plan or a finalized
Flatpak Next specification. The [initial milestone](design.md) is agreed; the
proposals below need focused design and validation before implementation. There
is no timeline, and the order can change as upstream interfaces become available.

## Architectural direction

The host-side CLI and, eventually, app stores communicate with the management
service over Varlink. The service owns installation transactions, resolves app
and runtime revisions, and asks a storage backend for the content needed to launch
an instance. Sandbox setup consumes that content without knowing how it was fetched.

App-facing services form a separate boundary. Apps request mediated access through
portals; identity and permission infrastructure lets the service providing a resource
enforce access. Apps must never gain access to the installation-management interface
as a side effect of enabling portals.

The long-term storage boundary should support an immutable, mountable deployment
with a defined lifetime, potentially exposed through file descriptors as proposed
in the talk. OSTree is the first backend; OCI and disk images are candidates, not
promised implementations. Avoid defining a universal backend API before a second
backend demonstrates which operations genuinely need to be shared.

## Constraints carried forward

- Collection plus app ID identifies an application; architecture and branch select
  its variant. Mirrors do not create new collection identities.
- Collection names alone do not authenticate publishers.
- Unknown or unavailable permissions grant no access and do not block installation
  or launch. Do not introduce required/optional permission categories. Applications
  must tolerate unavailable functionality; capability discovery and clear API errors
  should help them do so.
- Confined execution must remain confined. Failure to establish its isolation is
  an execution error, not a reason to silently grant access or run unconfined.
- Keep Flatpak 1 coexistence and compatibility choices explicit. Reused tests establish
  particular behavior, not blanket CLI or library compatibility.
- Variant-private persistent data remains the default. Any future sharing or migration
  needs explicit ownership and user-visible semantics.

## 1. Reliable installation lifecycle

Extend the local demonstrator with updates, rollback, repair, and storage reclamation.
An update should prepare and validate a new deployment before switching the selected
revision. Running instances must retain the revisions they actually launched with;
garbage collection must account for those references as well as installed apps and
runtime dependencies.

Define transaction cancellation, concurrent clients, crash recovery, disk exhaustion,
and progress reporting before exposing these operations to app stores. Preserve the
distinction between removing a deployment and deleting user data. Rollback of app
content does not automatically roll back data written by a newer app version.

Exit criterion: lifecycle and fault-injection tests demonstrate that failed operations
leave a usable prior state and do not delete content still in use.

## 2. Authenticated distribution

Add remote configuration, discovery, download, and verification. Separate a collection's
identity from transport locations and from the trust policy authorizing a publisher to
provide that collection. A mirror must not gain authority merely by claiming an ID.

Design key rotation, conflicting claims, stale or rolled-back metadata, and revocation.
Resolve runtime providers explicitly and retain provenance rather than guessing by
name. Decide whether to reuse existing Flatpak/OSTree trust metadata or introduce a
new contract before accepting network-distributed artifacts.

Exit criterion: verified downloads support the lifecycle above, and adversarial tests
cover wrong publishers, tampering, ambiguous identity, and interrupted transfers.

## 3. Identity, portals, and useful desktop apps

Integrate with appd or the upstream identity mechanism that actually emerges. Internal
instance bookkeeping cannot substitute for authenticated app identity. Validate identity
across process creation, subsandboxes, service reconnection, and instance teardown.
Do not assume proposed appd interfaces already exist or are stable.

Start desktop integration with a small end-to-end use case: a graphical app opening a
user-selected document. This requires decisions about compositor access, document
mediation, activation, and desktop exports. Add resource services incrementally:

- Audio playback separate from microphone access, with PipeWire-side enforcement.
- Network isolation distinguishing internet, local-network, and host-local access,
  including DNS policy and any port-specific access.
- GPU selection and mediated device access.
- USB, HID, and serial devices where a concrete app needs them.

Where services enforce policy directly, expose their endpoints only after establishing
reliable identity and enforcement. Socket availability alone is not authorization.
Permission changes need defined revocation semantics for existing connections and
already-issued descriptors; revocability must be demonstrated per resource.

The talk proposes peer-to-peer Varlink portals and service activation. Track upstream
work rather than assuming this project must implement the portal ecosystem itself.
Entitlements are another proposal to evaluate: any adoption must respect our rule
that unfamiliar permission declarations do not prevent launch.

Exit criterion: a real app uses a mediated resource, denial behaves correctly, and
cross-application identity confusion and resource-specific revocation are tested.

## 4. Broader execution and packaging support

Relax demonstrator limitations selectively: interactive terminals, multiple instances,
background apps, and processes surviving management-service restarts. Separate instance
supervision from a particular client connection before adding detached execution.

Evaluate extensions, foreign architectures, extra-data handling, and build tooling from
concrete use cases. Each introduces artifact, trust, or execution semantics beyond a
simple immutable app/runtime pair. BuildStream is a candidate from the talk, not a
decision to replace or implement flatpak-builder here.

Consider OCI or disk-image support once lifecycle and trust contracts can be preserved
across backends. System-wide installations require a separately designed privilege and
authorization boundary; the per-user management service must not simply become a
privileged service with its existing assumptions.

For X11-only apps, investigate per-app X servers and the compositor integration proposed
in the talk. Sharing the host X server would violate the confined-mode direction.

## 5. Explicit unconfined mode, if justified

The talk proposes unconfined execution with the selected runtime while retaining broad
host access. This remains a candidate feature, not an automatic next step. Confined
execution must never fall back to it when a permission or portal is unavailable.

Before adoption, decide who can authorize it, whether particular remotes may distribute
apps in that mode, how it is presented to users, and how data and identity relate to
confined variants. Preserve a clear distinction between runtime packaging and security
isolation. Broader portal coverage should reduce the need for this mode.

## Ecosystem research backlog

The talk also proposes richer document access, writing assistance, VPN integration,
password-manager/autofill APIs, and FIDO/passkey support. These are collaboration and
research topics rather than promises for flatpak-rs. VPNs in particular cross the
user-session/system boundary and need a dedicated authorization design.

## Testing and compatibility throughout

Keep an explicit mapping of reused upstream and black-box scenarios to supported
behavior, intentional differences, and unsupported functionality. Expand the target
adapter as interfaces mature, without teaching tests to hide meaningful failures.

Add regression cases for every new identity, storage, permission, or lifetime boundary.
Retain independent fixtures and reference implementations where useful. Before moving
from a demonstrator to a platform for untrusted real-world apps, conduct a focused
security review of sandbox setup, artifact processing, IPC, and authorization.

## Source and decision status

The [LAS 2026 talk](https://www.youtube.com/watch?v=NsVhkz2Xl0E), using the user-supplied
transcript, motivates identity/permissions (0:53–4:03), resource mediation and portals
(4:22–16:22), the service/storage/identity direction (18:21–20:55), confined/unconfined
execution (21:08–23:31), and Varlink portals (23:40–25:32). The speakers explicitly
state that there is no timeline (25:40 onward).

The sequencing, exit criteria, and operational details in this document are project
proposals. Accepted decisions remain in the initial design and `docs/adr/`; future
trade-offs should be recorded there when resolved rather than inferred from this roadmap.
