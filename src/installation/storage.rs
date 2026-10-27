//! libostree boundary: resolve pinned trees and prepare independent checkouts.
use super::{Result, Revision, Variant};
use gio::prelude::*;
use std::path::Path;

/// Atomic moves are confined to validated service-owned directories, never sources.
pub(super) fn rename_private(source: &Path, destination: &Path, exchange: bool) -> Result<()> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let open = |path: &Path| {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)
    };
    let old = open(source.parent().ok_or("missing source parent")?)?;
    let new = open(destination.parent().ok_or("missing destination parent")?)?;
    let a = std::ffi::CString::new(
        source
            .file_name()
            .ok_or("missing source name")?
            .as_encoded_bytes(),
    )?;
    let b = std::ffi::CString::new(
        destination
            .file_name()
            .ok_or("missing destination name")?
            .as_encoded_bytes(),
    )?;
    if exchange {
        if !std::fs::symlink_metadata(source)?.is_dir()
            || !std::fs::symlink_metadata(destination)?.is_dir()
        {
            return Err("atomic update requires real deployment directories".into());
        }
        // SAFETY: fstatfs fills a correctly sized output for an open directory.
        let mut stats: libc::statfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstatfs(new.as_raw_fd(), &mut stats) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if stats.f_type == 0x6969 {
            return Err("NFS is not supported for atomic local updates".into());
        }
    }
    // SAFETY: directory descriptors and NUL-terminated single-component names
    // remain live; the kernel performs the atomic exchange/non-overwriting move.
    if unsafe {
        libc::renameat2(
            old.as_raw_fd(),
            a.as_ptr(),
            new.as_raw_fd(),
            b.as_ptr(),
            if exchange {
                libc::RENAME_EXCHANGE
            } else {
                libc::RENAME_NOREPLACE
            },
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

pub(super) struct Source(ostree::Repo);

impl Source {
    pub(super) fn empty(destination: &Path) -> Result<Self> {
        let repo = ostree::Repo::new(&gio::File::for_path(destination));
        repo.create(ostree::RepoMode::BareUserOnly, gio::Cancellable::NONE)?;
        Ok(Self(repo))
    }

    pub(super) fn import_revision(
        &self,
        revision: &Revision,
        kind: &str,
        destination: &Self,
    ) -> Result<()> {
        if self.0.collection_id().as_deref() != Some(&revision.variant.collection) {
            return Err(
                "explicit collection does not match local repository collection identity".into(),
            );
        }
        let reference = format!(
            "{kind}/{}/{}/{}",
            revision.variant.id, revision.variant.arch, revision.variant.branch
        );
        let options = glib::VariantDict::new(None);
        options.insert(
            "collection-refs",
            &[(
                revision.variant.collection.as_str(),
                reference.as_str(),
                revision.commit.as_str(),
            )][..],
        );
        options.insert("flags", ostree::RepoPullFlags::UNTRUSTED.bits() as i32);
        options.insert("depth", 0i32);
        options.insert("gpg-verify", false);
        options.insert("gpg-verify-summary", false);
        options.insert("disable-static-deltas", true);
        destination.0.pull_with_options(
            &self.0.path().uri(),
            &options.end(),
            None,
            gio::Cancellable::NONE,
        )?;
        let (commit, _) = destination.0.load_commit(&revision.commit)?;
        let attributes = glib::VariantDict::new(Some(&commit.child_value(0)));
        let collection = attributes.lookup::<String>("ostree.collection-binding")?;
        let refs = attributes.lookup::<Vec<String>>("ostree.ref-binding")?;
        if collection.as_deref() != Some(&revision.variant.collection)
            || !refs.is_some_and(|refs| refs.contains(&reference))
        {
            return Err(
                "pinned commit collection/ref binding does not match selected identity".into(),
            );
        }
        Ok(())
    }

    pub(super) fn open(path: &Path) -> Result<Self> {
        use std::os::unix::ffi::OsStrExt;
        if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
            return Err("local source path must be absolute and contain no NUL bytes".into());
        }
        let repo = ostree::Repo::new(&gio::File::for_path(path));
        repo.open(gio::Cancellable::NONE)?;
        Ok(Self(repo))
    }

    pub(super) fn resolve(&self, variant: &Variant, kind: &str) -> Result<Revision> {
        if self.0.collection_id().as_deref() != Some(&variant.collection) {
            return Err(
                "explicit collection does not match local repository collection identity".into(),
            );
        }
        let reference = format!("{kind}/{}/{}/{}", variant.id, variant.arch, variant.branch);
        let commit = self
            .0
            .resolve_rev(&reference, false)?
            .ok_or("missing selected ref")?;
        Ok(Revision {
            variant: variant.clone(),
            commit: commit.into(),
        })
    }

    pub(super) fn prepare(&self, commits: &[&str], destination: &Path) -> Result<Self> {
        let repo = ostree::Repo::new(&gio::File::for_path(destination));
        repo.create(ostree::RepoMode::BareUserOnly, gio::Cancellable::NONE)?;
        let prepared = Self(repo);
        self.import_to(commits, &prepared)?;
        Ok(prepared)
    }

    pub(super) fn import_to(&self, commits: &[&str], destination: &Self) -> Result<()> {
        let options = glib::VariantDict::new(None);
        options.insert("refs", commits);
        // libostree's option schema requires signed int32, not GFlags' uint32.
        options.insert("flags", ostree::RepoPullFlags::UNTRUSTED.bits() as i32);
        options.insert("depth", 0i32);
        options.insert("gpg-verify", false);
        options.insert("gpg-verify-summary", false);
        options.insert("disable-static-deltas", true);
        // A file URI, never a remote nickname: no remote discovery or provider
        // inference. UNTRUSTED verifies imported objects instead of hardlinking
        // and trusting a local repository merely because it is local.
        destination.0.pull_with_options(
            &self.0.path().uri(),
            &options.end(),
            None,
            gio::Cancellable::NONE,
        )?;
        Ok(())
    }

    pub(super) fn metadata(&self, revision: &Revision) -> Result<glib::KeyFile> {
        let (tree, _) = self
            .0
            .read_commit(&revision.commit, gio::Cancellable::NONE)?;
        let (commit_data, _) = self.0.load_commit(&revision.commit)?;
        let attributes = glib::VariantDict::new(Some(&commit_data.child_value(0)));
        if attributes.contains("xa.extra-data-sources") {
            return Err("unsupported structural requirement: xa.extra-data-sources".into());
        }
        let file = tree.child("metadata");
        let info = file.query_info(
            "standard::type,standard::size",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            gio::Cancellable::NONE,
        )?;
        if info.file_type() != gio::FileType::Regular || info.size() > 64 * 1024 {
            return Err("artifact metadata must be a regular file of at most 64 KiB".into());
        }
        // libostree returns a generic input stream. Avoid GIO load_contents'
        // file-stream-only query_info path, and bound the actual read as well.
        let mut contents = Vec::new();
        // OstreeRepoFile implements read with a GInputStream rather than the
        // GFileInputStream promised by GIO's generated binding.
        let stream: gio::InputStream = unsafe {
            use glib::translate::*;
            let mut error = std::ptr::null_mut();
            let pointer =
                gio::ffi::g_file_read(file.to_glib_none().0, std::ptr::null_mut(), &mut error);
            if !error.is_null() {
                return Err(glib::Error::from_glib_full(error).into());
            }
            from_glib_full(pointer.cast::<gio::ffi::GInputStream>())
        };
        loop {
            let bytes = stream.read_bytes(4096, gio::Cancellable::NONE)?;
            if bytes.is_empty() {
                break;
            }
            contents.extend_from_slice(&bytes);
            if contents.len() > 64 * 1024 {
                return Err("artifact metadata exceeds 64 KiB".into());
            }
        }
        let metadata = glib::KeyFile::new();
        metadata.load_from_data(std::str::from_utf8(&contents)?, glib::KeyFileFlags::NONE)?;
        let payload = tree.child("files").query_info(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            gio::Cancellable::NONE,
        )?;
        if payload.file_type() != gio::FileType::Directory {
            return Err("artifact files must be a real directory".into());
        }
        Ok(metadata)
    }

    pub(super) fn checkout(&self, commit: &str, destination: &Path) -> Result<()> {
        let options = ostree::RepoCheckoutAtOptions {
            mode: ostree::RepoCheckoutMode::User,
            force_copy: true,
            enable_fsync: true,
            ..Default::default()
        };
        self.0.checkout_at(
            Some(&options),
            libc::AT_FDCWD,
            destination,
            commit,
            gio::Cancellable::NONE,
        )?;
        Ok(())
    }
}

pub(super) fn validate(
    app: &Revision,
    app_metadata: &glib::KeyFile,
    runtime: &Revision,
    runtime_metadata: &glib::KeyFile,
) -> Result<Vec<String>> {
    let mut diagnostics = Vec::new();
    let mut ignored = 0usize;
    for (revision, metadata, group) in [
        (app, app_metadata, "Application"),
        (runtime, runtime_metadata, "Runtime"),
    ] {
        if metadata.string(group, "name")?.as_str() != revision.variant.id {
            return Err("artifact name does not match selected identity".into());
        }
        for section in metadata.groups().iter() {
            if let Some(extension) = section.strip_prefix("Extension ")
                && omitted_extension(
                    revision,
                    extension,
                    metadata.string(section, "directory")?.as_str(),
                )
            {
                for key in metadata.keys(section)?.iter() {
                    if !matches!(
                        key.as_str(),
                        "directory"
                            | "version"
                            | "versions"
                            | "subdirectories"
                            | "subdirectory-suffix"
                            | "no-autodownload"
                            | "autodelete"
                            | "locale-subset"
                            | "add-ld-path"
                            | "merge-dirs"
                            | "download-if"
                            | "enable-if"
                            | "autoprune-unless"
                    ) {
                        return Err(
                            format!("unsupported structural requirement: {section}.{key}").into(),
                        );
                    }
                }
                if diagnostics.len() < 32 {
                    diagnostics.push(format!(
                        "omitted {extension}; outside software-rendered Calculator baseline"
                    ));
                }
                continue;
            }
            if section.starts_with("Extension ")
                || matches!(
                    section.as_str(),
                    "Extension" | "ExtensionOf" | "Extra Data" | "ExtraData" | "Install" | "Hooks"
                )
            {
                return Err(format!("unsupported structural requirement: {section}").into());
            }
            for key in metadata.keys(section)?.iter() {
                if section == group
                    && matches!(key.as_str(), "name" | "runtime" | "command" | "sdk")
                {
                    continue;
                }
                if section == group
                    && matches!(
                        key.as_str(),
                        "extra-data" | "extensions" | "install-hook" | "uninstall-hook"
                    )
                {
                    return Err(
                        format!("unsupported structural requirement: {section}.{key}").into(),
                    );
                }
                ignored += 1;
                if diagnostics.len() < 32 {
                    let declaration = format!(
                        "{}: {section}.{key}={}",
                        revision.variant.id,
                        metadata.value(section, key)?
                    );
                    // Bound both count and text, including worst-case JSON
                    // escaping, so permissions never make replies exceed 64 KiB.
                    let preview: String = declaration.chars().take(192).collect();
                    diagnostics.push(format!("ignored {preview}; no access granted"));
                }
            }
        }
    }
    if ignored > diagnostics.len() {
        diagnostics.push(format!(
            "{} further declarations ignored; no access granted",
            ignored - diagnostics.len()
        ));
    }
    let required = format!(
        "{}/{}/{}",
        runtime.variant.id, runtime.variant.arch, runtime.variant.branch
    );
    if app_metadata.string("Application", "runtime")?.as_str() != required {
        return Err(
            "explicit runtime is incompatible with application's declared runtime requirement"
                .into(),
        );
    }
    Ok(diagnostics)
}

// Deliberately bounded compatibility profile, not general extension resolution.
fn omitted_extension(revision: &Revision, extension: &str, directory: &str) -> bool {
    if revision.variant.id == "org.gnome.Calculator" {
        return matches!(
            (extension, directory),
            ("org.gnome.Calculator.Locale", "share/runtime/locale")
                | ("org.gnome.Calculator.Debug", "lib/debug")
        );
    }
    if revision.variant.id != "org.gnome.Platform" || revision.variant.branch != "51" {
        return false;
    }
    matches!(
        (extension, directory),
        ("org.gnome.Platform.Locale", "share/runtime/locale")
            | ("org.freedesktop.Platform.GL", "lib/x86_64-linux-gnu/GL")
            | (
                "org.freedesktop.Platform.GL.Debug",
                "lib/debug/usr/lib/x86_64-linux-gnu/GL"
            )
            | (
                "org.freedesktop.Platform.VulkanLayer",
                "lib/extensions/vulkan"
            )
            | ("org.freedesktop.Platform.Timezones", "share/zoneinfo")
            | (
                "org.freedesktop.Platform.GStreamer",
                "lib/extensions/gstreamer-1.0"
            )
            | (
                "org.freedesktop.Platform.Icontheme",
                "share/runtime/share/icons"
            )
            | ("org.gtk.Gtk3theme", "share/runtime/share/themes")
            | (
                "org.freedesktop.Platform.VAAPI.Intel",
                "lib/x86_64-linux-gnu/dri/intel-vaapi-driver"
            )
            | (
                "org.freedesktop.Platform.VAAPI.nvidia",
                "lib/x86_64-linux-gnu/dri/nvidia-vaapi-driver"
            )
            | (
                "org.freedesktop.Platform.codecs-extra",
                "lib/x86_64-linux-gnu/codecs-extra"
            )
    )
}
