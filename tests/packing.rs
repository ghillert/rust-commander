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
    let parent = std::env::temp_dir().join("packing test.v1");
    for (initial, expected) in [
        ("report.v2.tar.gz", "report.v2.7z"),
        ("data.TGZ", "data.7z"),
        ("data.zip", "data.7z"),
        ("report.csv", "report.csv.7z"),
    ] {
        let mut path = parent.join(initial).display().to_string();
        let expected = parent.join(expected);
        options.set_format("7z", &mut path).unwrap();
        assert_eq!(Path::new(&path), expected);
        options.set_format("TAR.BZ2", &mut path).unwrap();
        assert_eq!(Path::new(&path), expected.with_extension("tar.bz2"));
        options.set_format("ZIP", &mut path).unwrap();
        assert_eq!(Path::new(&path), expected.with_extension("zip"));
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
        matches!(legacy, Operation::Pack {comment, background: false, compression_level: None, ..} if comment.is_empty())
    );
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("queue.json");
    let mut queue = Queue::default();
    queue.push_back(Operation::Pack {
        sources: vec!["one".into()],
        destination: "archive.zip".into(),
        compression_level: Some(9),
        comment: "saved comment".into(),
        background: true,
    });
    queue.save_in(&path).unwrap();
    let restored = Queue::load_in(&path).unwrap();
    assert!(
        matches!(&restored[0], Operation::Pack {comment, background: true, compression_level: Some(9), ..} if comment == "saved comment")
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

#[test]
fn compression_ranges_and_format_switches_do_not_keep_unsupported_levels() {
    for (format, _, _) in packing::FORMATS {
        assert!(packing::validate_level(format, None).is_ok());
        assert!(packing::validate_level(format, Some(10)).is_err());
        for choice in packing::level_choices(format) {
            let level = packing::parse_level(format, &choice).unwrap();
            packing::validate_level(format, level).unwrap();
        }
    }
    assert!(packing::validate_level(archive::Format::Bzip2, Some(0)).is_err());
    assert!(packing::validate_level(archive::Format::TarBz2, Some(0)).is_err());
    assert!(packing::validate_level(archive::Format::Tar, Some(6)).is_err());
    assert!(packing::parse_level(archive::Format::Zip, "100").is_err());
    let mut options = Options {
        compression_level: Some(0),
        ..Default::default()
    };
    let mut path = "result.zip".to_owned();
    options.set_format("7z", &mut path).unwrap();
    assert_eq!(options.compression_level, Some(0));
    assert_eq!(packing::level_label(options.format, Some(0)), "0 — Fastest");
    options.set_format("TAR.BZ2", &mut path).unwrap();
    assert_eq!(options.compression_level, None);
    options.compression_level = Some(9);
    options.set_format("7z", &mut path).unwrap();
    assert_eq!(options.compression_level, Some(9));
    options.set_format("TAR", &mut path).unwrap();
    assert_eq!(options.compression_level, None);
}

#[test]
fn zip_levels_change_real_compression_without_losing_contents_or_comments() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("sample.csv");
    let content = b"12345,Name,Family,Country,Description,2026\n".repeat(16384);
    fs::write(&source, &content).unwrap();
    let mut compressed = Vec::new();
    for level in [0, 1, 9] {
        let destination = root.path().join(format!("level-{level}.zip"));
        archive::pack_with_compression(
            std::slice::from_ref(&source),
            &destination,
            "level test",
            Some(level),
            &mut Task::default(),
        )
        .unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(&destination).unwrap()).unwrap();
        assert_eq!(zip.comment(), b"level test");
        let entry = zip.by_index(0).unwrap();
        assert_eq!(
            entry.compression(),
            if level == 0 {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            }
        );
        compressed.push(entry.compressed_size());
        drop(entry);
        let output = root.path().join(format!("out-{level}"));
        archive::extract(
            &destination,
            &[],
            Path::new(""),
            &output,
            &mut Task::default(),
        )
        .unwrap();
        assert_eq!(fs::read(output.join("sample.csv")).unwrap(), content);
    }
    assert!(compressed[2] < compressed[1]);
    assert!(compressed[1] < compressed[0] / 10);
}

#[test]
fn other_formats_round_trip_at_their_minimum_and_maximum_levels() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("sample.csv");
    let content = b"taxon,family,country\n".repeat(1024);
    fs::write(&source, &content).unwrap();
    for extension in ["7z", "tar.gz", "tar.bz2", "tar.xz", "gz", "bz2", "xz"] {
        let minimum = if extension.contains("bz2") { 1 } else { 0 };
        for level in [minimum, 9] {
            let destination = root.path().join(format!("level-{level}.{extension}"));
            archive::pack_with_compression(
                std::slice::from_ref(&source),
                &destination,
                "",
                Some(level),
                &mut Task::default(),
            )
            .unwrap();
            let name = archive::list(&destination, Path::new("")).unwrap()[0]
                .name
                .clone();
            let output = root.path().join(format!("out-{extension}-{level}"));
            archive::extract(
                &destination,
                &[],
                Path::new(""),
                &output,
                &mut Task::default(),
            )
            .unwrap();
            assert_eq!(
                fs::read(output.join(name)).unwrap(),
                content,
                "{extension}, level {level}"
            );
        }
    }
}

#[test]
fn invalid_levels_fail_before_creating_any_archive() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("sample");
    fs::write(&source, b"source").unwrap();
    for (extension, level) in [("zip", 10), ("7z", 255), ("xz", 10), ("bz2", 0), ("tar", 1)] {
        let destination = root.path().join(format!("bad.{extension}"));
        assert!(
            archive::pack_with_compression(
                std::slice::from_ref(&source),
                &destination,
                "",
                Some(level),
                &mut Task::default()
            )
            .is_err()
        );
        assert!(!destination.exists());
    }
}

#[test]
fn encrypted_archives_honor_compression_levels() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("secret.csv");
    let content = b"secret,entry\n".repeat(1024);
    fs::write(&source, &content).unwrap();
    for (extension, level) in [("zip", 0), ("zip", 9), ("7z", 1)] {
        let destination = root.path().join(format!("secret-{level}.{extension}"));
        archive::pack_with_password_and_level(
            std::slice::from_ref(&source),
            &destination,
            "password",
            Some(level),
            &mut Task::default(),
        )
        .unwrap();
        let output = root.path().join(format!("out-{extension}-{level}"));
        archive::extract_with_password(
            &destination,
            &[],
            Path::new(""),
            &output,
            "password",
            &mut Task::default(),
        )
        .unwrap();
        assert_eq!(fs::read(output.join("secret.csv")).unwrap(), content);
    }
}
