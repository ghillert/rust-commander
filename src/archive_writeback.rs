//! Prepare every containing archive privately, then publish only the outer source.
use crate::{archive_tools, filesystem::Location, operations::Task};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

const LIMIT: u64 = 128 * 1024 * 1024;
static WRITEBACK: Mutex<()> = Mutex::new(());

fn hash(file: &Path, task: &Task) -> Result<String> {
    crate::storage::hash(&Location::Local(file.into()), file, task)
}
fn copy_password(from: &Path, to: &Path) -> Result<()> {
    if let Some(password) = crate::archive::password(from) {
        crate::archive::unlock(to, &password)?;
    }
    Ok(())
}

pub(crate) fn working_copy(file: &Path) -> Result<(tempfile::TempDir, String)> {
    ensure!(
        archive_tools::editable(file),
        "This archive source is read-only. Format overrides and HTTP archives cannot be saved back."
    );
    let expected = archive_tools::cached_origin(file)
        .context("Cached archive is no longer available; reopen it")?
        .hash;
    let directory = tempfile::Builder::new()
        .prefix("rust-commander-writeback-")
        .tempdir()?;
    let target = directory
        .path()
        .join(file.file_name().context("Missing archive name")?);
    fs::copy(file, &target)?;
    copy_password(file, &target)?;
    Ok((directory, expected))
}
pub(crate) fn edit(file: &Path, edit: &archive_tools::ZipEdit, task: &mut Task) -> Result<()> {
    let (working, expected) = working_copy(file)?;
    let output = working
        .path()
        .join(file.file_name().context("Missing archive name")?);
    crate::drag_drop::transient(task, working.path(), |task| {
        archive_tools::edit_archive(&output, edit, task)
    })?;
    publish(file, &output, &expected, task)
}
fn replace_local(
    source: &Path,
    output: &Path,
    expected: &Option<String>,
    task: &mut Task,
) -> Result<()> {
    ensure!(
        !fs::symlink_metadata(source)?.file_type().is_symlink(),
        "Archive source is a link"
    );
    ensure!(
        crate::history::fingerprint(source, task)? == *expected,
        "Containing archive changed; reopen it before saving"
    );
    let native_before = crate::file_metadata::digest(source)?;
    task.capture(source)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(source.parent().context("Missing archive folder")?)?;
    task.stream(&mut fs::File::open(output)?, &mut temporary, source)?;
    temporary
        .as_file()
        .set_permissions(fs::metadata(source)?.permissions())?;
    crate::file_metadata::copy(source, temporary.path())?;
    temporary.as_file().sync_all()?;
    ensure!(
        crate::history::fingerprint(source, task)? == *expected
            && crate::file_metadata::digest(source)? == native_before,
        "Containing archive changed while staging; source preserved"
    );
    task.check()?;
    temporary.persist(source).map_err(|e| e.error)?;
    Ok(())
}
pub(crate) fn publish(file: &Path, edited: &Path, expected: &str, task: &mut Task) -> Result<()> {
    let _lease = WRITEBACK
        .lock()
        .map_err(|_| anyhow::anyhow!("Archive write-back lock failed"))?;
    ensure!(
        archive_tools::editable(file),
        "This archive source is read-only"
    );
    ensure!(
        archive_tools::cached_origin(file).is_some_and(|cached| cached.hash == expected),
        "Cached archive changed while preparing the edit; reopen it before saving"
    );
    let stage = tempfile::Builder::new()
        .prefix("rust-commander-archive-chain-")
        .tempdir()?;
    let mut cursor = file.to_owned();
    let mut output = edited.to_owned();
    let mut updates = Vec::<(PathBuf, PathBuf)>::new();
    let mut root_expected = None;
    for depth in 0..=8 {
        task.check()?;
        ensure!(
            fs::metadata(&output)?.len() <= LIMIT,
            "Edited container exceeds 128 MiB"
        );
        let Some(cached) = archive_tools::cached_origin(&cursor) else {
            archive_tools::verify_cache_updates(&updates)?;
            replace_local(&cursor, &output, &root_expected, task)?;
            break;
        };
        ensure!(
            depth < 8 && cached.writable,
            "Archive nesting limit or read-only source"
        );
        ensure!(
            hash(&cursor, task)? == cached.hash,
            "Cached archive changed; reopen it before saving"
        );
        updates.push((cursor.clone(), output.clone()));
        match &cached.origin {
            Location::Archive { file: parent, .. } => {
                ensure!(
                    crate::history::fingerprint(parent, task)? == cached.parent_stamp,
                    "Containing archive changed since opening; reopen the nested archive before saving"
                );
                let folder = stage.path().join(depth.to_string());
                fs::create_dir(&folder)?;
                let working =
                    folder.join(parent.file_name().context("Missing parent archive name")?);
                fs::copy(parent, &working)?;
                copy_password(parent, &working)?;
                let member = crate::filesystem::portable_relative(&cached.source)?;
                let replacement = output.clone();
                crate::drag_drop::transient(task, stage.path(), |task| {
                    archive_tools::edit_tree(&working, task, |tree, task| {
                        let target = tree.join(&member);
                        ensure!(
                            target.is_file() && hash(&target, task)? == cached.hash,
                            "Nested archive entry changed; source preserved"
                        );
                        fs::remove_file(&target)?;
                        crate::operations::copy_item(&replacement, &target, task)
                    })
                })?;
                root_expected = cached.parent_stamp;
                cursor = parent.clone();
                output = working;
            }
            Location::Remote { connection, .. } => {
                archive_tools::verify_cache_updates(&updates)?;
                task.undo_blocked = true;
                let original = crate::remote::read_bytes(connection, &cached.source, LIMIT)?;
                use sha2::Digest;
                ensure!(
                    format!("{:x}", sha2::Sha256::digest(&original)) == cached.hash,
                    "Remote archive changed since opening; reopen it before saving"
                );
                let bytes = fs::read(&output)?;
                let message = crate::remote::save_bytes_bounded(
                    connection,
                    &cached.source,
                    &original,
                    &bytes,
                    LIMIT,
                    task,
                )?;
                task.notices.push(message);
                break;
            }
            Location::Local(_) => {
                archive_tools::verify_cache_updates(&updates)?;
                replace_local(&cached.source, &output, &cached.parent_stamp, task)?;
                break;
            }
        }
    }
    // Refresh only after the outer file has committed. Sibling cached archives
    // retain their old parent stamps and must be reopened before further edits.
    for (cached, output) in &updates {
        let refresh = (|| -> Result<()> {
            let mut temporary = tempfile::NamedTempFile::new_in(cached.parent().unwrap())?;
            std::io::copy(&mut fs::File::open(output)?, &mut temporary)?;
            temporary.as_file().sync_all()?;
            temporary.persist(cached).map_err(|e| e.error)?;
            Ok(())
        })();
        refresh
            .context("Source saved, but the cached archive could not be refreshed; reopen it")?;
    }
    archive_tools::refresh_cached_origins(
        &updates.into_iter().map(|(p, _)| p).collect::<Vec<_>>(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_prepared_before_another_save_cannot_overwrite_it() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a.txt");
        let b = root.path().join("b.txt");
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        let inner = root.path().join("inner.zip");
        crate::archive::pack(&[a, b], &inner, &mut Task::default()).unwrap();
        let outer = root.path().join("outer.zip");
        crate::archive::pack(&[inner], &outer, &mut Task::default()).unwrap();
        let cached = archive_tools::materialize(
            &Location::Archive {
                file: outer.clone(),
                prefix: "".into(),
            },
            Path::new("inner.zip"),
        )
        .unwrap();
        let (first, first_stamp) = working_copy(&cached).unwrap();
        let (second, second_stamp) = working_copy(&cached).unwrap();
        let first = first.path().join("inner.zip");
        let second = second.path().join("inner.zip");
        archive_tools::edit_archive(
            &first,
            &archive_tools::ZipEdit::Remove(vec!["a.txt".into()]),
            &mut Task::default(),
        )
        .unwrap();
        archive_tools::edit_archive(
            &second,
            &archive_tools::ZipEdit::Remove(vec!["b.txt".into()]),
            &mut Task::default(),
        )
        .unwrap();
        publish(&cached, &first, &first_stamp, &mut Task::default()).unwrap();
        let committed = fs::read(&outer).unwrap();
        assert!(publish(&cached, &second, &second_stamp, &mut Task::default()).is_err());
        assert_eq!(fs::read(&outer).unwrap(), committed);
        assert!(crate::archive::read_bytes(&cached, Path::new("a.txt"), 10).is_err());
        assert_eq!(
            crate::archive::read_bytes(&cached, Path::new("b.txt"), 10).unwrap(),
            b"b"
        );
    }
}
