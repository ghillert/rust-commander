//! These tests require scripts/test-remote.py. They never use saved profiles.
use rust_commander::{
    filesystem::{self, Location},
    operations::Task,
    remote::{self, Connection, Secrets},
    resumable,
    sync::{self, Choice},
    transfer::{ConflictPolicy, TransferOptions},
};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn profile(service: &str) -> Connection {
    let path = std::env::var_os("RUST_COMMANDER_TEST_SERVICES")
        .expect("Run this test through scripts/test-remote.py");
    let mut profiles: BTreeMap<String, Connection> =
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let connection = profiles.remove(service).unwrap();
    let url = url::Url::parse(&connection.endpoint).unwrap();
    assert_eq!(
        url.host_str(),
        Some("127.0.0.1"),
        "Test services must be loopback-only"
    );
    connection
}
fn location(connection: &Connection, path: &Path) -> Location {
    Location::Remote {
        connection: connection.id.clone(),
        path: path.into(),
    }
}
fn contents(connection: &Connection, path: &Path) -> Vec<u8> {
    remote::read_bytes(&connection.id, path, 4 * 1024 * 1024).unwrap()
}
fn exercise(service: &str) {
    println!("{service}: connecting");
    let connection = profile(service);
    let secret = Secrets {
        password: "commander-test-only".into(),
        ..Default::default()
    };
    remote::connect(&connection, secret, false).unwrap();
    println!("{service}: authentication refusal");
    if service != "sftp" {
        let mut bad = connection.clone();
        bad.id.push_str("-invalid-auth");
        let rejected = remote::connect(
            &bad,
            Secrets {
                password: "invalid-private-fixture-password".into(),
                ..Default::default()
            },
            false,
        );
        let message = rejected
            .expect_err("Incorrect credentials must be rejected")
            .to_string();
        assert!(!message.contains("invalid-private-fixture-password"));
    }
    let work = tempfile::tempdir().unwrap();
    let base = std::path::PathBuf::from(format!("case-{:016x}", rand::random::<u64>()));
    remote::mkdir(&connection.id, &base).unwrap();
    let destination = location(&connection, &base);
    let local = work.path().join("source");
    fs::create_dir_all(local.join("tree/empty")).unwrap();
    fs::write(local.join("tree/café notes.txt"), b"original content").unwrap();
    let large = vec![0x53; 1024 * 1024 + 137];
    fs::write(local.join("large.bin"), &large).unwrap();
    let from = Location::Local(local.clone());
    remote::transfer(
        &from,
        &filesystem::list(&from).unwrap(),
        &destination,
        false,
        &mut Task::default(),
    )
    .unwrap();
    let original = base.join("tree/café notes.txt");
    assert_eq!(contents(&connection, &original), b"original content");
    let mut range = remote::read_range(&connection.id, &base.join("large.bin"), 65_531).unwrap();
    let mut tail = Vec::new();
    range.read_to_end(&mut tail).unwrap();
    assert_eq!(tail, large[65_531..]);
    drop(range);

    println!("{service}: initial upload and range read passed");
    // The merge skips an existing child but still creates new descendants.
    fs::write(local.join("tree/café notes.txt"), b"replacement").unwrap();
    fs::write(local.join("tree/new.txt"), b"merged child").unwrap();
    let tree = filesystem::entry_from_path(&local.join("tree")).unwrap();
    assert!(
        !remote::conflicts_with_merge(&from, std::slice::from_ref(&tree), &destination, true)
            .unwrap()
            .is_empty()
    );
    remote::transfer_resolved(
        &from,
        &[tree],
        &destination,
        false,
        TransferOptions {
            conflicts: ConflictPolicy::Skip,
            merge_folders: true,
        },
        &BTreeMap::new(),
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(contents(&connection, &original), b"original content");
    assert_eq!(
        contents(&connection, &base.join("tree/new.txt")),
        b"merged child"
    );
    assert!(
        remote::save_bytes(
            &connection.id,
            &original,
            b"stale original",
            b"not saved",
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(contents(&connection, &original), b"original content");

    println!("{service}: recursive merge and guarded save passed");
    // Cancel a real streamed download and retry its durable, verified partial.
    let download = work.path().join("download");
    fs::create_dir(&download).unwrap();
    let to = Location::Local(download.clone());
    let chosen = remote::list(&connection.id, &base)
        .unwrap()
        .into_iter()
        .filter(|e| e.name == "large.bin")
        .collect::<Vec<_>>();
    assert_eq!(chosen.len(), 1);
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let mut interrupted = Task::new(cancel, move |progress| {
        if progress.bytes > 0 {
            stop.store(true, Ordering::Relaxed);
        }
    });
    interrupted = interrupted.report_every(Duration::ZERO);
    let checkpoints = work.path().join("checkpoints");
    let error = resumable::run_in(
        &checkpoints,
        "batch-download",
        &destination,
        &chosen,
        &to,
        &mut interrupted,
    )
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"), "{error:#}");
    assert!(!download.join("large.bin").exists());
    let partial = checkpoints.join("batch-download/0.part");
    assert!(partial.metadata().unwrap().len() > 0);
    assert!(partial.metadata().unwrap().len() < large.len() as u64);
    resumable::run_in(
        &checkpoints,
        "batch-download",
        &destination,
        &chosen,
        &to,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read(download.join("large.bin")).unwrap(), large);
    assert!(!checkpoints.join("batch-download").exists());

    println!("{service}: download cancellation/resume passed");
    // Upload through the same verified staging/publish path, then refuse an
    // unrelated existing destination even when the source has the same name.
    let upload_root = base.join("uploads");
    remote::mkdir(&connection.id, &upload_root).unwrap();
    let upload = location(&connection, &upload_root);
    let entry = filesystem::entry_from_path(&download.join("large.bin")).unwrap();
    resumable::run_in(
        &checkpoints,
        "batch-upload",
        &to,
        std::slice::from_ref(&entry),
        &upload,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(contents(&connection, &upload_root.join("large.bin")), large);
    assert!(
        resumable::run_in(
            &checkpoints,
            "batch-conflict",
            &to,
            &[entry],
            &upload,
            &mut Task::default()
        )
        .is_err()
    );

    println!("{service}: checkpoint publish/conflict passed");
    // Review and apply a local/remote sync, then reject a stale preview.
    let sync_local = work.path().join("sync");
    fs::create_dir(&sync_local).unwrap();
    fs::write(sync_local.join("a.txt"), b"synchronized").unwrap();
    let left = Location::Local(sync_local.clone());
    let sync_path = base.join("sync");
    remote::mkdir(&connection.id, &sync_path).unwrap();
    let right = location(&connection, &sync_path);
    let plan = sync::preview_locations(&left, &right, 1, &Task::default()).unwrap();
    assert!(plan.items.iter().any(|i| i.choice == Choice::LeftToRight));
    sync::apply(&plan, &mut Task::default()).unwrap();
    assert_eq!(
        contents(&connection, &sync_path.join("a.txt")),
        b"synchronized"
    );
    fs::write(sync_local.join("a.txt"), b"reviewed").unwrap();
    let mut stale = sync::preview_locations(&left, &right, 1, &Task::default()).unwrap();
    stale.items[0].choice = Choice::LeftToRight;
    fs::write(sync_local.join("a.txt"), b"external edit").unwrap();
    assert!(sync::apply(&stale, &mut Task::default()).is_err());
    assert_eq!(
        contents(&connection, &sync_path.join("a.txt")),
        b"synchronized"
    );

    println!("{service}: synchronization/stale-plan passed");
    // Exercise remote-to-remote copy and guarded rename/delete as well.
    remote::rename(&connection.id, &original, "renamed.txt").unwrap();
    let chosen = remote::list(&connection.id, &base.join("tree"))
        .unwrap()
        .into_iter()
        .filter(|e| e.name == "renamed.txt")
        .collect::<Vec<_>>();
    remote::transfer(
        &location(&connection, &base.join("tree")),
        &chosen,
        &right,
        false,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(
        contents(&connection, &sync_path.join("renamed.txt")),
        b"original content"
    );
    let folder = remote::list(&connection.id, Path::new(""))
        .unwrap()
        .into_iter()
        .find(|e| e.path == base)
        .unwrap();
    remote::delete(&connection.id, &[folder], &mut Task::default()).unwrap();
    assert!(
        !remote::list(&connection.id, Path::new(""))
            .unwrap()
            .iter()
            .any(|e| e.path == base)
    );
    println!(
        "{service}: auth, Unicode, range, merge, cancellation/resume, publish, conflict, sync, stale-plan, rename and delete passed"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable OpenSSH; run scripts/test-remote.py"]
fn sftp_protocol_workflows() {
    exercise("sftp");
}
#[test]
#[ignore = "requires disposable FTP server; run scripts/test-remote.py"]
fn ftp_protocol_workflows() {
    exercise("ftp");
}
#[test]
#[ignore = "requires disposable WsgiDAV; run scripts/test-remote.py"]
fn webdav_protocol_workflows() {
    exercise("webdav");
}
#[test]
#[ignore = "requires disposable Samba; run scripts/test-remote.py"]
fn smb_protocol_workflows() {
    exercise("smb");
}
