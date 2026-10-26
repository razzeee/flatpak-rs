//! Whole-deployment exchange and monotonic, service-owned recovery states.
use super::super::{Result, Variant, directory, read, remove_tree};
use super::{Error, Faults, Plan, Selection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub(in crate::installation) fn sync_dir(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)?;
    sync_file(&file)
}

fn sync_file(file: &File) -> Result<()> {
    loop {
        match file.sync_all() {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => return result.map_err(Into::into),
        }
    }
}

pub(in crate::installation) fn sync_tree(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            sync_tree(&entry?.path())?;
        }
        sync_dir(path)
    } else if meta.is_file() {
        sync_file(
            &OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)?,
        )
    } else {
        Err("unsupported transaction content type".into())
    }
}

fn private(path: &Path) -> Result<()> {
    directory(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    sync_dir(path)?;
    Ok(())
}

fn paths(root: &Path) -> Result<(PathBuf, PathBuf)> {
    let updates = root.join(".updates");
    private(&updates)?;
    let active = updates.join("active");
    let garbage = updates.join("garbage");
    private(&active)?;
    private(&garbage)?;
    sync_dir(&updates)?;
    sync_dir(root)?;
    Ok((active, garbage))
}

fn id(path: &Path) -> Result<&std::ffi::OsStr> {
    let name = path.file_name().ok_or("transaction has no ID")?;
    let text = name.to_str().ok_or("invalid transaction ID")?;
    if text.len() != 32
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid transaction ID".into());
    }
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err("transaction must be a real directory".into());
    }
    Ok(name)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    format_version: u32,
    target: Variant,
    expected: Selection,
    desired: Selection,
}

impl Intent {
    fn validate(&self) -> Result<()> {
        if self.format_version != 1 || self.expected == self.desired {
            return Err("invalid update intent".into());
        }
        self.target.validate()?;
        self.expected.validate(&self.target)?;
        self.desired.validate(&self.target)
    }
}

fn intent(path: &Path) -> Result<Option<Intent>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path.join("intent.json"))
    {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        result => result?,
    };
    if !file.metadata()?.is_file() {
        return Err("update intent must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 {
        return Err("oversized update intent".into());
    }
    let record: Intent = serde_json::from_value(crate::wire::strict_json(&bytes)?)?;
    record.validate()?;
    Ok(Some(record))
}

fn pair(path: &Path, target: &Variant) -> Result<Selection> {
    let selected = Selection::of(&read(path)?);
    selected.validate(target)?;
    Ok(selected)
}

fn sync_at(faults: &Faults, phase: &str, site: &str, path: &Path) -> Result<()> {
    faults.check(phase, site)?;
    sync_dir(path)
}

fn garbage(path: &Path, root: &Path, faults: &Faults, phase: &str) -> Result<PathBuf> {
    let destination = root.join(".updates/garbage").join(id(path)?);
    faults.check(phase, "mark-garbage")?;
    super::super::storage::rename_private(path, &destination, false)?;
    sync_at(faults, phase, "sync-active-parent", path.parent().unwrap())?;
    sync_at(
        faults,
        phase,
        "sync-garbage-parent",
        destination.parent().unwrap(),
    )?;
    Ok(destination)
}

fn cleanup(path: &Path, faults: &Faults, phase: &str) -> Result<()> {
    id(path)?;
    faults.checkpoint(phase, "cleanup-started");
    faults.check(phase, "delete-obsolete")?;
    for entry in fs::read_dir(path)? {
        remove_tree(&entry?.path())?;
        faults.checkpoint(phase, "cleanup-partial");
    }
    fs::remove_dir(path)?;
    sync_at(faults, phase, "sync-cleanup", path.parent().unwrap())
}

pub(in crate::installation) fn recover(root: &Path, faults: &Faults) -> Result<()> {
    match fs::symlink_metadata(root.join(".updates")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
        Ok(meta) if !meta.is_dir() => return Err("update state must be a real directory".into()),
        Ok(_) => {}
    }
    let (active, dead) = paths(root)?;
    let entries: Vec<_> = fs::read_dir(&active)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    // Validate all intents first; never resolve two competing transactions by order.
    let mut targets = HashSet::new();
    let mut records = Vec::new();
    for path in entries {
        id(&path)?;
        let record = intent(&path)?;
        if let Some(record) = &record
            && !targets.insert(record.target.clone())
        {
            return Err("multiple active updates for one variant".into());
        }
        records.push((path, record));
    }
    for (path, record) in records {
        if let Some(record) = record {
            let published = record.target.path(root)?;
            let current = pair(&published, &record.target)?;
            let slot = pair(&path.join("slot"), &record.target)?;
            if current == record.desired && slot == record.expected {
                sync_at(
                    faults,
                    "recovery",
                    "sync-installed-parent",
                    published.parent().unwrap(),
                )?;
                sync_at(faults, "recovery", "sync-slot-parent", &path)?;
            } else if current != record.expected || slot != record.desired {
                return Err("update recovery requires consistent old/new pair evidence; inspect service journal".into());
            }
        }
        garbage(&path, root, faults, "recovery")?;
    }
    // A process may have died between the garbage rename and parent syncs.
    if fs::read_dir(&dead)?.next().is_some() {
        sync_at(faults, "recovery", "sync-active-parent", &active)?;
        sync_at(faults, "recovery", "sync-garbage-parent", &dead)?;
    }
    for entry in fs::read_dir(&dead)? {
        let path = entry?.path();
        id(&path)?;
        if let Err(error) = cleanup(&path, faults, "recovery") {
            eprintln!("update cleanup pending: {error}");
        }
    }
    Ok(())
}

pub(super) struct Transaction {
    pub path: PathBuf,
    published: PathBuf,
    attempted: bool,
}
impl Transaction {
    pub fn new(root: &Path, target: &Variant) -> Result<Self> {
        let (active, _) = paths(root)?;
        let mut random = [0u8; 16];
        File::open("/dev/urandom")?.read_exact(&mut random)?;
        let path = active.join(hex::encode(random));
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        sync_dir(&active)?;
        Ok(Self {
            path,
            published: target.path(root)?,
            attempted: false,
        })
    }

    pub fn seal(&self, plan: &Plan, faults: &Faults) -> Result<()> {
        let record = Intent {
            format_version: 1,
            target: plan.target.clone(),
            expected: plan.expected.clone(),
            desired: plan.desired.clone(),
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.path.join("intent.tmp"))?;
        file.write_all(&serde_json::to_vec(&record)?)?;
        faults.check("apply", "sync-intent")?;
        sync_file(&file)?;
        fs::rename(self.path.join("intent.tmp"), self.path.join("intent.json"))?;
        sync_dir(&self.path)?;
        sync_dir(self.path.parent().unwrap())?;
        faults.checkpoint("apply", "intent");
        Ok(())
    }

    pub fn publish(
        &mut self,
        root: &Path,
        plan: &Plan,
        faults: &Faults,
    ) -> std::result::Result<Option<String>, Error> {
        let exchange = (|| -> Result<()> {
            faults.check("apply", "exchange")?;
            self.attempted = true;
            super::super::storage::rename_private(&self.path.join("slot"), &self.published, true)?;
            faults.check("apply", "exchange-result")
        })();
        if let Err(error) = exchange {
            if !self.attempted
                || matches!((pair(&self.published, &plan.target), pair(&self.path.join("slot"), &plan.target)), (Ok(current), Ok(slot)) if current == plan.expected && slot == plan.desired)
            {
                self.attempted = false;
                return Err(Error::failed(error));
            }
            return Err(Error::detail("UpdateOutcomeUncertain", error));
        }
        faults.checkpoint("apply", "exchanged");
        let committed = (|| -> Result<PathBuf> {
            sync_at(
                faults,
                "apply",
                "sync-installed-parent",
                self.published.parent().unwrap(),
            )?;
            sync_at(faults, "apply", "sync-slot-parent", &self.path)?;
            faults.checkpoint("apply", "parents-synced");
            garbage(&self.path, root, faults, "apply")
        })()
        .map_err(|error| Error::detail("UpdateOutcomeUncertain", error))?;
        faults.checkpoint("apply", "committed");
        Ok(cleanup(&committed, faults, "apply")
            .err()
            .map(|e| e.to_string()))
    }

    pub fn abort(&self, root: &Path, faults: &Faults) {
        if !self.attempted && self.path.exists() {
            let result = garbage(&self.path, root, faults, "apply")
                .and_then(|path| cleanup(&path, faults, "apply"));
            if let Err(error) = result {
                eprintln!("unpublished update cleanup pending: {error}");
            }
        }
    }
}

use std::os::unix::fs::DirBuilderExt;
