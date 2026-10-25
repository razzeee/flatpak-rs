# Confined execution, supervision and lifecycle contract (#5–#10)

This refines the approved design before implementing execution. It grants no
permission-derived access and provides no alternate/unconfined execution path.
The initial tested architecture is x86_64; unsupported syscall architectures
must fail before application execution.

## Setup and authority

The authenticated host-side service claims a variant under the same lock used
by installation/deletion. A lease survives until complete instance cleanup.
Separate variants have independent leases, persistent homes and cgroups.
Uninstall and data deletion reject a running variant. Missing data deletion is
idempotent; deleting retained data does not uninstall an application.

Re-exec a dedicated, single-threaded Rust setup helper, not fork application
setup from the multi-threaded management service. Only standard pipes and a
private configuration pipe reach that helper; close every other descriptor
before re-exec. The configuration pipe is closed before the app starts.

Before releasing configuration to the helper, the service enrolls it in its
instance cgroup and verifies membership. This startup barrier prevents cancellation
from killing an empty group before a late-joining helper starts. The helper also
checks its enrollment before creating any app process. Require
delegated cgroup v2 memory/pids controllers and cgroup.kill. Default limits are
256 MiB, 64 processes, no swap, and group OOM killing; explicit host-side run
requests may select 16–1024 MiB and 4–256 processes. Read limits back. There is
no advisory-only fallback. Instance cgroups stay inside the systemd service's
control group so KillMode=control-group also cleans up after service death.
Reject compulsory `vm.panic_on_oom=2`: that host policy panics even for a bounded
instance's cgroup OOM. The acceptance VM explicitly uses normal scoped OOM
killing (`vm.panic_on_oom=0`), never an unlimited-memory workaround.

Create user, mount, PID, network, IPC, UTS and cgroup namespaces directly using
Linux syscalls. Map one host UID/GID, deny setgroups, and make mounts private.
A namespace PID 1 owns the application process tree: its exit kills remaining
namespace processes. The external setup parent waits for it. No appd identity
or app-facing host services are provided.

The private `execution::instance` module owns helper startup/enrollment, cgroup
limits, and the installation lease together. Its interface has three transitions:
`Instance::start` supplies only the deliberate pipes; consuming `wait` verifies
process-tree cleanup and returns a `Completion` that still owns the variant;
consuming `Completion::finish` releases ownership after transport draining and
before the final exit reply. Early returns use the same scope-owned cleanup
path, retaining the lease if the cgroup cannot be verified empty. The connection
module owns framing, input credits, and peer-death monitoring through final reply
teardown; it does not manipulate the cgroup, helper, or lease directly. The
CLI/Varlink interface and its acceptance-test seam are unchanged.

## Filesystems, capabilities and environment

Construct a fresh tmpfs root in the private mount namespace. Bind only prepared
app/runtime files to /app and /usr, read-only with nosuid/nodev. Bind the
variant's persistent home at /home/user. Supply private bounded /tmp, a fresh
PID-namespace /proc (read-only, nosuid/nodev/noexec), and only null, zero, random
and urandom in /dev. Do not mount sysfs, cgroupfs, host /run, host /etc or home,
service sockets, GPU/audio/input devices, or the management socket. Device
bindings are nosuid/noexec; /dev becomes read-only after setup. Standard streams
are service-owned pipes, never arbitrary client-transferred descriptors.

Pivot into the new root and detach the old root. Close all non-standard
descriptors, drop the capability bounding/effective/permitted/inheritable and
ambient sets, lock securebits, and set no_new_privs. A fixed environment contains
HOME, PATH, USER/LOGNAME, LANG, TMPDIR and variant-local XDG directories only.
Permission metadata does not modify mounts, environment, namespaces or policy.

## Syscall policy

Install a native-architecture-checked seccomp BPF allowlist, default EPERM.
Allow ordinary file and memory operations, signals, clocks, process/thread
bookkeeping, pipes, polling, exec and wait needed by the static fixtures.
Allow only reviewed ordinary process/thread clone flags and a zero high word,
excluding namespace creation and unreviewed future flags; return ENOSYS for
clone3 so libc may use checked clone. Forked descendants inherit the filter.
Do not allow socket/connect/bind, unshare/setns, mount APIs, ptrace/process_vm,
kernel keyrings, bpf/perf, io_uring, userfaultfd, handle-based file opens,
modules, reboot or kexec. The source allowlist is the exact implemented policy;
new syscalls require explicit review rather than a permissive fallback.

This policy has been checked against the design: it permits the fixture's file,
stream, fork and private-data operations while denying every named host-access
category. Namespace/mount/capability/descriptor restrictions remain required
even where the syscall policy independently denies access. VM probes, not this
analysis or prerequisite reporting, establish successful enforcement.

## Foreground protocol and cleanup

Run uses a Varlink streaming request (`more: true`). Bounded hexadecimal chunks
carry stdout/stderr in continued replies and stdin in connection-scoped oneway
WriteInput calls. Each input chunk consumes one credit; `started` and
`input-ready` replies supply the next credit after the pipe writer consumes it.
A bounded queue and independent control reader keep Cancel/disconnect observable
even when the app refuses stdin. CloseInput conveys EOF without disconnecting. Cancel is
connection-scoped: it cannot target another client's instance. No descriptors
are transferred; ancillary descriptors are discarded rather than inherited.
The final reply reports exit status. No terminal allocation or background runs.

Use bounded pipe/socket buffers, concurrent input/output pumps, bounded socket
retry intervals and nonblocking stdin writes. Preserve partial frame progress
across output stalls; a healthy slow consumer must not lose bytes. Cancellation
or disconnect interrupts stalled writes; aborting a partial frame closes the
connection instead of emitting a corrupt subsequent frame. The control reader
remains live through output draining after app exit. Output failure or connection EOF cancels the run.
The connecting process's pidfd is monitored independently of helper waiting,
through output draining and final reply teardown. Its death cancels stalled
writes even if a different host process retains a socket copy and prevents EOF.
CLI SIGINT/SIGTERM requests cancellation; closed stdout/stderr disconnects.
Host output uses a bounded worker and cancellation-aware completion waits so
backpressure cannot trap the CLI or change the parent's descriptor flags. The
CLI exits without joining a stalled worker; it uses raw owned files, never global
Rust stdout/stderr locks that process-exit flushing could wait on. Inherited
output authority and file offsets are preserved. Standard descriptors are pinned
before connecting, preventing closed stdin from becoming a management socket.
Signal the instance for a bounded graceful period, then use cgroup.kill and
verify it is empty before releasing ownership. Never wait indefinitely for a
descendant holding a stream. A cleanup failure must not permit a duplicate run.

## Deletion and failure testing

Uninstall first retains a complete runtime copy, then atomically unpublishes the
application. Runtimes can be explicitly removed only when no published app
references their variant. Persistent data is separately retained until explicit
delete-data. All deletion walks unlink symlinks instead of following them.
No deletion operation takes a caller-supplied filesystem path.

The agreed VM seam must test allowed execution and denied filesystem, socket,
device, environment, capability, namespace and syscall access; stdin/stdout/
stderr/exit status; large/blocked streams and cancellation; memory/pids limits;
independent variants; service death; retained/deleted data and runtime dependency
protection. Disable actual kernel/systemd mechanisms to test fail-closed behavior
(namespace sysctl, mount/seccomp syscall filters, cgroup delegation). Missing
prerequisites are failures, never skips or passing isolation tests.
