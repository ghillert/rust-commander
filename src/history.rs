//! Local operation journal. Undo validates complete contents before changing files.
use crate::operations::{self, Operation, Task};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub path: PathBuf,
    pub backup: Option<PathBuf>,
    pub backup_stamp: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub label: String,
    pub result: String,
    pub success: bool,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub created_at: u64,
    pub undone: bool,
    pub changes: Vec<Change>,
}

fn root() -> Result<PathBuf> {
    Ok(crate::settings::path()?
        .parent()
        .context("Missing configuration directory")?
        .join("history"))
}

pub fn load() -> Result<Vec<Record>> {
    load_in(&root()?)
}

pub fn load_in(root: &Path) -> Result<Vec<Record>> {
    let mut records = match fs::read(root.join("index.json")) {
        Ok(data) => serde_json::from_slice::<Vec<Record>>(&data).unwrap_or_default(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    if root.exists() {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir()
                || !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("operation-")
            {
                continue;
            }
            if let Ok(data) = fs::read(entry.path().join("record.json"))
                && let Ok(record) = serde_json::from_slice::<Record>(&data)
            {
                if let Some(existing) = records.iter_mut().find(|existing| existing.id == record.id)
                {
                    existing.undone |= record.undone;
                } else {
                    records.push(record);
                }
            }
        }
    }
    records.sort_by_key(|record| record.created_at);
    Ok(records)
}

fn write_record(path: &Path, record: &Record) -> Result<()> {
    let parent = path.parent().context("Missing history directory")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec_pretty(record)?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

pub fn save(records: &[Record]) -> Result<()> {
    save_in(records, &root()?)
}

pub fn save_in(records: &[Record], directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    for record in records {
        crate::filesystem::validate_name(&record.id)?;
        write_record(&directory.join(&record.id).join("record.json"), record)?;
    }
    let index = directory.join("index.json");
    if let Ok(data) = fs::read(&index)
        && serde_json::from_slice::<Vec<Record>>(&data).is_err()
    {
        fs::copy(
            &index,
            directory.join(format!("index.invalid-{}.json", timestamp())),
        )?;
    }
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&serde_json::to_vec_pretty(records)?)?;
    file.as_file().sync_all()?;
    file.persist(index)?;
    Ok(())
}

pub(crate) fn fingerprint(path: &Path, task: &Task) -> Result<Option<String>> {
    if !operations::exists(path) {
        return Ok(None);
    }
    fn visit(path: &Path, hash: &mut Sha256, task: &Task) -> Result<()> {
        task.check()?;
        let metadata = fs::symlink_metadata(path)?;
        hash.update([
            metadata.is_dir() as u8,
            metadata.file_type().is_symlink() as u8,
            metadata.permissions().readonly() as u8,
        ]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            hash.update(metadata.permissions().mode().to_le_bytes());
        }
        if !metadata.is_dir() {
            hash.update(metadata.len().to_le_bytes());
        }
        if let Ok(time) = metadata.modified()?.duration_since(UNIX_EPOCH) {
            hash.update(time.as_nanos().to_le_bytes());
        }
        if metadata.file_type().is_symlink() {
            hash.update(fs::read_link(path)?.as_os_str().as_encoded_bytes());
        } else if metadata.is_dir() {
            let mut children = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
            children.sort_by_key(|entry| entry.file_name());
            for entry in children {
                hash.update((entry.file_name().as_encoded_bytes().len() as u64).to_le_bytes());
                hash.update(entry.file_name().as_encoded_bytes());
                visit(&entry.path(), hash, task)?;
            }
        } else if metadata.is_file() {
            let mut file = fs::File::open(path)?;
            let mut buffer = [0; 65536];
            loop {
                task.check()?;
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                hash.update(&buffer[..count]);
            }
        } else {
            bail!("Cannot journal special file: {}", path.display());
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    visit(path, &mut hash, task)?;
    Ok(Some(format!("{:x}", hash.finalize())))
}

struct Recorder {
    directory: PathBuf,
    changes: Vec<Change>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}
impl Recorder {
    fn capture(&mut self, path: &Path) -> Result<()> {
        let absolute = std::path::absolute(path)?;
        let path = absolute
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .and_then(|parent| absolute.file_name().map(|name| parent.join(name)))
            .unwrap_or(absolute);
        if self
            .changes
            .iter()
            .any(|change| path.starts_with(&change.path))
        {
            return Ok(());
        }
        if self.directory.starts_with(&path)
            || self
                .changes
                .iter()
                .any(|change| change.path.starts_with(&path))
        {
            bail!("Undo storage overlaps this operation. Choose another location.");
        }
        let mut task = Task::new(self.cancel.clone(), |_| {});
        let before = fingerprint(&path, &task)?;
        let backup = if before.is_some() {
            let backup = self.directory.join(self.changes.len().to_string());
            operations::copy_item(&path, &backup, &mut task)?;
            if before != fingerprint(&path, &task)? {
                bail!("File changed while preparing undo: {}", path.display());
            }
            Some(backup)
        } else {
            None
        };
        let backup_stamp = backup
            .as_deref()
            .map(|path| fingerprint(path, &task))
            .transpose()?
            .flatten();
        self.changes.push(Change {
            path,
            backup,
            backup_stamp,
            before,
            after: None,
        });
        Ok(())
    }
}

pub fn execute(operation: Operation, task: &mut Task) -> Result<Record> {
    execute_in(operation, task, &root()?)
}

pub fn execute_in(operation: Operation, task: &mut Task, root: &Path) -> Result<Record> {
    fs::create_dir_all(root)?;
    let directory = tempfile::Builder::new()
        .prefix("operation-")
        .tempdir_in(root)?
        .keep();
    let id = directory
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let label = operation.label().to_owned();
    let undo_of = match &operation {
        Operation::Undo(record) => Some((**record).clone()),
        _ => None,
    };
    let recorder = Arc::new(Mutex::new(Recorder {
        directory: directory.canonicalize()?,
        changes: Vec::new(),
        cancel: task.cancel.clone(),
    }));
    let capture = recorder.clone();
    let local_undo = !matches!(
        operation,
        Operation::RemoteTransfer { .. } | Operation::RemoteTransferResolved { .. }
    );
    task.before_change = if local_undo {
        Some(Box::new(move |path: &Path| {
            capture
                .lock()
                .map_err(|_| anyhow::anyhow!("Undo journal lock failed"))?
                .capture(path)
        }) as crate::operations::ChangeHook)
    } else {
        None
    };
    let result = operations::run(operation, task);
    task.before_change = None;
    let mut recorder = recorder
        .lock()
        .map_err(|_| anyhow::anyhow!("Undo journal lock failed"))?;
    // Cancellation must not prevent recording the actual partial result.
    let inspect = Task::default();
    for change in &mut recorder.changes {
        change.after = fingerprint(&change.path, &inspect).with_context(|| {
            format!(
                "Could not finalize the undo journal. Recovery copies remain in {}",
                directory.display()
            )
        })?;
    }
    for change in &recorder.changes {
        if change.before == change.after
            && let Some(backup) = &change.backup
        {
            operations::remove(backup).with_context(|| {
                format!(
                    "Could not remove an unused recovery copy: {}",
                    backup.display()
                )
            })?;
        }
    }
    recorder
        .changes
        .retain(|change| change.before != change.after);
    if result.is_ok()
        && let Some(mut original) = undo_of
    {
        original.undone = true;
        write_record(&root.join(&original.id).join("record.json"), &original)?;
    }
    let changes = std::mem::take(&mut recorder.changes);
    let record = Record {
        id,
        label,
        success: result.is_ok(),
        cancelled: result.is_err() && task.cancel.load(std::sync::atomic::Ordering::Relaxed),
        created_at: crate::oauth::now(),
        result: result.unwrap_or_else(|error| format!("{error:#}")),
        undone: false,
        changes,
    };
    write_record(&directory.join("record.json"), &record)?;
    Ok(record)
}

pub fn undo(record: &Record, task: &mut Task) -> Result<()> {
    if record.undone || record.changes.is_empty() {
        bail!("This operation has no pending undo.");
    }
    for change in &record.changes {
        if fingerprint(&change.path, task)? != change.after {
            bail!(
                "Undo refused: {} changed after the operation.",
                change.path.display()
            );
        }
        if let Some(backup) = &change.backup
            && fingerprint(backup, task)? != change.backup_stamp
        {
            bail!("Undo backup changed: {}", backup.display());
        }
    }
    let mut stages = Vec::new();
    for change in &record.changes {
        let parent = change
            .path
            .parent()
            .context("Cannot undo a filesystem root")?;
        let stage = tempfile::Builder::new()
            .prefix(".rust-commander-undo-")
            .tempdir_in(parent)?;
        if let Some(backup) = &change.backup {
            operations::copy_item(backup, &stage.path().join("restore"), task)?;
        }
        stages.push(stage);
    }
    // Re-check after staging, before committing any restoration.
    for change in &record.changes {
        if fingerprint(&change.path, task)? != change.after {
            bail!(
                "Undo refused: {} changed while preparing restoration.",
                change.path.display()
            );
        }
    }
    let mut committed = Vec::new();
    let result = (|| -> Result<()> {
        for (index, change) in record.changes.iter().enumerate() {
            task.check()?;
            let stage = stages[index].path();
            if operations::exists(&change.path) {
                crate::desktop::rename_exclusive(&change.path, &stage.join("current"))?;
            }
            committed.push((index, false));
            if change.backup.is_some() {
                crate::desktop::rename_exclusive(&stage.join("restore"), &change.path)?;
                committed.last_mut().unwrap().1 = true;
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut rollback_error = None;
        for (index, restored) in committed.into_iter().rev() {
            let change = &record.changes[index];
            let stage = stages[index].path();
            let rollback = (|| -> Result<()> {
                if restored {
                    crate::desktop::rename_exclusive(&change.path, &stage.join("restored"))?;
                }
                if operations::exists(&stage.join("current")) {
                    crate::desktop::rename_exclusive(&stage.join("current"), &change.path)?;
                }
                Ok(())
            })();
            if let Err(e) = rollback {
                rollback_error = Some(e);
            }
        }
        if let Some(rollback) = rollback_error {
            let locations = stages
                .into_iter()
                .map(|stage| stage.keep().display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "Undo failed: {error:#}. Rollback failed: {rollback:#}. Recovery folders:\n{locations}"
            );
        }
        return Err(error);
    }
    Ok(())
}

pub fn timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

/// Cancellation of a queued operation is recorded without serializing its parameters.
pub fn cancelled(label: &str) -> Record {
    Record {
        id: format!("operation-{}-{:016x}", timestamp(), rand::random::<u64>()),
        label: label.into(),
        result: "Cancelled before starting.".into(),
        success: false,
        cancelled: true,
        created_at: crate::oauth::now(),
        undone: false,
        changes: Vec::new(),
    }
}
