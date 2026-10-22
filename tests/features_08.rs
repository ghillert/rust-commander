use rust_commander::{
    archive, associations, directory_tree, external_edit,
    filesystem::{self, Location},
    history,
    operations::{self, Operation, Task},
    remote::{self, Connection, Secrets},
    resumable,
    settings::{Settings, WindowSession},
    sync::{self, Choice},
    transfer::{ConflictPolicy, TransferOptions},
    viewer, viewer_search,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
fn mount(root: &Path) -> Location {
    let profile = Connection {
        id: format!("features08-{}", rand::random::<u64>()),
        name: "Disposable storage".into(),
        service: "fs".into(),
        root: root.display().to_string(),
        ..Default::default()
    };
    remote::connect(&profile, Secrets::default(), false).unwrap();
    Location::Remote {
        connection: profile.id,
        path: PathBuf::new(),
    }
}
fn journal(op: Operation, root: &Path) -> history::Record {
    let record = history::execute_in(op, &mut Task::default(), root).unwrap();
    assert!(record.success, "{}", record.result);
    record
}
#[test]
fn lazy_tree_expands_only_requested_directories_and_excludes_links() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("a")).unwrap();
    fs::create_dir(root.path().join("a/deep")).unwrap();
    fs::create_dir(root.path().join(".hidden")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.path(), root.path().join("loop")).unwrap();
    let location = Location::Local(root.path().into());
    let mut expanded = BTreeSet::from([root.path().to_owned()]);
    let nodes = directory_tree::build(&location, &expanded, false, &Task::default()).unwrap();
    assert!(nodes.iter().any(|n| n.title == "a"));
    assert!(
        !nodes
            .iter()
            .any(|n| n.title == "deep" || n.title == ".hidden" || n.title == "loop")
    );
    expanded.insert(root.path().join("a"));
    let nodes = directory_tree::build(&location, &expanded, true, &Task::default()).unwrap();
    assert!(nodes.iter().any(|n| n.title == "deep"));
    assert!(nodes.iter().any(|n| n.title == ".hidden"));
}
#[test]
fn native_moves_preserve_file_identity_and_remain_undoable() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let file = source.join("file");
    fs::write(&file, b"move me").unwrap();
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(&file).unwrap().ino()
    };
    let moved = journal(
        Operation::Move {
            sources: vec![file.clone()],
            destination: destination.clone(),
        },
        &root.path().join("history"),
    );
    assert!(!file.exists());
    assert_eq!(fs::read(destination.join("file")).unwrap(), b"move me");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(destination.join("file")).unwrap().ino(), inode);
    }
    journal(
        Operation::Undo(Box::new(moved)),
        &root.path().join("history"),
    );
    assert_eq!(fs::read(file).unwrap(), b"move me");
    assert!(!destination.join("file").exists());
}
#[test]
fn extraction_merges_with_per_file_choices_and_undo_restores_targets() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    for name in ["a", "b", "new"] {
        fs::write(source.join(name), format!("new-{name}")).unwrap();
    }
    fs::write(target.join("a"), b"old-a").unwrap();
    fs::write(target.join("b"), b"old-b").unwrap();
    let archive_path = root.path().join("files.zip");
    archive::pack(&[source], &archive_path, &mut Task::default()).unwrap();
    let conflicts =
        archive::extraction_conflicts(&archive_path, &[], Path::new("source"), &target, true)
            .unwrap();
    assert_eq!(conflicts.len(), 2);
    let record = journal(
        Operation::ExtractResolved {
            archive: archive_path,
            selected: vec![],
            prefix: "source".into(),
            destination: target.clone(),
            options: TransferOptions {
                conflicts: ConflictPolicy::Replace,
                merge_folders: true,
            },
            overrides: BTreeMap::from([(target.join("a"), ConflictPolicy::Skip)]),
        },
        &root.path().join("history"),
    );
    assert_eq!(fs::read(target.join("a")).unwrap(), b"old-a");
    assert_eq!(fs::read(target.join("b")).unwrap(), b"new-b");
    assert_eq!(fs::read(target.join("new")).unwrap(), b"new-new");
    journal(
        Operation::Undo(Box::new(record)),
        &root.path().join("history"),
    );
    assert_eq!(fs::read(target.join("b")).unwrap(), b"old-b");
    assert!(!target.join("new").exists());
}
#[test]
fn streaming_search_finds_chunk_boundary_matches_and_utf16_alignment() {
    let mut data = vec![b'x'; 65534];
    data.extend_from_slice("needle".as_bytes());
    data.extend(vec![b'x'; 100]);
    assert_eq!(
        viewer_search::find(&mut Cursor::new(data), b"needle", 0, 1, &Task::default()).unwrap(),
        Some(65534)
    );
    let needle = viewer_search::needle(
        "日本",
        false,
        rust_commander::text_document::Encoding::Utf16Le,
    )
    .unwrap();
    let mut bytes = vec![0];
    bytes.extend(&needle);
    bytes.push(0);
    bytes.extend(&needle);
    assert_eq!(
        viewer_search::find(&mut Cursor::new(bytes), &needle, 0, 2, &Task::default()).unwrap(),
        Some(6)
    );
    assert!(viewer_search::needle("0q", true, Default::default()).is_err());
    assert_eq!(
        viewer_search::needle("DE ad 00", true, Default::default()).unwrap(),
        [0xde, 0xad, 0]
    );
    let cancel = Arc::new(AtomicBool::new(true));
    assert!(
        viewer_search::find(
            &mut Cursor::new(b"data"),
            b"a",
            0,
            1,
            &Task::new(cancel, |_| {})
        )
        .is_err()
    );
}
#[test]
fn external_editor_refuses_changed_original_and_can_update_archive_member() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("note.txt");
    fs::write(&source, b"original").unwrap();
    let session =
        external_edit::Session::prepare(Location::Local(root.path().into()), source.clone())
            .unwrap();
    fs::write(&session.file, b"editor").unwrap();
    fs::write(&source, b"other program").unwrap();
    assert!(operations::run(session.save_operation().unwrap(), &mut Task::default()).is_err());
    assert_eq!(fs::read(&source).unwrap(), b"other program");
    let archive_path = root.path().join("files.zip");
    archive::pack(&[source], &archive_path, &mut Task::default()).unwrap();
    let location = Location::Archive {
        file: archive_path.clone(),
        prefix: PathBuf::new(),
    };
    let session = external_edit::Session::prepare(location, PathBuf::from("note.txt")).unwrap();
    fs::write(&session.file, b"saved through editor").unwrap();
    let record = journal(
        session.save_operation().unwrap(),
        &root.path().join("history"),
    );
    assert_eq!(
        archive::read_bytes(&archive_path, Path::new("note.txt"), 1024).unwrap(),
        b"saved through editor"
    );
    journal(
        Operation::Undo(Box::new(record)),
        &root.path().join("history"),
    );
    assert_eq!(
        archive::read_bytes(&archive_path, Path::new("note.txt"), 1024).unwrap(),
        b"other program"
    );
}
#[test]
fn application_rules_match_filenames_in_order_without_shell_expansion() {
    let root = tempfile::tempdir().unwrap();
    let application = root.path().join("some app");
    fs::write(&application, []).unwrap();
    let rule = associations::Rule {
        pattern: "*.pdf".into(),
        application: application.clone(),
    };
    rule.validate().unwrap();
    assert!(rule.matches(Path::new("MY FILE.PDF")));
    assert!(!rule.matches(Path::new("report.pdf.exe")));
    assert_eq!(
        associations::application(&[rule], Path::new("a.pdf")),
        Some(application.as_path())
    );
    assert!(
        associations::Rule {
            pattern: "[".into(),
            application
        }
        .validate()
        .is_err()
    );
}
#[test]
fn remote_merge_previews_nested_conflicts_and_retains_skipped_move_sources() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("pack")).unwrap();
    fs::create_dir_all(target.join("pack")).unwrap();
    fs::write(source.join("pack/a"), b"new-a").unwrap();
    fs::write(source.join("pack/b"), b"new-b").unwrap();
    fs::write(target.join("pack/a"), b"old-a").unwrap();
    let from = mount(&source);
    let to = mount(&target);
    let entries = filesystem::list(&from).unwrap();
    let conflicts = remote::conflicts_with_merge(&from, &entries, &to, true).unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].target, Path::new("pack/a"));
    remote::transfer_resolved(
        &from,
        &entries,
        &to,
        true,
        TransferOptions {
            conflicts: ConflictPolicy::Skip,
            merge_folders: true,
        },
        &Default::default(),
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read(target.join("pack/a")).unwrap(), b"old-a");
    assert_eq!(fs::read(target.join("pack/b")).unwrap(), b"new-b");
    assert!(source.join("pack/b").exists());
}
#[test]
fn remote_synchronization_applies_reviewed_changes_and_rejects_stale_plans() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("folder")).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("folder/a"), b"a").unwrap();
    let left = Location::Local(source.clone());
    let right = mount(&target);
    let plan = sync::preview_locations(&left, &right, 1, &Task::default()).unwrap();
    assert_eq!(plan.items.len(), 1);
    sync::apply(&plan, &mut Task::default()).unwrap();
    assert_eq!(fs::read(target.join("folder/a")).unwrap(), b"a");
    fs::write(source.join("folder/a"), b"changed").unwrap();
    let mut plan = sync::preview_locations(&left, &right, 1, &Task::default()).unwrap();
    assert!(plan.items[0].conflict);
    plan.items[0].choice = Choice::LeftToRight;
    fs::write(target.join("folder/a"), b"external change").unwrap();
    assert!(sync::apply(&plan, &mut Task::default()).is_err());
    assert_eq!(
        fs::read(target.join("folder/a")).unwrap(),
        b"external change"
    );
}
#[test]
fn checkpointed_download_recovers_after_cancellation_and_detects_source_changes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    let bytes = vec![0x73; 8 * 1024 * 1024];
    fs::write(source.join("large"), &bytes).unwrap();
    let from = mount(&source);
    let to = Location::Local(target.clone());
    let entries = filesystem::list(&from).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let mut task = Task::new(cancel, move |_| stop.store(true, Ordering::Relaxed));
    std::thread::sleep(std::time::Duration::from_millis(50));
    let checkpoints = root.path().join("checkpoints");
    let interrupted = resumable::run_in(
        &checkpoints,
        "batch-recovery",
        &from,
        &entries,
        &to,
        &mut task,
    )
    .unwrap_err();
    assert!(
        format!("{interrupted:#}").contains("cancelled"),
        "{interrupted:#}"
    );
    assert!(!target.join("large").exists());
    assert!(
        checkpoints
            .join("batch-recovery/0.part")
            .metadata()
            .unwrap()
            .len()
            > 0
    );
    let result = resumable::run_in(
        &checkpoints,
        "batch-recovery",
        &from,
        &entries,
        &to,
        &mut Task::default(),
    )
    .unwrap();
    assert!(result.contains("recovered 1"), "{result}");
    assert_eq!(fs::read(target.join("large")).unwrap(), bytes);
    assert!(!checkpoints.join("batch-recovery").exists());
}
#[test]
fn checkpointed_uploads_publish_verified_files_and_refuse_existing_targets() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir_all(source.join("folder")).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("folder/a"), b"upload").unwrap();
    let from = Location::Local(source.clone());
    let to = mount(&target);
    let entries = filesystem::list(&from).unwrap();
    resumable::run_in(
        &root.path().join("checkpoints"),
        "batch-upload",
        &from,
        &entries,
        &to,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read(target.join("folder/a")).unwrap(), b"upload");
    assert!(
        resumable::run_in(
            &root.path().join("checkpoints"),
            "batch-another",
            &from,
            &entries,
            &to,
            &mut Task::default()
        )
        .is_err()
    );
}
#[test]
fn encrypted_pdf_password_rotation_and_page_cache_work() {
    let bytes = include_bytes!("fixtures/two-pages-protected.pdf");
    assert!(viewer::render_pdf_password(bytes, 0, "wrong").is_err());
    let mut cache = viewer::PdfCache::default();
    let (first, pages) = cache.render(bytes, 0, "commander-test").unwrap();
    assert_eq!(pages, 2);
    let rotated = viewer::rotate(&first, 1);
    assert_eq!(rotated.size, [first.size[1], first.size[0]]);
    assert_eq!(viewer::rotate(&rotated, 3).pixels, first.pixels);
    let (again, _) = cache.render(bytes, 0, "commander-test").unwrap();
    assert_eq!(again.pixels, first.pixels);
    assert!(cache.render(bytes, 0, "wrong").is_err());
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("protected.pdf");
    fs::write(&file, bytes).unwrap();
    assert!(matches!(
        viewer::load(&Location::Local(root.path().into()), &file).unwrap(),
        viewer::Content::LockedPdf { .. }
    ));
}
#[test]
fn multiple_window_sessions_round_trip_without_replacing_global_preferences() {
    let mut globals = Settings::default();
    globals.preferences.font_size = 18.0;
    let mut first = globals.clone();
    first.tabs[0] = vec![Location::Local(PathBuf::from("/first"))];
    let workspace = rust_commander::workspaces::Workspace {
        name: String::new(),
        tabs: first.tabs,
        tab_states: first.tab_states,
        active_tabs: [0, 0],
        active_pane: 1,
        preferences: first.preferences,
    };
    globals.window_sessions = vec![WindowSession {
        workspace,
        width: 1200.0,
        height: 800.0,
        position: None,
    }];
    let decoded: Settings = serde_json::from_slice(&serde_json::to_vec(&globals).unwrap()).unwrap();
    let restored = decoded.window_sessions[0].settings(&decoded);
    assert_eq!(restored.active_pane, 1);
    assert_eq!(restored.preferences.font_size, 18.0);
    assert_eq!(restored.tabs[0][0], Location::Local("/first".into()));
    let legacy: Settings = serde_json::from_str("{}").unwrap();
    assert!(legacy.window_sessions.is_empty());
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn native_extended_attributes_survive_copy_and_external_changes_block_undo() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    fn set(path: &Path, value: &[u8]) {
        let path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let name = CString::new("user.rust-commander-test").unwrap();
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::setxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::setxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        };
        assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    }
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"data").unwrap();
    fs::create_dir(&target).unwrap();
    set(&source, b"original tag");
    let before = rust_commander::file_metadata::digest(&source).unwrap();
    let record = journal(
        Operation::Copy {
            sources: vec![source],
            destination: target.clone(),
        },
        &root.path().join("history"),
    );
    assert_eq!(
        rust_commander::file_metadata::digest(&target.join("source")).unwrap(),
        before
    );
    set(&target.join("source"), b"edited in another app");
    let rejected = history::execute_in(
        Operation::Undo(Box::new(record)),
        &mut Task::default(),
        &root.path().join("history"),
    )
    .unwrap();
    assert!(!rejected.success);
    assert!(target.join("source").exists());
}

#[test]
fn checkpoint_retries_refuse_changed_sources_and_corrupt_partial_data() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("large"), vec![0x44; 1024 * 1024]).unwrap();
    let from = mount(&source);
    let to = Location::Local(target.clone());
    let entries = filesystem::list(&from).unwrap();
    let checkpoints = root.path().join("checkpoints");
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let mut task = Task::new(cancel, move |_| stop.store(true, Ordering::Relaxed));
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        resumable::run_in(
            &checkpoints,
            "batch-guards",
            &from,
            &entries,
            &to,
            &mut task
        )
        .unwrap_err()
        .to_string()
        .contains("cancelled")
    );
    let partial = checkpoints.join("batch-guards/0.part");
    let original = fs::read(&partial).unwrap();
    let mut corrupt = original.clone();
    corrupt[0] ^= 1;
    fs::write(&partial, corrupt).unwrap();
    assert!(
        resumable::run_in(
            &checkpoints,
            "batch-guards",
            &from,
            &entries,
            &to,
            &mut Task::default()
        )
        .unwrap_err()
        .to_string()
        .contains("differs")
    );
    assert!(!target.join("large").exists());
    fs::write(&partial, original).unwrap();
    fs::write(source.join("large"), vec![0x55; 1024 * 1024]).unwrap();
    assert!(
        resumable::run_in(
            &checkpoints,
            "batch-guards",
            &from,
            &entries,
            &to,
            &mut Task::default()
        )
        .unwrap_err()
        .to_string()
        .contains("Source content changed")
    );
    assert!(!target.join("large").exists());
}
#[test]
fn checkpoint_recognizes_a_published_file_after_an_interrupted_receipt() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    let bytes = vec![0x33; 1024 * 1024];
    fs::write(source.join("large"), &bytes).unwrap();
    let from = mount(&source);
    let to = Location::Local(target.clone());
    let entries = filesystem::list(&from).unwrap();
    let checkpoints = root.path().join("checkpoints");
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let mut task = Task::new(cancel, move |_| stop.store(true, Ordering::Relaxed));
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        resumable::run_in(
            &checkpoints,
            "batch-receipt",
            &from,
            &entries,
            &to,
            &mut task
        )
        .is_err()
    );
    // Model a crash after committing a verified destination but before saving its receipt.
    fs::write(target.join("large"), &bytes).unwrap();
    let result = resumable::run_in(
        &checkpoints,
        "batch-receipt",
        &from,
        &entries,
        &to,
        &mut Task::default(),
    )
    .unwrap();
    assert!(result.contains("recovered 1"));
    assert_eq!(fs::read(target.join("large")).unwrap(), bytes);
}
#[test]
fn old_journal_records_without_native_metadata_guards_remain_undoable() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"legacy record").unwrap();
    fs::create_dir(&target).unwrap();
    let record = journal(
        Operation::Copy {
            sources: vec![source],
            destination: target.clone(),
        },
        &root.path().join("history"),
    );
    let mut json = serde_json::to_value(record).unwrap();
    for change in json["changes"].as_array_mut().unwrap() {
        let fields = change.as_object_mut().unwrap();
        for name in ["before_native", "after_native", "backup_native"] {
            fields.remove(name);
        }
    }
    let old: history::Record = serde_json::from_value(json).unwrap();
    journal(Operation::Undo(Box::new(old)), &root.path().join("history"));
    assert!(!target.join("source").exists());
}
#[cfg(target_os = "macos")]
#[test]
fn macos_resource_forks_and_acls_survive_local_copy() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"data fork").unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("..namedfork/rsrc"), vec![0x7a; 128 * 1024]).unwrap();
    let status = std::process::Command::new("/bin/chmod")
        .args(["+a", "everyone allow read"])
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success());
    let before = rust_commander::file_metadata::digest(&source).unwrap();
    operations::run(
        Operation::Copy {
            sources: vec![source],
            destination: target.clone(),
        },
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read(target.join("source/..namedfork/rsrc")).unwrap(),
        vec![0x7a; 128 * 1024]
    );
    assert_eq!(
        rust_commander::file_metadata::digest(&target.join("source")).unwrap(),
        before
    );
}

#[test]
fn remote_sync_does_not_propagate_its_own_recovery_copies() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(source.join("a"), b"new").unwrap();
    fs::write(target.join("a"), b"previous").unwrap();
    let left = Location::Local(source);
    let right = mount(&target);
    let mut plan = sync::preview_locations(&left, &right, 1, &Task::default()).unwrap();
    assert_eq!(plan.items.len(), 1);
    plan.items[0].choice = Choice::LeftToRight;
    sync::apply(&plan, &mut Task::default()).unwrap();
    assert!(fs::read_dir(&target).unwrap().flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .starts_with(".rust-commander-replaced-")
    }));
    assert!(
        sync::preview_locations(&left, &right, 0, &Task::default())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn comparison_and_sync_cover_local_remote_and_remote_remote_pairs() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("left");
    let b = root.path().join("right");
    fs::create_dir_all(a.join("nested")).unwrap();
    fs::create_dir(&b).unwrap();
    fs::write(a.join("nested/café.txt"), b"left").unwrap();
    fs::write(a.join("same"), b"same").unwrap();
    fs::write(b.join("same"), b"same").unwrap();
    fs::write(a.join("different"), b"aaaa").unwrap();
    fs::write(b.join("different"), b"bbbb").unwrap();
    let local = Location::Local(a.clone());
    let left = mount(&a);
    let right = mount(&b);
    for from in [&local, &left] {
        let result =
            rust_commander::comparison::compare(from, &right, &mut Task::default()).unwrap();
        assert!(
            result
                .iter()
                .any(|v| v.name == "same" && v.status == "Equal")
        );
        assert!(
            result
                .iter()
                .any(|v| v.name == "different" && v.status == "Different")
        );
        assert!(
            result
                .iter()
                .any(|v| v.name.ends_with("café.txt") && v.status == "Only left")
        );
    }
    let mut plan = sync::preview_locations(&left, &right, 1, &Task::default()).unwrap();
    for item in &mut plan.items {
        if item.conflict {
            item.choice = Choice::LeftToRight;
        }
    }
    sync::apply(&plan, &mut Task::default()).unwrap();
    let result = rust_commander::comparison::compare(&local, &right, &mut Task::default()).unwrap();
    assert!(
        result
            .iter()
            .all(|v| matches!(v.status.as_str(), "Equal" | "Folder")
                || (v.status == "Only right" && v.name.starts_with(".rust-commander-replaced-")))
    );
}
#[test]
fn read_only_sync_choices_are_skipped_and_invalid_plans_fail_before_any_write() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("left");
    let b = root.path().join("right");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    fs::write(a.join("a"), b"left").unwrap();
    fs::write(b.join("b"), b"right").unwrap();
    let left = Location::Local(a.clone());
    let right = mount(&b);
    let Location::Remote { connection, .. } = &right else {
        unreachable!()
    };
    let mut profile = remote::profile(connection).unwrap();
    profile.service = "http".into();
    remote::register_profiles(&[profile]);
    let mut plan = sync::preview_locations(&left, &right, 0, &Task::default()).unwrap();
    let index = plan
        .items
        .iter()
        .position(|i| i.path == Path::new("a"))
        .unwrap();
    assert_eq!(plan.items[index].choice, Choice::Skip);
    assert!(!plan.allows(&plan.items[index], Choice::LeftToRight));
    plan.items[index].choice = Choice::LeftToRight;
    assert!(sync::apply(&plan, &mut Task::default()).is_err());
    assert!(!a.join("b").exists());
    assert!(!b.join("a").exists());
    plan.items[index].choice = Choice::Skip;
    sync::apply(&plan, &mut Task::default()).unwrap();
    assert_eq!(fs::read(a.join("b")).unwrap(), b"right");
}
