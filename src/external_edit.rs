//! Private working copies and explicit, guarded external-editor save-back.
use crate::{
    filesystem::Location,
    operations::{Operation, Task},
};
use anyhow::{Context, Result, ensure};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub const LIMIT: u64 = 2 * 1024 * 1024 * 1024;
#[derive(Clone)]
pub struct Session {
    directory: std::sync::Arc<tempfile::TempDir>,
    pub file: PathBuf,
    pub location: Location,
    pub source: PathBuf,
    original: std::sync::Arc<crate::private_bytes::Bytes>,
    archive_stamp: Option<String>,
}
impl Session {
    pub fn prepare(location: Location, source: PathBuf) -> Result<Self> {
        Self::prepare_with_task(location, source, &mut Task::default())
    }
    pub fn prepare_with_task(location: Location, source: PathBuf, task: &mut Task) -> Result<Self> {
        if matches!(location, Location::Local(_)) {
            ensure!(
                !std::fs::symlink_metadata(&source)?.file_type().is_symlink(),
                "Edit the symlink target directly"
            );
        }
        let archive_stamp = if let Location::Archive { file, .. } = &location {
            ensure!(
                crate::archive_tools::editable(file),
                "External save-back requires a writable ZIP/TAR/7z archive source"
            );
            crate::history::fingerprint(file, &Task::default())?
        } else {
            None
        };
        let original = crate::viewer::snapshot(&location, &source, LIMIT, task)?;
        let directory = tempfile::Builder::new()
            .prefix("rust-commander-editor-")
            .tempdir()?;
        let directory = std::sync::Arc::new(directory);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        let file = directory
            .path()
            .join(source.file_name().context("Select a file")?);
        std::fs::File::create(&file)?.write_all(original.as_ref().as_ref())?;
        Ok(Self {
            directory,
            file,
            location,
            source,
            original,
            archive_stamp,
        })
    }
    pub fn changed(&self) -> Result<bool> {
        ensure!(
            !std::fs::symlink_metadata(&self.file)?
                .file_type()
                .is_symlink(),
            "Editor output must be a regular file"
        );
        Ok(!crate::operations::matches_reader(
            &mut std::fs::File::open(&self.file)?,
            self.original.as_ref().as_ref(),
            &Task::default(),
        )?)
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        ensure!(
            !std::fs::symlink_metadata(&self.file)?
                .file_type()
                .is_symlink(),
            "Editor output must be a regular file"
        );
        let mut bytes = Vec::new();
        std::fs::File::open(&self.file)?
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= LIMIT, "Edited file exceeds 2 GiB");
        Ok(bytes)
    }
    pub fn save_operation(&self) -> Result<Operation> {
        let metadata = std::fs::symlink_metadata(&self.file)?;
        ensure!(
            metadata.is_file() && metadata.len() <= LIMIT,
            "Editor output must be a regular file of at most 2 GiB"
        );
        Ok(Operation::ExternalSave {
            location: self.location.clone(),
            path: self.source.clone(),
            working: self.file.clone(),
            original: Some(self.original.clone()),
            lease: Some(self.directory.clone()),
            expected: self.archive_stamp.clone(),
        })
    }
}
pub(crate) fn save(
    location: &Location,
    path: &Path,
    working: &Path,
    original: Option<std::sync::Arc<crate::private_bytes::Bytes>>,
    lease: Option<std::sync::Arc<tempfile::TempDir>>,
    expected: &Option<String>,
    task: &mut Task,
) -> Result<String> {
    let lease = lease.context(
        "Editor working copy cannot be resumed after restart; recover its draft or reopen the file",
    )?;
    let original = original.context("Editor original snapshot is unavailable")?;
    ensure!(
        working.starts_with(lease.path()) && std::fs::symlink_metadata(working)?.is_file(),
        "Editor output must stay in its private working directory"
    );
    let edited = crate::private_bytes::Bytes::capture(
        &mut std::fs::File::open(working)?,
        LIMIT,
        working,
        task,
    )?;
    let original = original.as_ref().as_ref();
    let edited = edited.as_ref().as_ref();
    ensure!(original != edited, "The editor copy has no changes");
    match location {
        Location::Network { .. } => anyhow::bail!("Open a share or export to edit files."),
        Location::Local(_) => {
            task.capture(path)?;
            crate::operations::save_bytes(path, original, edited)?;
            Ok("Saved external editor working copy".into())
        }
        Location::Remote { connection, .. } => {
            crate::remote::save_bytes_bounded(connection, path, original, edited, LIMIT, task)
        }
        Location::Archive { file, .. } => save_archive(file, path, expected, edited, task),
    }
}

pub fn save_archive(
    archive: &Path,
    path: &Path,
    expected: &Option<String>,
    bytes: &[u8],
    task: &mut Task,
) -> Result<String> {
    ensure!(
        crate::history::fingerprint(archive, task)? == *expected,
        "Archive changed since opening the editor; reopen it before saving"
    );
    ensure!(
        crate::archive_tools::editable(archive),
        "This archive source is read-only"
    );
    let cached = crate::archive_tools::cached_origin(archive);
    let path = crate::filesystem::portable_relative(path)?;
    let stage = tempfile::tempdir()?;
    let working = stage
        .path()
        .join(archive.file_name().context("Missing archive filename")?);
    std::fs::copy(archive, &working)?;
    if let Some(password) = crate::archive::password(archive) {
        crate::archive::unlock(&working, &password)?;
    }
    let entry = stage
        .path()
        .join("entry")
        .join(path.file_name().context("Missing member name")?);
    std::fs::create_dir_all(entry.parent().unwrap())?;
    std::fs::write(&entry, bytes)?;
    crate::drag_drop::transient(task, stage.path(), |task| {
        crate::archive_tools::edit_archive(
            &working,
            &crate::archive_tools::ZipEdit::Remove(vec![path.clone()]),
            task,
        )?;
        crate::archive_tools::edit_archive(
            &working,
            &crate::archive_tools::ZipEdit::Add {
                sources: vec![entry],
                prefix: path.parent().unwrap_or(Path::new("")).into(),
            },
            task,
        )?;
        Ok(())
    })?;
    if let Some(cached) = cached {
        ensure!(
            crate::history::fingerprint(archive, task)? == *expected,
            "Cached archive changed while editing"
        );
        crate::archive_writeback::publish(archive, &working, &cached.hash, task)?;
        return Ok("Saved edited entry through its containing archive source.".into());
    }
    task.capture(archive)?;
    ensure!(
        crate::history::fingerprint(archive, task)? == *expected,
        "Archive changed while preparing the replacement"
    );
    let metadata = std::fs::metadata(archive)?;
    let native_before = crate::file_metadata::digest(archive)?;
    let mut input = std::fs::File::open(working)?;
    let mut output =
        tempfile::NamedTempFile::new_in(archive.parent().context("Missing archive parent")?)?;
    task.stream(&mut input, &mut output, archive)?;
    output.as_file().set_permissions(metadata.permissions())?;
    crate::file_metadata::copy(archive, output.path())?;
    output.as_file().sync_all()?;
    ensure!(
        crate::history::fingerprint(archive, task)? == *expected
            && crate::file_metadata::digest(archive)? == native_before,
        "Archive changed while staging the edited container"
    );
    output.persist(archive).map_err(|e| e.error)?;
    Ok("Saved edited archive entry.".into())
}
