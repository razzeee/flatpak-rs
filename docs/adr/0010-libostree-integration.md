# Use libostree behind the storage boundary

Production storage operations will use libostree through Rust bindings or a narrow FFI wrapper rather than shelling out to the OSTree executable. This accepts a native library dependency and an FFI boundary in exchange for structured integration with an existing OSTree implementation. Reference CLI tooling remains useful for preparing independent fixtures.
