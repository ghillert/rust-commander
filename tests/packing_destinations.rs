use rust_commander::{
    archive,
    drag_drop::Selection,
    filesystem::{self, Location},
    operations::{Operation, Task},
    packing::{self, Options},
    queue::Queue,
    remote::{self, Connection, Secrets},
};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn remote(root: &Path) -> Connection {
    let connection = Connection {
        service: "fs".into(),
        root: root.display().to_string(),
        ..Default::default()
    };
    remote::connect(&connection, Secrets::default(), false).unwrap();
    connection
}
fn selection(from: Location) -> Selection {
    Selection {
        entries: filesystem::list(&from).unwrap(),
        from,
    }
}
#[test]
fn remote_output_is_verified_and_existing_archives_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let output = root.path().join("output");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(source.join("café.txt"), b"contents").unwrap();
    let connection = remote(&output);
    let selection = selection(Location::Local(source));
    let target = Location::Remote {
        connection: connection.id.clone(),
        path: "result.zip".into(),
    };
    packing::pack_to(
        &selection,
        &target,
        &Options::default(),
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(
        archive::read_bytes(&output.join("result.zip"), Path::new("café.txt"), 100).unwrap(),
        b"contents"
    );
    let existing = output.join("result.zip");
    assert_eq!(
        packing::destination(
            &existing.display().to_string(),
            root.path(),
            &Options::default(),
            &[]
        )
        .unwrap(),
        Location::Local(existing)
    );
    let before = fs::read(output.join("result.zip")).unwrap();
    assert!(
        packing::pack_to(
            &selection,
            &target,
            &Options::default(),
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(output.join("result.zip")).unwrap(), before);
    assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
    let parsed = packing::destination(
        &format!("remote://{}/next.zip", connection.id),
        root.path(),
        &Options::default(),
        &[connection],
    )
    .unwrap();
    assert!(matches!(parsed,Location::Remote{path,..} if path==Path::new("next.zip")));
}
#[test]
fn remote_and_archive_sources_can_be_encrypted_without_persisting_passwords() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("report.txt"), b"secret fixture contents").unwrap();
    let connection = remote(&source);
    let remote_selection = selection(Location::Remote {
        connection: connection.id,
        path: PathBuf::new(),
    });
    for extension in ["zip", "7z"] {
        let output = root.path().join(format!("protected.{extension}"));
        let options = Options {
            encrypted: true,
            password: "fixture-only-password".into(),
            format: archive::format(&output).unwrap(),
            ..Default::default()
        };
        packing::pack_to(
            &remote_selection,
            &Location::Local(output.clone()),
            &options,
            &mut Task::default(),
        )
        .unwrap();
        archive::unlock(&output, &options.password).unwrap();
        assert_eq!(
            archive::read_bytes(&output, Path::new("report.txt"), 100).unwrap(),
            b"secret fixture contents"
        );
    }
    let archive_path = root.path().join("source.zip");
    archive::pack(
        &[source.join("report.txt")],
        &archive_path,
        &mut Task::default(),
    )
    .unwrap();
    let archived = selection(Location::Archive {
        file: archive_path,
        prefix: PathBuf::new(),
    });
    let options = Options {
        encrypted: true,
        password: "fixture-only-password".into(),
        comment: "encrypted ZIP comment".into(),
        ..Default::default()
    };
    let output = root.path().join("from-archive.zip");
    packing::pack_to(
        &archived,
        &Location::Local(output.clone()),
        &options,
        &mut Task::default(),
    )
    .unwrap();
    let zip = zip::ZipArchive::new(fs::File::open(&output).unwrap()).unwrap();
    assert_eq!(zip.comment(), options.comment.as_bytes());
    archive::unlock(&output, &options.password).unwrap();
    assert_eq!(
        archive::read_bytes(&output, Path::new("report.txt"), 100).unwrap(),
        b"secret fixture contents"
    );
    let operation = Operation::PackTo {
        selection: archived,
        target: Location::Local(output),
        compression_level: None,
        comment: options.comment,
        encrypted: true,
        password: options.password,
        background: false,
    };
    let serialized = serde_json::to_string(&operation).unwrap();
    assert!(!serialized.contains("fixture-only-password"));
    let restored: Operation = serde_json::from_str(&serialized).unwrap();
    assert!(Queue::requires_input(&restored));
}
#[test]
fn cancelled_remote_pack_does_not_publish_or_leave_temporary_remote_files() {
    use std::sync::{Arc, atomic::AtomicBool};
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let output = root.path().join("output");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(source.join("file"), vec![42; 1024]).unwrap();
    let connection = remote(&output);
    let selected = selection(Location::Local(source));
    let mut task = Task::new(Arc::new(AtomicBool::new(true)), |_| {});
    assert!(
        packing::pack_to(
            &selected,
            &Location::Remote {
                connection: connection.id,
                path: "cancelled.zip".into()
            },
            &Options::default(),
            &mut task
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(output).unwrap().count(), 0);
}

#[test]
fn cancellation_during_remote_verification_cleans_staging_and_preserves_a_racing_destination() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let output = root.path().join("output");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&output).unwrap();
    fs::write(source.join("file"), vec![42; 256 * 1024]).unwrap();
    let connection = remote(&output);
    let selected = selection(Location::Local(source));
    for racing in [false, true] {
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = cancelled.clone();
        let destination = output.join("result.zip");
        let mut task = Task::new(cancelled, move |progress| {
            if progress.current == "result.zip" {
                if racing {
                    fs::write(&destination, b"concurrent writer").unwrap();
                }
                signal.store(true, Ordering::Relaxed);
            }
        })
        .report_every(std::time::Duration::ZERO);
        let result = packing::pack_to(
            &selected,
            &Location::Remote {
                connection: connection.id.clone(),
                path: "result.zip".into(),
            },
            &Options::default(),
            &mut task,
        );
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert_eq!(fs::read_dir(&output).unwrap().count(), usize::from(racing));
        if racing {
            assert_eq!(
                fs::read(output.join("result.zip")).unwrap(),
                b"concurrent writer"
            );
        }
    }
}
