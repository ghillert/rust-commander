use rust_commander::{
    filesystem::{self, Location},
    operations::Task,
    remote::{self, Connection, Secrets},
    resumable,
    transfer::{ConflictPolicy, TransferOptions},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

fn location(root: &Path, remote: bool) -> Location {
    if !remote {
        return Location::Local(root.into());
    }
    let connection = Connection {
        id: format!("transfer-controls-{}", rand::random::<u64>()),
        name: "Disposable transfer controls fixture".into(),
        service: "fs".into(),
        root: root.display().to_string(),
        ..Default::default()
    };
    remote::connect(&connection, Secrets::default(), false).unwrap();
    Location::Remote {
        connection: connection.id,
        path: PathBuf::new(),
    }
}

#[test]
#[ignore = "subprocess crash fixture, invoked by replacement_survives_process_termination"]
fn replacement_crash_child() {
    let root = PathBuf::from(
        std::env::var_os("RUST_COMMANDER_REPLACEMENT_FIXTURE").expect("parent fixture required"),
    );
    assert!(root.join("owned-recovery-fixture").is_file());
    let phase = std::env::var("RUST_COMMANDER_REPLACEMENT_PHASE").unwrap();
    let from = Location::Local(root.join("source"));
    let to = Location::Local(root.join("target"));
    let entries = filesystem::list(&from).unwrap();
    let watched = root.clone();
    let mut task = Task::new(Arc::new(AtomicBool::new(false)), move |p| {
        let stop = match phase.as_str() {
            "backup" => p.bytes > 0,
            "delete" => {
                !watched.join("target/tree/z").exists() && watched.join("target/tree/a").exists()
            }
            "publish" => watched.join("target/tree/new").exists(),
            _ => panic!("unknown crash phase"),
        };
        if stop {
            std::process::exit(73);
        }
    })
    .report_every(Duration::ZERO);
    resumable::run_resolved_in(
        &root.join("receipts"),
        "batch-process",
        &from,
        &entries,
        &to,
        replacement(false),
        &BTreeMap::new(),
        &mut task,
    )
    .unwrap();
    panic!("child should terminate at the selected recovery boundary");
}

#[test]
fn replacement_survives_process_termination() {
    for phase in ["backup", "delete", "publish"] {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("owned-recovery-fixture"), b"fixture").unwrap();
        fs::create_dir_all(root.path().join("source/tree")).unwrap();
        fs::create_dir_all(root.path().join("target/tree")).unwrap();
        fs::write(root.path().join("source/tree/new"), vec![71; 200_000]).unwrap();
        fs::write(root.path().join("target/tree/a"), vec![19; 200_000]).unwrap();
        fs::write(root.path().join("target/tree/z"), b"original z").unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "replacement_crash_child",
                "--ignored",
                "--nocapture",
            ])
            .env("RUST_COMMANDER_REPLACEMENT_FIXTURE", root.path())
            .env("RUST_COMMANDER_REPLACEMENT_PHASE", phase)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73), "{phase}");
        let from = Location::Local(root.path().join("source"));
        let to = Location::Local(root.path().join("target"));
        let entries = filesystem::list(&from).unwrap();
        resumable::run_resolved_in(
            &root.path().join("receipts"),
            "batch-process",
            &from,
            &entries,
            &to,
            replacement(false),
            &BTreeMap::new(),
            &mut Task::default(),
        )
        .unwrap();
        assert_eq!(
            fs::read(root.path().join("target/tree/new")).unwrap(),
            vec![71; 200_000]
        );
        assert_eq!(
            fs::read(root.path().join("receipts/batch-process/original-0/a")).unwrap(),
            vec![19; 200_000]
        );
        assert_eq!(
            fs::read(root.path().join("receipts/batch-process/original-0/z")).unwrap(),
            b"original z"
        );
        assert!(!root.path().join("target/tree/a").exists());
    }
}
fn target_path(location: &Location, relative: &str) -> PathBuf {
    match location {
        Location::Local(root) => root.join(relative),
        _ => PathBuf::from(relative),
    }
}
fn replacement(merge_folders: bool) -> TransferOptions {
    TransferOptions {
        conflicts: ConflictPolicy::Replace,
        merge_folders,
    }
}

#[test]
fn checkpoint_replacements_refuse_source_aliases_across_filesystem_locations() {
    for remote_source in [false, true] {
        for remote_target in [false, true] {
            let root = tempfile::tempdir().unwrap();
            fs::write(root.path().join("file"), b"source must survive").unwrap();
            let from = location(root.path(), remote_source);
            let to = location(root.path(), remote_target);
            let entries = filesystem::list(&from).unwrap();
            assert!(
                resumable::run_resolved_in(
                    &root.path().join("receipts"),
                    "batch-alias",
                    &from,
                    &entries,
                    &to,
                    replacement(false),
                    &BTreeMap::new(),
                    &mut Task::default()
                )
                .is_err()
            );
            assert_eq!(
                fs::read(root.path().join("file")).unwrap(),
                b"source must survive"
            );
        }
    }
    #[cfg(unix)]
    {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("source/folder/inner")).unwrap();
        std::os::unix::fs::symlink(
            root.path().join("source/folder/inner"),
            root.path().join("alias"),
        )
        .unwrap();
        let from = Location::Local(root.path().join("source"));
        let to = Location::Local(root.path().join("alias"));
        let entries = filesystem::list(&from).unwrap();
        let error = resumable::run_resolved_in(
            &root.path().join("receipts"),
            "batch-symlink",
            &from,
            &entries,
            &to,
            replacement(false),
            &BTreeMap::new(),
            &mut Task::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("source tree"));
        assert!(!root.path().join("source/folder/inner/folder").exists());
    }
}

#[test]
fn checkpoint_merge_preserves_fixed_per_file_decisions_and_originals_across_all_directions() {
    for remote_source in [false, true] {
        for remote_target in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source");
            let target = root.path().join("target");
            fs::create_dir_all(source.join("tree")).unwrap();
            fs::create_dir_all(target.join("tree")).unwrap();
            for name in ["new", "replace", "skip", "both"] {
                fs::write(source.join("tree").join(name), format!("new {name}")).unwrap();
            }
            for name in ["replace", "skip", "both", "extra"] {
                fs::write(target.join("tree").join(name), format!("old {name}")).unwrap();
            }
            let from = location(&source, remote_source);
            let to = location(&target, remote_target);
            let entries = filesystem::list(&from).unwrap();
            let overrides = BTreeMap::from([
                (target_path(&to, "tree/skip"), ConflictPolicy::Skip),
                (target_path(&to, "tree/both"), ConflictPolicy::Rename),
            ]);
            let checkpoints = root.path().join("receipts");
            let run = |task: &mut Task| {
                resumable::run_resolved_in(
                    &checkpoints,
                    "batch-merge",
                    &from,
                    &entries,
                    &to,
                    replacement(true),
                    &overrides,
                    task,
                )
            };
            let result = run(&mut Task::default()).unwrap();
            assert!(result.contains("Original destination items"));
            assert_eq!(
                fs::read(target.join("tree/replace")).unwrap(),
                b"new replace"
            );
            assert_eq!(fs::read(target.join("tree/new")).unwrap(), b"new new");
            assert_eq!(fs::read(target.join("tree/skip")).unwrap(), b"old skip");
            assert_eq!(fs::read(target.join("tree/both")).unwrap(), b"old both");
            assert_eq!(fs::read(target.join("tree/both (1)")).unwrap(), b"new both");
            assert_eq!(fs::read(target.join("tree/extra")).unwrap(), b"old extra");
            let receipt: serde_json::Value = serde_json::from_slice(
                &fs::read(checkpoints.join("batch-merge/manifest.json")).unwrap(),
            )
            .unwrap();
            let replaced = Path::new("tree").join("replace");
            let index = receipt["items"]
                .as_array()
                .unwrap()
                .iter()
                .position(|i| i["relative"].as_str().map(Path::new) == Some(replaced.as_path()))
                .expect("receipt must retain the replaced file's original contents");
            assert_eq!(
                fs::read(checkpoints.join(format!("batch-merge/original-{index}"))).unwrap(),
                b"old replace"
            );
            // A job can finish publishing before its durable queue record is
            // cleared. Replaying that recipe recognizes completed receipts.
            run(&mut Task::default()).unwrap();
            assert!(!target.join("tree/both (2)").exists());
            assert!(resumable::discard_in(&checkpoints, "batch-merge", &from, &to).is_err());
            assert!(
                checkpoints
                    .join(format!("batch-merge/original-{index}"))
                    .exists()
            );
        }
    }
}

#[test]
fn discard_cleans_empty_preflight_receipts_and_preserves_unrecognized_data() {
    let root = tempfile::tempdir().unwrap();
    let location = Location::Local(root.path().into());
    fs::create_dir(root.path().join("batch-empty")).unwrap();
    resumable::discard_in(root.path(), "batch-empty", &location, &location).unwrap();
    assert!(!root.path().join("batch-empty").exists());
    fs::create_dir(root.path().join("batch-review")).unwrap();
    fs::write(root.path().join("batch-review/original"), b"keep").unwrap();
    assert!(resumable::discard_in(root.path(), "batch-review", &location, &location).is_err());
    assert_eq!(
        fs::read(root.path().join("batch-review/original")).unwrap(),
        b"keep"
    );
}

#[test]
fn checkpoint_receipts_cannot_be_replayed_or_discarded_by_two_workers() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("source")).unwrap();
    fs::create_dir(root.path().join("target")).unwrap();
    fs::write(root.path().join("source/file"), vec![21; 200_000]).unwrap();
    let from = Location::Local(root.path().join("source"));
    let to = Location::Local(root.path().join("target"));
    let entries = filesystem::list(&from).unwrap();
    let receipts = root.path().join("receipts");
    let (ready_send, ready) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let (worker_root, worker_from, worker_to, worker_entries) =
        (receipts.clone(), from.clone(), to.clone(), entries.clone());
    let worker = std::thread::spawn(move || {
        let mut blocked = false;
        let mut task = Task::new(Arc::new(AtomicBool::new(false)), move |p| {
            if p.bytes > 0 && !blocked {
                blocked = true;
                ready_send.send(()).unwrap();
                released.recv_timeout(Duration::from_secs(10)).unwrap();
            }
        })
        .report_every(Duration::ZERO);
        resumable::run_in(
            &worker_root,
            "batch-exclusive",
            &worker_from,
            &worker_entries,
            &worker_to,
            &mut task,
        )
    });
    ready.recv_timeout(Duration::from_secs(10)).unwrap();
    let error = resumable::run_in(
        &receipts,
        "batch-exclusive",
        &from,
        &entries,
        &to,
        &mut Task::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("already in use"));
    assert!(resumable::discard_in(&receipts, "batch-exclusive", &from, &to).is_err());
    release.send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert_eq!(
        fs::read(root.path().join("target/file")).unwrap(),
        vec![21; 200_000]
    );
}

#[cfg(unix)]
#[test]
fn local_checkpoint_copy_preserves_file_and_directory_permissions_and_times() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("source/bin")).unwrap();
    fs::create_dir(root.path().join("target")).unwrap();
    let file = root.path().join("source/bin/run");
    fs::write(&file, b"executable bytes").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o751)).unwrap();
    fs::set_permissions(
        root.path().join("source/bin"),
        fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    let modified = filetime::FileTime::from_unix_time(1_600_000_000, 0);
    filetime::set_file_mtime(&file, modified).unwrap();
    filetime::set_file_mtime(root.path().join("source/bin"), modified).unwrap();
    let from = Location::Local(root.path().join("source"));
    let to = Location::Local(root.path().join("target"));
    resumable::run_in(
        &root.path().join("receipts"),
        "batch-metadata",
        &from,
        &filesystem::list(&from).unwrap(),
        &to,
        &mut Task::default(),
    )
    .unwrap();
    for (relative, mode) in [("bin", 0o750), ("bin/run", 0o751)] {
        let metadata = fs::metadata(root.path().join("target").join(relative)).unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, mode);
        assert_eq!(
            filetime::FileTime::from_last_modification_time(&metadata),
            modified
        );
    }
}

#[test]
fn keep_both_resume_uses_the_saved_name_and_refuses_a_changed_destination() {
    for changed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&target).unwrap();
        let bytes = vec![17; 200_000];
        fs::write(source.join("file.txt"), &bytes).unwrap();
        fs::write(target.join("file.txt"), b"original").unwrap();
        let from = location(&source, false);
        let to = location(&target, false);
        let entries = filesystem::list(&from).unwrap();
        let checkpoints = root.path().join("receipts");
        let options = TransferOptions {
            conflicts: ConflictPolicy::Rename,
            merge_folders: false,
        };
        let run = |task: &mut Task| {
            resumable::run_resolved_in(
                &checkpoints,
                "batch-names",
                &from,
                &entries,
                &to,
                options,
                &BTreeMap::new(),
                task,
            )
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let mut task = Task::new(cancel, move |p| {
            if p.bytes > 0 {
                stop.store(true, Ordering::Relaxed);
            }
        })
        .report_every(Duration::ZERO);
        assert!(run(&mut task).is_err());
        fs::write(target.join("file (2).txt"), b"another client").unwrap();
        if changed {
            fs::write(target.join("file (1).txt"), b"changed").unwrap();
        }
        let result = run(&mut Task::default());
        if changed {
            assert!(result.is_err());
            assert_eq!(fs::read(target.join("file (1).txt")).unwrap(), b"changed");
        } else {
            result.unwrap();
            assert_eq!(fs::read(target.join("file (1).txt")).unwrap(), bytes);
        }
        assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"original");
        assert_eq!(
            fs::read(target.join("file (2).txt")).unwrap(),
            b"another client"
        );
    }
}

#[test]
fn interrupted_directory_replacement_recovers_deletion_intents_and_checks_remaining_nodes() {
    for remote_target in [false, true] {
        for mutation in ["none", "file", "addition", "backup"] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source");
            let target = root.path().join("target");
            fs::create_dir_all(source.join("tree")).unwrap();
            fs::create_dir_all(target.join("tree")).unwrap();
            fs::write(source.join("tree/new"), b"replacement").unwrap();
            fs::write(target.join("tree/a"), b"original a").unwrap();
            fs::write(target.join("tree/z"), b"original z").unwrap();
            let from = location(&source, false);
            let to = location(&target, remote_target);
            let entries = filesystem::list(&from).unwrap();
            let checkpoints = root.path().join("receipts");
            let run = |task: &mut Task| {
                resumable::run_resolved_in(
                    &checkpoints,
                    "batch-replace",
                    &from,
                    &entries,
                    &to,
                    replacement(false),
                    &BTreeMap::new(),
                    task,
                )
            };
            let cancel = Arc::new(AtomicBool::new(false));
            let stop = cancel.clone();
            let watched = target.join("tree/z");
            let mut task = Task::new(cancel, move |_| {
                if !watched.exists() {
                    stop.store(true, Ordering::Relaxed);
                }
            })
            .report_every(Duration::ZERO);
            assert!(run(&mut task).is_err());
            assert!(target.join("tree/a").exists());
            assert!(!target.join("tree/z").exists());
            assert_eq!(
                fs::read(checkpoints.join("batch-replace/original-0/z")).unwrap(),
                b"original z"
            );
            match mutation {
                "file" => fs::write(target.join("tree/a"), b"externally changed").unwrap(),
                "addition" => fs::write(target.join("tree/added"), b"external addition").unwrap(),
                "backup" => {
                    fs::write(checkpoints.join("batch-replace/original-0/z"), b"corrupt").unwrap()
                }
                _ => (),
            }
            let result = run(&mut Task::default());
            if mutation == "none" {
                result.unwrap();
                assert_eq!(fs::read(target.join("tree/new")).unwrap(), b"replacement");
                assert!(!target.join("tree/a").exists());
            } else {
                assert!(result.is_err(), "{mutation}");
                assert!(target.join("tree/a").exists());
                assert!(!target.join("tree/new").exists());
            }
            assert_eq!(fs::read(source.join("tree/new")).unwrap(), b"replacement");
        }
    }
}
