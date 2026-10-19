//! Durable batch receipts and verified per-file staging. No credentials are stored.
use crate::{
    filesystem::{self, Entry, Location},
    operations::Task,
    remote, storage,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
#[derive(Serialize, Deserialize)]
struct Item {
    source: PathBuf,
    relative: PathBuf,
    directory: bool,
    size: u64,
    hash: Option<String>,
    done: bool,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    from: Location,
    to: Location,
    selection: Vec<PathBuf>,
    items: Vec<Item>,
}
fn save(path: &Path, manifest: &Manifest) -> Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing checkpoint directory")?)?;
    file.write_all(&serde_json::to_vec(manifest)?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
fn scan(
    from: &Location,
    entries: &[Entry],
    items: &mut Vec<Item>,
    depth: usize,
    prefix: &Path,
    task: &Task,
) -> Result<()> {
    ensure!(depth < 64, "Transfer tree is too deep");
    for listed in entries {
        task.check()?;
        let fresh = remote::entry_at(from, &listed.path)?.context("Transfer source disappeared")?;
        ensure!(
            fresh.is_dir == listed.is_dir,
            "Transfer source type changed"
        );
        let entry = &fresh;
        ensure!(!entry.is_symlink, "Resumable transfers do not follow links");
        filesystem::validate_name(&entry.name)?;
        let relative = filesystem::portable_relative(&prefix.join(&entry.name))?;
        items.push(Item {
            source: entry.path.clone(),
            relative: relative.clone(),
            directory: entry.is_dir,
            size: entry.size,
            hash: None,
            done: false,
        });
        ensure!(items.len() <= 100_000, "Transfer exceeds 100,000 entries");
        if entry.is_dir {
            scan(
                from,
                &filesystem::list(&crate::directory_tree::at(from, &entry.path))?,
                items,
                depth + 1,
                &relative,
                task,
            )?;
        }
    }
    Ok(())
}
fn ranged(location: &Location, path: &Path, start: u64) -> Result<Box<dyn Read>> {
    match location {
        Location::Local(_) => {
            let mut file = fs::File::open(path)?;
            file.seek(SeekFrom::Start(start))?;
            Ok(Box::new(file))
        }
        Location::Remote { connection, .. } => remote::read_range(connection, path, start),
        _ => anyhow::bail!("Extract archives first"),
    }
}
fn equal_prefix(
    location: &Location,
    path: &Path,
    local: &Path,
    length: u64,
    task: &Task,
) -> Result<()> {
    let mut a = storage::reader(location, path)?;
    let mut b = fs::File::open(local)?;
    let mut remaining = length;
    let mut x = [0; 65536];
    let mut y = [0; 65536];
    while remaining > 0 {
        task.check()?;
        let n = remaining.min(x.len() as u64) as usize;
        a.read_exact(&mut x[..n])?;
        b.read_exact(&mut y[..n])?;
        ensure!(
            x[..n] == y[..n],
            "Partial transfer differs from its source; checkpoint retained for review"
        );
        remaining -= n as u64;
    }
    Ok(())
}
pub fn run(
    id: &str,
    from: &Location,
    entries: &[Entry],
    to: &Location,
    task: &mut Task,
) -> Result<String> {
    let root = crate::settings::path()?.with_file_name("transfers");
    run_in(&root, id, from, entries, to, task)
}
pub fn run_in(
    root: &Path,
    id: &str,
    from: &Location,
    entries: &[Entry],
    to: &Location,
    task: &mut Task,
) -> Result<String> {
    filesystem::validate_name(id)?;
    ensure!(id.starts_with("batch-"), "Invalid transfer checkpoint ID");
    ensure!(
        !entries.is_empty()
            && !matches!(from, Location::Archive { .. })
            && !matches!(to, Location::Archive { .. }),
        "Choose local or remote files"
    );
    fs::create_dir_all(root)?;
    ensure!(
        !fs::symlink_metadata(root)?.file_type().is_symlink(),
        "Checkpoint storage is a link"
    );
    let directory = root.join(id);
    fs::create_dir_all(&directory)?;
    ensure!(
        !fs::symlink_metadata(&directory)?.file_type().is_symlink(),
        "Checkpoint directory is a link"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    let path = directory.join("manifest.json");
    let selection = entries.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
    let target_root = crate::directory_tree::path(to);
    let same = matches!((from,to),(Location::Remote{connection:a,..},Location::Remote{connection:b,..}) if a==b)
        || matches!((from, to), (Location::Local(_), Location::Local(_)));
    ensure!(
        !(same
            && entries.iter().any(|e| target_root.join(&e.name) == e.path
                || (e.is_dir && target_root.starts_with(&e.path)))),
        "Cannot transfer into the source tree"
    );
    let mut manifest: Manifest = if crate::operations::exists(&path) {
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "Checkpoint manifest is a link"
        );
        let manifest: Manifest = serde_json::from_slice(&fs::read(&path)?)?;
        ensure!(
            manifest.from == *from && manifest.to == *to && manifest.selection == selection,
            "Checkpoint belongs to another transfer"
        );
        manifest
    } else {
        for entry in entries {
            ensure!(
                remote::entry_at(to, &target_root.join(&entry.name))?.is_none(),
                "A destination exists; resumable copies require new targets"
            );
        }
        let mut items = Vec::new();
        scan(from, entries, &mut items, 0, Path::new(""), task)?;
        let mut names = std::collections::BTreeSet::new();
        ensure!(
            items.iter().all(|item| names.insert(item.relative.clone())),
            "Duplicate transfer targets"
        );
        let manifest = Manifest {
            from: from.clone(),
            to: to.clone(),
            selection,
            items,
        };
        save(&path, &manifest)?;
        manifest
    };
    ensure!(
        manifest.items.len() <= 100_000,
        "Invalid checkpoint entry count"
    );
    task.undo_blocked = true; // A resumed batch can span processes and remote stores.
    let mut resumed = 0;
    for i in 0..manifest.items.len() {
        task.check()?;
        let relative = filesystem::portable_relative(&manifest.items[i].relative)?;
        let source = manifest.items[i].source.clone();
        ensure!(
            entries
                .iter()
                .any(|e| source == e.path || (e.is_dir && source.starts_with(&e.path))),
            "Checkpoint source is outside the selection"
        );
        let target = target_root.join(&relative);
        if manifest.items[i].directory {
            let was_done = manifest.items[i].done;
            manifest.items[i].done = true;
            save(&path, &manifest)?;
            match remote::entry_at(to, &target)? {
                Some(entry) => ensure!(
                    entry.is_dir && !entry.is_symlink && was_done,
                    "Destination folder changed"
                ),
                None => {
                    if matches!(to, Location::Local(_)) {
                        fs::create_dir(&target)?;
                    } else if let Location::Remote { connection, .. } = to {
                        remote::mkdir(connection, &target)?;
                    }
                }
            }
            manifest.items[i].done = true;
            save(&path, &manifest)?;
            continue;
        }
        let item = &manifest.items[i];
        let current = remote::entry_at(from, &source)?.context("Transfer source disappeared")?;
        ensure!(
            !current.is_dir && !current.is_symlink && current.size == item.size,
            "Transfer source size/type changed"
        );
        let hash = storage::hash(from, &source, task)?;
        if let Some(expected) = &item.hash {
            ensure!(
                *expected == hash,
                "Source content changed; start a new transfer"
            );
        }
        manifest.items[i].hash = Some(hash.clone());
        save(&path, &manifest)?;
        if let Some(existing) = remote::entry_at(to, &target)? {
            ensure!(
                !existing.is_dir
                    && !existing.is_symlink
                    && storage::hash(to, &target, task)? == hash,
                "Destination changed or an uncommitted partial output needs review: {}",
                target.display()
            );
            manifest.items[i].done = true;
            save(&path, &manifest)?;
            resumed += 1;
            continue;
        }
        ensure!(
            !manifest.items[i].done,
            "A completed destination disappeared; start a new transfer"
        );
        let partial = directory.join(format!("{i}.part"));
        if crate::operations::exists(&partial) {
            ensure!(
                !fs::symlink_metadata(&partial)?.file_type().is_symlink(),
                "Partial file is a link"
            );
        }
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut output = options.open(&partial)?;
        let offset = output.metadata()?.len();
        ensure!(offset <= current.size, "Partial file exceeds source size");
        if offset > 0 {
            equal_prefix(from, &source, &partial, offset, task)?;
            resumed += 1;
        }
        output.seek(SeekFrom::End(0))?;
        let mut input = ranged(from, &source, offset)?;
        let copied = task.stream(&mut input, &mut output, &source);
        output.sync_all()?;
        copied?;
        ensure!(
            output.metadata()?.len() == current.size,
            "Source length changed during transfer"
        );
        drop(output);
        let local = Location::Local(directory.clone());
        ensure!(
            storage::hash(&local, &partial, task)? == hash
                && storage::hash(from, &source, task)? == hash,
            "Source changed during transfer; staging retained"
        );
        match to {
            Location::Local(_) => {
                crate::operations::copy_item(&partial, &target, task)?;
                if let Some(modified) = current.modified {
                    filetime::set_file_mtime(
                        &target,
                        filetime::FileTime::from_system_time(modified),
                    )?;
                }
                if matches!(from, Location::Local(_)) {
                    crate::file_metadata::copy(&source, &target)?;
                }
            }
            Location::Remote { connection, .. } => {
                let stage = target
                    .parent()
                    .context("Missing transfer parent")?
                    .join(format!(".rust-commander-{id}-{i}.part"));
                let offset = if let Some(entry) = remote::entry_at(to, &stage)? {
                    ensure!(
                        !entry.is_dir && entry.size <= current.size,
                        "Remote stage changed"
                    );
                    equal_prefix(to, &stage, &partial, entry.size, task)?;
                    entry.size
                } else {
                    0
                };
                remote::upload_checkpoint(connection, &stage, &partial, offset, task)?;
                ensure!(
                    storage::hash(to, &stage, task)? == hash,
                    "Remote stage failed verification"
                );
                remote::publish_checkpoint(connection, &stage, &target, task)?;
            }
            _ => unreachable!(),
        }
        ensure!(
            storage::hash(to, &target, task)? == hash,
            "Destination failed verification; checkpoint retained"
        );
        manifest.items[i].done = true;
        save(&path, &manifest)?;
        fs::remove_file(partial)?;
    }
    fs::remove_dir_all(&directory)?;
    Ok(format!(
        "Completed {} checkpointed items; recovered {resumed} partial/completed files.",
        manifest.items.len()
    ))
}

/// Discard only local checkpoint data belonging to this exact durable recipe.
/// Remote hidden staging files remain available for explicit server-side review.
pub fn discard(id: &str, from: &Location, to: &Location) -> Result<()> {
    filesystem::validate_name(id)?;
    ensure!(id.starts_with("batch-"), "Invalid checkpoint ID");
    let root = crate::settings::path()?.with_file_name("transfers");
    let directory = root.join(id);
    if !crate::operations::exists(&directory) {
        return Ok(());
    }
    ensure!(
        !fs::symlink_metadata(&root)?.file_type().is_symlink()
            && !fs::symlink_metadata(&directory)?.file_type().is_symlink(),
        "Checkpoint storage is a link"
    );
    let path = directory.join("manifest.json");
    ensure!(
        !fs::symlink_metadata(&path)?.file_type().is_symlink(),
        "Checkpoint manifest is a link"
    );
    let manifest: Manifest = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        manifest.from == *from && manifest.to == *to,
        "Checkpoint belongs to another transfer"
    );
    fs::remove_dir_all(directory)?;
    Ok(())
}
