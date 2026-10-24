use rust_commander::{
    archive,
    operations::{Operation, Task},
    packing::{self, Options},
    queue::Queue,
};
use std::{fs, path::Path};

#[test]
fn format_switching_replaces_compound_suffixes_and_preserves_dotted_names() {
    let mut options = Options::default();
    for (initial, expected) in [
        ("/tmp/report.v2.tar.gz", "/tmp/report.v2.7z"),
        ("/tmp/data.TGZ", "/tmp/data.7z"),
        ("/tmp/data.zip", "/tmp/data.7z"),
        ("/tmp/report.csv", "/tmp/report.csv.7z"),
    ] {
        let mut path = initial.to_owned();
        options.set_format("7z", &mut path).unwrap();
        assert_eq!(path, expected);
        options.set_format("TAR.BZ2", &mut path).unwrap();
        assert_eq!(
            path,
            expected.trim_end_matches(".7z").to_owned() + ".tar.bz2"
        );
        options.set_format("ZIP", &mut path).unwrap();
        assert_eq!(path, expected.trim_end_matches(".7z").to_owned() + ".zip");
    }
    let mut empty = String::new();
    options.set_format("7z", &mut empty).unwrap();
    assert!(empty.is_empty());
    assert!(options.set_format("RAR", &mut empty).is_err());
}

#[test]
fn zip_comment_round_trips_without_changing_member_contents() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("café.txt");
    fs::write(&file, b"payload").unwrap();
    let destination = root.path().join("result.zip");
    let comment = "A comment\nGrüße — 日本語";
    archive::pack_with_comment(&[file], &destination, comment, &mut Task::default()).unwrap();
    let mut zip = zip::ZipArchive::new(fs::File::open(&destination).unwrap()).unwrap();
    assert_eq!(zip.comment(), comment.as_bytes());
    assert_eq!(zip.by_index(0).unwrap().name(), "café.txt");
    let output = root.path().join("out");
    archive::extract(
        &destination,
        &[],
        Path::new(""),
        &output,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read(output.join("café.txt")).unwrap(), b"payload");
}

#[test]
fn comments_are_validated_before_creating_outputs() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("source");
    fs::write(&file, b"source").unwrap();
    for (name, comment) in [
        ("bad.zip", "é".repeat(32768)),
        ("bad.7z", "unsupported comment".into()),
        ("signature.zip", "PK\x05\x06".into()),
    ] {
        let destination = root.path().join(name);
        assert!(
            archive::pack_with_comment(
                std::slice::from_ref(&file),
                &destination,
                &comment,
                &mut Task::default()
            )
            .is_err()
        );
        assert!(!destination.exists());
    }
    assert!(packing::validate_comment(archive::Format::Zip, &"x".repeat(65535)).is_ok());
    let options = Options {
        format: archive::Format::SevenZ,
        comment: "retained ZIP draft".into(),
        ..Default::default()
    };
    assert_eq!(options.comment(), "");
    options.validate(&[file]).unwrap();
}

#[test]
fn sevenz_packs_multiple_files_and_nested_folders_and_extracts_them() {
    let root = tempfile::tempdir().unwrap();
    let nested = root.path().join("folder/nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("inner.txt"), b"inside").unwrap();
    let file = root.path().join("outer.txt");
    fs::write(&file, b"outside").unwrap();
    let sources = vec![file, root.path().join("folder")];
    for password in [None, Some("secret")] {
        let destination = root.path().join(if password.is_some() {
            "encrypted.7z"
        } else {
            "plain.7z"
        });
        if let Some(password) = password {
            archive::pack_with_password(&sources, &destination, password, &mut Task::default())
                .unwrap();
        } else {
            archive::pack(&sources, &destination, &mut Task::default()).unwrap();
        }
        let output = root.path().join(if password.is_some() {
            "encrypted-out"
        } else {
            "plain-out"
        });
        if let Some(password) = password {
            archive::extract_with_password(
                &destination,
                &[],
                Path::new(""),
                &output,
                password,
                &mut Task::default(),
            )
            .unwrap();
        } else {
            archive::extract(
                &destination,
                &[],
                Path::new(""),
                &output,
                &mut Task::default(),
            )
            .unwrap();
        }
        assert_eq!(fs::read(output.join("outer.txt")).unwrap(), b"outside");
        assert_eq!(
            fs::read(output.join("folder/nested/inner.txt")).unwrap(),
            b"inside"
        );
    }
}

#[test]
fn legacy_pack_recipes_load_and_new_options_survive_queue_persistence() {
    let legacy: Operation =
        serde_json::from_str(r#"{"Pack":{"sources":["one"],"destination":"archive.zip"}}"#)
            .unwrap();
    assert!(
        matches!(legacy, Operation::Pack {comment, background: false, ..} if comment.is_empty())
    );
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("queue.json");
    let mut queue = Queue::default();
    queue.push_back(Operation::Pack {
        sources: vec!["one".into()],
        destination: "archive.zip".into(),
        comment: "saved comment".into(),
        background: true,
    });
    queue.save_in(&path).unwrap();
    let restored = Queue::load_in(&path).unwrap();
    assert!(
        matches!(&restored[0], Operation::Pack {comment, background: true, ..} if comment == "saved comment")
    );
}

#[test]
fn stream_formats_reject_multiple_files_and_folders_before_starting() {
    let root = tempfile::tempdir().unwrap();
    let one = root.path().join("one");
    let two = root.path().join("two");
    fs::write(&one, b"one").unwrap();
    fs::write(&two, b"two").unwrap();
    for format in [
        archive::Format::Gzip,
        archive::Format::Bzip2,
        archive::Format::Xz,
    ] {
        let options = Options {
            format,
            ..Default::default()
        };
        options.validate(std::slice::from_ref(&one)).unwrap();
        assert!(options.validate(&[one.clone(), two.clone()]).is_err());
        assert!(options.validate(&[root.path().to_owned()]).is_err());
    }
}
