use rust_commander::{
    external_edit,
    filesystem::Location,
    operations::{self, Operation, Task},
    queue::Queue,
    remote,
};
use std::{fs, path::Path};

#[test]
fn external_remote_save_accepts_32_mib_and_retains_a_verified_recovery_copy() {
    let root = tempfile::tempdir().unwrap();
    let original = vec![1; external_edit::LIMIT as usize];
    fs::write(root.path().join("large.bin"), &original).unwrap();
    let profile = remote::Connection {
        service: "fs".into(),
        root: root.path().display().to_string(),
        ..Default::default()
    };
    remote::connect(&profile, Default::default(), false).unwrap();
    let session = external_edit::Session::prepare(
        Location::Remote {
            connection: profile.id.clone(),
            path: "".into(),
        },
        "large.bin".into(),
    )
    .unwrap();
    let edited = vec![2; external_edit::LIMIT as usize];
    fs::write(&session.file, &edited).unwrap();
    let op = session.save_operation().unwrap();
    assert!(matches!(op, Operation::RemoteExternalSave { .. }));
    assert!(Queue::requires_input(&op));
    let serialized = serde_json::to_value(&op).unwrap();
    assert!(serialized["RemoteExternalSave"].get("original").is_none());
    assert!(serialized["RemoteExternalSave"].get("bytes").is_none());
    operations::run(op, &mut Task::default()).unwrap();
    assert_eq!(fs::read(root.path().join("large.bin")).unwrap(), edited);
    let backup = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|e| e == "bak"))
        .unwrap();
    assert_eq!(fs::read(backup).unwrap(), original);
    // The built-in editor retains its own smaller bound.
    assert!(
        remote::save_bytes(
            &profile.id,
            Path::new("large.bin"),
            &edited,
            b"small",
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(root.path().join("large.bin")).unwrap(), edited);
}

#[test]
fn external_remote_save_rejects_changed_original_oversize_and_cancelled_work() {
    let root = tempfile::tempdir().unwrap();
    let original = vec![3; 3 * 1024 * 1024];
    let path = root.path().join("file.bin");
    fs::write(&path, &original).unwrap();
    let profile = remote::Connection {
        service: "fs".into(),
        root: root.path().display().to_string(),
        ..Default::default()
    };
    remote::connect(&profile, Default::default(), false).unwrap();
    let session = external_edit::Session::prepare(
        Location::Remote {
            connection: profile.id,
            path: "".into(),
        },
        "file.bin".into(),
    )
    .unwrap();
    fs::write(&session.file, b"new contents").unwrap();
    fs::write(&path, b"changed by another writer").unwrap();
    assert!(operations::run(session.save_operation().unwrap(), &mut Task::default()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"changed by another writer");
    fs::write(&path, &original).unwrap();
    let mut task = Task::new(
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        |_| {},
    );
    assert!(operations::run(session.save_operation().unwrap(), &mut task).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::File::create(&session.file)
        .unwrap()
        .set_len(external_edit::LIMIT + 1)
        .unwrap();
    assert!(session.save_operation().is_err());
    assert_eq!(
        fs::read_dir(root.path()).unwrap().count(),
        1,
        "No rejected save should create a recovery copy"
    );
}
