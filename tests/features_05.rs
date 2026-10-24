use rust_commander::{
    archive,
    archive_tools::{self, ZipEdit},
    filesystem::{self, Location},
    history,
    operations::{Operation, Task},
    remote::{self, Connection, Secrets},
    settings::Settings,
    sync::{self, Choice},
    workspaces::Workspace,
};
use std::{fs, path::Path};
fn run(op: Operation, journal: &Path) -> history::Record {
    let record = history::execute_in(op, &mut Task::default(), journal).unwrap();
    assert!(record.success, "{}", record.result);
    record
}
#[test]
fn text_save_undo_redo_survives_history_reload() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("file");
    let journal = d.path().join("journal");
    fs::write(&file, "before").unwrap();
    let original = run(
        Operation::SaveText {
            path: file.clone(),
            original: b"before".to_vec(),
            text: "after".into(),
        },
        &journal,
    );
    history::save_in(std::slice::from_ref(&original), &journal).unwrap();
    let undone = run(Operation::Undo(Box::new(original.clone())), &journal);
    assert_eq!(fs::read(&file).unwrap(), b"before");
    assert!(undone.can_redo());
    assert!(
        history::load_in(&journal)
            .unwrap()
            .iter()
            .find(|r| r.id == original.id)
            .unwrap()
            .undone
    );
    let redone = run(Operation::Redo(Box::new(undone)), &journal);
    assert_eq!(fs::read(&file).unwrap(), b"after");
    assert!(redone.can_undo());
    assert!(
        !history::load_in(&journal)
            .unwrap()
            .iter()
            .find(|r| r.id == original.id)
            .unwrap()
            .undone
    );
    run(Operation::Undo(Box::new(redone)), &journal);
    assert_eq!(fs::read(&file).unwrap(), b"before");
}
#[test]
fn new_change_invalidates_redo() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a");
    let journal = d.path().join("history");
    fs::write(&p, "a").unwrap();
    let r = run(
        Operation::SaveText {
            path: p.clone(),
            original: b"a".to_vec(),
            text: "b".into(),
        },
        &journal,
    );
    run(Operation::Undo(Box::new(r)), &journal);
    run(Operation::Mkfile(d.path().join("new")), &journal);
    assert!(
        !history::load_in(&journal)
            .unwrap()
            .iter()
            .any(|r| r.can_redo())
    );
}
#[test]
fn redo_refuses_external_edits() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a");
    let journal = d.path().join("history");
    fs::write(&p, "a").unwrap();
    let r = run(
        Operation::SaveText {
            path: p.clone(),
            original: b"a".to_vec(),
            text: "b".into(),
        },
        &journal,
    );
    let undo = run(Operation::Undo(Box::new(r)), &journal);
    fs::write(&p, "external").unwrap();
    let rejected = history::execute_in(
        Operation::Redo(Box::new(undo)),
        &mut Task::default(),
        &journal,
    )
    .unwrap();
    assert!(!rejected.success);
    assert_eq!(fs::read(&p).unwrap(), b"external");
}
#[test]
fn timestamp_changes_are_reversible() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a");
    let journal = d.path().join("history");
    fs::write(&p, "contents").unwrap();
    let before = fs::metadata(&p).unwrap().modified().unwrap();
    let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let r = run(
        Operation::Modified {
            paths: vec![p.clone()],
            time: modified,
        },
        &journal,
    );
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), modified);
    assert_eq!(fs::read(&p).unwrap(), b"contents");
    let undone = run(Operation::Undo(Box::new(r)), &journal);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), before);
    run(Operation::Redo(Box::new(undone)), &journal);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), modified);
    assert_eq!(fs::read(&p).unwrap(), b"contents");
}
#[test]
fn directory_timestamp_changes_are_reversible_without_changing_children() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("folder");
    let journal = d.path().join("history");
    fs::create_dir(&p).unwrap();
    let child = p.join("file");
    fs::write(&child, "contents").unwrap();
    let before = fs::metadata(&p).unwrap().modified().unwrap();
    let child_before = fs::metadata(&child).unwrap().modified().unwrap();
    let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let r = run(
        Operation::Modified {
            paths: vec![p.clone()],
            time: modified,
        },
        &journal,
    );
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), modified);
    let undone = run(Operation::Undo(Box::new(r)), &journal);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), before);
    run(Operation::Redo(Box::new(undone)), &journal);
    assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), modified);
    assert_eq!(
        fs::metadata(&child).unwrap().modified().unwrap(),
        child_before
    );
    assert_eq!(fs::read(child).unwrap(), b"contents");
}
#[test]
fn timestamp_changes_preserve_read_only_files() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("read-only");
    fs::write(&p, "contents").unwrap();
    let original = fs::metadata(&p).unwrap().permissions();
    let mut permissions = original.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&p, permissions).unwrap();
    let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let result = rust_commander::desktop::change_modified(std::slice::from_ref(&p), modified);
    let after = fs::metadata(&p).unwrap();
    // Restore permissions before assertions so Windows can clean up the temporary directory.
    fs::set_permissions(&p, original).unwrap();
    result.unwrap();
    assert_eq!(after.modified().unwrap(), modified);
    assert!(after.permissions().readonly());
    assert_eq!(fs::read(p).unwrap(), b"contents");
}
#[cfg(unix)]
#[test]
fn timestamp_changes_refuse_symlinks_without_changing_the_target() {
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("target");
    let link = d.path().join("link");
    fs::write(&target, "contents").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let before = fs::metadata(&target).unwrap().modified().unwrap();
    let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    assert!(rust_commander::desktop::change_modified(&[link], modified).is_err());
    assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), before);
    assert_eq!(fs::read(target).unwrap(), b"contents");
}
#[cfg(unix)]
#[test]
fn permission_changes_are_reversible() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a");
    fs::write(&p, "contents").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    let r = run(
        Operation::Permissions {
            paths: vec![p.clone()],
            mode: 0o600,
        },
        &d.path().join("history"),
    );
    run(Operation::Undo(Box::new(r)), &d.path().join("history"));
    assert_eq!(fs::metadata(p).unwrap().permissions().mode() & 0o777, 0o644);
}
#[test]
fn recovery_cleanup_protects_pinned_and_latest_and_expires_before_removal() {
    let d = tempfile::tempdir().unwrap();
    let journal = d.path().join("journal");
    let mut records = Vec::new();
    for n in 0..3 {
        let path = d.path().join(format!("f{n}"));
        fs::write(&path, "old").unwrap();
        records.push(run(
            Operation::SaveText {
                path,
                original: b"old".to_vec(),
                text: "new".into(),
            },
            &journal,
        ));
    }
    records[0].pinned = true;
    history::save_in(&records, &journal).unwrap();
    let ids = history::cleanup_candidates(
        &records,
        &journal,
        &history::Retention {
            max_mib: 0,
            max_days: 0,
            automatic: false,
        },
        u64::MAX,
    )
    .unwrap();
    assert_eq!(ids, vec![records[1].id.clone()]);
    let backup = records[1].changes[0].backup.clone().unwrap();
    let records = history::cleanup_in(&journal, &ids).unwrap();
    assert!(!backup.exists());
    let expired = records.iter().find(|r| ids.contains(&r.id)).unwrap();
    assert!(expired.expired);
    assert!(!expired.can_undo());
    assert!(
        !records
            .iter()
            .find(|r| r.pinned)
            .unwrap()
            .changes
            .is_empty()
    );
}
#[test]
fn synchronization_applies_reviewed_conflict_and_is_undoable() {
    let d = tempfile::tempdir().unwrap();
    let l = d.path().join("left");
    let r = d.path().join("right");
    fs::create_dir(&l).unwrap();
    fs::create_dir(&r).unwrap();
    fs::write(l.join("a"), "new").unwrap();
    fs::write(r.join("a"), "old").unwrap();
    fs::write(r.join("extra"), "keep").unwrap();
    let mut plan = sync::preview(&l, &r, 1, &Task::default()).unwrap();
    plan.items
        .iter_mut()
        .find(|i| i.path == Path::new("a"))
        .unwrap()
        .choice = Choice::LeftToRight;
    let record = run(Operation::Sync(plan), &d.path().join("journal"));
    assert_eq!(fs::read(r.join("a")).unwrap(), b"new");
    assert!(r.join("extra").exists());
    run(Operation::Undo(Box::new(record)), &d.path().join("journal"));
    assert_eq!(fs::read(r.join("a")).unwrap(), b"old");
}
#[test]
fn tar_and_sevenz_support_add_rename_remove_and_undo() {
    for ext in ["tar", "tar.gz", "tar.bz2", "tar.xz", "7z"] {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        fs::write(&a, "original").unwrap();
        fs::write(&b, "added").unwrap();
        let file = d.path().join(format!("archive.{ext}"));
        archive::pack(&[a], &file, &mut Task::default()).unwrap();
        let journal = d.path().join("journal");
        run(
            Operation::ArchiveEdit {
                archive: file.clone(),
                edit: ZipEdit::Add {
                    sources: vec![b],
                    prefix: "folder".into(),
                },
            },
            &journal,
        );
        assert_eq!(
            archive::read_bytes(&file, Path::new("folder/b"), 100).unwrap(),
            b"added"
        );
        run(
            Operation::ArchiveEdit {
                archive: file.clone(),
                edit: ZipEdit::Rename {
                    source: "a".into(),
                    target: "renamed".into(),
                },
            },
            &journal,
        );
        let remove = run(
            Operation::ArchiveEdit {
                archive: file.clone(),
                edit: ZipEdit::Remove(vec!["renamed".into()]),
            },
            &journal,
        );
        assert!(archive::read_bytes(&file, Path::new("renamed"), 100).is_err());
        run(Operation::Undo(Box::new(remove)), &journal);
        assert_eq!(
            archive::read_bytes(&file, Path::new("renamed"), 100).unwrap(),
            b"original"
        );
    }
}
#[test]
fn encrypted_zip_add_preserves_encryption_of_every_file() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    let b = d.path().join("b");
    let file = d.path().join("encrypted.zip");
    fs::write(&a, "secret one").unwrap();
    fs::write(&b, "secret two").unwrap();
    archive::pack_with_password(&[a], &file, "test-password", &mut Task::default()).unwrap();
    archive::unlock(&file, "test-password").unwrap();
    archive_tools::edit_archive(
        &file,
        &ZipEdit::Add {
            sources: vec![b],
            prefix: Default::default(),
        },
        &mut Task::default(),
    )
    .unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(&file).unwrap()).unwrap();
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).unwrap();
        if !entry.is_dir() {
            assert!(entry.encrypted());
        }
    }
    archive::unlock(&file, "test-password").unwrap();
    assert_eq!(
        archive::read_bytes(&file, Path::new("b"), 100).unwrap(),
        b"secret two"
    );
    assert!(archive::unlock(&file, "wrong").is_err());
}
#[test]
fn remote_search_and_guarded_save_use_actual_storage_operations() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("nested")).unwrap();
    fs::write(d.path().join("nested/a.txt"), "find this").unwrap();
    fs::write(d.path().join("nested/b.bin"), "other").unwrap();
    let c = Connection {
        id: format!("feature05-{}", rand::random::<u64>()),
        name: "Test FS".into(),
        service: "fs".into(),
        root: d.path().display().to_string(),
        ..Default::default()
    };
    remote::connect(&c, Secrets::default(), false).unwrap();
    let options = rust_commander::desktop::SearchOptions {
        root: Default::default(),
        pattern: "*.txt".into(),
        content: "find".into(),
        hidden: false,
        max_results: 100,
        filters: Default::default(),
    };
    let result = remote::search(&c.id, &options, &mut Task::default()).unwrap();
    assert_eq!(result.paths, vec![Path::new("nested/a.txt").to_owned()]);
    remote::save_text(
        &c.id,
        Path::new("nested/a.txt"),
        b"find this",
        "edited",
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read(d.path().join("nested/a.txt")).unwrap(), b"edited");
    assert!(
        fs::read_dir(d.path().join("nested"))
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with(".bak"))
    );
    fs::write(d.path().join("nested/a.txt"), "external").unwrap();
    assert!(
        remote::save_text(
            &c.id,
            Path::new("nested/a.txt"),
            b"edited",
            "overwrite",
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(
        fs::read(d.path().join("nested/a.txt")).unwrap(),
        b"external"
    );
}
#[test]
fn workspace_round_trip_preserves_tabs_and_layout() {
    let mut settings = Settings::default();
    let preferences = rust_commander::preferences::Preferences {
        single_pane: true,
        preview: true,
        ..Default::default()
    };
    settings.workspaces.push(Workspace {
        tab_states: Default::default(),
        name: "Development".into(),
        tabs: [
            vec![
                Location::Local("/first".into()),
                Location::Remote {
                    connection: "profile-id".into(),
                    path: "repo".into(),
                },
            ],
            vec![Location::Local("/other".into())],
        ],
        active_tabs: [1, 0],
        active_pane: 1,
        preferences,
    });
    let json = serde_json::to_string(&settings).unwrap();
    let read: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(read.workspaces[0].tabs[0].len(), 2);
    assert_eq!(read.workspaces[0].active_tabs, [1, 0]);
    assert!(read.workspaces[0].preferences.preview);
    assert!(read.workspaces[0].preferences.single_pane);
    assert!(Workspace::validate_name(" \n ").is_err());
    assert_eq!(Workspace::validate_name(" Work ").unwrap(), "Work");
}
#[test]
fn editor_queue_recipes_exclude_original_and_unsaved_contents() {
    let op = Operation::RemoteSave {
        connection: "profile".into(),
        path: "file.txt".into(),
        original: b"private original contents".to_vec(),
        text: "private unsaved draft".into(),
    };
    let json = serde_json::to_string(&op).unwrap();
    assert!(!json.contains("private"));
    let restored: Operation = serde_json::from_str(&json).unwrap();
    assert!(rust_commander::queue::Queue::requires_input(&restored));
}
#[test]
#[ignore = "requires disposable loopback Samba server; see docs/FEATURES_0.5.md"]
fn smb_live_list_stream_upload_download_rename_edit_search_delete() {
    let d = tempfile::tempdir().unwrap();
    let c = Connection {
        id: format!("smb-test-{}", rand::random::<u64>()),
        name: "Disposable Samba".into(),
        service: "smb".into(),
        endpoint: std::env::var("RUST_COMMANDER_TEST_SMB_ENDPOINT")
            .unwrap_or("smb://127.0.0.1:1445".into()),
        bucket: "commander".into(),
        username: "commander".into(),
        root: "/".into(),
        ..Default::default()
    };
    remote::connect(
        &c,
        Secrets {
            password: "commander-test-only".into(),
            ..Default::default()
        },
        false,
    )
    .unwrap();
    let folder = std::path::PathBuf::from(format!("test-{:016x}", rand::random::<u64>()));
    remote::mkdir(&c.id, &folder).unwrap();
    let local = d.path().join("hello.txt");
    fs::write(&local, "hello SMB").unwrap();
    let remote_location = Location::Remote {
        connection: c.id.clone(),
        path: folder.clone(),
    };
    remote::transfer(
        &Location::Local(d.path().into()),
        &[filesystem::entry_from_path(&local).unwrap()],
        &remote_location,
        false,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(
        remote::read_bytes(&c.id, &folder.join("hello.txt"), 100).unwrap(),
        b"hello SMB"
    );
    remote::rename(&c.id, &folder.join("hello.txt"), "renamed.txt").unwrap();
    remote::save_text(
        &c.id,
        &folder.join("renamed.txt"),
        b"hello SMB",
        "edited SMB",
        &mut Task::default(),
    )
    .unwrap();
    let result = remote::search(
        &c.id,
        &rust_commander::desktop::SearchOptions {
            root: folder.clone(),
            pattern: "*.txt".into(),
            content: "edited".into(),
            hidden: false,
            max_results: 100,
            filters: Default::default(),
        },
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(result.paths, vec![folder.join("renamed.txt")]);
    let entry = remote::list(&c.id, &folder)
        .unwrap()
        .into_iter()
        .find(|e| e.name == "renamed.txt")
        .unwrap();
    remote::transfer(
        &remote_location,
        &[entry],
        &Location::Local(d.path().into()),
        false,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read(d.path().join("renamed.txt")).unwrap(),
        b"edited SMB"
    );
    let folder_entry = remote::list(&c.id, Path::new(""))
        .unwrap()
        .into_iter()
        .find(|e| e.path == folder)
        .unwrap();
    remote::delete(&c.id, &[folder_entry], &mut Task::default()).unwrap();
    assert!(
        !remote::list(&c.id, Path::new(""))
            .unwrap()
            .iter()
            .any(|e| e.path == folder)
    );
}

#[test]
fn synchronization_refuses_changes_during_recovery_preparation() {
    let d = tempfile::tempdir().unwrap();
    let l = d.path().join("l");
    let r = d.path().join("r");
    fs::create_dir(&l).unwrap();
    fs::create_dir(&r).unwrap();
    fs::write(l.join("a"), "left").unwrap();
    let plan = sync::preview(&l, &r, 1, &Task::default()).unwrap();
    let changed = r.join("external");
    let mut task = Task::default();
    task.before_change = Some(Box::new(move |_| {
        fs::write(&changed, "external")?;
        Ok(())
    }));
    assert!(sync::apply(&plan, &mut task).is_err());
    assert!(task.undo_blocked);
    assert!(!r.join("a").exists());
    assert_eq!(fs::read(r.join("external")).unwrap(), b"external");
}

#[test]
fn workspace_archive_tabs_store_persistent_containers_instead_of_private_cache_paths() {
    let d = tempfile::tempdir().unwrap();
    let file = d.path().join("data");
    fs::write(&file, "content").unwrap();
    let inner = d.path().join("inner.zip");
    archive::pack(&[file], &inner, &mut Task::default()).unwrap();
    let outer = d.path().join("outer.zip");
    archive::pack(&[inner], &outer, &mut Task::default()).unwrap();
    let containing = Location::Archive {
        file: outer,
        prefix: Default::default(),
    };
    let cache = archive_tools::materialize(&containing, Path::new("inner.zip")).unwrap();
    assert_eq!(
        rust_commander::workspaces::persistent_location(&Location::Archive {
            file: cache,
            prefix: Default::default()
        }),
        containing
    );
}
