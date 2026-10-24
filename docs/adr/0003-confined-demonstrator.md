# Start with confined execution on a modern Linux host

The demonstrator will support confined execution with a deliberately small capability set, deferring unconfined execution so that the first execution path has explicit, tested isolation guarantees. It will target modern Linux with systemd and unprivileged user namespaces rather than broad initial distribution compatibility, using a reproducible test environment and reporting missing host capabilities clearly. Requiring appd, the precise isolation guarantees, and the sandbox implementation remain undecided.
