//! Local update public values and installation-owned orchestration.
use super::{
    Installation, Installed, Result, Revision, Variant, lifecycle, read, remove_tree, storage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

mod faults;
mod transaction;
pub(super) use faults::Faults;
pub(super) use transaction::{recover, sync_tree};

pub const MAX_PLAN: usize = 32 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    #[serde(deserialize_with = "revision")]
    pub app: Revision,
    #[serde(deserialize_with = "revision")]
    pub runtime: Revision,
}

pub(super) fn revision<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Revision, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Strict {
        collection: String,
        id: String,
        arch: String,
        branch: String,
        commit: String,
    }
    let r = Strict::deserialize(decoder)?;
    Ok(Revision {
        variant: Variant {
            collection: r.collection,
            id: r.id,
            arch: r.arch,
            branch: r.branch,
        },
        commit: r.commit,
    })
}

impl Selection {
    fn of(installed: &Installed) -> Self {
        Self {
            app: installed.app.clone(),
            runtime: installed.runtime.clone(),
        }
    }

    fn validate(&self, target: &Variant) -> Result<()> {
        self.app.variant.validate()?;
        self.runtime.variant.validate()?;
        checksum(&self.app.commit)?;
        checksum(&self.runtime.commit)?;
        if self.app.variant != *target || self.runtime.variant.arch != target.arch {
            return Err("plan revision identity does not match target".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Sources {
    pub app: PathBuf,
    pub runtime: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub format_version: u32,
    pub target: Variant,
    pub expected: Selection,
    pub desired: Selection,
    pub sources: Sources,
}

impl Plan {
    pub fn validate(&self) -> Result<()> {
        if self.format_version != 1 {
            return Err("unsupported update plan format version".into());
        }
        self.target.validate()?;
        self.expected.validate(&self.target)?;
        self.desired.validate(&self.target)?;
        source_path(&self.sources.app)?;
        source_path(&self.sources.runtime)?;
        if serde_json::to_vec_pretty(self)?.len() > MAX_PLAN {
            return Err("oversized update plan".into());
        }
        Ok(())
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_PLAN {
            return Err("oversized update plan".into());
        }
        let plan: Self = serde_json::from_value(crate::wire::strict_json(bytes)?)?;
        plan.validate()?;
        Ok(plan)
    }
}

fn checksum(commit: &str) -> Result<()> {
    if commit.len() != 64
        || !commit
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("commit must be exactly 64 lowercase hexadecimal characters".into());
    }
    Ok(())
}

fn source_path(path: &Path) -> Result<()> {
    let text = path.to_str().ok_or("source path must be UTF-8")?;
    if !path.is_absolute()
        || text.len() > 4096
        || text.contains('\0')
        || text.split('/').any(|p| p == "." || p == "..")
    {
        return Err("source must be an absolute bounded path without NUL or dot components".into());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Planning {
    pub source: PathBuf,
    pub app: Variant,
    pub runtime: Variant,
    pub runtime_source: Option<PathBuf>,
    pub app_commit: Option<String>,
    pub runtime_commit: Option<String>,
}

#[derive(Debug)]
pub struct Error {
    pub name: &'static str,
    pub parameters: Value,
}
impl Error {
    fn detail(name: &'static str, error: impl std::fmt::Display) -> Self {
        Self {
            name,
            parameters: json!({"detail":error.to_string().chars().take(2048).collect::<String>()}),
        }
    }
    fn failed(error: impl std::fmt::Display) -> Self {
        Self::detail("UpdateFailed", error)
    }
    fn invalid(error: impl std::fmt::Display) -> Self {
        Self::detail("UpdateInvalidPlan", error)
    }
    fn source(error: impl std::fmt::Display) -> Self {
        Self::detail("UpdateSourceInvalid", error)
    }
}

fn stage(root: &Path) -> Result<PathBuf> {
    lifecycle::stage_path(root, "update-plan")
}

fn prepare(
    sources: &Sources,
    selection: &Selection,
    stage: &Path,
) -> Result<(storage::Source, Installed)> {
    let objects = stage.join("object-repo");
    let prepared = storage::Source::empty(&objects)?;
    storage::Source::open(&sources.app)?.import_revision(&selection.app, "app", &prepared)?;
    storage::Source::open(&sources.runtime)?.import_revision(
        &selection.runtime,
        "runtime",
        &prepared,
    )?;
    let diagnostics = storage::validate(
        &selection.app,
        &prepared.metadata(&selection.app)?,
        &selection.runtime,
        &prepared.metadata(&selection.runtime)?,
    )?;
    Ok((
        prepared,
        Installed {
            app: selection.app.clone(),
            runtime: selection.runtime.clone(),
            diagnostics,
        },
    ))
}

impl Installation {
    pub fn plan_update(&self, request: Planning) -> std::result::Result<Plan, Error> {
        let validate = || -> Result<()> {
            request.app.validate()?;
            request.runtime.validate()?;
            source_path(&request.source)?;
            source_path(request.runtime_source.as_deref().unwrap_or(&request.source))?;
            if let Some(pin) = &request.app_commit {
                checksum(pin)?;
            }
            if let Some(pin) = &request.runtime_commit {
                checksum(pin)?;
            }
            Ok(())
        };
        validate().map_err(Error::invalid)?;
        let operation = self
            .operation()
            .map_err(|e| Error::detail("UpdateRecoveryRequired", e))?;
        let path = request.app.path(&operation.root).map_err(Error::failed)?;
        if !path.try_exists().map_err(Error::failed)? {
            return Err(Error {
                name: "UpdateNotInstalled",
                parameters: json!({"target":request.app}),
            });
        }
        let current = read(&path).map_err(Error::failed)?;
        let sources = Sources {
            runtime: request
                .runtime_source
                .unwrap_or_else(|| request.source.clone()),
            app: request.source,
        };
        let resolve = |source: &Path,
                       variant: Variant,
                       pin: Option<String>,
                       kind: &str|
         -> Result<Revision> {
            match pin {
                Some(commit) => Ok(Revision { variant, commit }),
                None => storage::Source::open(source)?.resolve(&variant, kind),
            }
        };
        let desired = Selection {
            app: resolve(&sources.app, request.app.clone(), request.app_commit, "app")
                .map_err(Error::source)?,
            runtime: resolve(
                &sources.runtime,
                request.runtime,
                request.runtime_commit,
                "runtime",
            )
            .map_err(Error::source)?,
        };
        let plan = Plan {
            format_version: 1,
            target: request.app,
            expected: Selection::of(&current),
            desired,
            sources,
        };
        plan.validate().map_err(Error::invalid)?;
        let stage = stage(&operation.root).map_err(Error::failed)?;
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&stage)
                .map_err(Error::failed)?;
        }
        let result = prepare(&plan.sources, &plan.desired, &stage)
            .map(|(_, installed)| installed)
            .map_err(Error::source);
        if let Err(error) = remove_tree(&stage) {
            eprintln!("planning cleanup pending: {error}");
        }
        result?;
        Ok(plan)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Updated,
    Unchanged,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Warning {
    pub code: String,
    pub detail: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Applied {
    pub outcome: Outcome,
    pub selection: Installed,
    pub warnings: Vec<Warning>,
}

impl Installation {
    pub fn apply_update(&self, plan: Plan) -> std::result::Result<Applied, Error> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        plan.validate().map_err(Error::invalid)?;
        self.update_faults.validate().map_err(Error::failed)?;
        let operation = self
            .operation()
            .map_err(|e| Error::detail("UpdateRecoveryRequired", e))?;
        let path = plan.target.path(&operation.root).map_err(Error::failed)?;
        if !path.try_exists().map_err(Error::failed)? {
            return Err(Error {
                name: "UpdateNotInstalled",
                parameters: json!({"target":plan.target}),
            });
        }
        if operation
            .running
            .lock()
            .map_err(|_| Error::failed("instance registry poisoned"))?
            .contains(&plan.target)
        {
            return Err(Error {
                name: "UpdateBusy",
                parameters: json!({"target":plan.target}),
            });
        }
        let current = read(&path).map_err(Error::failed)?;
        let pair = Selection::of(&current);
        if pair == plan.desired {
            return Ok(Applied {
                outcome: Outcome::Unchanged,
                selection: current,
                warnings: Vec::new(),
            });
        }
        if pair != plan.expected {
            return Err(Error {
                name: "UpdateConflict",
                parameters: json!({"current":pair}),
            });
        }
        let mut txn =
            transaction::Transaction::new(&operation.root, &plan.target).map_err(Error::failed)?;
        let result = (|| -> std::result::Result<Applied, Error> {
            let (prepared, selected) =
                prepare(&plan.sources, &plan.desired, &txn.path).map_err(Error::source)?;
            let faults = &self.update_faults;
            faults.checkpoint("apply", "imported");
            let slot = txn.path.join("slot");
            fs::create_dir(&slot).map_err(Error::failed)?;
            prepared
                .checkout(&selected.app.commit, &slot.join("app"))
                .map_err(Error::failed)?;
            prepared
                .checkout(&selected.runtime.commit, &slot.join("runtime"))
                .map_err(Error::failed)?;
            drop(prepared);
            remove_tree(&txn.path.join("object-repo")).map_err(Error::failed)?;
            // Reserve reply space, including worst-case JSON escaping of a warning.
            if serde_json::to_vec(&selected).map_err(Error::failed)?.len() + 16 * 1024
                > crate::MAX_MESSAGE
            {
                return Err(Error::failed(
                    "installed selection cannot fit bounded update reply",
                ));
            }
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(slot.join("selection.json"))
                .map_err(Error::failed)?;
            let bytes = serde_json::to_vec(&selected).map_err(Error::failed)?;
            let middle = bytes.len() / 2;
            file.write_all(&bytes[..middle]).map_err(Error::failed)?;
            faults
                .check("apply", "write-selection")
                .map_err(Error::failed)?;
            file.write_all(&bytes[middle..]).map_err(Error::failed)?;
            faults
                .check("apply", "sync-selection")
                .map_err(Error::failed)?;
            file.sync_all().map_err(Error::failed)?;
            faults
                .check("apply", "sync-payload")
                .map_err(Error::failed)?;
            sync_tree(&slot).map_err(Error::failed)?;
            transaction::sync_dir(&txn.path).map_err(Error::failed)?;
            faults.checkpoint("apply", "prepared");
            lifecycle::retain_runtime_checked(&operation.root, &path, &current.runtime, || {
                faults.check("apply", "sync-retained")
            })
            .map_err(Error::failed)?;
            faults.checkpoint("apply", "retained");
            txn.seal(&plan, faults).map_err(Error::failed)?;
            let warning = txn.publish(&operation.root, &plan, faults)?;
            Ok(Applied {
                outcome: Outcome::Updated,
                selection: selected,
                warnings: warning
                    .into_iter()
                    .map(|detail| Warning {
                        code: "cleanup_pending".into(),
                        detail: detail.chars().take(2048).collect(),
                    })
                    .collect(),
            })
        })();
        if result.is_err() {
            txn.abort(&operation.root, &self.update_faults);
        }
        result
    }
}
