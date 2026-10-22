//! Deferred native exports. Constructing or cancelling an offer performs no I/O.
use super::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

type Completion = Box<dyn Fn(Result<Vec<PathBuf>, String>) + Send + Sync>;
type Updates = Arc<dyn Fn(operations::Progress) + Send + Sync>;

pub(crate) struct PromiseSession {
    pub selection: Selection,
    pub cancel: Arc<AtomicBool>,
    state: Mutex<State>,
    export: Mutex<Option<Result<Vec<PathBuf>, String>>>,
    update: Updates,
    complete: Completion,
    deliveries: bool,
    cache: Option<PathBuf>,
}
#[derive(Default)]
struct State {
    accepted: bool,
    started: bool,
    served: bool,
    running: usize,
    finished: bool,
    delivered: BTreeSet<usize>,
}
struct Activity<'a>(&'a PromiseSession);
impl Drop for Activity<'_> {
    fn drop(&mut self) {
        let idle = {
            let mut state = self.0.state.lock().unwrap();
            state.running -= 1;
            state.running == 0
        };
        if idle && self.0.cancel.load(Ordering::Relaxed) {
            self.0.finish(Err("External drop cancelled.".into()));
        }
    }
}
impl PromiseSession {
    pub fn new(
        selection: Selection,
        deliveries: bool,
        update: Updates,
        complete: Completion,
    ) -> Arc<Self> {
        Arc::new(Self {
            selection,
            cancel: Arc::new(AtomicBool::new(false)),
            state: Mutex::new(State::default()),
            export: Mutex::new(None),
            update,
            complete,
            deliveries,
            cache: None,
        })
    }
    pub fn idle(&self) -> bool {
        self.state.lock().unwrap().running == 0
    }
    pub fn routable(&self) -> bool {
        !self.cancel.load(Ordering::Acquire) && !self.state.lock().unwrap().finished
    }
    pub fn waiting(&self) -> bool {
        !self.state.lock().unwrap().started
    }
    /// Only a native accepted-drop/content callback may call this method.
    pub fn materialize(&self) -> Result<Vec<PathBuf>, String> {
        self.state.lock().unwrap().running += 1;
        let _activity = Activity(self);
        let mut exported = self.export.lock().unwrap();
        if self.cancel.load(Ordering::Relaxed) {
            return Err("External drop cancelled.".into());
        }
        self.state.lock().unwrap().started = true;
        if exported.is_none() {
            let mut task = Task::new(self.cancel.clone(), {
                let update = self.update.clone();
                move |p| update(p)
            });
            let result = self
                .cache
                .clone()
                .map_or_else(export_cache, Ok)
                .and_then(|cache| super::export_in(&self.selection, &mut task, Some(&cache)))
                .and_then(|export| {
                    task.check()?;
                    retain_export(export)
                })
                .map_err(|e| format!("{e:#}"));
            *exported = Some(result);
        }
        let result = exported.as_ref().unwrap().clone();
        self.state.lock().unwrap().served = true;
        if let Err(error) = &result {
            self.finish(Err(error.clone()));
        }
        result
    }
    #[cfg(any(test, target_os = "macos", target_os = "windows"))]
    pub fn local_path(&self, relative: &Path) -> Result<PathBuf, String> {
        filesystem::portable_relative(relative).map_err(|e| e.to_string())?;
        let paths = self.materialize()?;
        let root = paths
            .first()
            .and_then(|p| p.parent())
            .ok_or("Empty native export")?;
        let path = root.join(relative);
        let physical = path.canonicalize().map_err(|e| e.to_string())?;
        if !physical.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
            return Err("A promised link escapes the exported selection.".into());
        }
        if !operations::exists(&path) {
            return Err("The promised entry no longer exists.".into());
        }
        Ok(path)
    }
    /// AppKit calls this with the final, collision-resolved file URL, on its operation queue.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn write(&self, index: usize, destination: &Path) -> Result<(), String> {
        self.state.lock().unwrap().running += 1;
        let _activity = Activity(self);
        let result = (|| -> Result<()> {
            ensure!(
                destination.is_absolute(),
                "The drop destination must be absolute."
            );
            let entry = self
                .selection
                .entries
                .get(index)
                .context("Invalid promised entry")?;
            let source = self
                .local_path(Path::new(&entry.name))
                .map_err(anyhow::Error::msg)?;
            let parent = destination.parent().context("Missing destination folder")?;
            // Keep incomplete content private and publish without replacing a racing file.
            let stage = tempfile::Builder::new()
                .prefix(".rust-commander-drop-")
                .tempdir_in(parent)?;
            let temporary = stage.path().join("payload");
            let mut task = Task::new(self.cancel.clone(), {
                let update = self.update.clone();
                move |p| update(p)
            });
            operations::copy_item(&source, &temporary, &mut task)?;
            task.check()?;
            publish_noreplace(&temporary, destination)?;
            Ok(())
        })()
        .map_err(|e| format!("{e:#}"));
        match &result {
            Ok(()) => {
                self.state.lock().unwrap().delivered.insert(index);
                self.maybe_finish();
            }
            Err(error) => self.finish(Err(error.clone())),
        }
        result
    }
    /// An in-process drop transfers the typed selection; native export must never run.
    pub fn internal_drop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.finish(Ok(Vec::new()));
    }
    pub fn end_drag(&self, accepted: bool) {
        if !accepted {
            self.cancel.store(true, Ordering::Relaxed);
            self.finish(Err("External drag cancelled.".into()));
        } else {
            self.state.lock().unwrap().accepted = true;
            self.maybe_finish();
        }
    }
    pub fn abort(&self, message: &str) {
        self.cancel.store(true, Ordering::Relaxed);
        // A running task observes cancellation and owns its staging lifetime.
        if self.state.lock().unwrap().running == 0 {
            self.finish(Err(message.into()));
        }
    }
    fn maybe_finish(&self) {
        let done = {
            let state = self.state.lock().unwrap();
            state.accepted
                && if self.deliveries {
                    state.delivered.len() == self.selection.entries.len()
                } else {
                    state.served
                }
        };
        if done {
            let paths = self
                .export
                .lock()
                .unwrap()
                .as_ref()
                .cloned()
                .unwrap_or_else(|| Err("The receiving application did not request files.".into()));
            self.finish(paths);
        }
    }
    fn finish(&self, result: Result<Vec<PathBuf>, String>) {
        let mut state = self.state.lock().unwrap();
        if state.finished {
            return;
        }
        state.finished = true;
        drop(state);
        if result.is_err() {
            self.cancel.store(true, Ordering::Relaxed);
            if self.selection.from.local().is_none()
                && let Ok(exported) = self.export.try_lock()
                && let Some(Ok(paths)) = exported.as_ref()
                && let Some(directory) = paths
                    .first()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.parent())
            {
                // The cache is owned by this session; published destinations are
                // separate copies and are never removed during cleanup.
                let _ = fs::remove_dir_all(directory);
            }
        }
        (self.complete)(result);
    }
}

pub(crate) fn export_cache() -> Result<PathBuf> {
    let cache = crate::settings::path()?
        .parent()
        .context("Missing configuration folder")?
        .join("drag-exports");
    fs::create_dir_all(&cache)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o700))?;
    }
    Ok(cache.canonicalize()?)
}
pub(crate) fn retain_export(export: Export) -> Result<Vec<PathBuf>> {
    let cache = export
        .directory
        .path()
        .parent()
        .context("Missing export cache")?
        .to_owned();
    fs::write(
        export.directory.path().join("created.json"),
        serde_json::to_vec(&std::time::SystemTime::now())?,
    )?;
    let paths = export.paths;
    let _ = export.directory.keep();
    cleanup_expired(&cache);
    Ok(paths)
}

fn cleanup_expired(cache: &Path) {
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("export-")
            || !entry.file_type().is_ok_and(|t| t.is_dir())
        {
            continue;
        }
        let directory = entry.path();
        let lease = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.join("lease.lock"))
            .ok();
        if lease.as_ref().is_some_and(|file| file.try_lock().is_err()) {
            continue;
        }
        // Read via the locked handle. Windows byte-range locks reject reading
        // the same bytes through a second handle, even in this process.
        let owned_lease = lease.as_ref().is_some_and(|file| {
            use std::io::Read;
            let mut bytes = Vec::new();
            file.take(64).read_to_end(&mut bytes).is_ok() && bytes == b"RustCommanderExportLease1"
        });
        let created = fs::read(directory.join("created.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<std::time::SystemTime>(&b).ok())
            .or_else(|| {
                owned_lease
                    .then(|| entry.metadata().ok()?.modified().ok())
                    .flatten()
            });
        if created
            .and_then(|v| v.elapsed().ok())
            .is_some_and(|v| v > std::time::Duration::from_secs(86400))
        {
            // Close the Windows handle before removal. The private timestamped
            // export is no longer owned by a running worker at this point.
            drop(lease);
            let _ = fs::remove_dir_all(directory);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn publish_noreplace(source: &Path, target: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let a = std::ffi::CString::new(source.as_os_str().as_bytes())?;
        let b = std::ffi::CString::new(target.as_os_str().as_bytes())?;
        let result = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_EXCL) };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        // This writer is currently used only by AppKit. File-stream/URI receivers
        // own their destination publishing and conflict handling on other systems.
        let _ = (source, target);
        anyhow::bail!("Direct promise publishing is available on macOS only.");
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Windows virtual file descriptors enumerate names/metadata without reading contents.
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
pub(crate) fn manifest(
    selection: &Selection,
    cancel: &AtomicBool,
) -> Result<Vec<(PathBuf, bool, Option<u64>)>> {
    selection.validate()?;
    let mut result = Vec::new();
    let mut pending = selection
        .entries
        .iter()
        .rev()
        .map(|e| (e.clone(), PathBuf::from(&e.name), 0usize))
        .collect::<Vec<_>>();
    while let Some((entry, relative, depth)) = pending.pop() {
        ensure!(!cancel.load(Ordering::Relaxed), "External drag cancelled.");
        ensure!(
            result.len() < 100_000 && depth < 128,
            "The virtual selection is too large or deeply nested."
        );
        ensure!(
            !entry.is_symlink,
            "External virtual drags cannot export symbolic links."
        );
        filesystem::portable_relative(&relative)?;
        result.push((
            relative.clone(),
            entry.is_dir,
            // HTTP index sizes may be rounded; other remote metadata can change
            // before the drop. Let the receiver query the actual stream length.
            (selection.from.local().is_some()
                || matches!(selection.from, Location::Archive { .. }))
            .then_some(entry.size)
            .filter(|_| entry.size_known),
        ));
        if entry.is_dir {
            let location = crate::directory_tree::at(&selection.from, &entry.path);
            let children = filesystem::list(&location)?;
            for child in children.into_iter().rev() {
                filesystem::safe_relative(Path::new(&child.name))?;
                ensure!(
                    Path::new(&child.name).components().count() == 1,
                    "Unsafe virtual entry name."
                );
                pending.push((child.clone(), relative.join(&child.name), depth + 1));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    type Finished = mpsc::Receiver<Result<Vec<PathBuf>, String>>;
    fn setup(deliveries: bool) -> (tempfile::TempDir, Arc<PromiseSession>, Finished) {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input");
        fs::create_dir(&input).unwrap();
        fs::write(input.join("café.txt"), b"promised content").unwrap();
        fs::create_dir(input.join("folder")).unwrap();
        fs::write(input.join("folder/nested.txt"), b"nested content").unwrap();
        let zip = root.path().join("source.zip");
        crate::archive::pack(
            &[input.join("café.txt"), input.join("folder")],
            &zip,
            &mut Task::default(),
        )
        .unwrap();
        let from = Location::Archive {
            file: zip,
            prefix: PathBuf::new(),
        };
        let selection = Selection {
            entries: filesystem::list(&from).unwrap(),
            from,
        };
        let (sender, receiver) = mpsc::channel();
        let mut session = PromiseSession::new(
            selection,
            deliveries,
            Arc::new(|_| {}),
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        );
        Arc::get_mut(&mut session).unwrap().cache = Some(root.path().join("exports"));
        (root, session, receiver)
    }
    #[test]
    fn abandoned_and_cancelled_promises_never_materialize_content() {
        let (root, session, result) = setup(false);
        let entries = manifest(&session.selection, &session.cancel).unwrap();
        assert!(
            entries
                .iter()
                .any(|(p, _, _)| p == Path::new("folder/nested.txt"))
        );
        assert!(session.waiting());
        assert!(!root.path().join("exports").exists());
        session.end_drag(false);
        assert!(result.recv().unwrap().is_err());
        assert!(session.materialize().is_err());
        assert!(!root.path().join("exports").exists());
    }
    #[test]
    fn concurrent_accepted_requests_share_one_export_and_finish_once() {
        let (root, session, result) = setup(false);
        fs::create_dir(root.path().join("exports")).unwrap();
        let threads = (0..4)
            .map(|_| {
                let s = session.clone();
                std::thread::spawn(move || s.materialize().unwrap())
            })
            .collect::<Vec<_>>();
        let exports = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
        assert!(exports.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(
            fs::read_dir(root.path().join("exports")).unwrap().count(),
            1
        );
        assert!(result.try_recv().is_err());
        session.end_drag(true);
        assert!(result.recv().unwrap().is_ok());
        session.end_drag(true);
        assert!(result.try_recv().is_err());
        assert_eq!(
            fs::read(session.local_path(Path::new("folder/nested.txt")).unwrap()).unwrap(),
            b"nested content"
        );
        assert!(session.local_path(Path::new("../outside")).is_err());
    }
    #[test]
    fn export_failure_is_terminal_and_leaves_no_partial_export() {
        let (root, session, result) = setup(false);
        fs::create_dir(root.path().join("exports")).unwrap();
        if let Location::Archive { file, .. } = &session.selection.from {
            fs::write(file, b"corrupt archive").unwrap();
        }
        assert!(session.materialize().is_err());
        assert!(result.recv().unwrap().is_err());
        assert_eq!(
            fs::read_dir(root.path().join("exports")).unwrap().count(),
            0
        );
        session.end_drag(true);
        assert!(result.try_recv().is_err());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_delivery_is_atomic_preserves_collisions_and_copies_folders() {
        let (root, session, result) = setup(true);
        fs::create_dir(root.path().join("exports")).unwrap();
        let destination = root.path().join("destination");
        fs::create_dir(&destination).unwrap();
        session.end_drag(true);
        assert!(result.try_recv().is_err());
        for (index, entry) in session.selection.entries.iter().enumerate() {
            session
                .write(index, &destination.join(&entry.name))
                .unwrap();
        }
        assert!(result.recv().unwrap().is_ok());
        assert_eq!(
            fs::read(destination.join("café.txt")).unwrap(),
            b"promised content"
        );
        assert_eq!(
            fs::read(destination.join("folder/nested.txt")).unwrap(),
            b"nested content"
        );
        fs::write(destination.join("café.txt"), b"keep existing").unwrap();
        let index = session
            .selection
            .entries
            .iter()
            .position(|e| e.name == "café.txt")
            .unwrap();
        assert!(session.write(index, &destination.join("café.txt")).is_err());
        assert_eq!(
            fs::read(destination.join("café.txt")).unwrap(),
            b"keep existing"
        );
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 2);
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[test]
    fn cancelling_after_materialization_finishes_and_removes_private_exports() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("file.txt");
        fs::write(&input, b"keep source").unwrap();
        let file = root.path().join("source.zip");
        crate::archive::pack(std::slice::from_ref(&input), &file, &mut Task::default()).unwrap();
        let from = Location::Archive {
            file,
            prefix: PathBuf::new(),
        };
        let selection = Selection {
            entries: filesystem::list(&from).unwrap(),
            from,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut session = PromiseSession::new(
            selection,
            true,
            Arc::new(|_| {}),
            Box::new(move |v| {
                let _ = tx.send(v);
            }),
        );
        let cache = root.path().join("exports");
        fs::create_dir(&cache).unwrap();
        Arc::get_mut(&mut session).unwrap().cache = Some(cache.clone());
        session.materialize().unwrap();
        session.end_drag(true);
        assert!(rx.try_recv().is_err());
        session.abort("External drop cancelled.");
        assert!(rx.recv().unwrap().is_err());
        assert_eq!(fs::read_dir(cache).unwrap().count(), 0);
        assert_eq!(fs::read(input).unwrap(), b"keep source");
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn interrupted_exports_are_cleaned_but_locked_or_unowned_directories_survive() {
        let root = tempfile::tempdir().unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(90000);
        let timestamp = filetime::FileTime::from_system_time(old);
        let abandoned = root.path().join("export-abandoned");
        fs::create_dir(&abandoned).unwrap();
        fs::write(abandoned.join("lease.lock"), b"RustCommanderExportLease1").unwrap();
        filetime::set_file_mtime(&abandoned, timestamp).unwrap();
        let active = root.path().join("export-active");
        fs::create_dir(&active).unwrap();
        let mut lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(active.join("lease.lock"))
            .unwrap();
        lock.write_all(b"RustCommanderExportLease1").unwrap();
        lock.try_lock().unwrap();
        filetime::set_file_mtime(&active, timestamp).unwrap();
        let other = root.path().join("export-user-folder");
        fs::create_dir(&other).unwrap();
        filetime::set_file_mtime(&other, timestamp).unwrap();
        cleanup_expired(root.path());
        assert!(!abandoned.exists());
        assert!(active.exists());
        assert!(other.exists());
        drop(lock);
        cleanup_expired(root.path());
        assert!(!active.exists());
        assert!(other.exists());
    }
}
