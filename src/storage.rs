//! Shared local/remote inspection used by synchronization and transfer recovery.
use crate::{
    filesystem::{self, Entry, Location},
    operations::Task,
};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
pub fn reader(location: &Location, path: &Path) -> Result<Box<dyn Read>> {
    Ok(match location {
        Location::Local(_) => Box::new(std::fs::File::open(path)?),
        Location::Remote { connection, .. } => crate::remote::read(connection, path)?,
        _ => anyhow::bail!("Extract archive entries first"),
    })
}
pub fn hash(location: &Location, path: &Path, task: &Task) -> Result<String> {
    let mut input = reader(location, path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        task.check()?;
        let n = input.read(&mut buffer[..task.chunk_size()])?;
        if n == 0 {
            break;
        }
        task.pace(n as u64)?;
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn stamp(location: &Location, path: &Path, task: &Task) -> Result<Option<String>> {
    if matches!(location, Location::Local(_)) {
        return crate::history::fingerprint(path, task);
    }
    fn visit(
        location: &Location,
        entry: &Entry,
        task: &Task,
        hash: &mut Sha256,
        depth: usize,
        count: &mut usize,
    ) -> Result<()> {
        task.check()?;
        *count += 1;
        ensure!(
            depth < 64 && *count <= 100_000,
            "Remote tree exceeds the inspection limit"
        );
        ensure!(!entry.is_symlink, "Remote tree inspection refuses links");
        // Hash the same canonical key used to address the remote object. Native
        // joins and portable receipt paths can spell it differently on Windows.
        let path = std::path::PathBuf::from(crate::remote::key(&entry.path, false)?);
        hash.update(format!(
            "{:?}|{}|{}|{:?}",
            path, entry.is_dir, entry.size, entry.modified
        ));
        if entry.is_dir {
            let mut entries = filesystem::list(&crate::directory_tree::at(location, &entry.path))?;
            entries.sort_by(|a, b| a.path.cmp(&b.path));
            for child in entries {
                visit(location, &child, task, hash, depth + 1, count)?;
            }
        } else {
            hash.update(crate::storage::hash(location, &entry.path, task)?);
        }
        Ok(())
    }
    let Some(entry) = crate::remote::entry_at(location, path)? else {
        return Ok(None);
    };
    let mut hash = Sha256::new();
    visit(location, &entry, task, &mut hash, 0, &mut 0)?;
    Ok(Some(format!("{:x}", hash.finalize())))
}
