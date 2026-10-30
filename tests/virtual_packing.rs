use rust_commander::{
    archive,
    drag_drop::Selection,
    filesystem::{self, Location},
    history,
    operations::{self, Operation, Task},
    packing,
    queue::Queue,
    remote,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn mount(root: &Path) -> Location {
    let connection = remote::Connection {
        service: "fs".into(),
        root: root.display().to_string(),
        ..Default::default()
    };
    remote::connect(&connection, Default::default(), false).unwrap();
    Location::Remote {
        connection: connection.id,
        path: PathBuf::new(),
    }
}
fn selection(from: Location) -> Selection {
    Selection {
        entries: filesystem::list(&from).unwrap(),
        from,
    }
}
fn pack(selection: Selection, destination: PathBuf) -> Operation {
    Operation::PackSelection {
        selection,
        destination,
        compression_level: Some(0),
        comment: String::new(),
        background: true,
    }
}

#[test]
fn remote_packing_preserves_sources_and_journals_only_the_finished_archive() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("remote");
    fs::create_dir_all(source.join("folder/nested")).unwrap();
    fs::write(source.join("café.txt"), b"file contents").unwrap();
    fs::write(source.join("folder/nested/data.bin"), [0, 128, 255]).unwrap();
    let from = mount(&source);
    let destination = root.path().join("output.zip");
    let op = pack(selection(from.clone()), destination.clone());
    let encoded = serde_json::to_string(&op).unwrap();
    let restored: Operation = serde_json::from_str(&encoded).unwrap();
    assert!(!Queue::requires_input(&restored));
    let journal = root.path().join("history");
    let record = history::execute_in(restored, &mut Task::default(), &journal).unwrap();
    assert!(record.success, "{}", record.result);
    assert_eq!(
        record.changes.len(),
        1,
        "Private staging paths must not enter Undo"
    );
    assert_eq!(record.changes[0].path, destination.canonicalize().unwrap());
    assert_eq!(
        archive::read_bytes(&destination, Path::new("café.txt"), 100).unwrap(),
        b"file contents"
    );
    assert_eq!(
        archive::read_bytes(&destination, Path::new("folder/nested/data.bin"), 100).unwrap(),
        [0, 128, 255]
    );
    assert_eq!(fs::read(source.join("café.txt")).unwrap(), b"file contents");
    assert_eq!(
        fs::read(source.join("folder/nested/data.bin")).unwrap(),
        [0, 128, 255]
    );
    operations::run(Operation::Undo(Box::new(record)), &mut Task::default()).unwrap();
    assert!(!destination.exists());
    assert_eq!(filesystem::list(&from).unwrap().len(), 2);
}

#[test]
fn archive_packing_respects_the_current_prefix_and_format_options() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("folder");
    fs::create_dir_all(folder.join("nested")).unwrap();
    fs::write(folder.join("nested/report.txt"), b"inside").unwrap();
    fs::write(folder.join("excluded.txt"), b"outside selection").unwrap();
    let original = root.path().join("source.zip");
    archive::pack(&[folder], &original, &mut Task::default()).unwrap();
    let original_bytes = fs::read(&original).unwrap();
    let chosen = selection(Location::Archive {
        file: original.clone(),
        prefix: "folder/nested".into(),
    });
    for extension in ["zip", "7z", "tar.gz", "gz"] {
        let destination = root.path().join(format!("packed.{extension}"));
        let mut op = pack(chosen.clone(), destination.clone());
        if let Operation::PackSelection { comment, .. } = &mut op
            && extension == "zip"
        {
            *comment = "reviewed comment".into();
        }
        operations::run(op, &mut Task::default()).unwrap();
        let output = root.path().join(format!("extracted-{extension}"));
        archive::extract(
            &destination,
            &[],
            Path::new(""),
            &output,
            &mut Task::default(),
        )
        .unwrap();
        let files = fs::read_dir(&output)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(files.len(), 1);
        assert_eq!(fs::read(&files[0]).unwrap(), b"inside");
        if extension == "zip" {
            assert_eq!(
                zip::ZipArchive::new(fs::File::open(destination).unwrap())
                    .unwrap()
                    .comment(),
                b"reviewed comment"
            );
        }
    }
    assert_eq!(fs::read(original).unwrap(), original_bytes);
}

#[test]
fn virtual_packing_refuses_conflicts_unsafe_paths_stream_folders_and_cancellation() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("remote");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("large.bin"), vec![7; 1024 * 1024]).unwrap();
    let chosen = selection(mount(&source));
    let destination = root.path().join("packed.zip");
    fs::write(&destination, b"existing archive").unwrap();
    assert!(
        operations::run(
            pack(chosen.clone(), destination.clone()),
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&destination).unwrap(), b"existing archive");
    fs::remove_file(&destination).unwrap();
    let mut unsafe_selection = chosen.clone();
    unsafe_selection.entries[0].path = "../large.bin".into();
    assert!(
        operations::run(
            pack(unsafe_selection, destination.clone()),
            &mut Task::default()
        )
        .is_err()
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let mut task = Task::new(cancel, move |progress| {
        if progress.bytes > 0 {
            worker_cancel.store(true, Ordering::Relaxed);
        }
    })
    .report_every(Duration::ZERO);
    assert!(operations::run(pack(chosen.clone(), destination.clone()), &mut task).is_err());
    assert!(!destination.exists());
    assert_eq!(
        fs::metadata(source.join("large.bin")).unwrap().len(),
        1024 * 1024
    );
    let mut folder = chosen;
    folder.entries[0].is_dir = true;
    let options = packing::Options {
        format: archive::Format::Gzip,
        ..Default::default()
    };
    assert!(options.validate_selection(&folder).is_err());
}
