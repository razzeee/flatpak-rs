# Implement sandbox setup directly and defer appd integration

The initial sandbox will use namespace and mount setup implemented directly in Rust rather than invoking Bubblewrap, making isolation implementation and verification central to the demonstrator. The first milestone will work without appd and defer host-service access that requires trustworthy application identity. Internal launched-instance bookkeeping is not a replacement for appd authentication.
