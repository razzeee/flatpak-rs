# Keep persistent data private to each application variant

Persistent app data will be isolated by collection, app ID, architecture, and branch, rather than shared automatically across branches or architectures. This avoids implicit writable-data coupling between independently runnable variants at the cost of duplicated data and explicit future migration or sharing work. Uninstall retains that data; deletion requires a separate explicit action.
