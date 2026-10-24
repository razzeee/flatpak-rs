# Use Varlink for the experimental service interface

The CLI and per-user service will communicate using Varlink through a small, explicitly experimental lifecycle interface. This is our architectural choice: the talk proposes Varlink for next-generation portals, not explicitly for the Flatpak management service. Choosing it now exercises the service boundary without first establishing another IPC contract to replace.
