use rust_commander::{
    archive,
    filesystem::{self, Entry, Location, Sort},
    operations::{self, Operation, Task},
};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::SystemTime,
};
use tempfile::TempDir;
use zip::{ZipWriter, write::SimpleFileOptions};

fn write(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
}
fn run(operation: Operation) -> anyhow::Result<String> {
    operations::run(operation, &mut Task::default())
}
fn fixture() -> (TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    (root, source, destination)
}

#[test]
fn recursive_copy_preserves_contents_and_empty_directories() {
    let (_root, source, destination) = fixture();
    let folder = source.join("folder");
    fs::create_dir_all(folder.join("empty")).unwrap();
    write(&folder.join("hello.txt"), "hello μCommander");
    run(Operation::Copy {
        sources: vec![folder.clone()],
        destination: destination.clone(),
    })
    .unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("folder/hello.txt")).unwrap(),
        "hello μCommander"
    );
    assert!(destination.join("folder/empty").is_dir());
    assert!(folder.exists());
}

#[test]
fn conflict_rejects_whole_batch_before_writing() {
    let (_root, source, destination) = fixture();
    write(&source.join("a"), "new");
    write(&source.join("b"), "new");
    write(&destination.join("b"), "original");
    assert!(
        run(Operation::Copy {
            sources: vec![source.join("a"), source.join("b")],
            destination: destination.clone()
        })
        .is_err()
    );
    assert!(!destination.join("a").exists());
    assert_eq!(
        fs::read_to_string(destination.join("b")).unwrap(),
        "original"
    );
}

#[test]
fn copy_and_move_into_self_are_rejected_even_through_symlink() {
    let (_root, source, _) = fixture();
    fs::create_dir(source.join("child")).unwrap();
    assert!(operations::preflight(std::slice::from_ref(&source), &source.join("child")).is_err());
    #[cfg(unix)]
    {
        let link = source.parent().unwrap().join("alias");
        std::os::unix::fs::symlink(source.join("child"), &link).unwrap();
        assert!(operations::preflight(&[source], &link).is_err());
    }
}

#[test]
fn move_removes_original_only_after_copy_succeeds() {
    let (_root, source, destination) = fixture();
    write(&source.join("a"), "hello");
    run(Operation::Move {
        sources: vec![source.join("a")],
        destination: destination.clone(),
    })
    .unwrap();
    assert!(!source.join("a").exists());
    assert_eq!(fs::read_to_string(destination.join("a")).unwrap(), "hello");
}

#[test]
fn failed_move_leaves_original_and_existing_target() {
    let (_root, source, destination) = fixture();
    write(&source.join("a"), "new");
    write(&destination.join("a"), "old");
    assert!(
        run(Operation::Move {
            sources: vec![source.join("a")],
            destination: destination.clone()
        })
        .is_err()
    );
    assert_eq!(fs::read_to_string(source.join("a")).unwrap(), "new");
    assert_eq!(fs::read_to_string(destination.join("a")).unwrap(), "old");
}

#[test]
fn rename_refuses_conflicts_and_parent_traversal() {
    let (_root, source, _) = fixture();
    write(&source.join("a"), "one");
    write(&source.join("b"), "two");
    for name in ["b", "../escaped", "a/b", "..", ""] {
        assert!(
            run(Operation::Rename {
                source: source.join("a"),
                name: name.into()
            })
            .is_err()
        );
    }
    run(Operation::Rename {
        source: source.join("a"),
        name: "renamed".into(),
    })
    .unwrap();
    assert!(!source.join("a").exists());
    assert_eq!(fs::read_to_string(source.join("renamed")).unwrap(), "one");
}

#[test]
fn cancellation_keeps_source_and_creates_no_output() {
    let (_root, source, destination) = fixture();
    write(&source.join("a"), "hello");
    let mut task = Task::new(Arc::new(AtomicBool::new(true)), |_| {});
    assert!(
        operations::run(
            Operation::Move {
                sources: vec![source.join("a")],
                destination: destination.clone()
            },
            &mut task
        )
        .is_err()
    );
    assert!(source.join("a").exists());
    assert!(!destination.join("a").exists());
}

#[test]
fn cancelling_after_directory_copy_starts_removes_incomplete_output() {
    let (_root, source, destination) = fixture();
    let folder = source.join("folder");
    fs::create_dir(&folder).unwrap();
    write(&folder.join("a"), "hello");
    let cancel = Arc::new(AtomicBool::new(false));
    let report_cancel = cancel.clone();
    let mut task = Task::new(cancel, move |_| {
        report_cancel.store(true, std::sync::atomic::Ordering::Relaxed)
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        operations::run(
            Operation::Copy {
                sources: vec![folder.clone()],
                destination: destination.clone()
            },
            &mut task
        )
        .is_err()
    );
    assert!(folder.join("a").exists());
    assert!(!destination.join("folder").exists());
}

#[test]
fn move_does_not_delete_a_source_that_changes_during_transfer() {
    let (_root, source, destination) = fixture();
    let file = source.join("a");
    write(&file, "original");
    let modified = file.clone();
    let mut task = Task::new(Arc::new(AtomicBool::new(false)), move |_| {
        write(&modified, "externally changed")
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        operations::run(
            Operation::Move {
                sources: vec![file.clone()],
                destination: destination.clone()
            },
            &mut task
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(file).unwrap(), "externally changed");
    assert!(destination.join("a").exists());
}

#[test]
fn oversized_tar_is_rejected_before_creating_an_extraction_folder() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("huge.tar");
    let mut header = tar::Header::new_gnu();
    header.set_path("huge").unwrap();
    header.set_size(11 * 1024 * 1024 * 1024);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    File::create(&output)
        .unwrap()
        .write_all(header.as_bytes())
        .unwrap();
    let destination = root.path().join("extracted");
    let error = archive::extract(
        &output,
        &[],
        Path::new(""),
        &destination,
        &mut Task::default(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("extraction limit"));
    assert!(!destination.exists());
}

#[cfg(unix)]
#[test]
fn archives_preserve_executable_permissions_without_special_bits() {
    use std::os::unix::fs::PermissionsExt;
    for name in ["test.zip", "test.tar", "test.tgz"] {
        let (root, source, _) = fixture();
        let file = source.join("tool");
        write(&file, "#!/bin/sh\nexit 0\n");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o751)).unwrap();
        let output = root.path().join(name);
        archive::pack(&[file], &output, &mut Task::default()).unwrap();
        let extracted = root.path().join("extracted");
        archive::extract(
            &output,
            &[],
            Path::new(""),
            &extracted,
            &mut Task::default(),
        )
        .unwrap();
        assert_eq!(
            fs::metadata(extracted.join("tool"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o751
        );
    }
}

#[cfg(unix)]
#[test]
fn copy_preserves_symlinks_without_following_them() {
    let (_root, source, destination) = fixture();
    let folder = source.join("folder");
    fs::create_dir(&folder).unwrap();
    std::os::unix::fs::symlink("..", folder.join("loop")).unwrap();
    std::os::unix::fs::symlink("missing", folder.join("broken")).unwrap();
    run(Operation::Copy {
        sources: vec![folder],
        destination: destination.clone(),
    })
    .unwrap();
    assert_eq!(
        fs::read_link(destination.join("folder/loop")).unwrap(),
        Path::new("..")
    );
    assert_eq!(
        fs::read_link(destination.join("folder/broken")).unwrap(),
        Path::new("missing")
    );
}

#[test]
fn all_supported_archives_round_trip_nested_files() {
    for name in ["test.zip", "test.tar", "test.tar.gz", "test.tgz"] {
        let (root, source, _) = fixture();
        fs::create_dir_all(source.join("nested/empty")).unwrap();
        write(&source.join("nested/μ.txt"), "archive contents");
        let output = root.path().join(name);
        archive::pack(std::slice::from_ref(&source), &output, &mut Task::default()).unwrap();
        let listing = archive::list(&output, Path::new("")).unwrap();
        assert_eq!(listing.len(), 1);
        assert!(listing[0].is_dir);
        let nested = archive::list(&output, Path::new("source/nested")).unwrap();
        assert_eq!(nested.len(), 2);
        assert_eq!(
            archive::read_text(&output, Path::new("source/nested/μ.txt"), 1024).unwrap(),
            "archive contents"
        );
        let extracted = root.path().join("extracted");
        archive::extract(
            &output,
            &[],
            Path::new(""),
            &extracted,
            &mut Task::default(),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(extracted.join("source/nested/μ.txt")).unwrap(),
            "archive contents"
        );
        assert!(extracted.join("source/nested/empty").is_dir());
    }
}

#[test]
fn extract_selection_is_relative_to_current_archive_folder() {
    let (root, source, _) = fixture();
    write(&source.join("a"), "one");
    write(&source.join("b"), "two");
    let output = root.path().join("test.zip");
    archive::pack(&[source], &output, &mut Task::default()).unwrap();
    let extracted = root.path().join("extracted");
    archive::extract(
        &output,
        &[PathBuf::from("source/a")],
        Path::new("source"),
        &extracted,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(extracted.join("a")).unwrap(), "one");
    assert!(!extracted.join("b").exists());
}

fn zip_with_members(path: &Path, names: &[&str]) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    for name in names {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(b"hello").unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn zip_slip_and_windows_path_traversal_are_rejected_before_extraction() {
    for name in [
        "../escaped",
        "/absolute",
        "C:/escape",
        "..\\escape",
        "source\\nested",
        "C:\\escape",
        "\\\\server\\share",
        "safe/../../escaped",
    ] {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("bad.zip");
        zip_with_members(&archive, &["safe", name]);
        let target = root.path().join("output");
        assert!(
            archive::extract(&archive, &[], Path::new(""), &target, &mut Task::default()).is_err(),
            "{name}"
        );
        assert!(!target.exists());
        assert!(!root.path().join("escaped").exists());
    }
}

#[test]
fn duplicate_zip_members_cannot_overwrite_and_failed_extraction_is_cleaned_up() {
    let root = tempfile::tempdir().unwrap();
    let archive = root.path().join("duplicates.zip");
    // ZIP writers reject duplicates, so use two different names with the same normalized path.
    zip_with_members(&archive, &["a", "./a"]);
    let target = root.path().join("output");
    assert!(archive::extract(&archive, &[], Path::new(""), &target, &mut Task::default()).is_err());
    assert!(!target.exists());
}

#[test]
fn archive_does_not_replace_existing_destination() {
    let (root, source, destination) = fixture();
    write(&source.join("a"), "hello");
    let output = root.path().join("test.zip");
    archive::pack(std::slice::from_ref(&source), &output, &mut Task::default()).unwrap();
    let original = fs::read(&output).unwrap();
    assert!(archive::pack(&[source], &output, &mut Task::default()).is_err());
    assert_eq!(fs::read(&output).unwrap(), original);
    write(&destination.join("keep"), "original");
    assert!(
        archive::extract(
            &output,
            &[],
            Path::new(""),
            &destination,
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(destination.join("keep")).unwrap(),
        "original"
    );
}

#[test]
fn packing_inside_source_is_rejected() {
    let (_root, source, _) = fixture();
    let output = source.join("recursive.zip");
    assert!(archive::pack(&[source], &output, &mut Task::default()).is_err());
    assert!(!output.exists());
}

#[cfg(unix)]
#[test]
fn archives_reject_symlinks_when_packing_or_extracting() {
    let (root, source, _) = fixture();
    std::os::unix::fs::symlink("/etc/passwd", source.join("link")).unwrap();
    let output = root.path().join("bad.zip");
    assert!(archive::pack(&[source], &output, &mut Task::default()).is_err());
    assert!(!output.exists());
    let mut zip = ZipWriter::new(File::create(&output).unwrap());
    zip.add_symlink("link", "../escaped", SimpleFileOptions::default())
        .unwrap();
    zip.finish().unwrap();
    let destination = root.path().join("extracted");
    assert!(
        archive::extract(
            &output,
            &[],
            Path::new(""),
            &destination,
            &mut Task::default()
        )
        .is_err()
    );
    assert!(!destination.exists());
}

#[test]
fn tar_links_are_rejected_before_extraction() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("bad.tar");
    let mut builder = tar::Builder::new(File::create(&output).unwrap());
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    header.set_cksum();
    builder
        .append_link(&mut header, "link", "../escaped")
        .unwrap();
    builder.finish().unwrap();
    drop(builder);
    let destination = root.path().join("extracted");
    assert!(
        archive::extract(
            &output,
            &[],
            Path::new(""),
            &destination,
            &mut Task::default()
        )
        .is_err()
    );
    assert!(!destination.exists());
}

#[test]
fn text_editor_refuses_external_changes_and_preserves_content() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file.txt");
    write(&file, "original");
    operations::save_text(&file, b"original", "edited").unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "edited");
    write(&file, "external change");
    assert!(operations::save_text(&file, b"edited", "overwrite").is_err());
    assert_eq!(fs::read_to_string(&file).unwrap(), "external change");
}

#[test]
fn text_reader_bounds_memory_and_rejects_binary_or_invalid_utf8() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    write(&file, "12345");
    assert!(operations::read_text(&file, 4).is_err());
    fs::write(&file, [0, 1, 2]).unwrap();
    assert!(operations::read_text(&file, 10).is_err());
    fs::write(&file, [255, 255]).unwrap();
    assert!(operations::read_text(&file, 10).is_err());
}

#[test]
fn checksum_matches_known_sha256() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("abc");
    write(&file, "abc");
    assert!(
        run(Operation::Checksum(file))
            .unwrap()
            .ends_with("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    );
}

#[test]
fn directories_sort_first_even_when_descending() {
    let mut entries = vec![
        entry("z", false, 30),
        entry("a", false, 10),
        entry("folder", true, 0),
    ];
    filesystem::sort_entries(&mut entries, Sort::Size, true);
    assert_eq!(
        entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["folder", "z", "a"]
    );
}

fn entry(name: &str, is_dir: bool, size: u64) -> Entry {
    Entry {
        path: name.into(),
        name: name.into(),
        is_dir,
        size,
        size_known: true,
        is_symlink: false,
        modified: Some(SystemTime::UNIX_EPOCH),
    }
}

#[test]
fn archive_parent_navigation_returns_to_local_folder() {
    let local = tempfile::tempdir().unwrap();
    let archive = local.path().join("a.zip");
    let root = Location::Archive {
        file: archive.clone(),
        prefix: PathBuf::new(),
    };
    let nested = Location::Archive {
        file: archive,
        prefix: "a/b".into(),
    };
    assert_eq!(nested.parent().unwrap().parent().unwrap(), root);
    assert_eq!(
        root.parent(),
        Some(Location::Local(local.path().to_owned()))
    );
}

#[test]
fn model_filters_selection_and_never_uses_hidden_marked_files() {
    let root = tempfile::tempdir().unwrap();
    let ctx = rust_commander::ui_signal::UiSignal::default();
    let mut tab = rust_commander::model::Tab::new(Location::Local(root.path().to_owned()), &ctx);
    tab.entries = vec![entry(".hidden", false, 1), entry("visible", false, 2)];
    tab.selected.insert(".hidden".into());
    tab.cursor = Some("visible".into());
    assert_eq!(tab.chosen(false)[0].name, "visible");
    assert_eq!(tab.chosen(true)[0].name, ".hidden");
    tab.filter = "VIS".into();
    assert_eq!(tab.visible(true), [1]);
}
