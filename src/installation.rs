//! Local installation orchestration. OSTree details are confined to storage.
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

mod lifecycle;
mod storage;
pub use lifecycle::{delete_data, list_runtimes, remove_runtime, uninstall};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
// The service is the only installation writer. Inspection shares its visibility lock.
static OPERATIONS: Mutex<()> = Mutex::new(());
static RECOVERED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static RECOVERY_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Remove only a tree owned by the service, never following payload symlinks.
pub(crate) fn remove_tree(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        for entry in fs::read_dir(path)? {
            remove_tree(&entry?.path())?;
        }
        fs::remove_dir(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn recover() -> Result<()> {
    let _lock = OPERATIONS
        .lock()
        .map_err(|_| "installation lock poisoned")?;
    recover_locked()
}

fn recover_locked() -> Result<()> {
    let staging = root()?.join(".staging");
    directory(&staging)?;
    let mut count = 0;
    for entry in fs::read_dir(&staging)? {
        remove_tree(&entry?.path())?;
        count += 1;
    }
    File::open(staging)?.sync_all()?;
    RECOVERED.store(count, std::sync::atomic::Ordering::Relaxed);
    RECOVERY_READY.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

fn ensure_recovered() -> Result<()> {
    if !RECOVERY_READY.load(std::sync::atomic::Ordering::Relaxed) {
        recover_locked()?;
    }
    Ok(())
}

pub fn recovered_stages() -> usize {
    RECOVERED.load(std::sync::atomic::Ordering::Relaxed)
}

fn checkpoint(name: &str) {
    // Host-side fault injection only: never accepts artifact/request input.
    if std::env::var("FLATPAK_RS_INSTALL_STOP_AFTER")
        .ok()
        .as_deref()
        == Some(name)
    {
        eprintln!("installation checkpoint {name}");
        // SAFETY: stop this service for the external crash/restart acceptance test.
        unsafe {
            libc::raise(libc::SIGSTOP);
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub collection: String,
    pub id: String,
    pub arch: String,
    pub branch: String,
}

static RUNNING: std::sync::LazyLock<Mutex<std::collections::HashSet<Variant>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

pub(crate) struct Lease(pub Variant);
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut running) = RUNNING.lock() {
            running.remove(&self.0);
        }
    }
}

pub(crate) struct Deployment {
    pub lease: Lease,
    pub app: PathBuf,
    pub runtime: PathBuf,
    pub home: PathBuf,
}

pub fn instances() -> Vec<Variant> {
    RUNNING
        .lock()
        .map(|set| set.iter().cloned().collect())
        .unwrap_or_default()
}

pub(crate) fn acquire(variant: Variant) -> Result<Deployment> {
    use std::os::unix::fs::PermissionsExt;
    let _lock = OPERATIONS
        .lock()
        .map_err(|_| "installation lock poisoned")?;
    variant.validate()?;
    ensure_recovered()?;
    let root = root()?;
    let path = variant.path(&root)?;
    if !path.try_exists()? {
        return Err("application variant is not installed".into());
    }
    read(&path)?;
    if !RUNNING
        .lock()
        .map_err(|_| "instance lock poisoned")?
        .insert(variant.clone())
    {
        return Err("application variant is already running".into());
    }
    let lease = Lease(variant.clone());
    let data_root = root.join("data");
    directory(&data_root)?;
    let home = variant.path(&data_root)?;
    directory(&home)?;
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700))?;
    Ok(Deployment {
        lease,
        app: path.join("app/files"),
        runtime: path.join("runtime/files"),
        home,
    })
}

impl Variant {
    fn validate(&self) -> Result<()> {
        for component in [&self.collection, &self.id, &self.arch, &self.branch] {
            if component.is_empty()
                || component.len() > 200
                || !component
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                || !component.as_bytes()[0].is_ascii_alphanumeric()
            {
                return Err("invalid identity component (expected a bounded alphanumeric name with '.', '_' or '-')".into());
            }
        }
        if self.arch != std::env::consts::ARCH {
            return Err("only host-native architecture is supported".into());
        }
        Ok(())
    }

    fn path(&self, root: &Path) -> Result<PathBuf> {
        let mut path = root.join("installed");
        for component in [&self.collection, &self.id, &self.arch] {
            directory(&path)?;
            path.push(component);
        }
        directory(&path)?;
        Ok(path.join(&self.branch))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct Revision {
    #[serde(flatten)]
    pub variant: Variant,
    pub commit: String,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Installed {
    pub app: Revision,
    pub runtime: Revision,
    pub diagnostics: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Install {
    pub source: PathBuf,
    #[serde(default)]
    pub runtime_source: Option<PathBuf>,
    pub app: Variant,
    pub runtime: Variant,
}

fn directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(format!(
            "installation path is not a real directory: {}",
            path.display()
        )
        .into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Check every ancestor, including configured XDG paths, for symlinks.
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                directory(parent)?;
            }
            fs::create_dir(path)?;
            if let Some(parent) = path.parent() {
                File::open(parent)?.sync_all()?;
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn root() -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(base) => PathBuf::from(base),
        None => {
            PathBuf::from(std::env::var_os("HOME").ok_or("HOME is missing")?).join(".local/share")
        }
    };
    if !base.is_absolute() {
        return Err("XDG_DATA_HOME/HOME must be absolute".into());
    }
    // Validate existing ancestors too, not just newly created ones.
    let mut checked = PathBuf::from("/");
    for part in base.components().skip(1) {
        checked.push(part);
        directory(&checked)?;
    }
    let root = base.join("flatpak-rs");
    directory(&root)?;
    Ok(root)
}

fn read(path: &Path) -> Result<Installed> {
    directory(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path.join("selection.json"))?;
    Ok(serde_json::from_reader(file)?)
}

pub fn inspect(variant: Variant) -> Result<Installed> {
    let _lock = OPERATIONS
        .lock()
        .map_err(|_| "installation lock poisoned")?;
    variant.validate()?;
    ensure_recovered()?;
    let path = variant.path(&root()?)?;
    if !path.try_exists()? {
        return Err("application variant is not installed".into());
    }
    read(&path)
}

pub fn install(request: Install) -> Result<Installed> {
    let _lock = OPERATIONS
        .lock()
        .map_err(|_| "installation lock poisoned")?;
    request.app.validate()?;
    request.runtime.validate()?;
    ensure_recovered()?;
    if !request.source.is_absolute() {
        return Err("local source path must be absolute".into());
    }
    let source = storage::Source::open(&request.source)?;
    let app = source.resolve(&request.app, "app")?;
    let runtime_source =
        storage::Source::open(request.runtime_source.as_deref().unwrap_or(&request.source))?;
    let runtime = runtime_source.resolve(&request.runtime, "runtime")?;
    let root = root()?;
    let destination = request.app.path(&root)?;
    let staging = root.join(".staging");
    directory(&staging)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let stage = staging.join(format!("{}-{nonce}", std::process::id()));
    fs::create_dir(&stage)?;
    let result = (|| -> Result<Installed> {
        let object_path = stage.join("object-repo");
        let prepared = source.prepare(&[&app.commit], &object_path)?;
        runtime_source.import_to(&[&runtime.commit], &prepared)?;
        let app_metadata = prepared.metadata(&app)?;
        let runtime_metadata = prepared.metadata(&runtime)?;
        let diagnostics = storage::validate(&app, &app_metadata, &runtime, &runtime_metadata)?;
        let selected = Installed {
            app,
            runtime,
            diagnostics,
        };
        let exists = match fs::symlink_metadata(&destination) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        if exists {
            let installed = read(&destination)?;
            if installed.app == selected.app && installed.runtime == selected.runtime {
                return Ok(installed);
            }
            return Err("unsupported update: variant is already installed with a different app or runtime selection".into());
        }
        prepared.checkout(&selected.app.commit, &stage.join("app"))?;
        prepared.checkout(&selected.runtime.commit, &stage.join("runtime"))?;
        checkpoint("prepared");
        drop(prepared);
        fs::remove_dir_all(object_path)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(stage.join("selection.json"))?;
        file.write_all(&serde_json::to_vec(&selected)?)?;
        file.sync_all()?;
        File::open(&stage)?.sync_all()?;
        fs::rename(&stage, &destination)?;
        File::open(destination.parent().unwrap())?.sync_all()?;
        checkpoint("published");
        Ok(selected)
    })();
    // Only our unique, unpublished stage is removed; never follow a source path.
    match fs::remove_dir_all(&stage) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            eprintln!(
                "cannot remove unpublished staging {}: {error}",
                stage.display()
            );
        }
        _ => {}
    }
    result
}
