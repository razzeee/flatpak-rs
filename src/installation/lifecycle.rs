//! Lifecycle operations share the installation/lease lock and never take paths
//! from clients. Retention is prepared before atomically unpublishing an app.
use super::{Installation, Installed, Result, Revision, Variant, directory, read, remove_tree};
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct Removed {
    removed: bool,
}
#[derive(Serialize)]
pub struct Runtimes {
    runtimes: Vec<Revision>,
}

pub(super) fn stage_path(root: &Path, prefix: &str) -> Result<PathBuf> {
    let staging = root.join(".staging");
    directory(&staging)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    Ok(staging.join(format!("{prefix}-{}-{nonce}", std::process::id())))
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        symlink(fs::read_link(source)?, target)?;
    } else if metadata.is_dir() {
        fs::create_dir(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
        File::open(target)?.sync_all()?;
        fs::set_permissions(
            target,
            fs::Permissions::from_mode(metadata.permissions().mode() & 0o777),
        )?;
    } else if metadata.is_file() {
        fs::copy(source, target)?;
        File::open(target)?.sync_all()?;
    } else {
        return Err("unsupported retained content type".into());
    }
    Ok(())
}

fn records(root: &Path) -> Result<Vec<(PathBuf, Installed)>> {
    fn walk(path: &Path, depth: usize, result: &mut Vec<(PathBuf, Installed)>) -> Result<()> {
        directory(path)?;
        if depth == 4 {
            result.push((path.to_path_buf(), read(path)?));
            return Ok(());
        }
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                return Err("unsafe entry in published installation index".into());
            }
            walk(&entry.path(), depth + 1, result)?;
        }
        Ok(())
    }
    let mut result = Vec::new();
    walk(&root.join("installed"), 0, &mut result)?;
    Ok(result)
}

pub(super) fn retain_runtime(root: &Path, deployment: &Path, revision: &Revision) -> Result<()> {
    retain_runtime_checked(root, deployment, revision, || Ok(()))
}

pub(super) fn retain_runtime_checked(
    root: &Path,
    deployment: &Path,
    revision: &Revision,
    before_parent_sync: impl Fn() -> Result<()>,
) -> Result<()> {
    let pool = root.join("runtimes");
    directory(&pool)?;
    let branch = revision.variant.path(&pool)?;
    directory(&branch)?;
    let destination = branch.join(&revision.commit);
    if fs::symlink_metadata(&destination).is_ok() {
        directory(&destination)?;
        let stored: Revision = serde_json::from_reader(
            OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(destination.join("revision.json"))?,
        )?;
        if stored != *revision {
            return Err("retained runtime identity mismatch".into());
        }
        super::update::sync_tree(&destination)?;
        before_parent_sync()?;
        File::open(&branch)?.sync_all()?;
        return Ok(());
    }
    let stage = stage_path(root, "retain-runtime")?;
    fs::create_dir(&stage)?;
    let result = (|| -> Result<()> {
        copy_tree(&deployment.join("runtime"), &stage.join("content"))?;
        let mut record = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(stage.join("revision.json"))?;
        record.write_all(&serde_json::to_vec(revision)?)?;
        record.sync_all()?;
        File::open(&stage)?.sync_all()?;
        fs::rename(&stage, &destination)?;
        super::update::sync_tree(&destination)?;
        before_parent_sync()?;
        File::open(&branch)?.sync_all()?;
        Ok(())
    })();
    if stage.exists() {
        let _ = remove_tree(&stage);
    }
    result
}

fn unpublish(root: &Path, path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    let stage = stage_path(root, "delete")?;
    fs::rename(path, &stage)?;
    File::open(path.parent().unwrap())?.sync_all()?;
    remove_tree(&stage)?;
    Ok(true)
}

impl Installation {
    pub fn uninstall(&self, variant: Variant) -> Result<Removed> {
        variant.validate()?;
        let operation = self.operation()?;
        operation.stopped(&variant)?;
        let root = &operation.root;
        let path = variant.path(root)?;
        if !path.try_exists()? {
            return Err("application variant is not installed".into());
        }
        let installed = read(&path)?;
        retain_runtime(root, &path, &installed.runtime)?;
        Ok(Removed {
            removed: unpublish(root, &path)?,
        })
    }

    pub fn delete_data(&self, variant: Variant) -> Result<Removed> {
        variant.validate()?;
        let operation = self.operation()?;
        operation.stopped(&variant)?;
        let root = &operation.root;
        let data = root.join("data");
        directory(&data)?;
        let path = variant.path(&data)?;
        Ok(Removed {
            removed: unpublish(root, &path)?,
        })
    }

    pub fn remove_runtime(&self, variant: Variant) -> Result<Removed> {
        variant.validate()?;
        let operation = self.operation()?;
        let root = &operation.root;
        if records(root)?
            .iter()
            .any(|(_, installed)| installed.runtime.variant == variant)
        {
            return Err(
                "runtime has installed application dependents; uninstall them first".into(),
            );
        }
        let pool = root.join("runtimes");
        directory(&pool)?;
        let path = variant.path(&pool)?;
        Ok(Removed {
            removed: unpublish(root, &path)?,
        })
    }

    pub fn list_runtimes(&self) -> Result<Runtimes> {
        let operation = self.operation()?;
        let root = &operation.root;
        let mut revisions: std::collections::HashSet<Revision> =
            records(root)?.into_iter().map(|(_, i)| i.runtime).collect();
        let pool = root.join("runtimes/installed");
        fn walk(
            path: &Path,
            depth: usize,
            revisions: &mut std::collections::HashSet<Revision>,
        ) -> Result<()> {
            directory(path)?;
            if depth == 5 {
                let record = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(path.join("revision.json"))?;
                revisions.insert(serde_json::from_reader(record)?);
                return Ok(());
            }
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    return Err("unsafe retained runtime entry".into());
                }
                walk(&entry.path(), depth + 1, revisions)?;
            }
            Ok(())
        }
        walk(&pool, 0, &mut revisions)?;
        let mut runtimes: Vec<_> = revisions.into_iter().collect();
        runtimes.sort_by_cached_key(|r| {
            (
                r.variant.collection.clone(),
                r.variant.id.clone(),
                r.variant.arch.clone(),
                r.variant.branch.clone(),
                r.commit.clone(),
            )
        });
        Ok(Runtimes { runtimes })
    }
}
