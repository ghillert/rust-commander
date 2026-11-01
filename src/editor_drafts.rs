//! Private, atomic editor recovery snapshots. Discard receipts prevent a late
//! autosave worker from resurrecting a draft after an explicit discard/save.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 192 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
pub struct Draft {
    pub id: u64,
    pub title: String,
    pub location: crate::filesystem::Location,
    pub path: PathBuf,
    pub document: crate::text_document::Document,
    pub text: String,
}
fn root() -> Result<PathBuf> {
    Ok(crate::settings::path()?
        .parent()
        .context("Missing settings folder")?
        .join("editor-drafts"))
}
fn locked(root: &Path) -> Result<fs::File> {
    fs::create_dir_all(root)?;
    ensure!(
        !fs::symlink_metadata(root)?.file_type().is_symlink(),
        "Draft folder is a link"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    }
    let path = root.join("drafts.lock");
    if path.exists() {
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "Draft lock is a link"
        );
    }
    let mut opts = fs::OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock = opts.open(path)?;
    lock.lock()?;
    Ok(lock)
}
fn name(id: u64) -> String {
    format!("{id:016x}.json")
}
fn save_at(root: &Path, draft: &Draft) -> Result<()> {
    ensure!(
        draft.document.original.len() as u64 <= crate::viewer::EDIT_LIMIT
            && draft.text.len() <= 64 * 1024 * 1024,
        "Editor draft exceeds recovery limit"
    );
    let _lock = locked(root)?;
    if root.join(format!("{:016x}.discarded", draft.id)).exists() {
        return Ok(());
    }
    let mut temp = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer(&mut temp, draft)?;
    ensure!(
        temp.as_file().metadata()?.len() <= LIMIT,
        "Encoded editor draft exceeds recovery limit"
    );
    temp.as_file_mut().flush()?;
    temp.as_file().sync_all()?;
    temp.persist(root.join(name(draft.id)))
        .map_err(|e| e.error)?;
    let mut info = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer(
        &mut info,
        &draft.title.chars().take(1024).collect::<String>(),
    )?;
    info.as_file().sync_all()?;
    info.persist(root.join(format!("{:016x}.info", draft.id)))
        .map_err(|e| e.error)?;
    #[cfg(unix)]
    fs::File::open(root)?.sync_all()?;
    Ok(())
}
pub fn save(draft: &Draft) -> Result<()> {
    save_at(&root()?, draft)
}
fn discard_at(root: &Path, id: u64) -> Result<()> {
    let _lock = locked(root)?;
    let receipt = root.join(format!("{id:016x}.discarded"));
    if !receipt.exists() {
        let mut temp = tempfile::NamedTempFile::new_in(root)?;
        temp.write_all(b"discarded\n")?;
        temp.as_file().sync_all()?;
        temp.persist(&receipt).map_err(|e| e.error)?;
    }
    let _ = fs::remove_file(root.join(format!("{id:016x}.info")));
    match fs::remove_file(root.join(name(id))) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub fn discard(id: u64) -> Result<()> {
    discard_at(&root()?, id)
}
pub fn list() -> Result<Vec<(u64, String)>> {
    let root = root()?;
    let _lock = locked(&root)?;
    let mut result = Vec::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let filename = entry.file_name();
        let Some(id) = filename
            .to_str()
            .and_then(|s| s.strip_suffix(".json"))
            .filter(|s| s.len() == 16)
            .and_then(|s| u64::from_str_radix(s, 16).ok())
        else {
            continue;
        };
        if entry.file_type()?.is_file() {
            // Read on explicit recovery, rather than loading every draft into RAM.
            let info = root.join(format!("{id:016x}.info"));
            let title = (|| -> Result<String> {
                ensure!(
                    fs::symlink_metadata(&info)?.file_type().is_file(),
                    "Draft information is not a file"
                );
                let mut bytes = Vec::new();
                fs::File::open(info)?.take(16385).read_to_end(&mut bytes)?;
                ensure!(bytes.len() <= 16384, "Draft information exceeds the limit");
                Ok(serde_json::from_slice::<String>(&bytes)?)
            })()
            .unwrap_or_else(|_| format!("Editor draft {id:016x}"));
            result.push((id, title));
        }
    }
    result.sort_by_key(|(id, _)| *id);
    Ok(result)
}
pub fn load(id: u64) -> Result<Draft> {
    let root = root()?;
    let _lock = locked(&root)?;
    let path = root.join(name(id));
    ensure!(
        fs::symlink_metadata(&path)?.file_type().is_file(),
        "Draft is not a regular file"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= LIMIT, "Editor draft is too large");
    let draft: Draft =
        serde_json::from_slice(&bytes).context("Cannot read editor recovery draft")?;
    ensure!(
        draft.id == id
            && draft.document.original.len() as u64 <= crate::viewer::EDIT_LIMIT
            && draft.text.len() <= 64 * 1024 * 1024,
        "Invalid editor draft"
    );
    Ok(draft)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discard_wins_against_late_autosave() {
        let root = tempfile::tempdir().unwrap();
        let draft = Draft {
            id: 123,
            title: "test".into(),
            location: crate::filesystem::Location::Local("/tmp".into()),
            path: "/tmp/test".into(),
            document: crate::text_document::Document::detect(b"before".to_vec()).unwrap(),
            text: "edited".into(),
        };
        save_at(root.path(), &draft).unwrap();
        let stored: Draft =
            serde_json::from_slice(&fs::read(root.path().join(name(123))).unwrap()).unwrap();
        assert_eq!(stored.text, "edited");
        assert_eq!(stored.document.original, b"before");
        discard_at(root.path(), 123).unwrap();
        save_at(root.path(), &draft).unwrap();
        assert!(!root.path().join(name(123)).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
}
