//! Local installation orchestration. OSTree details are confined to storage.
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

mod storage;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
// The service is the only installation writer. Inspection shares its visibility lock.
static OPERATIONS: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub collection: String,
    pub id: String,
    pub arch: String,
    pub branch: String,
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
    if !request.source.is_absolute() {
        return Err("local source path must be absolute".into());
    }
    let source = storage::Source::open(&request.source)?;
    let app = source.resolve(&request.app, "app")?;
    let runtime = source.resolve(&request.runtime, "runtime")?;
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
        let prepared = source.prepare(&[&app.commit, &runtime.commit], &object_path)?;
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
