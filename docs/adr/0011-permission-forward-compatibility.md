# Permit launch with unavailable permissions

Unknown permission declarations are ignored with a diagnostic and grant no access; known permissions that cannot be provided likewise leave access denied without blocking installation or launch. We reject a required/optional permission classification so that adding declarations does not itself prevent apps from launching on older implementations, while apps remain responsible for handling unavailable capabilities. This does not permit sandbox exceptions, weaken fail-closed sandbox setup, or require accepting unsupported artifact structures.
