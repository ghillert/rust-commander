//! Discover retained originals and restore copies without overwriting current files.
//!
//! [`inventory`] brings together backups from the [operation journal](crate::history),
//! [transfer checkpoints](crate::resumable), and registered replacements.
//! Select an [`Item`] and pass it to [`restore`] to copy its contents into a
//! writable local or remote directory. A name collision produces a numbered
//! sibling. The retained original remains available for another restore.
//!
//! [`cleanup`] permanently removes backup data. [`Item::cleanup_scope`] describes
//! what selecting that item would remove, which can include a whole Undo/Redo chain.
//!
//! # Examples
//!
//! Recover the version from before a journaled edit into the file's original directory.
//!
//! ```
//! use rust_commander::operations::{Operation, Task};
//! use rust_commander::{history, recovery};
//! use std::fs;
//!
//! let workspace = tempfile::tempdir()?;
//! let config = workspace.path().join("config");
//! let path = workspace.path().join("notes.txt");
//! fs::write(&path, "first draft")?;
//!
//! let edit = Operation::SaveText {
//!     path: path.clone(),
//!     original: fs::read(&path)?,
//!     text: "published".into(),
//! };
//! let mut task = Task::default();
//! let record = history::execute_in(edit, &mut task, &config.join("history"))?;
//! assert!(record.success, "{}", record.result);
//!
//! let inventory = recovery::inventory_in(&config, &task)?;
//! let item = inventory.items.iter()
//!     .find(|item| item.name == "notes.txt")
//!     .expect("the journal retained the first draft");
//! recovery::restore_in(&config, item, &item.destination, &mut task)?;
//!
//! assert_eq!(fs::read_to_string(&path)?, "published");
//! assert_eq!(fs::read_to_string(workspace.path().join("notes (1).txt"))?, "first draft");
//! assert_eq!(fs::read_to_string(&item.path)?, "first draft");
//! # Ok::<(), anyhow::Error>(())
//! ```

use crate::{
    filesystem::{self, Location},
    operations::{self, Task},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

/// Identifies the journal change, checkpoint entry, or registered backup for an [`Item`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Key {
    Journal { record: String, change: usize },
    Checkpoint { batch: String, item: usize },
    Registered(String),
}
/// A snapshot of a retained original. Restore and cleanup revalidate it before use.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub key: Key,
    pub label: String,
    /// Location holding the backup.
    pub source: Location,
    /// Path of the backup within [`source`](Self::source).
    pub path: PathBuf,
    /// Original parent directory, suggested as a restore destination.
    pub destination: Location,
    /// Original filename to try first when restoring.
    pub name: String,
    /// Fingerprint used to detect changes to the retained original.
    pub stamp: String,
    /// Whether the backup group was eligible for cleanup when inventoried.
    pub cleanup: bool,
    /// Human-readable description of the backups that cleanup would remove.
    pub cleanup_scope: String,
    /// Items sharing this value are cleaned together.
    pub cleanup_group: String,
}
/// Available originals and any warnings encountered while discovering them.
#[derive(Default)]
pub struct Inventory {
    pub items: Vec<Item>,
    /// Problems with individual backups or checkpoints that could be skipped.
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Registered {
    id: String,
    source: Location,
    path: PathBuf,
    target: PathBuf,
    stamp: String,
}
fn config() -> Result<PathBuf> {
    Ok(crate::settings::path()?
        .parent()
        .context("Missing settings folder")?
        .into())
}
fn registry_file(config: &Path) -> PathBuf {
    config.join("recovery").join("items.json")
}
fn registry_lock(config: &Path) -> Result<fs::File> {
    let root = config.join("recovery");
    fs::create_dir_all(&root)?;
    ensure!(
        !fs::symlink_metadata(&root)?.file_type().is_symlink(),
        "Recovery registry is a link"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    let path = root.join("registry.lock");
    if operations::exists(&path) {
        ensure!(
            !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "Recovery lock is a link"
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
    file.lock()?;
    Ok(file)
}
fn load_registry(config: &Path) -> Result<Vec<Registered>> {
    let path = registry_file(config);
    if !operations::exists(&path) {
        return Ok(Vec::new());
    }
    ensure!(
        !fs::symlink_metadata(&path)?.file_type().is_symlink(),
        "Recovery index is a link"
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "Recovery index exceeds 4 MiB"
    );
    let records: Vec<Registered> = serde_json::from_slice(&bytes)
        .context("Recovery index is invalid; its file was preserved")?;
    ensure!(
        records.len() <= 10_000,
        "Recovery index exceeds 10,000 items"
    );
    Ok(records)
}
fn save_registry(config: &Path, records: &[Registered]) -> Result<()> {
    let path = registry_file(config);
    let bytes = serde_json::to_vec_pretty(records)?;
    ensure!(
        bytes.len() <= 4 * 1024 * 1024 && records.len() <= 10_000,
        "Recovery index is full; review recovery storage first"
    );
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn owned_path(path: &Path) -> bool {
    path.components().any(|part| {
        part.as_os_str().to_str().is_some_and(|name| {
            name.starts_with(".rust-commander-replaced-")
                || name.starts_with(".rust-commander-edit-")
                || name.starts_with(".rust-commander-sync-")
                || name.starts_with(".rust-commander-archive-backup-")
        })
    })
}
fn registered_path(source: &Location, path: &Path) -> Result<PathBuf> {
    if matches!(source, Location::Local(_)) {
        let resolved = path
            .parent()
            .context("Missing recovery parent")?
            .canonicalize()?
            .join(path.file_name().context("Missing recovery name")?);
        ensure!(
            owned_path(&resolved),
            "Recovery path escapes its backup container"
        );
        Ok(resolved)
    } else {
        crate::remote::key(path, false)?;
        ensure!(owned_path(path), "Unrecognized remote recovery path");
        Ok(path.into())
    }
}
pub(crate) fn record_backup(
    source: &Location,
    path: &Path,
    target: &Path,
    task: &Task,
) -> Result<()> {
    if matches!(source, Location::Local(_))
        && task
            .private_roots
            .iter()
            .any(|root| root.as_os_str().is_empty() || path.starts_with(root))
    {
        return Ok(());
    }
    record_backup_in(&config()?, source, path, target, task)
}
/// Registers a retained copy at `path` under `config`, recording its fingerprint and
/// the original `target`. Registering the same source and path replaces the prior record.
///
/// # Errors
///
/// Returns an error if the recovery name is unrecognized, the copy cannot be
/// fingerprinted, or its registry cannot be locked or saved.
#[doc(hidden)]
pub fn record_backup_in(
    config: &Path,
    source: &Location,
    path: &Path,
    target: &Path,
    task: &Task,
) -> Result<()> {
    ensure!(owned_path(path), "Unrecognized recovery name");
    let path = registered_path(source, path)?;
    let stamp = crate::storage::stamp(source, &path, task)?.context("Recovery copy is missing")?;
    let _lock = registry_lock(config)?;
    let mut records = load_registry(config)?;
    records.retain(|record| record.source != *source || record.path != path);
    records.push(Registered {
        id: format!("backup-{:016x}", rand::random::<u64>()),
        source: source.clone(),
        path,
        target: target.into(),
        stamp,
    });
    save_registry(config, &records)
}
/// Lists retained originals under the application's configuration directory.
/// See [`inventory_in`] to choose the configuration directory explicitly.
pub fn inventory(task: &Task) -> Result<Inventory> {
    inventory_in(&config()?, task)
}
/// Import a pre-index recovery directory created by the local replacement engine.
pub fn import_folder(folder: &Path, task: &Task) -> Result<()> {
    import_folder_in(&config()?, folder, task)
}
/// Registers the entries of an older local replacement folder under `config`.
/// The retained files stay in the folder so the registry can find them there.
pub fn import_folder_in(config: &Path, folder: &Path, task: &Task) -> Result<()> {
    ensure!(
        folder
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".rust-commander-replaced-")),
        "Choose a replacement recovery folder"
    );
    ensure!(
        !fs::symlink_metadata(folder)?.file_type().is_symlink(),
        "Recovery folder is a link"
    );
    let folder = folder.canonicalize()?;
    let marker = folder.join("RECOVERY.txt");
    ensure!(
        !fs::symlink_metadata(&marker)?.file_type().is_symlink(),
        "Recovery marker is a link"
    );
    let mut marker_text = String::new();
    fs::File::open(marker)?
        .take(8193)
        .read_to_string(&mut marker_text)?;
    ensure!(marker_text.len() <= 8192 && marker_text.starts_with("Original destination items are kept under items/, with their original relative paths."), "Unrecognized replacement recovery folder");
    let items = folder.join("items");
    ensure!(
        !fs::symlink_metadata(&items)?.file_type().is_symlink() && items.is_dir(),
        "Recovery items folder is a link or missing"
    );
    let destination = folder.parent().context("Missing original destination")?;
    let mut pending = vec![(items.clone(), 0usize)];
    let mut count = 0usize;
    while let Some((path, depth)) = pending.pop() {
        task.check()?;
        count += 1;
        ensure!(
            depth < 64 && count <= 5000,
            "Recovery import exceeds the inspection limit"
        );
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            let children = fs::read_dir(&path)?.collect::<Result<Vec<_>, _>>()?;
            if !children.is_empty() {
                pending.extend(children.into_iter().map(|entry| (entry.path(), depth + 1)));
                continue;
            }
        }
        if path == items {
            continue;
        }
        let relative = path.strip_prefix(&items)?;
        record_backup_in(
            config,
            &Location::Local(destination.into()),
            &path,
            &destination.join(relative),
            task,
        )?;
    }
    Ok(())
}
/// Discovers originals using the `history`, `transfers`, and `recovery` directories
/// beneath `config`. Inspect [`Inventory::warnings`] alongside the returned items.
///
/// # Errors
///
/// Fails if the inventory cannot be read safely, exceeds its size limit, or
/// encounters cancellation. Some individual discovery failures are reported as warnings.
pub fn inventory_in(config: &Path, task: &Task) -> Result<Inventory> {
    let mut result = Inventory::default();
    let history = config.join("history");
    let records = crate::history::load_in(&history)?;
    let latest = records
        .iter()
        .rev()
        .find(|r| r.can_undo() || r.can_redo())
        .map(|r| r.logical_root.as_ref().unwrap_or(&r.id));
    for record in &records {
        if record.expired {
            continue;
        }
        filesystem::validate_name(&record.id)?;
        let group = record.logical_root.as_ref().unwrap_or(&record.id);
        let protected = Some(group) == latest
            || records
                .iter()
                .any(|r| r.logical_root.as_ref().unwrap_or(&r.id) == group && r.pinned);
        for (index, change) in record.changes.iter().enumerate() {
            task.check()?;
            let Some(path) = &change.backup else {
                continue;
            };
            let Some(stamp) = &change.backup_stamp else {
                continue;
            };
            let directory = history.join(&record.id);
            if !operations::exists(path) {
                continue;
            }
            if fs::symlink_metadata(&directory)?.file_type().is_symlink()
                || path.parent() != Some(directory.canonicalize()?.as_path())
                || path
                    .file_name()
                    .is_none_or(|n| n.to_str().is_none_or(|s| s.parse::<usize>().is_err()))
            {
                result
                    .warnings
                    .push(format!("Unrecognized journal backup in {}", record.id));
                continue;
            }
            let Some(name) = change.path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            result.items.push(Item {
                key: Key::Journal {
                    record: record.id.clone(),
                    change: index,
                },
                label: record.label.clone(),
                source: Location::Local(directory),
                path: path.clone(),
                destination: Location::Local(change.path.parent().unwrap().into()),
                name: name.into(),
                stamp: stamp.clone(),
                cleanup: !protected,
                cleanup_group: format!("journal:{group}"),
                cleanup_scope: "Expire all backups in this operation's Undo/Redo chain".into(),
            });
        }
    }
    let transfers = config.join("transfers");
    if transfers.is_dir() {
        ensure!(
            !fs::symlink_metadata(&transfers)?.file_type().is_symlink(),
            "Checkpoint storage is a link"
        );
        for entry in fs::read_dir(&transfers)?.take(10_001) {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && let Some(id) = entry
                    .file_name()
                    .to_str()
                    .filter(|s| s.starts_with("batch-"))
            {
                match crate::resumable::recovery_items_in(&transfers, id, task) {
                    Ok(items) => result.items.extend(items),
                    Err(error) => result.warnings.push(format!("{id}: {error:#}")),
                }
            }
        }
    }
    let _lock = registry_lock(config)?;
    for record in load_registry(config)? {
        task.check()?;
        ensure!(
            owned_path(&record.path),
            "Unrecognized path in recovery index"
        );
        if matches!(record.source, Location::Local(_)) && !operations::exists(&record.path) {
            continue;
        }
        ensure!(
            registered_path(&record.source, &record.path)? == record.path,
            "Recovery path changed through a filesystem alias"
        );
        let Some(name) = record.target.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        result.items.push(Item {
            key: Key::Registered(record.id.clone()),
            label: "Retained replacement original".into(),
            destination: crate::directory_tree::at(
                &record.source,
                record.target.parent().context("Missing original folder")?,
            ),
            source: record.source,
            path: record.path,
            name: name.into(),
            stamp: record.stamp,
            cleanup: true,
            cleanup_group: format!("registered:{}", record.id),
            cleanup_scope: "Delete this retained original; current destination files stay in place"
                .into(),
        });
    }
    ensure!(
        result.items.len() <= 10_000,
        "Recovery inventory exceeds 10,000 items"
    );
    Ok(result)
}
fn verify(config: &Path, item: &Item, task: &Task) -> Result<Item> {
    let fresh = inventory_in(config, task)?
        .items
        .into_iter()
        .find(|fresh| fresh.key == item.key)
        .context("Recovery item is no longer available")?;
    ensure!(
        fresh.source == item.source
            && fresh.path == item.path
            && fresh.stamp == item.stamp
            && fresh.name == item.name,
        "Recovery item changed; refresh the list"
    );
    ensure!(
        crate::storage::stamp(&fresh.source, &fresh.path, task)?.as_ref() == Some(&fresh.stamp),
        "Recovery copy changed; it was preserved for review"
    );
    Ok(fresh)
}
/// Restores a verified copy into `destination` using the application configuration.
/// See [`restore_in`] for naming and error behavior.
pub fn restore(item: &Item, destination: &Location, task: &mut Task) -> Result<String> {
    restore_in(&config()?, item, destination, task)
}
/// Restores 1 to 100 selected originals using the application configuration.
/// See [`restore_many_in`] for partial completion behavior.
pub fn restore_many(items: &[Item], destination: &Location, task: &mut Task) -> Result<String> {
    restore_many_in(&config()?, items, destination, task)
}
/// Verifies all selected originals before restoring copies in selection order.
///
/// # Errors
///
/// Rejects selections outside the range of 1 to 100 items and stale originals.
/// If a later copy fails, the error reports how many copies were restored.
pub fn restore_many_in(
    config: &Path,
    items: &[Item],
    destination: &Location,
    task: &mut Task,
) -> Result<String> {
    ensure!(
        !items.is_empty() && items.len() <= 100,
        "Select between 1 and 100 originals"
    );
    for item in items {
        verify(config, item, task)?;
    }
    let mut messages = Vec::new();
    for item in items {
        let message = restore_in(config, item, destination, task).with_context(|| {
            format!(
                "{} copies restored before this error; recovery originals were retained",
                messages.len()
            )
        })?;
        messages.push(message);
    }
    Ok(format!(
        "Restored {} recovery copies.\n{}",
        items.len(),
        messages.join("\n")
    ))
}
/// Copies a retained original from the inventory under `config` into `destination`.
///
/// Uses [`Item::name`] when available, or an unused numbered sibling such as
/// `notes (1).txt`. Checks the backup fingerprint before and after staging its
/// contents and returns a display summary on success.
///
/// # Errors
///
/// Fails if the original disappeared or changed, the destination is unsuitable,
/// or staging or transferring the copy fails. A writable local or remote
/// directory outside the retained original is required.
pub fn restore_in(
    config: &Path,
    item: &Item,
    destination: &Location,
    task: &mut Task,
) -> Result<String> {
    let item = verify(config, item, task)?;
    let _checkpoint_lease = match &item.key {
        Key::Checkpoint { batch, .. } => Some(crate::resumable::lock_recovery_in(
            &config.join("transfers"),
            batch,
        )?),
        _ => None,
    };
    let _registry_lease = if matches!(item.key, Key::Registered(_)) {
        Some(registry_lock(config)?)
    } else {
        None
    };
    ensure!(
        crate::storage::stamp(&item.source, &item.path, task)?.as_ref() == Some(&item.stamp),
        "Recovery copy changed before restore"
    );
    filesystem::validate_name(&item.name)?;
    ensure!(
        crate::drag_drop::writable(destination) && !matches!(destination, Location::Archive { .. }),
        "Choose a writable local or remote folder"
    );
    let same = matches!(
        (&item.source, destination),
        (Location::Local(_), Location::Local(_))
    ) || matches!((&item.source, destination), (Location::Remote {connection:a,..}, Location::Remote {connection:b,..}) if a == b);
    if same {
        let destination_path = crate::directory_tree::path(destination);
        let (source, destination) = if matches!(destination, Location::Local(_)) {
            (item.path.canonicalize()?, destination_path.canonicalize()?)
        } else {
            (item.path.clone(), destination_path.into())
        };
        ensure!(
            !destination.starts_with(source),
            "Do not restore into the recovery original itself"
        );
    }
    let stage = tempfile::tempdir()?;
    let path = stage.path().join(&item.name);
    let entry =
        crate::remote::entry_at(&item.source, &item.path)?.context("Recovery copy disappeared")?;
    crate::drag_drop::transient(task, stage.path(), |task| match item.source {
        Location::Local(_) => operations::copy_item(&entry.path, &path, task),
        _ => crate::remote::copy_entry(
            &item.source,
            &entry,
            &Location::Local(stage.path().into()),
            &path,
            task,
        ),
    })?;
    ensure!(
        crate::storage::stamp(&item.source, &item.path, task)?.as_ref() == Some(&item.stamp),
        "Recovery copy changed while reading; destination was not changed"
    );
    if matches!(destination, Location::Remote { .. }) {
        task.undo_blocked = true;
    }
    let message = crate::drag_drop::transient(task, stage.path(), |task| {
        crate::remote::transfer_resolved(
            &Location::Local(stage.path().into()),
            &[filesystem::entry_from_path(&path)?],
            destination,
            false,
            crate::transfer::TransferOptions {
                conflicts: crate::transfer::ConflictPolicy::Rename,
                merge_folders: false,
            },
            &Default::default(),
            task,
        )
    })?;
    Ok(format!(
        "Restored a copy of {}. Existing files and the recovery original were retained.\n{message}",
        item.name
    ))
}
/// Permanently cleans the group described by [`Item::cleanup_scope`] using the
/// application configuration. See [`cleanup_in`] for eligibility rules.
pub fn cleanup(item: &Item, task: &mut Task) -> Result<String> {
    cleanup_in(&config()?, item, task)
}
/// Cleans the distinct backup groups for 1 to 100 items using the application configuration.
pub fn cleanup_many(items: &[Item], task: &mut Task) -> Result<String> {
    cleanup_many_in(&config()?, items, task)
}
/// Checks the selected items for eligibility and permanently cleans each distinct group once.
///
/// # Errors
///
/// Rejects invalid selections, stale backups, and protected groups. An error
/// during cleanup reports how many groups were already cleaned.
pub fn cleanup_many_in(config: &Path, items: &[Item], task: &mut Task) -> Result<String> {
    ensure!(
        !items.is_empty() && items.len() <= 100,
        "Select between 1 and 100 originals"
    );
    let mut groups = std::collections::BTreeSet::new();
    let mut selected = Vec::new();
    for item in items {
        let fresh = verify(config, item, task)?;
        ensure!(
            fresh.cleanup,
            "Pinned and latest reversible recovery chains cannot be removed"
        );
        if groups.insert(fresh.cleanup_group) {
            selected.push(item);
        }
    }
    for (index, item) in selected.iter().enumerate() {
        cleanup_in(config, item, task).with_context(|| {
            format!(
                "{index} backup groups cleaned before this error; remaining backups were preserved"
            )
        })?;
    }
    Ok(format!(
        "Cleaned {} reviewed backup groups. Current destination files were preserved.",
        selected.len()
    ))
}
/// Permanently removes a registered backup, a completed checkpoint with its
/// originals, or all backups in a journaled Undo/Redo chain under `config`.
///
/// # Errors
///
/// Refuses stale originals, incomplete checkpoints, pinned journal chains, and
/// the latest reversible chain. Also propagates cancellation and storage errors.
pub fn cleanup_in(config: &Path, item: &Item, task: &mut Task) -> Result<String> {
    let item = verify(config, item, task)?;
    ensure!(
        item.cleanup,
        "Pinned and latest reversible recovery chains cannot be removed"
    );
    task.check()?;
    task.undo_blocked = true;
    match &item.key {
        Key::Journal { record, .. } => {
            let root = config.join("history");
            let records = crate::history::load_in(&root)?;
            let chosen = records
                .iter()
                .find(|r| &r.id == record)
                .context("Journal record missing")?;
            let group = chosen.logical_root.as_ref().unwrap_or(&chosen.id);
            let ids = records
                .iter()
                .filter(|r| r.logical_root.as_ref().unwrap_or(&r.id) == group)
                .map(|r| r.id.clone())
                .collect::<Vec<_>>();
            crate::history::cleanup_in(&root, &ids)?;
        }
        Key::Checkpoint { batch, .. } => {
            crate::resumable::cleanup_recovery_in(&config.join("transfers"), batch, task)?
        }
        Key::Registered(id) => {
            let _lock = registry_lock(config)?;
            let mut records = load_registry(config)?;
            ensure!(
                records
                    .iter()
                    .any(|r| &r.id == id && r.path == item.path && r.stamp == item.stamp),
                "Recovery index changed"
            );
            let entry = crate::remote::entry_at(&item.source, &item.path)?
                .context("Recovery copy missing")?;
            match item.source {
                Location::Local(_) => operations::remove(&entry.path)?,
                _ => crate::remote::remove_entry(&item.source, &entry, task)?,
            }
            records.retain(|record| &record.id != id);
            save_registry(config, &records)?;
        }
    }
    Ok("Removed the reviewed recovery backups. Current destination files were preserved.".into())
}
