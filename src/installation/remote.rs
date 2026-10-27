//! Publisher authorization is independent of long-running installation work.
use super::{Installation, Result, Revision, Variant, directory};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Remote {
    pub name: String,
    pub collection: String,
    pub url: String,
    pub fingerprints: Vec<String>,
    pub generation: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Remotes {
    pub remotes: Vec<Remote>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddRemote {
    pub name: String,
    pub collection: String,
    pub descriptor: String,
    pub replace: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedRemote {
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub name: String,
    pub generation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub format_version: u32,
    #[serde(deserialize_with = "super::update::revision")]
    pub app: Revision,
    #[serde(deserialize_with = "super::update::revision")]
    pub runtime: Revision,
    pub app_provider: Provider,
    pub runtime_provider: Provider,
}

impl Plan {
    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            return Err("unsupported remote installation plan version".into());
        }
        for revision in [&self.app, &self.runtime] {
            revision.variant.validate()?;
            if revision.commit.len() != 64
                || !revision
                    .commit
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("remote installation requires lowercase SHA-256 pins".into());
            }
        }
        for provider in [&self.app_provider, &self.runtime_provider] {
            validate_name(&provider.name)?;
            if provider.generation.len() != 32
                || !provider.generation.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("invalid remote authorization generation".into());
            }
        }
        Ok(())
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 32 * 1024 {
            return Err("remote installation plan exceeds 32 KiB".into());
        }
        let plan: Self = serde_json::from_value(crate::wire::strict_json(bytes)?)?;
        plan.validate()?;
        Ok(plan)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInstall {
    pub remote: String,
    pub id: String,
    pub arch: String,
    pub branch: String,
    pub runtime_remote: String,
}

impl Remote {
    fn provider(&self) -> Provider {
        Provider {
            name: self.name.clone(),
            generation: self.generation.clone(),
        }
    }
    fn repo(&self, root: &Path) -> Result<ostree::Repo> {
        let repo = ostree::Repo::new(&gio::File::for_path(
            root.join("providers").join(&self.generation),
        ));
        repo.open(gio::Cancellable::NONE)?;
        Ok(repo)
    }
    fn resolve(&self, root: &Path, variant: Variant, kind: &str) -> Result<Revision> {
        variant.validate()?;
        let repo = self.repo(root)?;
        let summary = verified_summary(&repo, &self.collection)?;
        let name = format!("{kind}/{}/{}/{}", variant.id, variant.arch, variant.branch);
        for entry in summary.child_value(0).iter() {
            if entry.child_value(0).str() == Some(name.as_str()) {
                let bytes = entry
                    .child_value(1)
                    .child_value(1)
                    .get::<Vec<u8>>()
                    .ok_or("invalid summary checksum")?;
                if bytes.len() != 32 {
                    return Err("invalid summary checksum length".into());
                }
                return Ok(Revision {
                    variant,
                    commit: hex::encode(bytes),
                });
            }
        }
        Err(format!("selected remote ref not found: {name}").into())
    }

    fn pull(
        &self,
        root: &Path,
        revision: &Revision,
        kind: &str,
        metadata_only: bool,
    ) -> Result<glib::KeyFile> {
        if revision.variant.collection != self.collection {
            return Err("pin collection differs from provider authorization".into());
        }
        let repo = self.repo(root)?;
        let reference = format!(
            "{kind}/{}/{}/{}",
            revision.variant.id, revision.variant.arch, revision.variant.branch
        );
        let options = glib::VariantDict::new(None);
        options.insert("refs", &[reference.as_str()][..]);
        options.insert("override-commit-ids", &[revision.commit.as_str()][..]);
        // Identify this client explicitly; Flathub rejects the generic
        // libostree user agent for object downloads.
        options.insert(
            "http-headers",
            &[(
                "User-Agent",
                concat!("flatpak-rs/", env!("CARGO_PKG_VERSION")),
            )][..],
        );
        let flags = ostree::RepoPullFlags::UNTRUSTED
            | if metadata_only {
                ostree::RepoPullFlags::COMMIT_ONLY
            } else {
                ostree::RepoPullFlags::NONE
            };
        options.insert("flags", flags.bits() as i32);
        options.insert("depth", 0i32);
        options.insert("disable-static-deltas", true);
        options.insert("gpg-verify", true);
        options.insert("gpg-verify-summary", true);
        repo.pull_with_options("provider", &options.end(), None, gio::Cancellable::NONE)?;
        repo.verify_commit_for_remote(&revision.commit, "provider", gio::Cancellable::NONE)?
            .require_valid_signature()?;
        let (commit, _) = repo.load_commit(&revision.commit)?;
        let attributes = glib::VariantDict::new(Some(&commit.child_value(0)));
        if attributes.contains("xa.extra-data-sources") {
            return Err("unsupported structural requirement: xa.extra-data-sources".into());
        }
        if attributes
            .lookup::<String>("ostree.collection-binding")?
            .as_deref()
            != Some(&self.collection)
            || !attributes
                .lookup::<Vec<String>>("ostree.ref-binding")?
                .is_some_and(|refs| refs.contains(&reference))
        {
            return Err(
                "signed commit collection/ref binding differs from selected identity".into(),
            );
        }
        let text = attributes
            .lookup::<String>("xa.metadata")?
            .ok_or("remote artifact requires signed xa.metadata")?;
        if text.len() > 64 * 1024 {
            return Err("oversized signed artifact metadata".into());
        }
        let metadata = glib::KeyFile::new();
        metadata.load_from_data(&text, glib::KeyFileFlags::NONE)?;
        Ok(metadata)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyInstall {
    pub plan: Plan,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Applied {
    pub status: ApplyStatus,
    pub installed: super::Installed,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApplyStatus {
    Installed,
    Unchanged,
}

#[derive(Debug)]
pub struct OutcomeUncertain(std::io::Error);
impl std::fmt::Display for OutcomeUncertain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "remote installation outcome uncertain; inspect or retry the same plan: {}",
            self.0
        )
    }
}
impl std::error::Error for OutcomeUncertain {}

fn fault(site: &str) -> std::io::Result<()> {
    // Host-controlled acceptance injection; neither requests nor artifact
    // metadata can configure the service process environment.
    if std::env::var("FLATPAK_RS_REMOTE_FAIL_AT").as_deref() == Ok(site) {
        return Err(std::io::Error::from_raw_os_error(
            if site == "write-selection" {
                libc::ENOSPC
            } else {
                libc::EIO
            },
        ));
    }
    Ok(())
}

fn enabled(remotes: &Remotes, name: &str) -> Result<Remote> {
    validate_name(name)?;
    let remote = remotes
        .remotes
        .iter()
        .find(|r| r.name == name)
        .ok_or("remote is not configured")?;
    if !remote.enabled {
        return Err("remote authorization is disabled".into());
    }
    Ok(remote.clone())
}

fn current(remotes: &Remotes, provider: &Provider) -> Result<Remote> {
    let remote = enabled(remotes, &provider.name)?;
    if remote.generation != provider.generation {
        return Err("remote authorization changed; create a new plan".into());
    }
    Ok(remote)
}

fn validate_name(name: &str) -> Result<()> {
    Variant {
        collection: name.into(),
        id: "remote".into(),
        arch: std::env::consts::ARCH.into(),
        branch: "stable".into(),
    }
    .validate()
}

fn nonce() -> Result<String> {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(hex::encode(bytes))
}

fn write_remotes(root: &Path, remotes: &Remotes) -> Result<()> {
    let bytes = serde_json::to_vec(remotes)?;
    if bytes.len() > crate::MAX_MESSAGE / 2 {
        return Err("too many configured remotes".into());
    }
    let temp = root.join(format!(".remotes-{}", nonce()?));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp, root.join("remotes.json"))?;
        fs::File::open(root)?
            .sync_all()
            .map_err(|e| format!("remote configuration outcome uncertain; inspect remotes: {e}"))?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}

fn repository_url(url: &str) -> Result<()> {
    let parsed = glib::Uri::parse(url, glib::UriFlags::NONE)?;
    let allowed = parsed.scheme() == "https"
        || (parsed.scheme() == "http"
            && matches!(parsed.host().as_deref(), Some("127.0.0.1" | "::1")));
    if !allowed
        || parsed.host().is_none()
        || parsed.userinfo().is_some()
        || parsed.fragment().is_some()
        || url.len() > 2048
    {
        return Err("repository URL requires HTTPS (or HTTP loopback for local tests), no credentials or fragment".into());
    }
    Ok(())
}

fn verified_summary(repo: &ostree::Repo, collection: &str) -> Result<glib::Variant> {
    let (summary, signatures) = repo.remote_fetch_summary("provider", gio::Cancellable::NONE)?;
    if summary.len() > 32 * 1024 * 1024 || signatures.len() > 1024 * 1024 {
        return Err("oversized signed summary".into());
    }
    repo.verify_summary("provider", &summary, &signatures, gio::Cancellable::NONE)?
        .require_valid_signature()?;
    let summary = glib::Variant::from_bytes_with_type(
        &summary,
        glib::VariantTy::new("(a(s(taya{sv}))a{sv})")?,
    );
    if !summary.is_normal_form() {
        return Err("invalid summary encoding".into());
    }
    let metadata = glib::VariantDict::new(Some(&summary.child_value(1)));
    if metadata
        .lookup::<String>("ostree.summary.collection-id")?
        .as_deref()
        != Some(collection)
    {
        return Err("signed summary collection does not match authorized collection".into());
    }
    Ok(summary)
}

// The generated bindings cannot express NULL key_ids (all keys); &[] means
// an explicitly empty selection to libostree. Keep this distinction at the FFI seam.
fn import_keys(repo: &ostree::Repo, input: &gio::MemoryInputStream) -> Result<Vec<glib::Variant>> {
    use gio::prelude::*;
    use glib::translate::*;
    let mut error = std::ptr::null_mut();
    let mut imported = 0;
    let mut keys = std::ptr::null_mut();
    // SAFETY: live GObjects, NUL-terminated static name and initialized output
    // pointers; libostree owns errors/array until transferred below.
    unsafe {
        let ok = ostree::ffi::ostree_repo_remote_gpg_import(
            repo.to_glib_none().0,
            c"provider".as_ptr(),
            input.upcast_ref::<gio::InputStream>().to_glib_none().0,
            std::ptr::null_mut(),
            &mut imported,
            std::ptr::null_mut(),
            &mut error,
        );
        if ok == 0 {
            return Err(glib::Error::from_glib_full(error).into());
        }
        if imported == 0 {
            return Err("descriptor contains no importable signing keys".into());
        }
        let ok = ostree::ffi::ostree_repo_remote_get_gpg_keys(
            repo.to_glib_none().0,
            c"provider".as_ptr(),
            std::ptr::null_mut(),
            &mut keys,
            std::ptr::null_mut(),
            &mut error,
        );
        if ok == 0 {
            return Err(glib::Error::from_glib_full(error).into());
        }
        Ok(FromGlibPtrContainer::from_glib_container(keys))
    }
}

fn read_remotes(root: &Path) -> Result<Remotes> {
    match fs::read(root.join("remotes.json")) {
        Ok(bytes) => Ok(serde_json::from_value(crate::wire::strict_json(&bytes)?)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Remotes::default()),
        Err(e) => Err(e.into()),
    }
}

impl Installation {
    pub fn apply_install(&self, request: ApplyInstall) -> Result<Applied> {
        let plan = request.plan;
        plan.validate()?;
        let operation = self.operation()?;
        let root = &operation.root;
        let destination = plan.app.variant.path(root)?;
        if destination.try_exists()? {
            let installed = super::read(&destination)?;
            if installed.app == plan.app && installed.runtime == plan.runtime {
                return Ok(Applied {
                    status: ApplyStatus::Unchanged,
                    installed,
                });
            }
            return Err("variant already installed with a different selection; remote updates are unsupported".into());
        }
        let (app_remote, runtime_remote) = {
            let _guard = self
                .remote_authorization
                .lock()
                .map_err(|_| "remote authorization lock poisoned")?;
            let remotes = read_remotes(root)?;
            (
                current(&remotes, &plan.app_provider)?,
                current(&remotes, &plan.runtime_provider)?,
            )
        };
        app_remote.pull(root, &plan.app, "app", false)?;
        runtime_remote.pull(root, &plan.runtime, "runtime", false)?;
        let staging = root.join(".staging");
        directory(&staging)?;
        let stage = staging.join(format!("remote-{}", nonce()?));
        fs::create_dir(&stage)?;
        let result = (|| -> Result<Applied> {
            let source =
                super::storage::Source::open(&root.join("providers").join(&app_remote.generation))?;
            let runtime_source = super::storage::Source::open(
                &root.join("providers").join(&runtime_remote.generation),
            )?;
            let objects = stage.join("object-repo");
            let prepared = source.prepare(&[&plan.app.commit], &objects)?;
            runtime_source.import_to(&[&plan.runtime.commit], &prepared)?;
            let diagnostics = super::storage::validate(
                &plan.app,
                &prepared.metadata(&plan.app)?,
                &plan.runtime,
                &prepared.metadata(&plan.runtime)?,
            )?;
            prepared.checkout(&plan.app.commit, &stage.join("app"))?;
            prepared.checkout(&plan.runtime.commit, &stage.join("runtime"))?;
            drop(prepared);
            super::remove_tree(&objects)?;
            let installed = super::Installed {
                app: plan.app,
                runtime: plan.runtime,
                diagnostics,
            };
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(stage.join("selection.json"))?;
            fault("write-selection")?;
            file.write_all(&serde_json::to_vec(&installed)?)?;
            fault("sync-selection")?;
            file.sync_all()?;
            fs::File::open(&stage)?.sync_all()?;
            super::checkpoint("prepared");
            let _guard = self
                .remote_authorization
                .lock()
                .map_err(|_| "remote authorization lock poisoned")?;
            let remotes = read_remotes(root)?;
            current(&remotes, &plan.app_provider)?;
            current(&remotes, &plan.runtime_provider)?;
            super::storage::rename_private(&stage, &destination, false)?;
            fault("sync-publication").map_err(OutcomeUncertain)?;
            fs::File::open(destination.parent().unwrap())
                .and_then(|f| f.sync_all())
                .map_err(OutcomeUncertain)?;
            super::checkpoint("published");
            Ok(Applied {
                status: ApplyStatus::Installed,
                installed,
            })
        })();
        // Cleanup must not replace an uncertain publication outcome with an
        // ordinary failure. Startup recovery owns any stage we cannot remove.
        if let Err(error) = super::remove_tree(&stage)
            && !error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            eprintln!(
                "cannot remove unpublished staging {}: {error}",
                stage.display()
            );
        }
        result
    }

    pub fn plan_install(&self, request: PlanInstall) -> Result<Plan> {
        let operation = self.operation()?;
        let root = &operation.root;
        let (app_remote, runtime_remote) = {
            let _guard = self
                .remote_authorization
                .lock()
                .map_err(|_| "remote authorization lock poisoned")?;
            let remotes = read_remotes(root)?;
            (
                enabled(&remotes, &request.remote)?,
                enabled(&remotes, &request.runtime_remote)?,
            )
        };
        let app = app_remote.resolve(
            root,
            Variant {
                collection: app_remote.collection.clone(),
                id: request.id,
                arch: request.arch,
                branch: request.branch,
            },
            "app",
        )?;
        let metadata = app_remote.pull(root, &app, "app", true)?;
        let required = metadata.string("Application", "runtime")?;
        let components: Vec<_> = required.split('/').collect();
        if components.len() != 3 || components[1] != app.variant.arch {
            return Err("invalid or incompatible declared runtime".into());
        }
        let runtime = runtime_remote.resolve(
            root,
            Variant {
                collection: runtime_remote.collection.clone(),
                id: components[0].into(),
                arch: components[1].into(),
                branch: components[2].into(),
            },
            "runtime",
        )?;
        let runtime_metadata = runtime_remote.pull(root, &runtime, "runtime", true)?;
        super::storage::validate(&app, &metadata, &runtime, &runtime_metadata)?;
        let plan = Plan {
            format_version: 1,
            app,
            runtime,
            app_provider: app_remote.provider(),
            runtime_provider: runtime_remote.provider(),
        };
        plan.validate()?;
        let _guard = self
            .remote_authorization
            .lock()
            .map_err(|_| "remote authorization lock poisoned")?;
        let remotes = read_remotes(root)?;
        current(&remotes, &plan.app_provider)?;
        current(&remotes, &plan.runtime_provider)?;
        Ok(plan)
    }

    pub fn add_remote(&self, request: AddRemote) -> Result<Remote> {
        validate_name(&request.name)?;
        validate_name(&request.collection)?;
        if request.descriptor.len() > 48 * 1024 {
            return Err("repository descriptor exceeds 48 KiB".into());
        }
        let descriptor = glib::KeyFile::new();
        descriptor.load_from_data(&request.descriptor, glib::KeyFileFlags::NONE)?;
        let url = descriptor.string("Flatpak Repo", "Url")?.to_string();
        repository_url(&url)?;
        if let Ok(collection) = descriptor.string("Flatpak Repo", "CollectionID")
            && collection != request.collection
        {
            return Err("descriptor collection differs from authorization".into());
        }
        let encoded = descriptor.string("Flatpak Repo", "GPGKey")?;
        let key = glib::base64_decode(encoded.as_str());
        if key.is_empty() || glib::base64_encode(&key).as_str() != encoded.as_str() {
            return Err("invalid repository signing key encoding".into());
        }
        let _guard = self
            .remote_authorization
            .lock()
            .map_err(|_| "remote authorization lock poisoned")?;
        let root = self.root()?;
        let mut remotes = read_remotes(&root)?;
        let position = remotes.remotes.iter().position(|r| r.name == request.name);
        if position.is_some() != request.replace {
            return Err(
                "remote-add requires a new name; remote-replace requires an existing name".into(),
            );
        }
        let generation = nonce()?;
        let providers = root.join("providers");
        directory(&providers)?;
        let path = providers.join(&generation);
        let result = (|| -> Result<Remote> {
            let repo = ostree::Repo::new(&gio::File::for_path(&path));
            // Preserve original file metadata in the download cache: subsequent
            // untrusted imports must recompute the publisher's exact checksums.
            repo.create(ostree::RepoMode::Archive, gio::Cancellable::NONE)?;
            let options = glib::VariantDict::new(None);
            options.insert("gpg-verify", true);
            options.insert("gpg-verify-summary", true);
            options.insert("tls-permissive", false);
            repo.remote_add(
                "provider",
                Some(&url),
                Some(&options.end()),
                gio::Cancellable::NONE,
            )?;
            let bytes = glib::Bytes::from_owned(key);
            let input = gio::MemoryInputStream::from_bytes(&bytes);
            let mut fingerprints = Vec::new();
            for key in import_keys(&repo, &input)? {
                let subkeys = key.child_value(0);
                if subkeys.n_children() == 0 {
                    return Err("signing key has no primary fingerprint".into());
                }
                let attributes = glib::VariantDict::new(Some(&subkeys.child_value(0)));
                fingerprints.push(
                    attributes
                        .lookup::<String>("fingerprint")?
                        .ok_or("missing key fingerprint")?,
                );
            }
            fingerprints.sort();
            fingerprints.dedup();
            verified_summary(&repo, &request.collection)?;
            Ok(Remote {
                name: request.name,
                collection: request.collection,
                url,
                fingerprints,
                generation,
                enabled: true,
            })
        })();
        let remote = match result {
            Ok(remote) => remote,
            Err(error) => {
                let _ = super::remove_tree(&path);
                return Err(error);
            }
        };
        if let Some(position) = position {
            remotes.remotes[position] = remote.clone();
        } else {
            remotes.remotes.push(remote.clone());
        }
        // Persist the authority referenced by the configuration before publishing
        // its generation. Download objects are merely retryable cache data.
        for name in ["config", "provider.trustedkeys.gpg"] {
            fs::File::open(path.join(name))?.sync_all()?;
        }
        fs::File::open(&path)?.sync_all()?;
        fs::File::open(&providers)?.sync_all()?;
        write_remotes(&root, &remotes)?;
        Ok(remote)
    }

    pub fn disable_remote(&self, request: NamedRemote) -> Result<Remote> {
        validate_name(&request.name)?;
        let _guard = self
            .remote_authorization
            .lock()
            .map_err(|_| "remote authorization lock poisoned")?;
        let root = self.root()?;
        let mut remotes = read_remotes(&root)?;
        let remote = remotes
            .remotes
            .iter_mut()
            .find(|r| r.name == request.name)
            .ok_or("remote is not configured")?;
        remote.enabled = false;
        let remote = remote.clone();
        write_remotes(&root, &remotes)?;
        Ok(remote)
    }

    pub fn remotes(&self) -> Result<Remotes> {
        let _guard = self
            .remote_authorization
            .lock()
            .map_err(|_| "remote authorization lock poisoned")?;
        read_remotes(&self.root()?)
    }
}
