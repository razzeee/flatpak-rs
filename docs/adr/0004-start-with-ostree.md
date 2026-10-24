# Start with existing OSTree artifacts

The demonstrator will consume a local OSTree repository containing a prepared app and runtime, reusing reference Flatpak tooling for fixture preparation and an existing OSTree implementation for reading artifacts. This favors reusable real-world fixtures over inventing a new artifact format, while keeping storage behind a narrow boundary to allow future OCI or disk-image backends. The choice of OSTree integration remains open.
