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
pub const LIMIT: u64 = 32 * 1024 * 1024;
pub struct Session {
    directory: tempfile::TempDir,
    pub file: PathBuf,
    pub location: Location,
    pub source: PathBuf,
    original: Vec<u8>,
    archive_stamp: Option<String>,
}
impl Session {
    pub fn prepare(location: Location, source: PathBuf) -> Result<Self> {
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
        let original = crate::viewer::read(&location, &source, LIMIT)?;
        let directory = tempfile::Builder::new()
            .prefix("rust-commander-editor-")
            .tempdir()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
        }
        let file = directory
            .path()
            .join(source.file_name().context("Select a file")?);
        std::fs::File::create(&file)?.write_all(&original)?;
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
        Ok(self.bytes()? != self.original)
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        ensure!(
            self.file.starts_with(self.directory.path()),
            "Invalid editor path"
        );
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
        ensure!(bytes.len() as u64 <= LIMIT, "Edited file exceeds 32 MiB");
        Ok(bytes)
    }
    pub fn save_operation(&self) -> Result<Operation> {
        let bytes = self.bytes()?;
        ensure!(bytes != self.original, "The editor copy has no changes");
        Ok(match &self.location {
            Location::Local(_) => Operation::SaveBytes {
                path: self.source.clone(),
                original: self.original.clone(),
                bytes,
            },
            Location::Remote { connection, .. } => Operation::RemoteExternalSave {
                connection: connection.clone(),
                path: self.source.clone(),
                original: self.original.clone(),
                bytes,
            },
            Location::Archive { file, .. } => Operation::ArchiveSaveBytes {
                archive: file.clone(),
                path: self.source.clone(),
                expected: self.archive_stamp.clone(),
                bytes,
            },
        })
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
