//! Durable batch receipts and verified per-file staging. No credentials are stored.
use crate::{
    filesystem::{self, Entry, Location},
    operations::Task,
    remote, storage,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
mod conflicts;
use crate::transfer::{ConflictPolicy, TransferOptions};
use conflicts::{Decision, Original};

/// A sibling lock survives checkpoint cleanup, avoiding an unlinked-lock race
/// between separate app processes. OS locks are released on process death.
fn lease(root: &Path, id: &str) -> Result<fs::File> {
    let path = root.join(format!("{id}.lock"));
    if crate::operations::exists(&path) {
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "Checkpoint lock is a link"
        );
    }
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    file.try_lock()
        .map_err(|_| anyhow::anyhow!("Checkpoint is already in use or cannot be locked"))?;
    Ok(file)
}
#[derive(Serialize, Deserialize)]
struct Item {
    source: PathBuf,
    relative: PathBuf,
    directory: bool,
    size: u64,
    hash: Option<String>,
    done: bool,
    #[serde(default)]
    decision: Decision,
    #[serde(default)]
    original: Vec<Original>,
    #[serde(default)]
    backup_hash: Option<String>,
    #[serde(default)]
    cleared: bool,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    from: Location,
    to: Location,
    selection: Vec<PathBuf>,
    items: Vec<Item>,
    #[serde(default)]
    options: TransferOptions,
    #[serde(default)]
    overrides: BTreeMap<PathBuf, ConflictPolicy>,
}
fn save(path: &Path, manifest: &Manifest) -> Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing checkpoint directory")?)?;
    file.write_all(&serde_json::to_vec(manifest)?)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    #[cfg(unix)]
    fs::File::open(path.parent().context("Missing checkpoint directory")?)?.sync_all()?;
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
            decision: Decision::New,
            original: Vec::new(),
            backup_hash: None,
            cleared: false,
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
        let n = remaining.min(task.chunk_size() as u64) as usize;
        a.read_exact(&mut x[..n])?;
        b.read_exact(&mut y[..n])?;
        ensure!(
            x[..n] == y[..n],
            "Partial transfer differs from its source; checkpoint retained for review"
        );
        task.pace(n as u64)?;
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
    run_resolved_in(
        root,
        id,
        from,
        entries,
        to,
        TransferOptions::default(),
        &BTreeMap::new(),
        task,
    )
}
pub fn run_resolved(
    id: &str,
    from: &Location,
    entries: &[Entry],
    to: &Location,
    options: TransferOptions,
    overrides: &BTreeMap<PathBuf, ConflictPolicy>,
    task: &mut Task,
) -> Result<String> {
    let root = crate::settings::path()?.with_file_name("transfers");
    run_resolved_in(&root, id, from, entries, to, options, overrides, task)
}
#[allow(clippy::too_many_arguments)]
pub fn run_resolved_in(
    root: &Path,
    id: &str,
    from: &Location,
    entries: &[Entry],
    to: &Location,
    options: TransferOptions,
    overrides: &BTreeMap<PathBuf, ConflictPolicy>,
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
    let _lease = lease(root, id)?;
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
    let checkpoint_directory = directory.canonicalize()?;
    for entry in entries {
        if let Some(source) = remote::checkpoint_local_path(from, &entry.path)? {
            ensure!(
                !checkpoint_directory.starts_with(source.canonicalize()?),
                "Checkpoint storage is inside the source selection; choose an ordinary copy or another checkpoint location"
            );
        }
    }
    if let Some(root) = remote::checkpoint_local_path(to, target_root)? {
        let root = root
            .canonicalize()
            .context("Choose an existing destination folder")?;
        ensure!(root.is_dir(), "The destination is not a folder");
        for entry in entries {
            ensure!(
                !checkpoint_directory.starts_with(root.join(&entry.name)),
                "Checkpoint storage overlaps a destination item"
            );
            if let Some(source) = remote::checkpoint_local_path(from, &entry.path)? {
                let source = source.canonicalize()?;
                ensure!(
                    root.join(&entry.name) != source
                        && !(entry.is_dir && root.starts_with(&source)),
                    "Cannot transfer into the source tree, including through a filesystem alias"
                );
            }
        }
    }
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
            manifest.from == *from
                && manifest.to == *to
                && manifest.selection == selection
                && manifest.options == options
                && manifest.overrides == *overrides,
            "Checkpoint belongs to another transfer"
        );
        manifest
    } else {
        let mut items = Vec::new();
        scan(from, entries, &mut items, 0, Path::new(""), task)?;
        conflicts::plan(&mut items, to, options, overrides, task)?;
        let mut names = BTreeSet::new();
        ensure!(
            items.iter().all(|item| names.insert(item.relative.clone())),
            "Duplicate transfer targets"
        );
        let manifest = Manifest {
            from: from.clone(),
            to: to.clone(),
            selection,
            items,
            options,
            overrides: overrides.clone(),
        };
        save(&path, &manifest)?;
        manifest
    };
    ensure!(
        manifest.items.len() <= 100_000,
        "Invalid checkpoint entry count"
    );
    let passes = remote::checkpoint_passes(to)?;
    let total = manifest
        .items
        .iter()
        .filter(|item| !item.directory && item.decision != Decision::Skip)
        .try_fold(0u64, |sum, item| {
            sum.checked_add(item.size.checked_mul(passes)?)
        })
        .and_then(|sum| {
            manifest
                .items
                .iter()
                .flat_map(|i| &i.original)
                .try_fold(sum, |sum, node| sum.checked_add(node.size))
        });
    task.plan_total(total);
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
        if manifest.items[i].decision == Decision::Skip {
            continue;
        }
        if manifest.items[i].backup_hash.is_some() {
            let restored = manifest.items[i]
                .original
                .iter()
                .fold(0u64, |sum, node| sum.saturating_add(node.size));
            task.advance_progress(restored);
        }
        if manifest.items[i].directory {
            let current =
                remote::entry_at(from, &source)?.context("Transfer source disappeared")?;
            ensure!(
                current.is_dir && !current.is_symlink,
                "Transfer source type changed"
            );
            conflicts::prepare(&mut manifest, i, &path, &directory, &target, task)?;
            let was_done = manifest.items[i].done;
            manifest.items[i].done = true;
            save(&path, &manifest)?;
            match remote::entry_at(to, &target)? {
                Some(entry) => ensure!(
                    entry.is_dir
                        && !entry.is_symlink
                        && (was_done || manifest.items[i].decision == Decision::Merge),
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
        // Stage new bytes before clearing a replacement. Its old destination
        // stays available throughout the initial download.
        let pending_replacement =
            manifest.items[i].decision == Decision::Replace && !manifest.items[i].cleared;
        if manifest.items[i].decision == Decision::Replace && !pending_replacement {
            conflicts::prepare(&mut manifest, i, &path, &directory, &target, task)?;
        }
        if !pending_replacement && let Some(existing) = remote::entry_at(to, &target)? {
            ensure!(
                !existing.is_dir
                    && !existing.is_symlink
                    && storage::hash(to, &target, task)? == hash,
                "Destination changed or an uncommitted partial output needs review: {}",
                target.display()
            );
            manifest.items[i].done = true;
            save(&path, &manifest)?;
            task.advance_progress(current.size.saturating_mul(passes));
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
        task.advance_progress(offset);
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
        conflicts::prepare(&mut manifest, i, &path, &directory, &target, task)?;
        match to {
            Location::Local(_) => {
                let mut temporary = tempfile::NamedTempFile::new_in(
                    target.parent().context("Missing transfer parent")?,
                )?;
                task.stream(&mut fs::File::open(&partial)?, &mut temporary, &source)?;
                if let Some(modified) = current.modified {
                    temporary
                        .as_file()
                        .set_times(fs::FileTimes::new().set_modified(modified))?;
                }
                if matches!(from, Location::Local(_)) {
                    temporary
                        .as_file()
                        .set_permissions(fs::symlink_metadata(&source)?.permissions())?;
                    crate::file_metadata::copy(&source, temporary.path())?;
                }
                temporary.as_file().sync_all()?;
                task.check()?;
                temporary.persist_noclobber(&target).map_err(|e| e.error)?;
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
        // Publish a progress boundary after commit, before the completion
        // receipt. Recovery also handles termination at precisely this point.
        task.tick(&target, 0)?;
        manifest.items[i].done = true;
        save(&path, &manifest)?;
        fs::remove_file(partial)?;
    }
    // Apply directory metadata after children, preserving a merged directory's
    // existing metadata and keeping restrictive source permissions until last.
    if matches!((from, to), (Location::Local(_), Location::Local(_))) {
        for item in manifest
            .items
            .iter()
            .rev()
            .filter(|i| i.directory && matches!(i.decision, Decision::New | Decision::Replace))
        {
            task.check()?;
            let target = target_root.join(filesystem::portable_relative(&item.relative)?);
            let metadata = fs::symlink_metadata(&item.source)?;
            ensure!(
                metadata.is_dir()
                    && !metadata.is_symlink()
                    && !fs::symlink_metadata(&target)?.is_symlink(),
                "Transfer directory changed"
            );
            if let Ok(modified) = metadata.modified() {
                filetime::set_file_mtime(&target, filetime::FileTime::from_system_time(modified))?;
            }
            fs::set_permissions(&target, metadata.permissions())?;
            crate::file_metadata::copy(&item.source, &target)?;
        }
    }
    let replacements = manifest
        .items
        .iter()
        .filter(|item| item.decision == Decision::Replace)
        .count();
    if replacements == 0 {
        fs::remove_dir_all(&directory)?;
    }
    Ok(format!(
        "Completed {} checkpointed items; recovered {resumed} partial/completed files. {}",
        manifest.items.len(),
        if replacements > 0 {
            format!(
                "Original destination items and their receipts are retained at {}",
                directory.display()
            )
        } else {
            String::new()
        }
    ))
}

/// Discard only local checkpoint data belonging to this exact durable recipe.
/// Remote hidden staging files remain available for explicit server-side review.
pub fn discard(id: &str, from: &Location, to: &Location) -> Result<()> {
    let root = crate::settings::path()?.with_file_name("transfers");
    discard_in(&root, id, from, to)
}

fn recovery_manifest(root: &Path, id: &str) -> Result<(fs::File, Manifest)> {
    filesystem::validate_name(id)?;
    ensure!(id.starts_with("batch-"), "Invalid checkpoint ID");
    let directory = root.join(id);
    ensure!(
        !fs::symlink_metadata(root)?.file_type().is_symlink()
            && !fs::symlink_metadata(&directory)?.file_type().is_symlink(),
        "Checkpoint storage is a link"
    );
    let lease = lease(root, id)?;
    let path = directory.join("manifest.json");
    ensure!(
        !fs::symlink_metadata(&path)?.file_type().is_symlink(),
        "Checkpoint manifest is a link"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 32 * 1024 * 1024,
        "Checkpoint manifest exceeds 32 MiB"
    );
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    ensure!(
        manifest.items.len() <= 100_000,
        "Checkpoint has too many entries"
    );
    Ok((lease, manifest))
}
pub(crate) fn lock_recovery_in(root: &Path, id: &str) -> Result<fs::File> {
    Ok(recovery_manifest(root, id)?.0)
}
pub(crate) fn recovery_items_in(
    root: &Path,
    id: &str,
    task: &Task,
) -> Result<Vec<crate::recovery::Item>> {
    let (_lease, manifest) = recovery_manifest(root, id)?;
    let directory = root.join(id);
    let complete = manifest
        .items
        .iter()
        .all(|item| item.done || item.decision == Decision::Skip);
    let mut result = Vec::new();
    for (index, item) in manifest.items.iter().enumerate() {
        let Some(expected) = &item.backup_hash else {
            continue;
        };
        let path = directory.join(format!("original-{index}"));
        let local = Location::Local(directory.clone());
        ensure!(
            conflicts::content(&local, &path, task)? == *expected,
            "Retained checkpoint original is corrupt: {}",
            path.display()
        );
        let target = crate::directory_tree::path(&manifest.to)
            .join(filesystem::portable_relative(&item.relative)?);
        result.push(crate::recovery::Item {
            key: crate::recovery::Key::Checkpoint {
                batch: id.into(),
                item: index,
            },
            label: format!("Checkpoint {id}"),
            destination: crate::directory_tree::at(
                &manifest.to,
                target.parent().context("Missing target parent")?,
            ),
            name: target
                .file_name()
                .context("Missing original name")?
                .to_str()
                .context("Recovery names must be UTF-8")?
                .into(),
            stamp: storage::stamp(&local, &path, task)?.context("Recovery original missing")?,
            source: local,
            path,
            cleanup: complete,
            cleanup_group: format!("checkpoint:{id}"),
            cleanup_scope: "Delete this completed checkpoint and all its retained originals".into(),
        });
    }
    Ok(result)
}
pub(crate) fn cleanup_recovery_in(root: &Path, id: &str, task: &Task) -> Result<()> {
    let (_lease, manifest) = recovery_manifest(root, id)?;
    ensure!(
        manifest
            .items
            .iter()
            .all(|item| item.done || item.decision == Decision::Skip),
        "An incomplete checkpoint cannot be cleaned up"
    );
    let directory = root.join(id);
    for (index, item) in manifest.items.iter().enumerate() {
        if let Some(expected) = &item.backup_hash {
            let path = directory.join(format!("original-{index}"));
            ensure!(
                conflicts::content(&Location::Local(directory.clone()), &path, task)? == *expected,
                "Checkpoint original changed; preserved for review"
            );
        }
    }
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().context("Unrecognized checkpoint data")?;
        let known = name == "manifest.json"
            || name
                .strip_prefix("original-")
                .and_then(|s| s.parse::<usize>().ok())
                .is_some_and(|i| {
                    manifest
                        .items
                        .get(i)
                        .is_some_and(|item| item.backup_hash.is_some())
                })
            || name
                .strip_suffix(".part")
                .and_then(|s| s.parse::<usize>().ok())
                .is_some_and(|i| i < manifest.items.len());
        ensure!(
            known && !entry.file_type()?.is_symlink(),
            "Checkpoint has unrecognized data; preserved for review"
        );
    }
    task.check()?;
    fs::remove_dir_all(directory)?;
    Ok(())
}
pub fn discard_in(root: &Path, id: &str, from: &Location, to: &Location) -> Result<()> {
    filesystem::validate_name(id)?;
    ensure!(id.starts_with("batch-"), "Invalid checkpoint ID");
    let directory = root.join(id);
    if !crate::operations::exists(&directory) {
        return Ok(());
    }
    ensure!(
        !fs::symlink_metadata(root)?.file_type().is_symlink()
            && !fs::symlink_metadata(&directory)?.file_type().is_symlink(),
        "Checkpoint storage is a link"
    );
    let _lease = lease(root, id)?;
    let path = directory.join("manifest.json");
    if !crate::operations::exists(&path) {
        ensure!(
            fs::read_dir(&directory)?.next().is_none(),
            "Checkpoint has unrecognized recovery data; review it before discarding"
        );
        fs::remove_dir(directory)?;
        return Ok(());
    }
    ensure!(
        !fs::symlink_metadata(&path)?.file_type().is_symlink(),
        "Checkpoint manifest is a link"
    );
    let manifest: Manifest = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        manifest.from == *from && manifest.to == *to,
        "Checkpoint belongs to another transfer"
    );
    ensure!(
        manifest.items.iter().all(|item| item.backup_hash.is_none()),
        "Replacement originals are retained at {}; review this recovery folder before deleting it",
        directory.display()
    );
    fs::remove_dir_all(directory)?;
    Ok(())
}
