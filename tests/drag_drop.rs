use rust_commander::{
    archive,
    drag_drop::{self, Selection},
    filesystem::{self, Location},
    operations::{Operation, Task},
    remote,
    transfer::{ConflictPolicy, TransferOptions},
};
use std::{collections::BTreeMap, fs, path::Path};

fn archive_selection(file: &Path, prefix: &str) -> Selection {
    let from = Location::Archive {
        file: file.into(),
        prefix: prefix.into(),
    };
    Selection {
        entries: filesystem::list(&from).unwrap(),
        from,
    }
}
fn zip(root: &Path, name: &str) -> std::path::PathBuf {
    let file = root.join(name);
    let tree = root.join(format!("{name}-input"));
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("a.txt"), b"archive bytes").unwrap();
    fs::create_dir(tree.join("folder")).unwrap();
    fs::write(tree.join("folder/nested.txt"), b"nested").unwrap();
    archive::pack(
        &[tree.join("a.txt"), tree.join("folder")],
        &file,
        &mut Task::default(),
    )
    .unwrap();
    file
}
#[test]
fn archive_exports_keep_names_and_leave_sources_untouched() {
    let root = tempfile::tempdir().unwrap();
    let file = zip(root.path(), "source.zip");
    let before = fs::read(&file).unwrap();
    let selection = archive_selection(&file, "");
    let export = drag_drop::export(&selection, &mut Task::default()).unwrap();
    assert_eq!(export.paths.len(), 2);
    assert_eq!(
        fs::read(export.directory.path().join("files/a.txt")).unwrap(),
        b"archive bytes"
    );
    assert_eq!(
        fs::read(export.directory.path().join("files/folder/nested.txt")).unwrap(),
        b"nested"
    );
    assert_eq!(fs::read(&file).unwrap(), before);
    let staged = export.directory.path().to_owned();
    drop(export);
    assert!(!staged.exists());
}
#[test]
fn archive_drop_move_preserves_skipped_sources_and_moves_complete_entries() {
    let root = tempfile::tempdir().unwrap();
    let file = zip(root.path(), "source.zip");
    let destination = root.path().join("out");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("a.txt"), b"keep target").unwrap();
    let selection = archive_selection(&file, "");
    drag_drop::transfer(
        &selection,
        &Location::Local(destination.clone()),
        true,
        TransferOptions {
            conflicts: ConflictPolicy::Skip,
            merge_folders: true,
        },
        &BTreeMap::new(),
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"keep target");
    assert_eq!(
        fs::read(destination.join("folder/nested.txt")).unwrap(),
        b"nested"
    );
    let remaining = archive::list(&file, Path::new("")).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].name, "a.txt");
}
#[test]
fn drops_into_archive_support_replace_merge_and_local_undo_without_staging_paths() {
    let root = tempfile::tempdir().unwrap();
    let file = zip(root.path(), "target.zip");
    let source = root.path().join("a.txt");
    fs::write(&source, b"replacement").unwrap();
    let selection = Selection::local(std::slice::from_ref(&source)).unwrap();
    let original = fs::read(&file).unwrap();
    let journal = root.path().join("history");
    let mut task = Task::default();
    let record = rust_commander::history::execute_in(
        Operation::DropTransfer {
            selection,
            to: Location::Archive {
                file: file.clone(),
                prefix: "".into(),
            },
            moving: true,
            options: TransferOptions {
                conflicts: ConflictPolicy::Replace,
                merge_folders: true,
            },
            overrides: Default::default(),
        },
        &mut task,
        &journal,
    )
    .unwrap();
    assert!(record.success, "{}", record.result);
    assert!(!source.exists());
    assert_eq!(
        archive::read_bytes(&file, Path::new("a.txt"), 100).unwrap(),
        b"replacement"
    );
    assert!(
        archive::list(&file, Path::new(""))
            .unwrap()
            .iter()
            .all(|e| !e.name.starts_with(".rust-commander-replaced-"))
    );
    let undo = rust_commander::history::execute_in(
        Operation::Undo(Box::new(record)),
        &mut Task::default(),
        &journal,
    )
    .unwrap();
    assert!(undo.success, "{}", undo.result);
    assert_eq!(fs::read(&file).unwrap(), original);
    assert_eq!(fs::read(&source).unwrap(), b"replacement");
}
#[test]
fn in_archive_move_commits_addition_and_removal_together() {
    let root = tempfile::tempdir().unwrap();
    let file = zip(root.path(), "archive.zip");
    let mut selection = archive_selection(&file, "");
    selection.entries.retain(|e| e.name == "a.txt");
    drag_drop::transfer(
        &selection,
        &Location::Archive {
            file: file.clone(),
            prefix: "folder".into(),
        },
        true,
        Default::default(),
        &Default::default(),
        &mut Task::default(),
    )
    .unwrap();
    assert!(archive::read_bytes(&file, Path::new("a.txt"), 100).is_err());
    assert_eq!(
        archive::read_bytes(&file, Path::new("folder/a.txt"), 100).unwrap(),
        b"archive bytes"
    );
}
#[test]
fn virtual_remote_and_archive_pairs_use_the_existing_storage_guards() {
    let root = tempfile::tempdir().unwrap();
    let file = zip(root.path(), "source.zip");
    let server = root.path().join("server");
    fs::create_dir(&server).unwrap();
    let connection = remote::Connection {
        service: "fs".into(),
        root: server.display().to_string(),
        ..Default::default()
    };
    remote::connect(&connection, Default::default(), false).unwrap();
    let to = Location::Remote {
        connection: connection.id.clone(),
        path: "".into(),
    };
    drag_drop::transfer(
        &archive_selection(&file, ""),
        &to,
        false,
        Default::default(),
        &Default::default(),
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read(server.join("folder/nested.txt")).unwrap(),
        b"nested"
    );
    let selection = Selection {
        entries: filesystem::list(&to).unwrap(),
        from: to,
    };
    let other = zip(root.path(), "target.zip");
    drag_drop::transfer(
        &selection,
        &Location::Archive {
            file: other.clone(),
            prefix: "folder".into(),
        },
        true,
        Default::default(),
        &Default::default(),
        &mut Task::default(),
    )
    .unwrap();
    assert!(fs::read_dir(&server).unwrap().next().is_none());
    assert_eq!(
        archive::read_bytes(&other, Path::new("folder/a.txt"), 100).unwrap(),
        b"archive bytes"
    );
}
#[test]
fn invalid_recursive_and_read_only_drops_are_rejected_before_writing() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("folder");
    fs::create_dir(&folder).unwrap();
    let selection = Selection::local(std::slice::from_ref(&folder)).unwrap();
    assert!(
        selection
            .check_target(&Location::Local(folder.join("child")), false)
            .is_err()
    );
    assert!(Selection::local(&["relative".into()]).is_err());
    let profile = remote::Connection {
        service: "http".into(),
        endpoint: "https://example.test".into(),
        ..Default::default()
    };
    remote::register_profiles(std::slice::from_ref(&profile));
    assert!(
        selection
            .check_target(
                &Location::Remote {
                    connection: profile.id,
                    path: "".into()
                },
                false
            )
            .is_err()
    );
    assert!(fs::read_dir(&folder).unwrap().next().is_none());
}

#[test]
fn normal_local_drop_preserves_determinate_progress_and_rejects_overlapping_sources() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let to = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&to).unwrap();
    fs::write(source.join("file.txt"), b"progress").unwrap();
    let selection = Selection::local(&[source.join("file.txt")]).unwrap();
    let mut task = Task::default();
    rust_commander::operations::run(
        Operation::DropTransfer {
            selection,
            to: Location::Local(to.clone()),
            moving: false,
            options: Default::default(),
            overrides: Default::default(),
        },
        &mut task,
    )
    .unwrap();
    assert_eq!(task.progress.total_bytes, Some(8));
    assert_eq!(task.progress.bytes, 8);
    assert_eq!(fs::read(to.join("file.txt")).unwrap(), b"progress");
    assert!(Selection::local(&[source.clone(), source.join("file.txt")]).is_err());
}
#[test]
fn archive_replacements_without_undo_keep_a_durable_original_container() {
    let root = tempfile::tempdir().unwrap();
    let file = zip(root.path(), "target.zip");
    let before = fs::read(&file).unwrap();
    let source = root.path().join("a.txt");
    fs::write(&source, b"replacement").unwrap();
    let result = drag_drop::transfer(
        &Selection::local(&[source]).unwrap(),
        &Location::Archive {
            file: file.clone(),
            prefix: Path::new("").into(),
        },
        false,
        TransferOptions {
            conflicts: ConflictPolicy::Replace,
            merge_folders: true,
        },
        &Default::default(),
        &mut Task::default(),
    )
    .unwrap();
    let backup = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".rust-commander-archive-backup-")
        })
        .expect("durable replacement backup");
    assert_eq!(fs::read(&backup).unwrap(), before);
    assert!(result.contains(&backup.display().to_string()));
    assert!(!result.contains("archive-drop-"));
}
#[test]
fn cancelled_exports_leave_no_files_and_changed_archive_sources_are_kept() {
    use std::sync::{Arc, atomic::AtomicBool};
    let root = tempfile::tempdir().unwrap();
    let file = zip(root.path(), "source.zip");
    let original = fs::read(&file).unwrap();
    let selection = archive_selection(&file, "");
    let mut task = Task::new(Arc::new(AtomicBool::new(true)), |_| {});
    assert!(drag_drop::export(&selection, &mut task).is_err());
    assert_eq!(fs::read(&file).unwrap(), original);
    let to = root.path().join("target");
    fs::create_dir(&to).unwrap();
    let updated = file.clone();
    let mut task = Task::default();
    task.before_change = Some(Box::new(move |path| {
        if path.file_name() == Some(std::ffi::OsStr::new("a.txt")) && !path.exists() {
            let mut bytes = fs::read(&updated)?;
            bytes.push(0);
            fs::write(&updated, bytes)?;
        }
        Ok(())
    }));
    let result = drag_drop::transfer(
        &selection,
        &Location::Local(to.clone()),
        true,
        Default::default(),
        &Default::default(),
        &mut task,
    );
    assert!(
        result.is_err(),
        "source stamp must prevent deleting a container changed during the copy"
    );
    assert!(file.exists());
    assert!(archive::read_bytes(&file, Path::new("a.txt"), 100).is_ok());
    assert_eq!(fs::read(to.join("a.txt")).unwrap(), b"archive bytes");
}

#[test]
fn archive_conflict_preview_uses_metadata_and_includes_nested_merge_conflicts() {
    let root = tempfile::tempdir().unwrap();
    let file = zip(root.path(), "source.zip");
    let server = root.path().join("server");
    fs::create_dir_all(server.join("folder")).unwrap();
    fs::write(server.join("a.txt"), b"existing").unwrap();
    fs::write(server.join("folder/nested.txt"), b"existing nested").unwrap();
    let connection = remote::Connection {
        service: "fs".into(),
        root: server.display().to_string(),
        ..Default::default()
    };
    remote::connect(&connection, Default::default(), false).unwrap();
    let selection = archive_selection(&file, "");
    // Keep the directory intact but damage a file body. A metadata preview must
    // not download/extract it (a confirmed transfer will still reject its CRC).
    let mut reader = zip::ZipArchive::new(fs::File::open(&file).unwrap()).unwrap();
    let start = reader.by_name("a.txt").unwrap().data_start() as usize;
    drop(reader);
    let mut bytes = fs::read(&file).unwrap();
    bytes[start] ^= 0xff;
    fs::write(&file, &bytes).unwrap();
    let conflicts = drag_drop::conflicts(
        &selection,
        &Location::Remote {
            connection: connection.id,
            path: Path::new("").into(),
        },
        true,
    )
    .unwrap();
    assert_eq!(conflicts.len(), 2);
    assert!(
        conflicts
            .iter()
            .any(|c| c.target == Path::new("folder/nested.txt"))
    );
    assert_eq!(fs::read(&file).unwrap(), bytes);
    assert!(
        drag_drop::export(&selection, &mut Task::default()).is_err(),
        "corrupt contents are checked when actually materialized"
    );
}
