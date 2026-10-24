# Own installation and execution in a per-user service

The first implementation will place installation and execution behind a per-user service, with a CLI communicating over IPC, instead of starting with a linked library. This exercises the proposed Flatpak Next service boundary from the outset; a public libflatpak-compatible library is not an initial deliverable. The IPC protocol and service lifetime remain undecided.
