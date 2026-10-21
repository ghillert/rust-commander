use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Instant, SystemTime},
};

mod rate;
pub(crate) mod totals;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Operation {
    EmptyTrash,
    ResumeRemote {
        id: String,
        from: crate::filesystem::Location,
        entries: Vec<crate::filesystem::Entry>,
        to: crate::filesystem::Location,
    },
    ArchiveSaveBytes {
        archive: PathBuf,
        path: PathBuf,
        expected: Option<String>,
        #[serde(skip)]
        bytes: Vec<u8>,
    },
    ExtractResolved {
        archive: PathBuf,
        selected: Vec<PathBuf>,
        prefix: PathBuf,
        destination: PathBuf,
        options: crate::transfer::TransferOptions,
        overrides: std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    },
    SaveBytes {
        path: PathBuf,
        #[serde(skip)]
        original: Vec<u8>,
        #[serde(skip)]
        bytes: Vec<u8>,
    },
    RemoteSaveBytes {
        connection: String,
        path: PathBuf,
        #[serde(skip)]
        original: Vec<u8>,
        #[serde(skip)]
        bytes: Vec<u8>,
    },
    RemoteTransferResolved {
        from: crate::filesystem::Location,
        entries: Vec<crate::filesystem::Entry>,
        to: crate::filesystem::Location,
        moving: bool,
        options: crate::transfer::TransferOptions,
        overrides: std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    },

    Undo(Box<crate::history::Record>),
    Redo(Box<crate::history::Record>),
    SaveText {
        path: PathBuf,
        #[serde(skip)]
        original: Vec<u8>,
        #[serde(skip)]
        text: String,
    },
    Sync(crate::sync::Plan),
    RemoteSave {
        connection: String,
        path: PathBuf,
        #[serde(skip)]
        original: Vec<u8>,
        #[serde(skip)]
        text: String,
    },
    ResumeLocal {
        source: PathBuf,
        destination: PathBuf,
        moving: bool,
        partial: PathBuf,
        stamp: String,
    },
    ResolvedTransfer {
        sources: Vec<PathBuf>,
        destination: PathBuf,
        moving: bool,
        options: crate::transfer::TransferOptions,
        overrides: std::collections::BTreeMap<PathBuf, crate::transfer::ConflictPolicy>,
    },
    RemoteRename {
        connection: String,
        source: PathBuf,
        name: String,
    },
    ArchiveEdit {
        archive: PathBuf,
        edit: crate::archive_tools::ZipEdit,
    },
    PackProtected {
        sources: Vec<PathBuf>,
        destination: PathBuf,
        #[serde(skip)]
        password: String,
    },
    ExtractProtected {
        archive: PathBuf,
        selected: Vec<PathBuf>,
        prefix: PathBuf,
        destination: PathBuf,
        #[serde(skip)]
        password: String,
    },
    Transfer {
        sources: Vec<PathBuf>,
        destination: PathBuf,
        moving: bool,
        options: crate::transfer::TransferOptions,
    },
    BatchRename(Vec<(PathBuf, PathBuf)>),
    Properties(Vec<PathBuf>),
    Compare {
        left: PathBuf,
        right: PathBuf,
    },
    Split {
        source: PathBuf,
        destination: PathBuf,
        bytes: u64,
    },
    Combine {
        sources: Vec<PathBuf>,
        destination: PathBuf,
    },
    Mkfile(PathBuf),
    Permissions {
        paths: Vec<PathBuf>,
        mode: u32,
    },
    Modified {
        paths: Vec<PathBuf>,
        time: SystemTime,
    },
    PermanentDelete(Vec<PathBuf>),
    RemoteTransfer {
        from: crate::filesystem::Location,
        entries: Vec<crate::filesystem::Entry>,
        to: crate::filesystem::Location,
        moving: bool,
    },
    RemoteMkdir {
        connection: String,
        path: PathBuf,
    },
    RemoteDelete {
        connection: String,
        entries: Vec<crate::filesystem::Entry>,
    },
    Copy {
        sources: Vec<PathBuf>,
        destination: PathBuf,
    },
    Move {
        sources: Vec<PathBuf>,
        destination: PathBuf,
    },
    Rename {
        source: PathBuf,
        name: String,
    },
    Trash(Vec<PathBuf>),
    Mkdir(PathBuf),
    Pack {
        sources: Vec<PathBuf>,
        destination: PathBuf,
    },
    Extract {
        archive: PathBuf,
        selected: Vec<PathBuf>,
        prefix: PathBuf,
        destination: PathBuf,
    },
    Checksum(PathBuf),
}

impl Operation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::EmptyTrash => "Empty Trash",
            Self::Undo(_) => "Undo",
            Self::Redo(_) => "Redo",
            Self::SaveText { .. }
            | Self::RemoteSave { .. }
            | Self::SaveBytes { .. }
            | Self::RemoteSaveBytes { .. } => "Save text",
            Self::ResumeRemote { .. } => "Resumable remote copy",
            Self::ResumeLocal { .. } => "Resumable transfer",
            Self::Sync(_) => "Synchronize folders",
            Self::Transfer { moving: true, .. }
            | Self::ResolvedTransfer { moving: true, .. }
            | Self::Move { .. } => "Move files",
            Self::Transfer { .. } | Self::ResolvedTransfer { .. } | Self::Copy { .. } => {
                "Copy files"
            }
            Self::Rename { .. } | Self::RemoteRename { .. } => "Rename",
            Self::BatchRename(_) => "Batch rename",
            Self::Mkdir(_) | Self::RemoteMkdir { .. } => "New folder",
            Self::Mkfile(_) => "New file",
            Self::Trash(_) => "Move to Trash",
            Self::PermanentDelete(_) | Self::RemoteDelete { .. } => "Delete",
            Self::Pack { .. } | Self::PackProtected { .. } => "Create archive",
            Self::Extract { .. } | Self::ExtractProtected { .. } | Self::ExtractResolved { .. } => {
                "Extract archive"
            }
            Self::ArchiveEdit { .. } | Self::ArchiveSaveBytes { .. } => "Edit archive",
            Self::RemoteTransfer { .. } | Self::RemoteTransferResolved { .. } => "Remote transfer",
            Self::Properties(_) => "Properties",
            Self::Compare { .. } => "Compare folders",
            Self::Checksum(_) => "Checksums",
            Self::Permissions { .. } => "Permissions",
            Self::Modified { .. } => "Modification time",
            Self::Split { .. } => "Split file",
            Self::Combine { .. } => "Combine files",
        }
    }
    /// Recipe details exclude passwords and editor buffers.
    pub fn description(&self) -> String {
        match self {
            Self::ResumeLocal {
                source,
                destination,
                ..
            } => format!("{} → {}", source.display(), destination.display()),
            Self::Copy {
                sources,
                destination,
            }
            | Self::Move {
                sources,
                destination,
            }
            | Self::Transfer {
                sources,
                destination,
                ..
            }
            | Self::ResolvedTransfer {
                sources,
                destination,
                ..
            } => format!(
                "{}\n→ {}",
                sources
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
                destination.display()
            ),
            Self::RemoteTransfer {
                from, entries, to, ..
            }
            | Self::RemoteTransferResolved {
                from, entries, to, ..
            } => format!(
                "{}\n{}\n→ {}",
                from.label(),
                entries
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                to.label()
            ),
            Self::Sync(plan) => format!("{} ⇄ {}", plan.left.display(), plan.right.display()),
            Self::ArchiveEdit { archive, .. }
            | Self::Extract { archive, .. }
            | Self::ExtractProtected { archive, .. }
            | Self::ExtractResolved { archive, .. } => archive.display().to_string(),
            Self::SaveText { path, .. }
            | Self::RemoteSave { path, .. }
            | Self::SaveBytes { path, .. }
            | Self::RemoteSaveBytes { path, .. } => path.display().to_string(),
            _ => self.label().into(),
        }
    }
}

pub type ChangeHook = Box<dyn FnMut(&Path) -> Result<()> + Send>;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Progress {
    pub current: String,
    pub bytes: u64,
    pub files: usize,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub completed_bytes: u64,
    #[serde(skip)]
    rate: rate::TransferRate,
}

impl Progress {
    pub fn fraction(&self) -> Option<f32> {
        self.total_bytes.map(|total| {
            if total == 0 {
                0.0
            } else {
                (self.completed_bytes as f64 / total as f64).clamp(0.0, 1.0) as f32
            }
        })
    }

    /// Recent bytes processed per second, excluding restored checkpoint bytes.
    /// Idle time is included even when the worker has not sent another report.
    pub fn bytes_per_second(&self) -> Option<u64> {
        self.rate.per_second(Instant::now())
    }
}

pub struct Task {
    pub undo_blocked: bool,
    pub before_change: Option<ChangeHook>,
    pub cancel: Arc<AtomicBool>,
    pub progress: Progress,
    report: Box<dyn FnMut(Progress) + Send>,
    total_planned: bool,
    last_report: Instant,
    report_interval: std::time::Duration,
}

impl Default for Task {
    fn default() -> Self {
        Self::new(Arc::new(AtomicBool::new(false)), |_| {})
    }
}

impl Task {
    pub fn new(cancel: Arc<AtomicBool>, report: impl FnMut(Progress) + Send + 'static) -> Self {
        Self {
            undo_blocked: false,
            before_change: None,
            cancel,
            progress: Progress::default(),
            report: Box::new(report),
            total_planned: false,
            last_report: Instant::now(),
            report_interval: std::time::Duration::from_millis(40),
        }
    }

    /// Set the progress notification interval. Zero reports every tick, which
    /// lets scripted callers coordinate deterministic interruption/recovery.
    pub fn report_every(mut self, interval: std::time::Duration) -> Self {
        self.report_interval = interval;
        self
    }

    pub(crate) fn needs_total(&self) -> bool {
        !self.total_planned
    }

    /// Only the outer operation owns the total; nested copies share its budget.
    pub(crate) fn plan_total(&mut self, total: Option<u64>) {
        if !self.total_planned {
            self.total_planned = true;
            self.progress.total_bytes = total;
            self.progress.completed_bytes = 0;
            // Publish with the next ordinary tick so existing cancellation and
            // race-detection callbacks still observe actual execution points.
        }
    }

    /// Credit work already present in a verified checkpoint or a native rename,
    /// without counting it as newly transferred bytes or inflating the speed.
    pub(crate) fn advance_progress(&mut self, bytes: u64) {
        self.progress.completed_bytes = self.progress.completed_bytes.saturating_add(bytes);
        if self
            .progress
            .total_bytes
            .is_some_and(|total| self.progress.completed_bytes > total)
        {
            self.progress.total_bytes = None;
        }
    }

    pub fn capture(&mut self, path: &Path) -> Result<()> {
        self.check()?;
        if let Some(mut hook) = self.before_change.take() {
            let result = hook(path);
            self.before_change = Some(hook);
            result?;
        }
        Ok(())
    }

    pub fn check(&self) -> Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            bail!("Operation cancelled. Completed items were kept.");
        }
        Ok(())
    }

    pub fn tick(&mut self, path: &Path, bytes: u64) -> Result<()> {
        self.check()?;
        self.progress.current = path.display().to_string();
        self.progress.bytes += bytes;
        self.advance_progress(bytes);
        self.progress.rate.record(bytes, Instant::now());
        if self.last_report.elapsed() >= self.report_interval {
            (self.report)(self.progress.clone());
            self.last_report = Instant::now();
        }
        Ok(())
    }

    pub fn stream(
        &mut self,
        input: &mut dyn Read,
        output: &mut dyn Write,
        path: &Path,
    ) -> Result<u64> {
        self.progress.rate.record(0, Instant::now());
        let mut buffer = [0u8; 64 * 1024];
        let mut total = 0;
        loop {
            self.check()?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            total += count as u64;
            self.tick(path, count as u64)?;
        }
        self.progress.files += 1;
        Ok(total)
    }
}

pub fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

pub(crate) fn remove(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(crate) fn validate_sources(sources: &[PathBuf]) -> Result<()> {
    if sources.is_empty() {
        bail!("Select at least one item.");
    }
    for (i, source) in sources.iter().enumerate() {
        fs::symlink_metadata(source)
            .with_context(|| format!("Cannot read {}", source.display()))?;
        for other in &sources[..i] {
            if source == other || source.starts_with(other) || other.starts_with(source) {
                bail!("The selection contains duplicate or nested paths.");
            }
        }
    }
    Ok(())
}

pub fn preflight(sources: &[PathBuf], destination: &Path) -> Result<Vec<PathBuf>> {
    validate_sources(sources)?;
    let destination =
        fs::canonicalize(destination).context("The destination must be an existing folder")?;
    if !destination.is_dir() {
        bail!("The destination is not a folder.");
    }
    let mut targets = Vec::new();
    for source in sources {
        let name = source
            .file_name()
            .context("Cannot copy a filesystem root")?;
        let target = destination.join(name);
        if exists(&target) || targets.contains(&target) {
            bail!(
                "Destination already exists: {}. Rename it or choose another folder.",
                target.display()
            );
        }
        let metadata = fs::symlink_metadata(source)?;
        if metadata.is_dir() && destination.starts_with(fs::canonicalize(source)?) {
            bail!("Cannot copy or move a folder into itself.");
        }
        targets.push(target);
    }
    Ok(targets)
}

/// Creates new paths exclusively. Symlinks are copied as links, never followed.
pub fn copy_item(source: &Path, target: &Path, task: &mut Task) -> Result<()> {
    task.tick(source, 0)?;
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(source)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(link, target)?;
        #[cfg(windows)]
        if source.is_dir() {
            std::os::windows::fs::symlink_dir(link, target)?;
        } else {
            std::os::windows::fs::symlink_file(link, target)?;
        }
        filetime::set_symlink_file_times(
            target,
            filetime::FileTime::from_last_access_time(&metadata),
            filetime::FileTime::from_last_modification_time(&metadata),
        )?;
        task.progress.files += 1;
    } else if metadata.is_dir() {
        fs::create_dir(target)?;
        let result: Result<()> = (|| {
            for item in fs::read_dir(source)? {
                let item = item?;
                copy_item(&item.path(), &target.join(item.file_name()), task)?;
            }
            filetime::set_file_mtime(
                target,
                filetime::FileTime::from_last_modification_time(&metadata),
            )?;
            fs::set_permissions(target, metadata.permissions())?;
            crate::file_metadata::copy(source, target)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(target);
        }
        result?;
    } else if metadata.is_file() {
        let mut input = File::open(source)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(target.parent().context("Missing parent folder")?)?;
        task.stream(&mut input, &mut temporary, source)?;
        let after = fs::symlink_metadata(source)?;
        if metadata.len() != after.len() || metadata.modified().ok() != after.modified().ok() {
            task.undo_blocked = true;
            bail!("Source changed while copying: {}", source.display());
        }
        if let Ok(modified) = metadata.modified() {
            temporary
                .as_file()
                .set_times(fs::FileTimes::new().set_modified(modified))?;
        }
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
        crate::file_metadata::copy(source, temporary.path())?;
        temporary.as_file().sync_all()?;
        temporary.persist_noclobber(target).map_err(|e| e.error)?;
    } else {
        bail!("Special files are not supported: {}", source.display());
    }
    Ok(())
}

pub fn execute(operation: Operation, task: &mut Task) -> Result<String> {
    task.check()?;
    match operation {
        Operation::ResumeRemote {
            id,
            from,
            entries,
            to,
        } => crate::resumable::run(&id, &from, &entries, &to, task),
        Operation::ArchiveSaveBytes {
            archive,
            path,
            expected,
            bytes,
        } => crate::external_edit::save_archive(&archive, &path, &expected, &bytes, task),
        Operation::ExtractResolved {
            archive,
            selected,
            prefix,
            destination,
            options,
            overrides,
        } => crate::archive::extract_resolved(
            &archive,
            &selected,
            &prefix,
            &destination,
            options,
            &overrides,
            task,
        ),
        Operation::EmptyTrash => {
            crate::native_actions::trash_action(true)?;
            Ok("Emptied system Trash.".into())
        }
        Operation::RemoteTransferResolved {
            from,
            entries,
            to,
            moving,
            options,
            overrides,
        } => crate::remote::transfer_resolved(
            &from, &entries, &to, moving, options, &overrides, task,
        ),
        Operation::SaveBytes {
            path,
            original,
            bytes,
        } => {
            task.capture(&path)?;
            save_bytes(&path, &original, &bytes)?;
            Ok("Saved text.".into())
        }
        Operation::RemoteSaveBytes {
            connection,
            path,
            original,
            bytes,
        } => crate::remote::save_bytes(&connection, &path, &original, &bytes, task),
        Operation::RemoteSave {
            connection,
            path,
            original,
            text,
        } => crate::remote::save_text(&connection, &path, &original, &text, task),
        Operation::ResumeLocal {
            source,
            destination,
            moving,
            partial,
            stamp,
        } => crate::queue::resume_local(&source, &destination, moving, &partial, &stamp, task),
        Operation::Sync(plan) => crate::sync::apply(&plan, task),
        Operation::SaveText {
            path,
            original,
            text,
        } => {
            task.capture(&path)?;
            save_text(&path, &original, &text)?;
            Ok("Saved text.".into())
        }
        Operation::Redo(record) => {
            crate::history::undo(&record, task)?;
            Ok("Redid operation.".into())
        }
        Operation::Undo(record) => {
            crate::history::undo(&record, task)?;
            Ok(format!("Undid {}", record.label))
        }
        Operation::ResolvedTransfer {
            sources,
            destination,
            moving,
            options,
            overrides,
        } => crate::transfer::transfer_resolved(
            &sources,
            &destination,
            moving,
            options,
            &overrides,
            task,
        ),
        Operation::RemoteRename {
            connection,
            source,
            name,
        } => {
            crate::remote::rename_with_task(&connection, &source, &name, task)?;
            Ok("Remote item renamed.".into())
        }
        Operation::ArchiveEdit { archive, edit } => {
            task.capture(&archive)?;
            crate::archive_tools::edit_archive(&archive, &edit, task)?;
            Ok("ZIP archive updated.".into())
        }
        Operation::PackProtected {
            sources,
            destination,
            password,
        } => {
            task.capture(&destination)?;
            crate::archive::pack_with_password(&sources, &destination, &password, task)?;
            Ok("Encrypted archive created.".into())
        }
        Operation::ExtractProtected {
            archive,
            selected,
            prefix,
            destination,
            password,
        } => {
            task.capture(&destination)?;
            crate::archive::extract_with_password(
                &archive,
                &selected,
                &prefix,
                &destination,
                &password,
                task,
            )?;
            Ok("Archive extracted.".into())
        }
        Operation::Transfer {
            sources,
            destination,
            moving,
            options,
        } => crate::transfer::transfer(&sources, &destination, moving, options, task),
        Operation::BatchRename(plan) => {
            for (source, target) in &plan {
                task.capture(source)?;
                task.capture(target)?;
            }
            crate::desktop::batch_rename(&plan, task)?;
            Ok(format!("Renamed {} item(s).", plan.len()))
        }
        Operation::Properties(paths) => Ok(format!(
            "Properties\n{}",
            crate::desktop::properties(&paths, task)?
        )),
        Operation::Compare { left, right } => {
            let entries = crate::desktop::compare(&left, &right, task)?;
            Ok(format!(
                "Folder comparison\n{}\n{}\n\n{}",
                left.display(),
                right.display(),
                entries
                    .iter()
                    .map(|e| format!("{}  {}", e.status, e.name))
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
        }
        Operation::Split {
            source,
            destination,
            bytes,
        } => {
            crate::desktop::split(&source, &destination, bytes, task)?;
            Ok("Split completed.".into())
        }
        Operation::Combine {
            sources,
            destination,
        } => {
            crate::desktop::combine(&sources, &destination, task)?;
            Ok("Combined files successfully.".into())
        }
        Operation::Mkfile(path) => {
            task.capture(&path)?;
            File::options().write(true).create_new(true).open(path)?;
            Ok("Created file.".into())
        }
        Operation::Permissions { paths, mode } => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                for path in paths {
                    task.capture(&path)?;
                    task.check()?;
                    if fs::symlink_metadata(&path)?.file_type().is_symlink() {
                        bail!("Change the symlink target directly.");
                    }
                    fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))?;
                }
                Ok("Permissions updated.".into())
            }
            #[cfg(not(unix))]
            {
                let _ = (paths, mode);
                bail!("POSIX permissions are not available on this platform.");
            }
        }
        Operation::Modified { paths, time } => {
            for path in &paths {
                task.capture(path)?;
            }
            task.check()?;
            crate::desktop::change_modified(&paths, time)?;
            Ok("Modification time updated.".into())
        }
        Operation::PermanentDelete(paths) => {
            validate_sources(&paths)?;
            for path in &paths {
                task.tick(path, 0)?;
                remove(path)?;
            }
            Ok(format!("Permanently deleted {} item(s).", paths.len()))
        }
        Operation::RemoteTransfer {
            from,
            entries,
            to,
            moving,
        } => {
            crate::remote::transfer(&from, &entries, &to, moving, task)?;
            Ok(format!("Transferred {} item(s).", entries.len()))
        }
        Operation::RemoteMkdir { connection, path } => {
            crate::remote::mkdir(&connection, &path)?;
            Ok("Remote folder created.".into())
        }
        Operation::RemoteDelete {
            connection,
            entries,
        } => {
            crate::remote::delete(&connection, &entries, task)?;
            Ok("Remote items deleted.".into())
        }
        Operation::Copy {
            sources,
            destination,
        } => transfer(&sources, &destination, false, task),
        Operation::Move {
            sources,
            destination,
        } => crate::transfer::transfer(&sources, &destination, true, Default::default(), task),
        Operation::Rename { source, name } => {
            crate::filesystem::validate_name(&name)?;
            let target = source
                .parent()
                .context("Cannot rename a filesystem root")?
                .join(name);
            if exists(&target) {
                bail!("That name already exists.");
            }
            task.capture(&source)?;
            task.capture(&target)?;
            crate::desktop::rename_exclusive(&source, &target)?;
            Ok("Renamed successfully.".into())
        }
        Operation::Trash(sources) => {
            validate_sources(&sources)?;
            for source in &sources {
                task.tick(source, 0)?;
                trash::delete(source)
                    .with_context(|| format!("Could not move {} to Trash", source.display()))?;
                task.progress.files += 1;
            }
            Ok(format!("Moved {} item(s) to Trash.", sources.len()))
        }
        Operation::Mkdir(path) => {
            task.capture(&path)?;
            fs::create_dir(&path)?;
            Ok(format!("Created {}", path.display()))
        }
        Operation::Pack {
            sources,
            destination,
        } => {
            validate_sources(&sources)?;
            task.capture(&destination)?;
            crate::archive::pack(&sources, &destination, task)?;
            Ok(format!("Created {}", destination.display()))
        }
        Operation::Extract {
            archive,
            selected,
            prefix,
            destination,
        } => {
            task.capture(&destination)?;
            crate::archive::extract(&archive, &selected, &prefix, &destination, task)?;
            Ok(format!("Extracted to {}", destination.display()))
        }
        Operation::Checksum(path) => {
            let mut input = File::open(&path)?;
            let mut digest = Sha256::new();
            let mut md5 = md5::Md5::new();
            let mut sha1 = sha1::Sha1::new();
            let mut sha512 = sha2::Sha512::new();
            let mut buffer = [0u8; 64 * 1024];
            loop {
                task.tick(&path, 0)?;
                let n = input.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                digest.update(&buffer[..n]);
                md5.update(&buffer[..n]);
                sha1.update(&buffer[..n]);
                sha512.update(&buffer[..n]);
                task.tick(&path, n as u64)?;
            }
            Ok(format!(
                "Checksums · {}\nMD5     {:x}\nSHA-1   {:x}\nSHA-512 {:x}\nSHA-256 {:x}",
                path.display(),
                md5.finalize(),
                sha1.finalize(),
                sha512.finalize(),
                digest.finalize()
            ))
        }
    }
}

pub fn run(operation: Operation, task: &mut Task) -> Result<String> {
    // Other operations may make nested copies without having a whole-job byte
    // budget. Keep those indeterminate rather than reporting one sub-copy as all.
    if !matches!(
        operation,
        Operation::Copy { .. }
            | Operation::Move { .. }
            | Operation::Transfer { .. }
            | Operation::ResolvedTransfer { .. }
            | Operation::RemoteTransfer { .. }
            | Operation::RemoteTransferResolved { .. }
            | Operation::ResumeLocal { .. }
            | Operation::ResumeRemote { .. }
    ) {
        task.plan_total(None);
    }
    execute(operation, task)
}

fn transfer(
    sources: &[PathBuf],
    destination: &Path,
    moving: bool,
    task: &mut Task,
) -> Result<String> {
    let targets = preflight(sources, destination)?;
    if task.needs_total() {
        task.plan_total(totals::local(sources, task)?);
    }
    let before = if moving {
        sources
            .iter()
            .map(|p| source_stamp(p, task))
            .collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    for (source, target) in sources.iter().zip(&targets) {
        task.capture(target)?;
        copy_item(source, target, task)
            .with_context(|| format!("Could not copy {}", source.display()))?;
    }
    task.check()?;
    if moving {
        for (source, before) in sources.iter().zip(&before) {
            if source_stamp(source, task)? != *before {
                bail!(
                    "A source changed during the move. All originals and completed copies were kept."
                );
            }
        }
        for source in sources {
            task.capture(source)?;
            remove(source).context("Copy completed, but an original could not be removed")?;
        }
    }
    Ok(format!(
        "{} {} item(s).",
        if moving { "Moved" } else { "Copied" },
        sources.len()
    ))
}

pub(crate) fn source_stamp(path: &Path, task: &Task) -> Result<[u8; 32]> {
    fn visit(path: &Path, digest: &mut Sha256, task: &Task) -> Result<()> {
        task.check()?;
        let metadata = fs::symlink_metadata(path)?;
        digest.update(path.as_os_str().as_encoded_bytes());
        digest.update([
            0,
            metadata.is_dir() as u8,
            metadata.file_type().is_symlink() as u8,
        ]);
        digest.update(metadata.len().to_le_bytes());
        if let Ok(modified) = metadata.modified()?.duration_since(SystemTime::UNIX_EPOCH) {
            digest.update(modified.as_nanos().to_le_bytes());
        }
        if metadata.file_type().is_symlink() {
            digest.update(fs::read_link(path)?.as_os_str().as_encoded_bytes());
        } else if metadata.is_dir() {
            let mut children = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
            children.sort_by_key(|e| e.file_name());
            for child in children {
                visit(&child.path(), digest, task)?;
            }
        }
        Ok(())
    }
    let mut digest = Sha256::new();
    visit(path, &mut digest, task)?;
    Ok(digest.finalize().into())
}

pub fn read_text(path: &Path, limit: u64) -> Result<String> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!(
            "Text files larger than {} cannot be opened in the built-in editor.",
            crate::filesystem::human_size(limit)
        );
    }
    if bytes.contains(&0) {
        bail!("This is a binary file. Use Open externally to view it.");
    }
    String::from_utf8(bytes).context("This file is not UTF-8. Use Open externally to view it.")
}

/// Refuses external changes and replaces the target only after a complete write.
pub fn save_text(path: &Path, original: &[u8], text: &str) -> Result<()> {
    save_bytes(path, original, text.as_bytes())
}
pub fn save_bytes(path: &Path, original: &[u8], bytes: &[u8]) -> Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        bail!("Edit the symlink target directly.");
    }
    let mut current = Vec::new();
    File::open(path)?
        .take(original.len() as u64 + 1)
        .read_to_end(&mut current)?;
    if current != original {
        bail!("This file changed on disk. Reopen it before saving to avoid losing those changes.");
    }
    let permissions = fs::metadata(path)?.permissions();
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing parent folder")?)?;
    temporary.write_all(bytes)?;
    temporary.as_file().set_permissions(permissions)?;
    crate::file_metadata::copy(path, temporary.path())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}
