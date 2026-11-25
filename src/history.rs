//! Local operation journal. Undo validates complete contents before changing files.
//!
//! [`execute`] records an operation's outcome and snapshots the local paths it
//! changes when journaling is supported. A failed or cancelled operation can still
//! have reversible changes. Check [`Record::success`] for the outcome and
//! [`Record::can_undo`] to decide whether to offer Undo.
//!
//! Run [`Operation::Undo`] through the journal to record the inverse operation
//! for Redo. To retrieve a backup as a separate file, use [`crate::recovery`].

use crate::operations::{self, Operation, Task};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

// Atomic replacement on Windows can fail while another thread is reading the
// target. Keep journal snapshots and replacements from overlapping.
static STORAGE: RwLock<()> = RwLock::new(());

/// A path's recorded states before and after an operation, with a backup when one exists.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    /// Absolute local path whose state the journal tracks.
    pub path: PathBuf,
    /// Local recovery copy of the entry as it existed before the operation, when retained.
    pub backup: Option<PathBuf>,
    /// Fingerprint of the recovery copy, checked before restoring it.
    pub backup_stamp: Option<String>,
    /// Fingerprint of the original path before execution. `None` means it did not exist.
    pub before: Option<String>,
    /// Fingerprint of the path after execution. `None` means it no longer exists.
    pub after: Option<String>,
    /// Fingerprint of the original entry's platform-specific metadata, when recorded.
    #[serde(default)]
    pub before_native: Option<String>,
    /// Platform-specific metadata fingerprint used to detect subsequent changes.
    #[serde(default)]
    pub after_native: Option<String>,
    /// Platform-specific metadata fingerprint of the recovery copy, when recorded.
    #[serde(default)]
    pub backup_native: Option<String>,
}

/// Where the user can recover originals displaced by an operation that replaces entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplacementRecovery {
    /// A local directory holds the originals displaced by replacement.
    Folder(PathBuf),
    /// Use the recovery manager to find retained originals.
    Manager,
    /// Undo of the journaled operation can restore the replaced contents.
    Undo,
}

/// The persisted outcome of an operation and the changes available for Undo or Redo.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    /// Stable journal identifier used as the record's directory name.
    pub id: String,
    /// Human-readable operation name displayed in history.
    pub label: String,
    /// The operation's display message on success or its error on failure.
    pub result: String,
    /// Where to find originals displaced by this operation, when reported by the worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_recovery: Option<ReplacementRecovery>,
    /// Whether the operation itself succeeded.
    pub success: bool,
    /// Whether cancellation was recorded for this operation.
    #[serde(default)]
    pub cancelled: bool,
    /// Time the record was created, in whole seconds since the Unix epoch.
    #[serde(default)]
    pub created_at: u64,
    /// Whether this action has been reversed or its Redo opportunity has been consumed.
    pub undone: bool,
    /// Whether the worker disabled automatic reversal, for example after an external change.
    #[serde(default)]
    pub undo_blocked: bool,
    /// Local paths with recorded changes, in the order in which they were captured.
    pub changes: Vec<Change>,
    /// ID of the record targeted by this Undo or Redo, including failed attempts.
    #[serde(default)]
    pub inverse_of: Option<String>,
    /// ID of the original operation in an Undo/Redo chain. A root record uses its own ID when
    /// this is absent.
    #[serde(default)]
    pub logical_root: Option<String>,
    /// Whether this record describes an Undo, making it a possible Redo candidate.
    #[serde(default)]
    pub is_undo: bool,
    /// Whether to protect this record's recovery chain from retention cleanup.
    #[serde(default)]
    pub pinned: bool,
    /// Whether cleanup has retired this record's recovery data.
    #[serde(default)]
    pub expired: bool,
    /// Last revision time in Unix nanoseconds, used to select the latest saved state.
    #[serde(default)]
    pub revision: u128,
}

/// Returns the history directory beside the application settings file.
///
/// # Errors
///
/// Returns an error if the settings path or its parent cannot be determined.
pub fn root() -> Result<PathBuf> {
    Ok(crate::settings::path()?
        .parent()
        .context("Missing configuration directory")?
        .join("history"))
}

/// Loads recovery and Undo records from the application's history directory.
///
/// # Errors
///
/// Returns an error if the directory cannot be located or [`load_in`] fails.
pub fn load() -> Result<Vec<Record>> {
    load_in(&root()?)
}

/// Loads the index and merges individually saved operation records by ID and revision. A
/// missing or malformed index can be rebuilt in memory from those records. Results are
/// ordered by creation time and revision.
///
/// # Errors
///
/// Returns an error if the storage lock or directory cannot be read.
pub fn load_in(root: &Path) -> Result<Vec<Record>> {
    let _storage = STORAGE
        .read()
        .map_err(|_| anyhow::anyhow!("History storage lock failed"))?;
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
                    if record.revision >= existing.revision {
                        *existing = record;
                    }
                } else {
                    records.push(record);
                }
            }
        }
    }
    records.sort_by_key(|record| (record.created_at, record.revision));
    Ok(records)
}

fn write_record(path: &Path, record: &Record) -> Result<()> {
    let _storage = STORAGE
        .write()
        .map_err(|_| anyhow::anyhow!("History storage lock failed"))?;
    write_record_locked(path, record)
}

fn write_record_locked(path: &Path, record: &Record) -> Result<()> {
    let parent = path.parent().context("Missing history directory")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec_pretty(record)?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

/// Persists records and the history index in the application's history directory.
///
/// # Errors
///
/// Returns an error if the directory cannot be located or [`save_in`] fails.
pub fn save(records: &[Record]) -> Result<()> {
    save_in(records, &root()?)
}

/// Writes per-operation records and replaces the index in `directory`. Backs up a malformed
/// existing index before replacing it.
///
/// # Errors
///
/// Returns an error for an invalid record ID or if storage cannot be locked, created, or
/// written.
pub fn save_in(records: &[Record], directory: &Path) -> Result<()> {
    let _storage = STORAGE
        .write()
        .map_err(|_| anyhow::anyhow!("History storage lock failed"))?;
    fs::create_dir_all(directory)?;
    for record in records {
        crate::filesystem::validate_name(&record.id)?;
        write_record_locked(&directory.join(&record.id).join("record.json"), record)?;
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

fn native_fingerprint(path: &Path, task: &Task) -> Result<Option<String>> {
    if !operations::exists(path) {
        return Ok(None);
    }
    fn visit(path: &Path, hash: &mut Sha256, task: &Task) -> Result<()> {
        task.check()?;
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        hash.update(crate::file_metadata::digest(path)?);
        if metadata.is_dir() {
            let mut entries = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                hash.update(entry.file_name().as_encoded_bytes());
                visit(&entry.path(), hash, task)?;
            }
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
    control: Option<Arc<crate::transfer_control::Control>>,
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
        task.control = self.control.clone();
        let before = fingerprint(&path, &task)?;
        let before_native = native_fingerprint(&path, &task)?;
        let backup = if before.is_some() {
            let backup = self.directory.join(self.changes.len().to_string());
            operations::copy_item(&path, &backup, &mut task)?;
            if before != fingerprint(&path, &task)?
                || before_native != native_fingerprint(&path, &task)?
            {
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
        let backup_native = backup
            .as_deref()
            .map(|path| native_fingerprint(path, &task))
            .transpose()?
            .flatten();
        self.changes.push(Change {
            before_native,
            after_native: None,
            backup_native,
            path,
            backup,
            backup_stamp,
            before,
            after: None,
        });
        Ok(())
    }
}

/// Runs and journals an operation in the application's history directory.
/// See [`execute_in`] for the difference between an operation failure and a journal error.
pub fn execute(operation: Operation, task: &mut Task) -> Result<Record> {
    execute_in(operation, task, &root()?)
}

/// Runs an operation and persists its record under the history directory `root`.
///
/// The record contains the outcome even when the operation fails. Its
/// [`result`](Record::result) holds the operation's message or error.
///
/// # Errors
///
/// Returns an error if the journal cannot be prepared, the changes cannot be
/// inspected, or the record cannot be stored.
///
/// # Examples
///
/// Journal a directory creation, then undo it and obtain the record for Redo.
///
/// ```
/// use rust_commander::history;
/// use rust_commander::operations::{Operation, Task};
///
/// let workspace = tempfile::tempdir()?;
/// let journal = workspace.path().join("history");
/// let folder = workspace.path().join("drafts");
/// let create = Operation::Mkdir(folder.clone());
/// let record = history::execute_in(create, &mut Task::default(), &journal)?;
/// assert!(record.can_undo());
/// assert!(folder.is_dir());
///
/// let undo = Operation::Undo(Box::new(record));
/// let inverse = history::execute_in(undo, &mut Task::default(), &journal)?;
/// assert!(inverse.success, "{}", inverse.result);
/// assert!(inverse.can_redo());
/// assert!(!folder.exists());
/// # Ok::<(), anyhow::Error>(())
/// ```
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
        Operation::Undo(record) | Operation::Redo(record) => Some((**record).clone()),
        _ => None,
    };
    let is_undo = matches!(operation, Operation::Undo(_));
    let logical_root = undo_of
        .as_ref()
        .map(|r| r.logical_root.clone().unwrap_or_else(|| r.id.clone()));
    let recorder = Arc::new(Mutex::new(Recorder {
        directory: directory.canonicalize()?,
        changes: Vec::new(),
        cancel: task.cancel.clone(),
        control: task.control.clone(),
    }));
    let capture = recorder.clone();
    let local_undo = !matches!(
        operation,
        Operation::RemoteTransfer { .. } | Operation::RemoteTransferResolved { .. }
    ) && !matches!(&operation, Operation::DropTransfer{selection,to,..} if matches!(selection.from, crate::filesystem::Location::Remote{..}) || matches!(to,crate::filesystem::Location::Remote{..}));
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
    task.replacement_recovery = None;
    let result = operations::run(operation, task);
    task.before_change = None;
    let mut recorder = recorder
        .lock()
        .map_err(|_| anyhow::anyhow!("Undo journal lock failed"))?;
    // Cancellation must not prevent recording the actual partial result.
    let inspect = Task::default();
    for change in &mut recorder.changes {
        change.after_native = native_fingerprint(&change.path, &inspect)?;
        change.after = fingerprint(&change.path, &inspect).with_context(|| {
            format!(
                "Could not finalize the undo journal. Recovery copies remain in {}",
                directory.display()
            )
        })?;
    }
    for change in &recorder.changes {
        if change.before == change.after
            && change.before_native == change.after_native
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
    recorder.changes.retain(|change| {
        change.before != change.after || change.before_native != change.after_native
    });
    if result.is_ok()
        && let Some(mut original) = undo_of.clone()
    {
        original.undone = true;
        original.revision = revision();
        if let Some(root_id) = &logical_root
            && root_id != &original.id
            && let Some(mut logical) = load_in(root)?.into_iter().find(|r| &r.id == root_id)
        {
            logical.undone = is_undo;
            logical.revision = revision();
            write_record(&root.join(&logical.id).join("record.json"), &logical)?;
        }
        write_record(&root.join(&original.id).join("record.json"), &original)?;
    }
    let changes = std::mem::take(&mut recorder.changes);
    let mut record = Record {
        id,
        label,
        success: result.is_ok(),
        cancelled: result.is_err() && task.cancel.load(std::sync::atomic::Ordering::Relaxed),
        created_at: crate::oauth::now(),
        result: result.unwrap_or_else(|error| format!("{error:#}")),
        replacement_recovery: task.replacement_recovery.take(),
        undone: false,
        undo_blocked: task.undo_blocked,
        changes,
        inverse_of: undo_of.as_ref().map(|r| r.id.clone()),
        logical_root,
        is_undo,
        pinned: false,
        expired: false,
        revision: revision(),
    };
    if record.undo_blocked && !record.changes.is_empty() {
        record.result.push_str(&format!("\nAutomatic Undo is unavailable because files changed externally during the operation. Recovery copies remain in {}.",directory.display()));
    }
    if record.inverse_of.is_none() && !record.changes.is_empty() {
        for mut stale in load_in(root)?.into_iter().filter(|r| r.can_redo()) {
            stale.undone = true;
            stale.revision = revision();
            write_record(&root.join(&stale.id).join("record.json"), &stale)?;
        }
    }
    write_record(&directory.join("record.json"), &record)?;
    Ok(record)
}

/// Reverses the paths in a record after validating current files and retained backups.
/// Use [`execute`] with [`Operation::Undo`] to journal the inverse for Redo.
///
/// # Errors
///
/// Fails if Undo is unavailable, a path or backup differs from its recorded
/// state, or staging or applying the changes fails.
pub fn undo(record: &Record, task: &mut Task) -> Result<()> {
    if record.undone || record.undo_blocked || record.changes.is_empty() {
        bail!("This operation has no pending undo.");
    }
    for change in &record.changes {
        if fingerprint(&change.path, task)? != change.after
            || (change.after_native.is_some()
                && native_fingerprint(&change.path, task)? != change.after_native)
        {
            bail!(
                "Undo refused: {} changed after the operation.",
                change.path.display()
            );
        }
        if let Some(backup) = &change.backup
            && (fingerprint(backup, task)? != change.backup_stamp
                || (change.backup_native.is_some()
                    && native_fingerprint(backup, task)? != change.backup_native))
        {
            bail!("Undo backup changed: {}", backup.display());
        }
    }
    for change in &record.changes {
        task.capture(&change.path)?;
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
        if fingerprint(&change.path, task)? != change.after
            || (change.after_native.is_some()
                && native_fingerprint(&change.path, task)? != change.after_native)
        {
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

/// Returns the current Unix time in whole seconds as a decimal string. Multiple calls
/// within one second can produce the same value.
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
        replacement_recovery: None,
        success: false,
        cancelled: true,
        created_at: crate::oauth::now(),
        undone: false,
        undo_blocked: false,
        changes: Vec::new(),
        inverse_of: None,
        logical_root: None,
        is_undo: false,
        pinned: false,
        expired: false,
        revision: revision(),
    }
}

fn revision() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
impl Record {
    /// Returns whether this record has pending changes eligible for Undo.
    /// Applying Undo rechecks the on-disk state.
    pub fn can_undo(&self) -> bool {
        !self.undone
            && !self.undo_blocked
            && !self.is_undo
            && !self.expired
            && !self.changes.is_empty()
    }
    /// Returns whether this Undo record has pending changes eligible for Redo.
    pub fn can_redo(&self) -> bool {
        !self.undone
            && !self.undo_blocked
            && self.is_undo
            && !self.expired
            && !self.changes.is_empty()
    }
}
/// Age and size limits used when choosing which unpinned recovery records to expire.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(default)]
pub struct Retention {
    /// History storage budget in mebibytes used to select cleanup candidates.
    pub max_mib: u64,
    /// Retention age in days, measured from the newest record in a recovery chain.
    pub max_days: u64,
    /// Whether to run retention cleanup automatically after an operation.
    pub automatic: bool,
}
impl Default for Retention {
    fn default() -> Self {
        Self {
            max_mib: 2048,
            max_days: 30,
            automatic: false,
        }
    }
}
/// Summary of history storage, referenced backup paths, and bytes in unreferenced
/// directories.
#[derive(Default, Debug)]
pub struct Storage {
    /// Sum of file and symlink lengths beneath the history directory, in bytes.
    pub bytes: u64,
    /// Number of backup paths referenced by journaled changes.
    pub backups: usize,
    /// Bytes in history subdirectories whose names match no loaded record ID.
    pub orphan_bytes: u64,
}
fn disk_usage(path: &Path) -> Result<u64> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(meta.len());
    }
    if meta.is_dir() {
        let mut total = 0;
        for entry in fs::read_dir(path)? {
            total += disk_usage(&entry?.path())?;
        }
        Ok(total)
    } else {
        Ok(meta.len())
    }
}
/// Reports total history bytes, bytes in unreferenced directories, and the number of
/// recorded backup references. A missing directory has zero usage.
///
/// # Errors
///
/// Returns an error if the records or their disk usage cannot be read.
pub fn storage_in(root: &Path) -> Result<Storage> {
    let mut info = Storage::default();
    if !root.exists() {
        return Ok(info);
    }
    let records = load_in(root)?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let size = disk_usage(&entry.path())?;
        info.bytes += size;
        if entry.file_type()?.is_dir()
            && !records
                .iter()
                .any(|r| entry.file_name() == std::ffi::OsStr::new(&r.id))
        {
            info.orphan_bytes += size;
        }
    }
    info.backups = records
        .iter()
        .flat_map(|r| &r.changes)
        .filter(|c| c.backup.is_some())
        .count();
    Ok(info)
}
/// Select whole recovery chains; pinned chains and the most recent reversible chain are protected.
pub fn cleanup_candidates(
    records: &[Record],
    root: &Path,
    policy: &Retention,
    now: u64,
) -> Result<Vec<String>> {
    let mut total = storage_in(root)?.bytes;
    let latest = records
        .iter()
        .rev()
        .find(|r| r.can_undo() || r.can_redo())
        .map(|r| r.logical_root.as_ref().unwrap_or(&r.id));
    let mut groups: std::collections::BTreeMap<String, Vec<&Record>> = Default::default();
    for r in records {
        groups
            .entry(r.logical_root.clone().unwrap_or_else(|| r.id.clone()))
            .or_default()
            .push(r);
    }
    let mut groups = groups.into_iter().collect::<Vec<_>>();
    groups.sort_by_key(|(_, rs)| rs.iter().map(|r| r.created_at).max().unwrap_or(0));
    let mut result = Vec::new();
    for (id, rs) in groups {
        if Some(&id) == latest || rs.iter().any(|r| r.pinned) {
            continue;
        }
        let age = now.saturating_sub(rs.iter().map(|r| r.created_at).max().unwrap_or(now));
        if total > policy.max_mib.saturating_mul(1024 * 1024)
            || age > policy.max_days.saturating_mul(86400)
        {
            for r in rs {
                if !r.expired && !r.changes.is_empty() {
                    result.push(r.id.clone());
                    for c in &r.changes {
                        if let Some(p) = &c.backup {
                            total = total.saturating_sub(disk_usage(p).unwrap_or(0));
                        }
                    }
                }
            }
        }
    }
    Ok(result)
}
/// Expires the selected records and saves their new state before removing their recovery
/// copies. Returns the updated record list, including expired entries.
///
/// # Errors
///
/// Returns an error for a pinned record, an invalid backup path, or an I/O failure. A
/// deletion failure can leave a backup after its record was expired.
pub fn cleanup_in(root: &Path, ids: &[String]) -> Result<Vec<Record>> {
    let mut records = load_in(root)?;
    let mut remove = Vec::new();
    for r in &mut records {
        if !ids.contains(&r.id) {
            continue;
        }
        if r.pinned {
            bail!("Pinned recovery record: {}", r.label);
        }
        crate::filesystem::validate_name(&r.id)?;
        let directory = root.join(&r.id);
        if fs::symlink_metadata(&directory)?.file_type().is_symlink() {
            bail!("Recovery directory is a symlink");
        }
        for c in &r.changes {
            if let Some(p) = &c.backup {
                if p.parent() != Some(directory.canonicalize()?.as_path())
                    || p.file_name()
                        .is_none_or(|n| n.to_string_lossy().parse::<usize>().is_err())
                {
                    bail!("Invalid recovery path");
                }
                remove.push(p.clone());
            }
        }
        r.changes.clear();
        r.expired = true;
        r.revision = revision();
    }
    // Persist the expiry before deleting; a crash can leave extra copies, never a false Undo promise.
    save_in(&records, root)?;
    for p in remove {
        if operations::exists(&p) {
            operations::remove(&p)?;
        }
    }
    Ok(records)
}
