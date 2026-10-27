# Explicit Wayland execution

Omitting `-- COMMAND` launches the installed app's verified `Application.command`
metadata. It must be a single bounded executable basename: the service constructs
`/app/bin/NAME`, with no shell interpretation, argument splitting or host PATH
lookup. Malformed/missing default commands fail clearly. Explicit command
overrides retain the existing foreground protocol; an empty command vector on
Run requests selects this metadata-based default.

`run COLLECTION APP ARCH BRANCH --wayland NAME -- COMMAND` opts into a named
Wayland endpoint under the service's XDG_RUNTIME_DIR. NAME must be a single
bounded filename beginning with `wayland-`; paths, symlinks, non-sockets and
sockets owned by another UID are rejected. The service pins the socket inode
using an O_PATH descriptor. After creating its mount namespace, the helper reopens
the path and checks device/inode equality against the pinned descriptor, then
bind-mounts the new descriptor at a fixed private runtime path. This is required
because Linux rejects a bind source referring to a foreign mount namespace.
Replacement before reopening fails validation; replacement after reopening cannot
affect the descriptor-based mount. The host pathname cannot substitute another
endpoint after validation; compositor termination causes ordinary connection
failure, never fallback. Unsandboxed processes of the same host user remain trusted.

The helper receives the pinned descriptor as fd 4 solely for setup. It closes
that descriptor and every other unintended descriptor before payload execution.
Only the fixed socket is mounted, never the host runtime directory. The compositor
is a trusted resource provider; Wayland access is not a guarantee against every
compositor protocol operation or abuse. No app-facing management authority is added.

Graphical mode supplies a fixed private XDG_RUNTIME_DIR and WAYLAND_DISPLAY,
GDK_BACKEND=wayland and GSK_RENDERER=cairo. It allows AF_UNIX socket creation and
the IPC/shared-memory calls required for Wayland; network-family socket creation
remains denied. The private network namespace also isolates abstract Unix sockets
from the host. No GPU, X11, host session bus or host home is mounted. Baseline
non-graphical execution keeps its existing syscall/environment policy.

The graphical environment searches libraries in `/app/lib` and the runtime's
x86_64 library directory, data in `/app/share:/usr/share`, and font configuration
in `/usr/etc/fonts`. GSettings uses variant-private keyfile storage. None of these
settings is taken from the host environment or grants host filesystem access.

Failure to establish these mounts, descriptors or the syscall policy fails the
launch. Foreground ownership, cgroup limits, private data, and cleanup are unchanged.
