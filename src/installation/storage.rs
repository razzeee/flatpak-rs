//! libostree boundary: resolve pinned trees and prepare independent checkouts.
use super::{Result, Revision, Variant};
use gio::prelude::*;
use std::path::Path;

pub(super) struct Source(ostree::Repo);

impl Source {
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
        let stream = file
            .read(gio::Cancellable::NONE)?
            .upcast::<gio::InputStream>();
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
