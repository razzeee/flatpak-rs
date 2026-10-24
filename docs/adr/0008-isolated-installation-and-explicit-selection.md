# Isolate installations and select revisions explicitly

The demonstrator will use project-owned per-user storage rather than modify existing Flatpak installations, selecting each application variant by collection, app ID, architecture, and branch and recording its exact commit. Runtime selection is explicit and validated against the application's declared requirement, avoiding ambiguous provider selection when existing metadata does not identify a runtime collection. This sacrifices automatic dependency resolution initially in favor of predictable selection and safe coexistence.
