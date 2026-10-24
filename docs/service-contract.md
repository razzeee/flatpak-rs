# Experimental management service contract

This specifies issue #2 before implementation. It refines ADRs 0002 and 0005:
systemd user socket activation settles the previously undecided lifetime.

## Authentication and descriptors

Trust the kernel, systemd, and unsandboxed processes of the owning account,
as in the initial design. The service never runs as root. systemd creates
`%t/flatpak-rs/management.sock` with mode 0600 in a mode-0700 directory.
The service accepts exactly one AF_UNIX stream listener via LISTEN_PID/FDS,
validates its address, owner, and permissions, and never binds a fallback socket.

Before reading requests, obtain SO_PEERCRED and SO_PEERPIDFD (Linux >= 6.5).
Require the peer UID to equal the service UID and the peer's user, mount, PID,
and network namespace identities to equal those of the service. Open the peer
proc directory and check the pidfd for exit both before and after namespace
inspection; a dead peer or any inspection error is denied. The kernel-provided
pidfd pins the connecting process, not a later process reusing its PID.
Namespace identities here define *host-side*, not application identity.
The socket must remain absent from future app mounts and descriptor tables;
this check is defense in depth, not permission to expose it to apps.
Do not enter private namespaces in the service unit, since these checks compare
with the service. A peer can delegate its connection to another process; protecting
against deliberate delegation by trusted host-side same-user code is out of scope.

The management listener, accepted streams, pidfds, and proc handles are
close-on-exec (standard input/output/error are deliberately preserved). No descriptors are
received/transferred by this protocol; ancillary descriptors are discarded by
ordinary stream reads. No application execution is implemented in this ticket.

## Wire and failures

Use NUL-terminated Varlink JSON messages. Expose experimental
`org.flatpakrs.Manager.GetHostStatus() -> (ready: bool, checks: []Check)` and
the standard `org.varlink.service` discovery methods. Each check has `name`,
`available`, and `detail`. Unknown methods/interfaces and invalid parameters
use standard Varlink errors. Reject malformed JSON, unsupported streaming/
upgrade flags, and messages over 64 KiB. Each connection has a bounded idle
timeout; the service handles clients concurrently with a bounded connection count.
An unauthorized peer is disconnected before dispatch (no privileged operation).

Checks report Linux user-namespace creation (in a short-lived child), systemd
user-manager reachability, cgroup v2, and delegated memory/pids controller
availability. Probe cgroup creation and limits in a disposable child cgroup,
never move the service or modify ancestor limits. Remove the probe afterwards.
The empty probe uses 64 MiB and 16 processes for readback only; these are not
application limit defaults, and no process is moved into it.
These are prerequisites only: `ready` is NOT proof that an app sandbox can be
constructed. Future launch must establish every isolation step or fail closed.
The CLI prints structured JSON; missing capabilities return status 2, connection/
protocol errors return status 1 with an actionable diagnostic.

## Acceptance scope

The pre-agreed primary seam is the CLI plus the socket-activated real service in
a NixOS VM. Direct Varlink clients exercise malformed requests, same-UID peers
in another namespace, and foreign-UID peers (including bypassing socket mode
in a root-controlled test). Ordinary Rust checks supplement this seam.
No Flatpak 1 tests are reused in this ticket; lifecycle, permission compatibility,
artifact handling, and sandbox tests belong to subsequent tickets. No installation
or execution permissions are interpreted or granted here.
