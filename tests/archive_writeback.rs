use rust_commander::{
    archive,
    archive_tools::{self, ZipEdit},
    external_edit,
    filesystem::Location,
    history,
    operations::{self, Operation, Task},
    remote,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

fn nested(root: &Path, extension: &str) -> (PathBuf, PathBuf) {
    let input = root.join("input");
    fs::create_dir(&input).unwrap();
    fs::write(input.join("a.txt"), b"original a").unwrap();
    fs::write(input.join("keep.txt"), b"keep").unwrap();
    let folder = root.join("folder");
    fs::create_dir(&folder).unwrap();
    let inner = folder.join(format!("inner.{extension}"));
    archive::pack(
        &[input.join("a.txt"), input.join("keep.txt")],
        &inner,
        &mut Task::default(),
    )
    .unwrap();
    let middle = root.join("middle.tar");
    archive::pack(&[folder], &middle, &mut Task::default()).unwrap();
    let outer = root.join("outer.zip");
    archive::pack(&[middle], &outer, &mut Task::default()).unwrap();
    let cached = open_nested(&outer, extension);
    (outer, cached)
}
fn open_nested(outer: &Path, extension: &str) -> PathBuf {
    let middle = archive_tools::materialize(
        &Location::Archive {
            file: outer.into(),
            prefix: "".into(),
        },
        Path::new("middle.tar"),
    )
    .unwrap();
    archive_tools::materialize(
        &Location::Archive {
            file: middle,
            prefix: "folder".into(),
        },
        &Path::new("folder").join(format!("inner.{extension}")),
    )
    .unwrap()
}
#[test]
fn nested_zip_tar_and_sevenz_edits_commit_through_all_containers_and_keep_working() {
    for extension in ["zip", "tar", "7z"] {
        let root = tempfile::tempdir().unwrap();
        let (outer, cached) = nested(root.path(), extension);
        assert!(archive_tools::editable(&cached));
        archive_tools::edit_archive(
            &cached,
            &ZipEdit::Rename {
                source: "a.txt".into(),
                target: "renamed.txt".into(),
            },
            &mut Task::default(),
        )
        .unwrap();
        let addition = root.path().join("café.txt");
        fs::write(&addition, b"added").unwrap();
        archive_tools::edit_archive(
            &cached,
            &ZipEdit::Add {
                sources: vec![addition],
                prefix: "new".into(),
            },
            &mut Task::default(),
        )
        .unwrap();
        archive_tools::edit_archive(
            &cached,
            &ZipEdit::Remove(vec!["renamed.txt".into()]),
            &mut Task::default(),
        )
        .unwrap();
        let actual = open_nested(&outer, extension);
        assert!(archive::read_bytes(&actual, Path::new("a.txt"), 100).is_err());
        assert!(archive::read_bytes(&actual, Path::new("renamed.txt"), 100).is_err());
        assert_eq!(
            archive::read_bytes(&actual, Path::new("new/café.txt"), 100).unwrap(),
            b"added"
        );
        assert_eq!(
            archive::read_bytes(&actual, Path::new("keep.txt"), 100).unwrap(),
            b"keep"
        );
    }
}
#[test]
fn nested_edits_journal_only_the_outer_source_and_undo_makes_old_caches_stale() {
    let root = tempfile::tempdir().unwrap();
    let (outer, cached) = nested(root.path(), "zip");
    let original = fs::read(&outer).unwrap();
    let record = history::execute_in(
        Operation::ArchiveEdit {
            archive: cached.clone(),
            edit: ZipEdit::Remove(vec!["a.txt".into()]),
        },
        &mut Task::default(),
        &root.path().join("history"),
    )
    .unwrap();
    assert!(record.success, "{}", record.result);
    assert_eq!(record.changes.len(), 1);
    assert_eq!(record.changes[0].path, outer.canonicalize().unwrap());
    history::undo(&record, &mut Task::default()).unwrap();
    assert_eq!(fs::read(&outer).unwrap(), original);
    assert!(
        archive_tools::edit_archive(
            &cached,
            &ZipEdit::Remove(vec!["keep.txt".into()]),
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&outer).unwrap(), original);
}
#[test]
fn changed_outer_sources_and_cancellation_preserve_every_container() {
    let root = tempfile::tempdir().unwrap();
    let (outer, cached) = nested(root.path(), "zip");
    let cache_before = fs::read(&cached).unwrap();
    let original = fs::read(&outer).unwrap();
    let mut cancelled = Task::new(Arc::new(AtomicBool::new(true)), |_| {});
    assert!(
        archive_tools::edit_archive(
            &cached,
            &ZipEdit::Remove(vec!["a.txt".into()]),
            &mut cancelled
        )
        .is_err()
    );
    assert_eq!(fs::read(&outer).unwrap(), original);
    fs::OpenOptions::new()
        .append(true)
        .open(&outer)
        .unwrap()
        .write_all(b"external")
        .unwrap();
    let changed = fs::read(&outer).unwrap();
    assert!(
        archive_tools::edit_archive(
            &cached,
            &ZipEdit::Remove(vec!["a.txt".into()]),
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&outer).unwrap(), changed);
    assert_eq!(fs::read(&cached).unwrap(), cache_before);
}
#[test]
fn external_editor_saves_nested_members_back_to_the_original_outer_file() {
    let root = tempfile::tempdir().unwrap();
    let (outer, cached) = nested(root.path(), "zip");
    let session = external_edit::Session::prepare(
        Location::Archive {
            file: cached,
            prefix: "".into(),
        },
        "keep.txt".into(),
    )
    .unwrap();
    fs::write(&session.file, b"edited externally").unwrap();
    operations::run(session.save_operation().unwrap(), &mut Task::default()).unwrap();
    let actual = open_nested(&outer, "zip");
    assert_eq!(
        archive::read_bytes(&actual, Path::new("keep.txt"), 100).unwrap(),
        b"edited externally"
    );
}

#[test]
fn encrypted_nested_edits_preserve_both_container_passwords() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("a.txt");
    fs::write(&input, b"secret member").unwrap();
    let inner = root.path().join("inner.7z");
    archive::pack_with_password(&[input], &inner, "inner password", &mut Task::default()).unwrap();
    let outer = root.path().join("outer.zip");
    archive::pack_with_password(&[inner], &outer, "outer password", &mut Task::default()).unwrap();
    archive::unlock(&outer, "outer password").unwrap();
    let location = Location::Archive {
        file: outer.clone(),
        prefix: "".into(),
    };
    let cached = archive_tools::materialize(&location, Path::new("inner.7z")).unwrap();
    archive::unlock(&cached, "inner password").unwrap();
    archive_tools::edit_archive(
        &cached,
        &ZipEdit::Rename {
            source: "a.txt".into(),
            target: "renamed.txt".into(),
        },
        &mut Task::default(),
    )
    .unwrap();
    let mut reader = zip::ZipArchive::new(fs::File::open(&outer).unwrap()).unwrap();
    assert!(reader.by_index_raw(0).unwrap().encrypted());
    drop(reader);
    let reopened = archive_tools::materialize(&location, Path::new("inner.7z")).unwrap();
    assert!(archive::read_bytes(&reopened, Path::new("renamed.txt"), 100).is_err());
    archive::unlock(&reopened, "inner password").unwrap();
    assert_eq!(
        archive::read_bytes(&reopened, Path::new("renamed.txt"), 100).unwrap(),
        b"secret member"
    );
}

#[test]
fn a_source_change_during_journal_capture_stops_publication() {
    let root = tempfile::tempdir().unwrap();
    let (outer, cached) = nested(root.path(), "zip");
    let cache_before = fs::read(&cached).unwrap();
    let watched = outer.clone();
    let mut task = Task::default();
    task.before_change = Some(Box::new(move |path| {
        if path == watched {
            fs::OpenOptions::new()
                .append(true)
                .open(&watched)?
                .write_all(b"changed during capture")?;
        }
        Ok(())
    }));
    assert!(
        archive_tools::edit_archive(&cached, &ZipEdit::Remove(vec!["a.txt".into()]), &mut task)
            .is_err()
    );
    assert!(
        fs::read(&outer)
            .unwrap()
            .ends_with(b"changed during capture")
    );
    assert_eq!(fs::read(cached).unwrap(), cache_before);
}
#[test]
fn remote_archives_larger_than_text_limits_save_with_a_verified_recovery_original() {
    let root = tempfile::tempdir().unwrap();
    let payload = root.path().join("payload.bin");
    fs::write(&payload, vec![51; 3 * 1024 * 1024]).unwrap();
    let server = root.path().join("server");
    fs::create_dir(&server).unwrap();
    let archive_path = server.join("remote.zip");
    archive::pack_with_compression(&[payload], &archive_path, "", Some(0), &mut Task::default())
        .unwrap();
    let original = fs::read(&archive_path).unwrap();
    assert!(original.len() > 2 * 1024 * 1024);
    let connection = remote::Connection {
        service: "fs".into(),
        root: server.display().to_string(),
        ..Default::default()
    };
    remote::connect(&connection, Default::default(), false).unwrap();
    let location = Location::Remote {
        connection: connection.id.clone(),
        path: "".into(),
    };
    let cached = archive_tools::materialize(&location, Path::new("remote.zip")).unwrap();
    let mut task = Task::default();
    operations::run(
        Operation::ArchiveEdit {
            archive: cached.clone(),
            edit: ZipEdit::Rename {
                source: "payload.bin".into(),
                target: "renamed.bin".into(),
            },
        },
        &mut task,
    )
    .unwrap();
    assert!(task.undo_blocked);
    assert_eq!(
        archive::read_bytes(&archive_path, Path::new("renamed.bin"), 4 * 1024 * 1024).unwrap(),
        vec![51; 3 * 1024 * 1024]
    );
    let backup = fs::read_dir(&server)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".rust-commander-edit-")
        })
        .unwrap();
    assert_eq!(fs::read(&backup).unwrap(), original);
    let inventory = rust_commander::recovery::inventory(&Task::default()).unwrap();
    assert!(inventory.items.iter().any(
        |item| matches!(&item.source, Location::Remote {connection: id, ..} if id == &connection.id)
    ));
    fs::write(&archive_path, b"external update").unwrap();
    assert!(
        archive_tools::edit_archive(
            &cached,
            &ZipEdit::Remove(vec!["renamed.bin".into()]),
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&archive_path).unwrap(), b"external update");
}

#[test]
fn replacing_cached_archive_members_retains_the_real_outer_original() {
    for server_backed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (outer, nested_cache) = nested(root.path(), "zip");
        let original = fs::read(&outer).unwrap();
        let cached = if server_backed {
            let connection = remote::Connection {
                service: "fs".into(),
                root: root.path().display().to_string(),
                ..Default::default()
            };
            remote::connect(&connection, Default::default(), false).unwrap();
            let remote_outer = archive_tools::materialize(
                &Location::Remote {
                    connection: connection.id,
                    path: "".into(),
                },
                Path::new("outer.zip"),
            )
            .unwrap();
            open_nested(&remote_outer, "zip")
        } else {
            nested_cache
        };
        let replacement = root.path().join("a.txt");
        fs::write(&replacement, b"replacement").unwrap();
        let operation = Operation::DropTransfer {
            selection: rust_commander::drag_drop::Selection::local(&[replacement]).unwrap(),
            to: Location::Archive {
                file: cached.clone(),
                prefix: "".into(),
            },
            moving: false,
            options: rust_commander::transfer::TransferOptions {
                conflicts: rust_commander::transfer::ConflictPolicy::Replace,
                merge_folders: true,
            },
            overrides: Default::default(),
        };
        let message = if server_backed {
            let mut task = Task::default();
            let record =
                history::execute_in(operation, &mut task, &root.path().join("history")).unwrap();
            assert!(record.success, "{}", record.result);
            assert!(record.undo_blocked);
            record.result
        } else {
            operations::run(operation, &mut Task::default()).unwrap()
        };
        assert!(!message.contains("can be restored with Undo"), "{message}");
        let prefix = if server_backed {
            ".rust-commander-edit-"
        } else {
            ".rust-commander-archive-backup-"
        };
        let backup = fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(prefix)
            })
            .unwrap();
        assert_eq!(fs::read(backup).unwrap(), original);
        assert_eq!(
            archive::read_bytes(&cached, Path::new("a.txt"), 100).unwrap(),
            b"replacement"
        );
        assert_ne!(fs::read(outer).unwrap(), original);
    }
}
