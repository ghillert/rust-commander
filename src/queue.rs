//! Durable recipes. Passwords and editor contents are never serialized.
//!
//! [`Queue`] persists waiting, active, and failed operations. On restart,
//! [`Queue::load_in`] puts the interrupted operation first and pauses the queue
//! for review. [`Queue::requires_input`] identifies jobs that need fresh input
//! or confirmation before they can run.
//!
//! For a single local file, [`Queue::prepare`] can produce a resumable copy.
//! [`crate::resumable`] handles checkpointed batches.

use crate::operations::{Operation, Task};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::RwLock,
};

// Windows can reject replacement while another reader/writer still holds the
// current snapshot open. Keep handles within the same coordinated access.
static STORAGE: RwLock<()> = RwLock::new(());

/// A snapshot of pending and failed work. Deque operations on a `Queue` act
/// on [`waiting`](Self::waiting) in execution order.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Queue {
    /// Operations awaiting execution, with the next job at the front.
    pub waiting: VecDeque<Operation>,
    /// The operation in progress when this snapshot was saved.
    pub active: Option<Operation>,
    /// Failed operations retained for review or retry.
    pub failed: Vec<Operation>,
    /// Whether the scheduler should wait for the user before starting another job.
    #[serde(skip)]
    pub paused: bool,
    /// A load failure reported by [`Queue::load`] that blocks [`Queue::save`].
    #[serde(skip)]
    pub error: Option<String>,
}
impl Deref for Queue {
    type Target = VecDeque<Operation>;
    fn deref(&self) -> &Self::Target {
        &self.waiting
    }
}
impl DerefMut for Queue {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.waiting
    }
}
impl Queue {
    /// Returns the path of `queue.json` beside the application's settings file.
    pub fn path() -> Result<PathBuf> {
        Ok(crate::settings::path()?.with_file_name("queue.json"))
    }
    /// Loads the application's queue. A missing snapshot yields an empty queue.
    /// Other failures yield a paused queue with a message in [`Queue::error`].
    pub fn load() -> Self {
        match Self::path().and_then(|p| Self::load_in(&p)) {
            Ok(q) => q,
            Err(e) => Self {
                paused: true,
                error: Some(format!(
                    "Queue could not be loaded; its file is preserved: {e:#}"
                )),
                ..Default::default()
            },
        }
    }
    /// Loads the snapshot at `path`, or an empty queue if the file is missing.
    ///
    /// Any saved `active` operation moves to the front of `waiting`. A queue
    /// with waiting work starts paused, and failed jobs stay in `failed`.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot cannot be read or decoded.
    ///
    /// # Examples
    ///
    /// An interrupted directory creation is offered again before the file
    /// creation that depended on it.
    ///
    /// ```
    /// use rust_commander::{operations::Operation, queue::Queue};
    ///
    /// let workspace = tempfile::tempdir()?;
    /// let path = workspace.path().join("queue.json");
    /// let mut queue = Queue {
    ///     active: Some(Operation::Mkdir("drafts".into())),
    ///     ..Queue::default()
    /// };
    /// queue.waiting.push_back(Operation::Mkfile("drafts/notes.txt".into()));
    /// queue.save_in(&path)?;
    ///
    /// let restored = Queue::load_in(&path)?;
    /// assert!(restored.paused);
    /// assert!(restored.active.is_none());
    /// assert_eq!(restored.waiting.len(), 2);
    /// assert!(matches!(restored.waiting[0], Operation::Mkdir(_)));
    /// assert!(matches!(restored.waiting[1], Operation::Mkfile(_)));
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn load_in(path: &Path) -> Result<Self> {
        let _storage = STORAGE
            .read()
            .map_err(|_| anyhow::anyhow!("Queue storage lock failed"))?;
        let mut q: Self = match fs::read(path) {
            Ok(data) => serde_json::from_slice(&data)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.into()),
        };
        if let Some(op) = q.active.take() {
            q.waiting.push_front(op);
        }
        q.paused = !q.waiting.is_empty();
        Ok(q)
    }
    /// Saves the queue at [`Queue::path`]. An outstanding [`Queue::error`]
    /// blocks the write so the original snapshot can be repaired.
    pub fn save(&self) -> Result<()> {
        if self.error.is_some() {
            bail!("Repair the saved queue before overwriting it");
        }
        self.save_in(&Self::path()?)
    }
    /// Writes a snapshot to `path`, creating parent directories as needed.
    /// The replacement is staged and synced in the destination directory.
    /// This explicit-path method writes the supplied state even if [`Queue::error`] is set.
    ///
    /// # Errors
    ///
    /// Fails if the snapshot cannot be serialized, written, or installed at `path`.
    pub fn save_in(&self, path: &Path) -> Result<()> {
        let _storage = STORAGE
            .write()
            .map_err(|_| anyhow::anyhow!("Queue storage lock failed"))?;
        let parent = path.parent().context("Missing queue folder")?;
        fs::create_dir_all(parent)?;
        let mut f = tempfile::NamedTempFile::new_in(parent)?;
        f.write_all(&serde_json::to_vec_pretty(self)?)?;
        f.as_file().sync_all()?;
        f.persist(path)?;
        Ok(())
    }
    /// Returns whether the scheduler must pause this recipe for input or confirmation.
    ///
    /// Encrypted archive jobs need a password if theirs is empty. Editor
    /// saves, recovery actions, and emptying the trash always require review.
    pub fn requires_input(op: &Operation) -> bool {
        matches!(op,Operation::PackProtected{password,..}|Operation::ExtractProtected{password,..} if password.is_empty())
            || matches!(op,Operation::PackTo{encrypted:true,password,..} if password.is_empty())
            || matches!(
                op,
                Operation::RecoveryRestore { .. }
                    | Operation::RecoveryCleanup(_)
                    | Operation::EmptyTrash
                    | Operation::SaveText { .. }
                    | Operation::RemoteSave { .. }
                    | Operation::SaveBytes { .. }
                    | Operation::RemoteSaveBytes { .. }
                    | Operation::RemoteExternalSave { .. }
                    | Operation::ExternalSave { .. }
                    | Operation::ArchiveSaveBytes { .. }
            )
    }
    /// Prepares an eligible single-file local copy for interruption and restart.
    ///
    /// [`Operation::Copy`] qualifies for a single regular file. [`Operation::Transfer`]
    /// and [`Operation::ResolvedTransfer`] also qualify for such a file if they
    /// copy, refuse conflicts, and have no per-target overrides. The returned
    /// [`Operation::ResumeLocal`] records the destination filename, a path for
    /// partial data, and a stamp of the source. Other recipes pass through as supplied.
    ///
    /// # Errors
    ///
    /// Fails if the eligible copy's paths or source stamp cannot be resolved,
    /// or its checkpoint directory cannot be prepared.
    pub fn prepare(op: Operation) -> Result<Operation> {
        let single = resumable(&op);
        if let Some((source, destination, moving)) = single
            && fs::symlink_metadata(source)
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        {
            let source = source.canonicalize()?;
            let root = Self::path()?.with_file_name("transfers");
            fs::create_dir_all(&root)?;
            if fs::symlink_metadata(&root)?.file_type().is_symlink() {
                bail!("Transfer storage is a symlink");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
            }
            return Ok(Operation::ResumeLocal {
                source: source.clone(),
                destination: destination
                    .canonicalize()?
                    .join(source.file_name().context("Missing filename")?),
                moving,
                partial: root.join(format!("transfer-{:016x}.part", rand::random::<u64>())),
                stamp: format!(
                    "metadata:{:x?}",
                    crate::operations::source_stamp(&source, &Task::default())?
                ),
            });
        }
        Ok(op)
    }
}
fn resumable(op: &Operation) -> Option<(&PathBuf, &PathBuf, bool)> {
    match op {
        Operation::Copy {
            sources,
            destination,
        } if sources.len() == 1 => Some((
            &sources[0],
            destination,
            matches!(op, Operation::Move { .. }),
        )),
        Operation::Transfer {
            sources,
            destination,
            moving,
            options,
        } if !moving
            && sources.len() == 1
            && options.conflicts == crate::transfer::ConflictPolicy::Refuse =>
        {
            Some((&sources[0], destination, *moving))
        }
        Operation::ResolvedTransfer {
            sources,
            destination,
            moving,
            options,
            overrides,
        } if !moving
            && sources.len() == 1
            && options.conflicts == crate::transfer::ConflictPolicy::Refuse
            && overrides.is_empty() =>
        {
            Some((&sources[0], destination, *moving))
        }
        _ => None,
    }
}
/// Completes a single-file transfer from its saved `partial` path and source `stamp`.
///
/// `destination` is the full output filename. The worker compares existing
/// partial bytes with the start of `source`, appends the rest, and publishes
/// the copy at an unused destination. For a move it rechecks the source
/// fingerprint before deleting the original.
///
/// # Errors
///
/// Rejects an invalid checkpoint, a changed source, mismatching partial data,
/// or an occupied destination. An interruption during staging can leave a
/// checkpoint for another attempt. The output can already exist if cleanup fails.
pub fn resume_local(
    source: &Path,
    destination: &Path,
    moving: bool,
    partial: &Path,
    stamp: &str,
    task: &mut Task,
) -> Result<String> {
    let root = Queue::path()?.with_file_name("transfers");
    resume_local_in(&root, source, destination, moving, partial, stamp, task)
}
fn resume_local_in(
    root: &Path,
    source: &Path,
    destination: &Path,
    moving: bool,
    partial: &Path,
    stamp: &str,
    task: &mut Task,
) -> Result<String> {
    if partial.parent() != Some(root)
        || !partial
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("transfer-"))
    {
        bail!("Invalid transfer checkpoint");
    }
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        bail!("Transfer storage is a symlink");
    }
    let witness = partial.with_extension("witness");
    let actual = crate::history::fingerprint(source, task)?.context("Source missing")?;
    let expected = if stamp.starts_with("metadata:") {
        if format!(
            "metadata:{:x?}",
            crate::operations::source_stamp(source, task)?
        ) != stamp
        {
            bail!("Source changed since this transfer was queued. Queue a new copy.");
        }
        if crate::operations::exists(&witness) {
            if fs::symlink_metadata(&witness)?.file_type().is_symlink() {
                bail!("Checkpoint witness is a symlink");
            }
            fs::read_to_string(&witness)?
        } else {
            if crate::operations::exists(partial) {
                bail!(
                    "Checkpoint has no source witness; discard this recipe and queue a new copy."
                );
            }
            let mut f = tempfile::NamedTempFile::new_in(root)?;
            f.write_all(actual.as_bytes())?;
            f.as_file().sync_all()?;
            f.persist_noclobber(&witness)?;
            actual.clone()
        }
    } else {
        stamp.to_owned()
    };
    if actual != expected {
        bail!("Source changed since this transfer was queued. Queue a new copy.");
    }
    if crate::operations::exists(destination) {
        bail!("Destination now exists. Review it before retrying.");
    }
    if crate::operations::exists(partial) && fs::symlink_metadata(partial)?.file_type().is_symlink()
    {
        bail!("Checkpoint is a symlink");
    }
    task.capture(destination)?;
    if moving {
        task.capture(source)?;
    }
    let mut input = fs::File::open(source)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(partial)?;
    let offset = output.metadata()?.len();
    let source_size = input.metadata()?.len();
    task.plan_total(source_size.checked_mul(2));
    if offset > source_size {
        bail!("Checkpoint exceeds source size");
    }
    let mut a = [0; 65536];
    let mut b = [0; 65536];
    let mut remaining = offset;
    while remaining > 0 {
        task.check()?;
        let count = remaining.min(a.len() as u64) as usize;
        input.read_exact(&mut a[..count])?;
        output.read_exact(&mut b[..count])?;
        if a[..count] != b[..count] {
            bail!(
                "Checkpoint contents changed. Discard this queued transfer and queue a new copy."
            );
        }
        remaining -= count as u64;
    }
    input.seek(SeekFrom::Start(offset))?;
    output.seek(SeekFrom::Start(offset))?;
    task.progress.bytes = offset;
    task.advance_progress(offset);
    task.stream(&mut input, &mut output, source)?;
    output.sync_all()?;
    drop(output);
    if crate::history::fingerprint(source, task)?.as_deref() != Some(expected.as_str()) {
        task.undo_blocked = true;
        bail!("Source changed while copying; checkpoint retained for review");
    }
    let meta = fs::metadata(source)?;
    fs::set_permissions(partial, meta.permissions())?;
    filetime::set_file_mtime(
        partial,
        filetime::FileTime::from_last_modification_time(&meta),
    )?;
    crate::file_metadata::copy(source, partial)?;
    crate::operations::copy_item(partial, destination, task)?;
    if moving {
        if crate::history::fingerprint(source, task)?.as_deref() != Some(expected.as_str()) {
            task.undo_blocked = true;
            bail!("Source changed before removal; copied output and source retained");
        }
        fs::remove_file(source)?;
    }
    remove_checkpoint_file(partial)?;
    if crate::operations::exists(&witness) {
        remove_checkpoint_file(&witness)?;
    }
    Ok(format!(
        "{} {} (resumable transfer).",
        if moving { "Moved" } else { "Copied" },
        source.display()
    ))
}
fn remove_checkpoint_file(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let mut p = fs::metadata(path)?.permissions();
        #[allow(
            clippy::permissions_set_readonly_false,
            reason = "Windows-only: clear FILE_ATTRIBUTE_READONLY so the checkpoint can be deleted"
        )]
        p.set_readonly(false);
        fs::set_permissions(path, p)?;
    }
    fs::remove_file(path)?;
    Ok(())
}
/// Discards the local checkpoint data belonging to a queued transfer recipe.
///
/// For [`Operation::ResumeLocal`] this removes the partial file and its source
/// witness. For [`Operation::ResumeRemote`] it delegates to [`crate::resumable::discard`].
/// Calling it for another operation succeeds without changing any files.
///
/// # Errors
///
/// Rejects invalid checkpoint paths and batches that still hold replacement
/// originals. A filesystem error can interrupt cleanup.
pub fn discard_checkpoint(op: &Operation) -> Result<()> {
    if let Operation::ResumeRemote { id, from, to, .. } = op {
        return crate::resumable::discard(id, from, to);
    }
    if let Operation::ResumeLocal { partial, .. } = op {
        let root = Queue::path()?.with_file_name("transfers");
        if partial.parent() != Some(root.as_path())
            || !partial
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("transfer-"))
        {
            bail!("Invalid checkpoint path");
        }
        for p in [partial.clone(), partial.with_extension("witness")] {
            if crate::operations::exists(&p) {
                if fs::symlink_metadata(&p)?.file_type().is_symlink() {
                    bail!("Checkpoint is a symlink");
                }
                remove_checkpoint_file(&p)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_snapshots_can_be_loaded_and_replaced() {
        use std::sync::{Arc, Barrier};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("queue.json");
        let initial = Queue {
            waiting: VecDeque::from([Operation::Mkdir("initial".into())]),
            ..Default::default()
        };
        initial.save_in(&path).unwrap();
        let barrier = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let barrier = barrier.clone();
                let path = &path;
                scope.spawn(move || {
                    barrier.wait();
                    for snapshot in 0..100 {
                        if worker < 4 {
                            let queue = Queue {
                                waiting: VecDeque::from([Operation::Mkdir(
                                    format!("worker-{worker}-{snapshot}").into(),
                                )]),
                                ..Default::default()
                            };
                            queue.save_in(path).expect("Concurrent queue save");
                        } else {
                            let queue = Queue::load_in(path).expect("Concurrent queue load");
                            assert_eq!(queue.waiting.len(), 1);
                            assert!(matches!(queue.waiting[0], Operation::Mkdir(_)));
                        }
                    }
                });
            }
        });
        assert_eq!(Queue::load_in(&path).unwrap().waiting.len(), 1);
    }
    #[test]
    fn interrupted_jobs_restore_paused_without_passwords() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("q.json");
        let q = Queue {
            active: Some(Operation::PackProtected {
                sources: vec![],
                destination: "a.zip".into(),
                password: "private-password".into(),
                compression_level: Some(9),
            }),
            ..Default::default()
        };
        q.save_in(&path).unwrap();
        assert!(
            !fs::read_to_string(&path)
                .unwrap()
                .contains("private-password")
        );
        let q = Queue::load_in(&path).unwrap();
        assert!(q.paused);
        assert!(q.active.is_none());
        assert!(Queue::requires_input(&q[0]));
    }
    #[test]
    fn corrupt_queue_is_reported_without_changing_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("queue");
        fs::write(&p, "broken").unwrap();
        assert!(Queue::load_in(&p).is_err());
        assert_eq!(fs::read_to_string(p).unwrap(), "broken");
    }
}

#[cfg(test)]
mod resume_tests {
    use super::*;
    #[test]
    fn verified_partial_file_resumes_and_corrupt_checkpoint_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("transfers");
        fs::create_dir(&root).unwrap();
        let source = d.path().join("source");
        let dest = d.path().join("destination");
        let partial = root.join("transfer-test.part");
        let bytes = (0..150_000).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        fs::write(&source, &bytes).unwrap();
        let stamp = crate::history::fingerprint(&source, &Task::default())
            .unwrap()
            .unwrap();
        fs::write(&partial, &bytes[..70001]).unwrap();
        resume_local_in(
            &root,
            &source,
            &dest,
            false,
            &partial,
            &stamp,
            &mut Task::default(),
        )
        .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), bytes);
        assert!(!partial.exists());
        fs::remove_file(&dest).unwrap();
        fs::write(&partial, b"corrupted").unwrap();
        assert!(
            resume_local_in(
                &root,
                &source,
                &dest,
                false,
                &partial,
                &stamp,
                &mut Task::default()
            )
            .is_err()
        );
        assert!(!dest.exists());
        assert!(source.exists());
    }
    #[test]
    fn source_witness_detects_edits_even_if_size_and_timestamp_are_restored() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("transfers");
        fs::create_dir(&root).unwrap();
        let source = d.path().join("source");
        let dest = d.path().join("destination");
        let partial = root.join("transfer-witness.part");
        fs::write(&source, b"prefix ORIGINAL").unwrap();
        let modified =
            filetime::FileTime::from_last_modification_time(&fs::metadata(&source).unwrap());
        let stamp = format!(
            "metadata:{:x?}",
            crate::operations::source_stamp(&source, &Task::default()).unwrap()
        );
        fs::write(
            partial.with_extension("witness"),
            crate::history::fingerprint(&source, &Task::default())
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        fs::write(&partial, b"prefix ").unwrap();
        fs::write(&source, b"prefix EXTERNAL").unwrap();
        filetime::set_file_mtime(&source, modified).unwrap();
        assert!(
            resume_local_in(
                &root,
                &source,
                &dest,
                false,
                &partial,
                &stamp,
                &mut Task::default()
            )
            .is_err()
        );
        assert!(!dest.exists());
        assert_eq!(fs::read(&partial).unwrap(), b"prefix ");
    }
    #[test]
    fn discarding_a_recipe_cannot_remove_an_external_checkpoint_path() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("transfer-external.part");
        fs::write(&file, "keep").unwrap();
        let op = Operation::ResumeLocal {
            source: "source".into(),
            destination: "destination".into(),
            moving: false,
            partial: file.clone(),
            stamp: String::new(),
        };
        assert!(discard_checkpoint(&op).is_err());
        assert_eq!(fs::read(file).unwrap(), b"keep");
    }

    #[test]
    fn normal_copy_confirmation_selects_the_resumable_file_engine() {
        let op = Operation::ResolvedTransfer {
            sources: vec!["source".into()],
            destination: "target".into(),
            moving: false,
            options: Default::default(),
            overrides: Default::default(),
        };
        assert!(resumable(&op).is_some());
        let op = Operation::ResolvedTransfer {
            sources: vec!["source".into()],
            destination: "target".into(),
            moving: false,
            options: crate::transfer::TransferOptions {
                conflicts: crate::transfer::ConflictPolicy::Replace,
                ..Default::default()
            },
            overrides: Default::default(),
        };
        assert!(resumable(&op).is_none());
    }

    #[test]
    fn resumed_move_preserves_source_when_destination_appears() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("transfers");
        fs::create_dir(&root).unwrap();
        let source = d.path().join("source");
        let dest = d.path().join("destination");
        let partial = root.join("transfer-test.part");
        fs::write(&source, "source").unwrap();
        fs::write(&dest, "external").unwrap();
        let stamp = crate::history::fingerprint(&source, &Task::default())
            .unwrap()
            .unwrap();
        assert!(
            resume_local_in(
                &root,
                &source,
                &dest,
                true,
                &partial,
                &stamp,
                &mut Task::default()
            )
            .is_err()
        );
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(fs::read(&dest).unwrap(), b"external");
    }
}
