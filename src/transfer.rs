use crate::{
    desktop::rename_exclusive,
    operations::{self, Task},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConflictPolicy {
    #[default]
    Refuse,
    Skip,
    Rename,
    Replace,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TransferOptions {
    pub conflicts: ConflictPolicy,
    pub merge_folders: bool,
}

enum Action {
    Copy,
    Skip,
    Replace([u8; 32]),
    Merge(Vec<PlannedItem>),
}
struct PlannedItem {
    source: PathBuf,
    target: PathBuf,
    action: Action,
}

fn next_name(target: &Path, reserved: &BTreeSet<PathBuf>) -> Result<PathBuf> {
    let parent = target.parent().context("Missing target parent")?;
    let stem = target.file_stem().context("Missing filename")?;
    for number in 1..=100_000 {
        let mut name = OsString::from(stem);
        name.push(format!(" ({number})"));
        if let Some(extension) = target.extension() {
            name.push(".");
            name.push(extension);
        }
        let candidate = parent.join(name);
        if !operations::exists(&candidate) && !reserved.contains(&candidate) {
            return Ok(candidate);
        }
    }
    bail!("Could not find an unused filename for {}", target.display());
}

fn plan_item(
    source: &Path,
    target: PathBuf,
    options: TransferOptions,
    reserved: &mut BTreeSet<PathBuf>,
    overrides: &BTreeMap<PathBuf, ConflictPolicy>,
    task: &Task,
) -> Result<PlannedItem> {
    task.check()?;
    let options = TransferOptions {
        conflicts: overrides.get(&target).copied().unwrap_or(options.conflicts),
        ..options
    };
    let source_metadata = fs::symlink_metadata(source)?;
    let target_metadata = match fs::symlink_metadata(&target) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let mut target = target;
    let action = if let Some(target_metadata) = target_metadata {
        if options.conflicts == ConflictPolicy::Rename {
            target = next_name(&target, reserved)?;
            Action::Copy
        } else if fs::canonicalize(source).ok() == fs::canonicalize(&target).ok()
            && !source_metadata.file_type().is_symlink()
        {
            if options.conflicts == ConflictPolicy::Skip {
                Action::Skip
            } else {
                bail!(
                    "The source and target are the same item: {}",
                    source.display()
                );
            }
        } else if options.merge_folders && source_metadata.is_dir() && target_metadata.is_dir() {
            let mut children = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
            children.sort_by_key(|entry| entry.file_name());
            let mut plans = Vec::new();
            for child in children {
                plans.push(plan_item(
                    &child.path(),
                    target.join(child.file_name()),
                    options,
                    reserved,
                    overrides,
                    task,
                )?);
            }
            Action::Merge(plans)
        } else {
            match options.conflicts {
                ConflictPolicy::Refuse => bail!("Destination already exists: {}", target.display()),
                ConflictPolicy::Skip => Action::Skip,
                ConflictPolicy::Replace => {
                    Action::Replace(operations::source_stamp(&target, task)?)
                }
                ConflictPolicy::Rename => unreachable!(),
            }
        }
    } else if reserved.contains(&target) {
        if options.conflicts == ConflictPolicy::Rename {
            target = next_name(&target, reserved)?;
            Action::Copy
        } else {
            bail!(
                "Multiple sources have the same target: {}",
                target.display()
            );
        }
    } else {
        Action::Copy
    };
    if !reserved.insert(target.clone()) {
        bail!(
            "Multiple sources have the same target: {}",
            target.display()
        );
    }
    Ok(PlannedItem {
        source: source.into(),
        target,
        action,
    })
}

struct Execution {
    destination: PathBuf,
    recovery: Option<PathBuf>,
    replacements: usize,
    skipped: usize,
}
impl Execution {
    fn backup_path(&mut self, target: &Path) -> Result<PathBuf> {
        if self.recovery.is_none() {
            let directory = tempfile::Builder::new()
                .prefix(".rust-commander-replaced-")
                .tempdir_in(&self.destination)?;
            fs::create_dir(directory.path().join("items"))?;
            fs::write(
                directory.path().join("RECOVERY.txt"),
                format!(
                    "Original destination items are kept under items/, with their original relative paths.\nDestination: {}\nReview these files before moving or deleting this recovery folder.\n",
                    self.destination.display()
                ),
            )?;
            self.recovery = Some(directory.keep());
        }
        let backup = self
            .recovery
            .as_ref()
            .unwrap()
            .join("items")
            .join(target.strip_prefix(&self.destination)?);
        fs::create_dir_all(backup.parent().context("Missing recovery parent")?)?;
        Ok(backup)
    }

    /// Returns false when any item was skipped. A partially copied source folder
    /// is deliberately retained during a move, including its copied children.
    fn copy(&mut self, plan: &PlannedItem, task: &mut Task) -> Result<bool> {
        task.check()?;
        if !matches!(plan.action, Action::Skip) {
            task.capture(&plan.target)?;
        }
        match &plan.action {
            Action::Copy => operations::copy_item(&plan.source, &plan.target, task)?,
            Action::Skip => {
                self.skipped += 1;
                return Ok(false);
            }
            Action::Merge(children) => {
                let metadata = fs::symlink_metadata(&plan.target)?;
                if !metadata.is_dir() {
                    bail!("Destination folder changed: {}", plan.target.display());
                }
                let mut complete = true;
                for child in children {
                    complete &= self.copy(child, task)?;
                }
                return Ok(complete);
            }
            Action::Replace(expected) => {
                // Copy completely before moving the existing target. The new
                // item and recovery folder stay on the destination filesystem.
                let stage = tempfile::Builder::new()
                    .prefix(".rust-commander-incoming-")
                    .tempdir_in(plan.target.parent().context("Missing target parent")?)?;
                let incoming = stage.path().join("item");
                operations::copy_item(&plan.source, &incoming, task)?;
                task.check()?;
                if operations::source_stamp(&plan.target, task)? != *expected {
                    bail!(
                        "Destination changed during transfer: {}",
                        plan.target.display()
                    );
                }
                let backup = self.backup_path(&plan.target)?;
                rename_exclusive(&plan.target, &backup)?;
                self.replacements += 1;
                if let Err(error) = rename_exclusive(&incoming, &plan.target) {
                    let restore = rename_exclusive(&backup, &plan.target);
                    if restore.is_ok() {
                        self.replacements -= 1;
                    }
                    return Err(error).with_context(|| {
                        if restore.is_ok() {
                            "Replacement failed; the original destination was restored.".into()
                        } else {
                            format!(
                                "Replacement failed; recover the original from {}",
                                backup.display()
                            )
                        }
                    });
                }
            }
        }
        Ok(true)
    }
}

pub fn transfer_resolved(
    sources: &[PathBuf],
    destination: &Path,
    moving: bool,
    options: TransferOptions,
    overrides: &BTreeMap<PathBuf, ConflictPolicy>,
    task: &mut Task,
) -> Result<String> {
    operations::validate_sources(sources)?;
    let normalized_overrides = overrides
        .iter()
        .map(|(path, policy)| {
            let normalized = path
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .and_then(|parent| path.file_name().map(|name| parent.join(name)))
                .unwrap_or_else(|| path.clone());
            (normalized, *policy)
        })
        .collect::<BTreeMap<_, _>>();
    let overrides = &normalized_overrides;
    let destination =
        fs::canonicalize(destination).context("Choose an existing destination folder")?;
    if !destination.is_dir() {
        bail!("The destination is not a folder.");
    }
    let mut reserved = BTreeSet::new();
    let mut plans = Vec::new();
    for source in sources {
        if fs::symlink_metadata(source)?.is_dir()
            && destination.starts_with(fs::canonicalize(source)?)
        {
            bail!("Cannot copy or move a folder into itself.");
        }
        let target = destination.join(
            source
                .file_name()
                .context("Cannot transfer a filesystem root")?,
        );
        if options.conflicts != ConflictPolicy::Rename
            && fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.is_dir())
        {
            let target_directory = fs::canonicalize(&target)?;
            for selected in sources {
                if fs::canonicalize(selected)?.starts_with(&target_directory) {
                    bail!(
                        "A destination folder contains a selected source: {}",
                        target.display()
                    );
                }
            }
        }
        plans.push(plan_item(
            source,
            target,
            options,
            &mut reserved,
            overrides,
            task,
        )?);
    }
    let before = if moving {
        sources
            .iter()
            .map(|p| operations::source_stamp(p, task))
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let mut execution = Execution {
        destination,
        recovery: None,
        replacements: 0,
        skipped: 0,
    };
    let result: Result<String> = (|| {
        let mut completed = Vec::new();
        for plan in &plans {
            completed.push(execution.copy(plan, task)?);
        }
        task.check()?;
        if moving {
            for (source, stamp) in sources.iter().zip(&before) {
                if operations::source_stamp(source, task)? != *stamp {
                    bail!(
                        "A source changed during the move. All originals and completed copies were kept."
                    );
                }
            }
            for (source, complete) in sources.iter().zip(&completed) {
                if *complete {
                    task.check()?;
                    task.capture(source)?;
                    operations::remove(source)
                        .context("Copies completed, but an original could not be removed")?;
                }
            }
        }
        Ok(format!(
            "{} {} item(s); skipped {} conflict(s).{}",
            if moving { "Moved" } else { "Copied" },
            completed.iter().filter(|v| **v).count(),
            execution.skipped,
            if moving && execution.skipped > 0 {
                " Source folders containing skipped items were kept in full."
            } else {
                ""
            }
        ))
    })();
    if let Some(recovery) = execution.recovery {
        if execution.replacements == 0 {
            let _ = fs::remove_dir_all(recovery);
        } else {
            return result
                .map(|message| {
                    format!(
                        "{message}\nReplaced originals are kept in {}",
                        recovery.display()
                    )
                })
                .with_context(|| format!("Replaced originals are kept in {}", recovery.display()));
        }
    }
    result
}

pub fn transfer(
    sources: &[PathBuf],
    destination: &Path,
    moving: bool,
    options: TransferOptions,
    task: &mut Task,
) -> Result<String> {
    transfer_resolved(
        sources,
        destination,
        moving,
        options,
        &BTreeMap::new(),
        task,
    )
}

#[derive(Clone)]
pub struct Conflict {
    pub source: PathBuf,
    pub target: PathBuf,
    pub source_size: u64,
    pub target_size: u64,
}

pub fn conflicts(sources: &[PathBuf], destination: &Path, merge: bool) -> Result<Vec<Conflict>> {
    fn visit(source: &Path, target: &Path, merge: bool, result: &mut Vec<Conflict>) -> Result<()> {
        let input = fs::symlink_metadata(source)?;
        let output = match fs::symlink_metadata(target) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        if merge && input.is_dir() && output.is_dir() {
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                visit(
                    &entry.path(),
                    &target.join(entry.file_name()),
                    merge,
                    result,
                )?;
            }
        } else {
            if result.len() >= 5000 {
                bail!("Too many conflicts to display; resolve a smaller selection.");
            }
            result.push(Conflict {
                source: source.into(),
                target: target.into(),
                source_size: input.len(),
                target_size: output.len(),
            });
        }
        Ok(())
    }
    let mut result = Vec::new();
    for source in sources {
        visit(
            source,
            &destination.join(source.file_name().context("Missing filename")?),
            merge,
            &mut result,
        )?;
    }
    Ok(result)
}
