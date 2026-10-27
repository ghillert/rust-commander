//! Drag selections and guarded transfers shared by every UI/native drop target.
pub(crate) mod promises;
use crate::{
    filesystem::{self, Entry, Location},
    operations::{self, Task},
    transfer::{self, ConflictPolicy, TransferOptions},
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Selection {
    pub from: Location,
    pub entries: Vec<Entry>,
}
impl Selection {
    pub fn local(paths: &[PathBuf]) -> Result<Self> {
        ensure!(
            !paths.is_empty() && paths.len() <= 100_000,
            "Select files to drag."
        );
        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();
        for path in paths {
            ensure!(path.is_absolute(), "Dropped file paths must be absolute.");
            if seen.insert(path.clone()) {
                entries.push(filesystem::entry_from_path(path)?);
            }
        }
        let from = Location::Local(
            paths[0]
                .parent()
                .context("Cannot drag a filesystem root")?
                .into(),
        );
        let selection = Self { from, entries };
        selection.validate()?;
        Ok(selection)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.entries.is_empty() && self.entries.len() <= 100_000,
            "Select files to drag."
        );
        let mut names = BTreeSet::new();
        let folders = self
            .entries
            .iter()
            .filter(|e| e.is_dir)
            .map(|e| e.path.as_path())
            .collect::<BTreeSet<_>>();
        for entry in &self.entries {
            let name = entry
                .path
                .file_name()
                .context("Cannot drag a filesystem root")?;
            ensure!(
                names.insert(name.to_owned()),
                "The selection has duplicate destination names."
            );
            ensure!(
                entry.name == name.to_string_lossy(),
                "Dragged entry names must match their paths."
            );
            ensure!(
                !entry
                    .path
                    .ancestors()
                    .skip(1)
                    .any(|parent| folders.contains(parent)),
                "Do not drag a folder together with one of its descendants."
            );
            if self.from.local().is_some() {
                ensure!(
                    entry.path.is_absolute(),
                    "Dropped file paths must be absolute."
                );
            } else {
                // Listings contain native paths; raw archive headers have
                // already passed strict validation in the archive reader.
                filesystem::portable_relative(&entry.path)?;
                ensure!(
                    entry.path.parent() == Some(crate::directory_tree::path(&self.from)),
                    "Dragged entries must belong to their source folder."
                );
            }
        }
        Ok(())
    }
    pub fn can_move(&self) -> bool {
        writable(&self.from)
    }
    pub fn check_target(&self, to: &Location, moving: bool) -> Result<()> {
        self.validate()?;
        ensure!(writable(to), "This drop target is read-only.");
        ensure!(
            !moving || self.can_move(),
            "The source is read-only; copy these entries instead."
        );
        let same = match (&self.from, to) {
            (Location::Local(_), Location::Local(_)) => true,
            (Location::Remote { connection: a, .. }, Location::Remote { connection: b, .. }) => {
                a == b
            }
            (Location::Archive { file: a, .. }, Location::Archive { file: b, .. }) => a == b,
            _ => false,
        };
        if same {
            let base = crate::directory_tree::path(to);
            for entry in &self.entries {
                let target = base.join(entry.path.file_name().unwrap());
                ensure!(
                    target != entry.path && !(entry.is_dir && base.starts_with(&entry.path)),
                    "Cannot drop an item onto itself or into its own folder."
                );
            }
        }
        if let (Location::Local(_), Location::Archive { file, .. }) = (&self.from, to) {
            for entry in &self.entries {
                ensure!(
                    file != &entry.path && !(entry.is_dir && file.starts_with(&entry.path)),
                    "An archive cannot contain itself or its parent folder."
                );
            }
        }
        Ok(())
    }
}
pub fn writable(location: &Location) -> bool {
    match location {
        Location::Local(_) => true,
        Location::Remote { connection, .. } => {
            crate::remote::profile(connection).is_none_or(|c| c.service != "http")
        }
        Location::Archive { file, .. } => crate::archive_tools::editable(file),
    }
}

/// Transient staging paths must not become durable Undo records.
pub(crate) fn transient<T>(
    task: &mut Task,
    root: &Path,
    f: impl FnOnce(&mut Task) -> Result<T>,
) -> Result<T> {
    let hook = task.before_change.take();
    let shared = std::sync::Arc::new(std::sync::Mutex::new(hook));
    let capture = shared.clone();
    let root = root.to_owned();
    let physical = root.canonicalize().unwrap_or_else(|_| root.clone());
    let private_root = root.clone();
    let private_physical = physical.clone();
    task.before_change = Some(Box::new(move |path| {
        if !path.starts_with(&root)
            && !path.starts_with(&physical)
            && let Some(hook) = capture
                .lock()
                .map_err(|_| anyhow::anyhow!("Undo capture lock failed"))?
                .as_mut()
        {
            hook(path)?;
        }
        Ok(())
    }));
    task.private_roots.push(private_root);
    task.private_roots.push(private_physical);
    let result = f(task);
    task.private_roots.pop();
    task.private_roots.pop();
    task.before_change = shared
        .lock()
        .map_err(|_| anyhow::anyhow!("Undo capture lock failed"))?
        .take();
    result
}

pub struct Export {
    pub directory: tempfile::TempDir,
    pub paths: Vec<PathBuf>,
    _lease: Option<fs::File>,
}
pub fn export(selection: &Selection, task: &mut Task) -> Result<Export> {
    selection.validate()?;
    export_in(selection, task, None)
}
pub(crate) fn export_in(
    selection: &Selection,
    task: &mut Task,
    parent: Option<&Path>,
) -> Result<Export> {
    selection.validate()?;
    let before = witness(selection, task)?;
    let directory = if let Some(parent) = parent {
        tempfile::Builder::new()
            .prefix("export-")
            .tempdir_in(parent)?
    } else {
        tempfile::Builder::new()
            .prefix("rust-commander-drag-")
            .tempdir()?
    };
    let lease = if parent.is_some() {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(directory.path().join("lease.lock"))?;
        file.try_lock()?;
        file.write_all(b"RustCommanderExportLease1")?;
        fs::write(
            directory.path().join("created.json"),
            serde_json::to_vec(&std::time::SystemTime::now())?,
        )?;
        Some(file)
    } else {
        None
    };
    let tree = directory.path().join("files");
    match &selection.from {
        Location::Archive { file, prefix } => {
            crate::archive::extract(
                file,
                &selection
                    .entries
                    .iter()
                    .map(|e| e.path.clone())
                    .collect::<Vec<_>>(),
                prefix,
                &tree,
                task,
            )?;
        }
        _ => {
            fs::create_dir(&tree)?;
            crate::remote::transfer(
                &selection.from,
                &selection.entries,
                &Location::Local(tree.clone()),
                false,
                task,
            )?;
        }
    }
    let paths = selection
        .entries
        .iter()
        .map(|e| tree.join(e.path.file_name().unwrap()))
        .collect::<Vec<_>>();
    ensure!(
        paths.iter().all(|p| operations::exists(p)),
        "A dragged entry was not materialized."
    );
    ensure!(
        before == witness(selection, task)?,
        "A source changed while preparing the drag. Try again with the updated selection."
    );
    Ok(Export {
        directory,
        paths,
        _lease: lease,
    })
}

pub fn transfer(
    selection: &Selection,
    to: &Location,
    moving: bool,
    options: TransferOptions,
    overrides: &BTreeMap<PathBuf, ConflictPolicy>,
    task: &mut Task,
) -> Result<String> {
    selection.check_target(to, moving)?;
    if !matches!(selection.from, Location::Archive { .. })
        && !matches!(to, Location::Archive { .. })
    {
        return crate::remote::transfer_resolved(
            &selection.from,
            &selection.entries,
            to,
            moving,
            options,
            overrides,
            task,
        );
    }
    // Archive extraction, repacking, and verification do not share one byte budget.
    task.plan_total(None);
    let before = if moving {
        Some(witness(selection, task)?)
    } else {
        None
    };
    let staged = transient(task, Path::new(""), |task| export(selection, task))?;
    let same_archive = matches!((&selection.from,to),(Location::Archive{file:a,..},Location::Archive{file:b,..}) if a==b);
    let (message, complete) = match to {
        Location::Archive { file, prefix } => {
            let (source, original) = crate::archive_tools::persistent_source(file)?;
            let durable_undo = task.before_change.is_some() && source.local().is_some();
            crate::archive_tools::edit_tree(file, task, |tree, task| {
                let destination = if prefix.as_os_str().is_empty() {
                    tree.to_owned()
                } else {
                    tree.join(filesystem::portable_relative(prefix)?)
                };
                ensure!(
                    destination.is_dir(),
                    "The archive destination folder no longer exists."
                );
                let translated = overrides
                    .iter()
                    .map(|(p, policy)| Ok((tree.join(filesystem::portable_relative(p)?), *policy)))
                    .collect::<Result<BTreeMap<_, _>>>()?;
                let mut report = transfer::transfer_report(
                    &staged.paths,
                    &destination,
                    false,
                    options,
                    &translated,
                    task,
                )?;
                if let Some(recovery) = report.recovery {
                    // The private tree is discarded after repacking. Preserve replaced
                    // originals through Undo, or a durable whole-archive backup when
                    // a remote move cannot offer a complete Undo transaction.
                    let suffix = if source.local().is_none() {
                        "The server recovery copy is reported after saving.".to_owned()
                    } else if durable_undo {
                        "Replaced archive entries can be restored with Undo.".to_owned()
                    } else {
                        let mut backup = tempfile::Builder::new()
                            .prefix(".rust-commander-archive-backup-")
                            .suffix(&format!(
                                ".{}",
                                original.extension().unwrap_or_default().to_string_lossy()
                            ))
                            .tempfile_in(original.parent().context("Missing archive folder")?)?;
                        task.stream(&mut fs::File::open(&original)?, &mut backup, &original)?;
                        backup.as_file().sync_all()?;
                        let (_, path) = backup.keep()?;
                        crate::recovery::record_backup(&source, &path, &original, task)?;
                        format!("Original archive is kept in {}", path.display())
                    };
                    report.message = format!(
                        "{}\n{suffix}",
                        report
                            .message
                            .split("\nReplaced originals are kept in ")
                            .next()
                            .unwrap_or(&report.message)
                    );
                    fs::remove_dir_all(recovery)?;
                }
                if moving && same_archive {
                    for (entry, complete) in selection.entries.iter().zip(&report.complete) {
                        if *complete {
                            operations::remove(
                                &tree.join(filesystem::portable_relative(&entry.path)?),
                            )?;
                        }
                    }
                }
                Ok((report.message, report.complete))
            })?
        }
        Location::Local(destination) => {
            let report = transient(task, staged.directory.path(), |task| {
                transfer::transfer_report(
                    &staged.paths,
                    destination,
                    false,
                    options,
                    overrides,
                    task,
                )
            })?;
            (report.message, report.complete)
        }
        Location::Remote { .. } => {
            let local = Location::Local(staged.paths[0].parent().unwrap().into());
            let entries = staged
                .paths
                .iter()
                .map(|p| filesystem::entry_from_path(p))
                .collect::<Result<Vec<_>>>()?;
            let message = crate::remote::transfer_resolved(
                &local, &entries, to, true, options, overrides, task,
            )?;
            let complete = staged
                .paths
                .iter()
                .map(|p| !operations::exists(p))
                .collect();
            (message, complete)
        }
    };
    task.check()?;
    if moving && !same_archive {
        ensure!(
            Some(witness(selection, task)?) == before,
            "A source changed during the drop. Completed copies and all originals were kept."
        );
        let entries = selection
            .entries
            .iter()
            .zip(&complete)
            .filter(|(_, complete)| **complete)
            .map(|(entry, _)| entry.clone())
            .collect::<Vec<_>>();
        match &selection.from {
            Location::Local(_) => {
                for entry in entries {
                    task.capture(&entry.path)?;
                    operations::remove(&entry.path)?;
                }
            }
            Location::Remote { connection, .. } => {
                if !entries.is_empty() {
                    crate::remote::delete(connection, &entries, task)?;
                }
            }
            Location::Archive { file, .. } => {
                if !entries.is_empty() {
                    crate::archive_tools::edit_archive(
                        file,
                        &crate::archive_tools::ZipEdit::Remove(
                            entries.into_iter().map(|e| e.path).collect(),
                        ),
                        task,
                    )?;
                }
            }
        }
    }
    Ok(format!(
        "{message}{}",
        if moving {
            " Moved completed sources; skipped sources were kept."
        } else {
            ""
        }
    ))
}
fn witness(selection: &Selection, task: &Task) -> Result<Vec<String>> {
    match &selection.from {
        Location::Archive { file, .. } => Ok(vec![
            crate::history::fingerprint(file, task)?.context("The source archive disappeared")?,
        ]),
        _ => crate::remote::snapshot(&selection.from, &selection.entries, task),
    }
}

/// Preview virtual conflicts from names/metadata, without exporting payload bytes.
pub fn conflicts(
    selection: &Selection,
    to: &Location,
    merge: bool,
) -> Result<Vec<transfer::Conflict>> {
    if !matches!(selection.from, Location::Archive { .. })
        && !matches!(to, Location::Archive { .. })
    {
        return crate::remote::conflicts_with_merge(&selection.from, &selection.entries, to, merge);
    }
    struct Preview<'a> {
        from: &'a Location,
        to: &'a Location,
        merge: bool,
        archive_folders: BTreeMap<PathBuf, Vec<Entry>>,
        conflicts: Vec<transfer::Conflict>,
        visited: usize,
    }
    impl Preview<'_> {
        fn visit(&mut self, source: &Entry, target: PathBuf, depth: usize) -> Result<()> {
            self.visited += 1;
            ensure!(
                depth <= 128 && self.visited <= 100_000,
                "The conflict preview exceeds its folder/entry limit."
            );
            let existing = if let Location::Archive { file, .. } = self.to {
                let parent = target
                    .parent()
                    .context("Missing archive destination parent")?;
                if !self.archive_folders.contains_key(parent) {
                    self.archive_folders
                        .insert(parent.to_owned(), crate::archive::list(file, parent)?);
                }
                self.archive_folders[parent]
                    .iter()
                    .find(|entry| entry.path == target)
                    .cloned()
            } else {
                crate::remote::entry_at(self.to, &target)?
            };
            if let Some(existing) = existing {
                if self.merge
                    && source.is_dir
                    && existing.is_dir
                    && !source.is_symlink
                    && !existing.is_symlink
                {
                    for child in
                        filesystem::list(&crate::directory_tree::at(self.from, &source.path))?
                    {
                        let child_target = target.join(
                            child
                                .path
                                .file_name()
                                .context("Missing archive entry name")?,
                        );
                        self.visit(&child, child_target, depth + 1)?;
                    }
                } else {
                    self.conflicts.push(transfer::Conflict {
                        source: source.path.clone(),
                        target,
                        source_size: source.size,
                        target_size: existing.size,
                    });
                }
            }
            Ok(())
        }
    }
    let mut preview = Preview {
        from: &selection.from,
        to,
        merge,
        archive_folders: BTreeMap::new(),
        conflicts: Vec::new(),
        visited: 0,
    };
    for source in &selection.entries {
        preview.visit(
            source,
            crate::directory_tree::path(to).join(
                source
                    .path
                    .file_name()
                    .context("Missing dragged filename")?,
            ),
            0,
        )?;
    }
    Ok(preview.conflicts)
}
