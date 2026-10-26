#!/bin/sh
set -eu
out=$1
mkdir -p "$out/app/files/bin" "$out/runtime/usr/bin" "$out/runtime/files"
cat > "$out/app/metadata" <<'EOF'
[Application]
name=org.example.Fixture
runtime=org.example.Platform/x86_64/stable
command=fixture
EOF
printf '#!/usr/bin/sh\necho fixture\n' > "$out/app/files/bin/fixture"
chmod +x "$out/app/files/bin/fixture"
"$FIXTURE_CC" -static -O2 "$FIXTURE_PROBE_SOURCE" -o "$out/app/files/bin/probe"
cat > "$out/runtime/metadata" <<'EOF'
[Runtime]
name=org.example.Platform
EOF
printf 'independently prepared runtime payload\n' > "$out/runtime/usr/payload"
# The VM supplies a static shell, so the fixture needs neither the host loader
# nor host libraries. This ticket prepares but deliberately never launches it.
cp "${FIXTURE_SHELL:?supply a static busybox fixture shell}" "$out/runtime/usr/bin/sh"
flatpak build-finish "$out/app"
flatpak build-finish "$out/runtime"
# These tiny disposable fixture repositories need a fixed reserve, not 3% of
# the build host's entire filesystem. Production repositories keep their policy.
for repo in repo other standalone; do
    ostree --repo="$out/$repo" init --mode=archive
    ostree --repo="$out/$repo" config set core.min-free-space-percent 0
    ostree --repo="$out/$repo" config set core.min-free-space-size 256MB
done
ostree --repo="$out/repo" config set core.collection-id org.example.Collection
ostree --repo="$out/other" config set core.collection-id org.example.OtherCollection
ostree --repo="$out/standalone" config set core.collection-id org.example.Standalone
flatpak build-export --disable-sandbox --collection-id=org.example.Collection "$out/repo" "$out/app" stable
flatpak build-export --disable-sandbox --runtime --collection-id=org.example.Collection "$out/repo" "$out/runtime" stable
ostree --repo="$out/repo" rev-parse app/org.example.Fixture/x86_64/stable > "$out/app-commit"
ostree --repo="$out/repo" rev-parse runtime/org.example.Platform/x86_64/stable > "$out/runtime-commit"
cp "$out/app/metadata" "$out/base-metadata"

# Unknown declarations (including familiar words in permission groups) must not
# be mistaken for structural artifact requirements. Known requests grant nothing.
cat >> "$out/app/metadata" <<'EOF'

[Context]
shared=network;future-subsystem;
sockets=session-bus;future-socket;
filesystems=home;
devices=dri;
extensions=future-permission;

[Future Permissions]
install-hook=not-a-structural-hook;

[Extensionless Permissions]
future-access=unavailable;
EOF
flatpak build-export --disable-sandbox --collection-id=org.example.Collection "$out/repo" "$out/app" permissions

# A real Flatpak structural requirement lives in commit metadata, not Context.
ostree --repo="$out/repo" commit --branch=app/org.example.Fixture/x86_64/extra-data \
    --tree=ref=app/org.example.Fixture/x86_64/stable \
    --add-metadata-string=xa.extra-data-sources=unsupported-download

export_app() {
    flatpak build-export --disable-sandbox --collection-id=org.example.Collection \
        --metadata="$out/base-metadata" "$out/repo" "$out/app" "$1"
}
export_app beta
flatpak build-export --disable-sandbox --runtime --collection-id=org.example.Collection "$out/repo" "$out/runtime" beta

for structure in extension hook; do
    cp "$out/base-metadata" "$out/$structure-metadata"
done
printf '\n[Extension org.example.Extension]\ndirectory=extension\n' >> "$out/extension-metadata"
printf 'install-hook=do-not-run\n' >> "$out/hook-metadata"
for structure in extension hook; do
    flatpak build-export --disable-sandbox --collection-id=org.example.Collection \
        --metadata="$out/$structure-metadata" "$out/repo" "$out/app" "$structure"
done

# A transport copy preserves identity. Another collection intentionally does not.
cp -a "$out/repo" "$out/mirror"
flatpak build-export --disable-sandbox --collection-id=org.example.OtherCollection \
    --metadata="$out/base-metadata" "$out/other" "$out/app" stable
flatpak build-export --disable-sandbox --runtime --collection-id=org.example.OtherCollection "$out/other" "$out/runtime" stable
flatpak build-export --disable-sandbox --collection-id=org.example.OtherCollection \
    --metadata="$out/base-metadata" "$out/other" "$out/app" shared-runtime
flatpak build-export --disable-sandbox --runtime --collection-id=org.example.Standalone "$out/standalone" "$out/runtime" stable

cp -a "$out/repo" "$out/update-app"
printf '\necho changed-app\n' >> "$out/app/files/bin/fixture"
flatpak build-export --disable-sandbox --collection-id=org.example.Collection \
    --metadata="$out/base-metadata" "$out/update-app" "$out/app" stable
cp -a "$out/repo" "$out/update-runtime"
printf 'changed runtime\n' >> "$out/runtime/usr/payload"
flatpak build-export --disable-sandbox --runtime --collection-id=org.example.Collection "$out/update-runtime" "$out/runtime" stable
ostree --repo="$out/update-app" rev-parse app/org.example.Fixture/x86_64/stable > "$out/update-app-commit"
ostree --repo="$out/update-runtime" rev-parse runtime/org.example.Platform/x86_64/stable > "$out/update-runtime-commit"

# Deliberately corrupt a payload in a valid bare repository without changing its
# advertised checksum. The service must not silently install this under that ID.
export_app corrupt
ostree --repo="$out/corrupt" init --mode=bare-user-only --collection-id=org.example.Collection
ostree --repo="$out/corrupt" config set core.min-free-space-percent 0
ostree --repo="$out/corrupt" config set core.min-free-space-size 256MB
ostree --repo="$out/corrupt" pull-local "$out/repo" app/org.example.Fixture/x86_64/corrupt runtime/org.example.Platform/x86_64/stable
payload=$(ostree --repo="$out/corrupt" ls -C app/org.example.Fixture/x86_64/corrupt /files/bin/fixture | awk '{print $5}')
prefix=$(printf '%s' "$payload" | cut -c1-2)
suffix=$(printf '%s' "$payload" | cut -c3-)
chmod u+w "$out/corrupt/objects/$prefix/$suffix.file"
printf 'corrupt payload\n' > "$out/corrupt/objects/$prefix/$suffix.file"

# Independent malformed inputs: reference Flatpak exporters correctly refuse
# some of these, so use the reference OSTree CLI for deliberate corruption.
mkdir -p "$out/symlink-tree/files"
ln -s /home/alice/untouched "$out/symlink-tree/metadata"
ostree --repo="$out/repo" commit --branch=app/org.example.Fixture/x86_64/symlink-metadata --tree=dir="$out/symlink-tree"
cp "$out/base-metadata" "$out/symlink-tree/metadata-file"
rm "$out/symlink-tree/metadata"
mv "$out/symlink-tree/metadata-file" "$out/symlink-tree/metadata"
ln -s /home/alice/untouched "$out/symlink-tree/files/outside"
ostree --repo="$out/repo" commit --branch=app/org.example.Fixture/x86_64/payload-symlink --tree=dir="$out/symlink-tree"
export_app unpublished
ln -s /home/alice/outside "$out/app/files/recovery-escape"
export_app recovery-prepared
export_app recovery-published

cp "$out/base-metadata" "$out/many-permissions-metadata"
printf '\n[Context]\n' >> "$out/many-permissions-metadata"
awk 'BEGIN { for (i = 0; i < 1500; i++) print "future-permission-" i "=unknown" }' >> "$out/many-permissions-metadata"
flatpak build-export --disable-sandbox --collection-id=org.example.Collection \
    --metadata="$out/many-permissions-metadata" "$out/repo" "$out/app" many-permissions
