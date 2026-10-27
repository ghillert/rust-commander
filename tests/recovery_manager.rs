use rust_commander::{
    filesystem::{self, Location},
    history,
    operations::{Operation, Task},
    recovery, resumable,
    transfer::{ConflictPolicy, TransferOptions},
};
use std::fs;

#[test]
fn multi_file_recovery_preflights_all_items_and_cleans_each_group_once() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let target = root.path().join("file.txt");
    fs::write(&target, b"current").unwrap();
    let location = Location::Local(root.path().into());
    let backups = [
        root.path().join(".rust-commander-edit-first.bak"),
        root.path().join(".rust-commander-edit-second.bak"),
    ];
    for (index, path) in backups.iter().enumerate() {
        fs::write(path, format!("original {index}")).unwrap();
        recovery::record_backup_in(&config, &location, path, &target, &Task::default()).unwrap();
    }
    let items = recovery::inventory_in(&config, &Task::default())
        .unwrap()
        .items;
    fs::write(&backups[1], b"changed").unwrap();
    assert!(recovery::restore_many_in(&config, &items, &location, &mut Task::default()).is_err());
    assert!(!root.path().join("file (1).txt").exists());
    fs::write(&backups[1], b"original 1").unwrap();
    recovery::record_backup_in(&config, &location, &backups[1], &target, &Task::default()).unwrap();
    let mut items = recovery::inventory_in(&config, &Task::default())
        .unwrap()
        .items;
    recovery::restore_many_in(&config, &items, &location, &mut Task::default()).unwrap();
    assert_eq!(
        fs::read(root.path().join("file (1).txt")).unwrap(),
        b"original 0"
    );
    assert_eq!(
        fs::read(root.path().join("file (2).txt")).unwrap(),
        b"original 1"
    );
    items.push(items[0].clone());
    recovery::cleanup_many_in(&config, &items, &mut Task::default()).unwrap();
    assert!(backups.iter().all(|path| !path.exists()));
    assert_eq!(fs::read(target).unwrap(), b"current");
}

#[test]
fn registered_restore_keeps_current_files_and_cleanup_removes_only_reviewed_backup() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let destination = root.path().join("destination");
    fs::create_dir_all(&destination).unwrap();
    let original = destination.join("report.txt");
    fs::write(&original, b"current").unwrap();
    let backup = destination.join(".rust-commander-edit-fixture.bak");
    fs::write(&backup, b"retained original").unwrap();
    recovery::record_backup_in(
        &config,
        &Location::Local(destination.clone()),
        &backup,
        &original,
        &Task::default(),
    )
    .unwrap();
    let item = recovery::inventory_in(&config, &Task::default())
        .unwrap()
        .items
        .remove(0);
    recovery::restore_in(&config, &item, &item.destination, &mut Task::default()).unwrap();
    assert_eq!(fs::read(&original).unwrap(), b"current");
    assert_eq!(
        fs::read(destination.join("report (1).txt")).unwrap(),
        b"retained original"
    );
    assert_eq!(fs::read(&backup).unwrap(), b"retained original");
    recovery::cleanup_in(&config, &item, &mut Task::default()).unwrap();
    assert!(!backup.exists());
    assert_eq!(fs::read(&original).unwrap(), b"current");
    assert!(
        recovery::inventory_in(&config, &Task::default())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn changed_and_forged_recovery_items_cannot_restore_or_delete_files() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let backup = root.path().join(".rust-commander-edit-fixture.bak");
    let live = root.path().join("live.txt");
    fs::write(&backup, b"original").unwrap();
    fs::write(&live, b"live file").unwrap();
    recovery::record_backup_in(
        &config,
        &Location::Local(root.path().into()),
        &backup,
        &live,
        &Task::default(),
    )
    .unwrap();
    let item = recovery::inventory_in(&config, &Task::default())
        .unwrap()
        .items
        .remove(0);
    let mut forged = item.clone();
    forged.path = live.clone();
    assert!(recovery::cleanup_in(&config, &forged, &mut Task::default()).is_err());
    fs::write(&backup, b"externally changed").unwrap();
    assert!(recovery::restore_in(&config, &item, &item.destination, &mut Task::default()).is_err());
    assert!(recovery::cleanup_in(&config, &item, &mut Task::default()).is_err());
    assert_eq!(fs::read(&live).unwrap(), b"live file");
    assert_eq!(fs::read(&backup).unwrap(), b"externally changed");
    assert!(
        recovery::record_backup_in(
            &config,
            &Location::Local(root.path().into()),
            &live,
            &live,
            &Task::default()
        )
        .is_err()
    );
}

#[test]
fn journal_browser_restores_real_names_and_protects_latest_and_pinned_chains() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let history_root = config.join("history");
    let file = root.path().join("first.txt");
    let second = root.path().join("second.txt");
    fs::write(&file, b"before").unwrap();
    fs::write(&second, b"second before").unwrap();
    history::execute_in(
        Operation::SaveBytes {
            path: file.clone(),
            original: b"before".to_vec(),
            bytes: b"after".to_vec(),
        },
        &mut Task::default(),
        &history_root,
    )
    .unwrap();
    let latest_only = recovery::inventory_in(&config, &Task::default())
        .unwrap()
        .items;
    assert!(!latest_only[0].cleanup);
    history::execute_in(
        Operation::SaveBytes {
            path: second,
            original: b"second before".to_vec(),
            bytes: b"second after".to_vec(),
        },
        &mut Task::default(),
        &history_root,
    )
    .unwrap();
    let item = recovery::inventory_in(&config, &Task::default())
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.name == "first.txt")
        .unwrap();
    assert!(item.cleanup);
    recovery::restore_in(
        &config,
        &item,
        &Location::Local(root.path().into()),
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read(root.path().join("first (1).txt")).unwrap(),
        b"before"
    );
    let recovery::Key::Journal {
        record: record_id, ..
    } = &item.key
    else {
        panic!("Expected a journal backup")
    };
    let mut records = history::load_in(&history_root).unwrap();
    records
        .iter_mut()
        .find(|r| &r.id == record_id)
        .unwrap()
        .pinned = true;
    history::save_in(&records, &history_root).unwrap();
    assert!(recovery::cleanup_in(&config, &item, &mut Task::default()).is_err());
    records
        .iter_mut()
        .find(|r| &r.id == record_id)
        .unwrap()
        .pinned = false;
    history::save_in(&records, &history_root).unwrap();
    recovery::cleanup_in(&config, &item, &mut Task::default()).unwrap();
    assert!(
        history::load_in(&history_root)
            .unwrap()
            .iter()
            .find(|r| &r.id == record_id)
            .unwrap()
            .expired
    );
    assert_eq!(fs::read(file).unwrap(), b"after");
}

#[test]
fn checkpoint_originals_are_restorable_and_only_complete_verified_batches_can_be_cleaned() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(source.join("file"), b"new").unwrap();
    fs::write(target.join("file"), b"old").unwrap();
    let from = Location::Local(source);
    let to = Location::Local(target.clone());
    resumable::run_resolved_in(
        &config.join("transfers"),
        "batch-recovery-test",
        &from,
        &filesystem::list(&from).unwrap(),
        &to,
        TransferOptions {
            conflicts: ConflictPolicy::Replace,
            merge_folders: true,
        },
        &Default::default(),
        &mut Task::default(),
    )
    .unwrap();
    let item = recovery::inventory_in(&config, &Task::default())
        .unwrap()
        .items
        .remove(0);
    assert!(item.cleanup);
    recovery::restore_in(&config, &item, &to, &mut Task::default()).unwrap();
    assert_eq!(fs::read(target.join("file (1)")).unwrap(), b"old");
    let manifest_path = config.join("transfers/batch-recovery-test/manifest.json");
    let original_manifest = fs::read(&manifest_path).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&original_manifest).unwrap();
    manifest["items"][0]["done"] = false.into();
    fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(recovery::cleanup_in(&config, &item, &mut Task::default()).is_err());
    fs::write(&manifest_path, &original_manifest).unwrap();
    let extra = config.join("transfers/batch-recovery-test/unknown");
    fs::write(&extra, b"must survive").unwrap();
    assert!(recovery::cleanup_in(&config, &item, &mut Task::default()).is_err());
    fs::remove_file(extra).unwrap();
    recovery::cleanup_in(&config, &item, &mut Task::default()).unwrap();
    assert!(!manifest_path.exists());
    assert_eq!(fs::read(target.join("file")).unwrap(), b"new");
}

#[test]
fn legacy_recovery_import_preserves_nested_target_names_and_rejects_unowned_folders() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let recovery_folder = root.path().join(".rust-commander-replaced-old");
    fs::create_dir_all(recovery_folder.join("items/tree")).unwrap();
    fs::write(recovery_folder.join("RECOVERY.txt"), "Original destination items are kept under items/, with their original relative paths.\nDestination: ignored display text\n").unwrap();
    fs::write(recovery_folder.join("items/tree/café.txt"), b"retained").unwrap();
    recovery::import_folder_in(&config, &recovery_folder, &Task::default()).unwrap();
    let item = recovery::inventory_in(&config, &Task::default())
        .unwrap()
        .items
        .remove(0);
    assert_eq!(item.name, "café.txt");
    assert_eq!(
        item.destination,
        Location::Local(root.path().canonicalize().unwrap().join("tree"))
    );
    assert!(recovery::import_folder_in(&config, root.path(), &Task::default()).is_err());
    fs::create_dir(root.path().join("tree")).unwrap();
    recovery::restore_in(&config, &item, &item.destination, &mut Task::default()).unwrap();
    assert_eq!(
        fs::read(root.path().join("tree/café.txt")).unwrap(),
        b"retained"
    );
    assert_eq!(
        fs::read(recovery_folder.join("items/tree/café.txt")).unwrap(),
        b"retained"
    );
}

#[cfg(unix)]
#[test]
fn recovery_paths_cannot_escape_through_directory_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("live"), b"must survive").unwrap();
    let owned = root.path().join(".rust-commander-replaced-links");
    fs::create_dir(&owned).unwrap();
    std::os::unix::fs::symlink(&outside, owned.join("items")).unwrap();
    assert!(
        recovery::record_backup_in(
            &config,
            &Location::Local(root.path().into()),
            &owned.join("items/live"),
            &root.path().join("live"),
            &Task::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(outside.join("live")).unwrap(), b"must survive");
}
