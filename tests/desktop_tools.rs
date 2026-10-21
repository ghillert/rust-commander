use rust_commander::{
    archive,
    desktop::{self, RenameOptions, SearchOptions},
    filesystem::{self, Location},
    operations::{self, Operation, Task},
    preferences,
    remote::{self, Connection, Secrets},
    viewer,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

#[test]
fn batch_rename_previews_numbering_and_regex() {
    let root = tempfile::tempdir().unwrap();
    let sources: Vec<_> = ["IMG_123.jpg", "IMG_456.jpg"]
        .map(|name| root.path().join(name))
        .into();
    for path in &sources {
        fs::write(path, b"photo").unwrap();
    }
    let options = RenameOptions {
        find: "IMG_\\d+".into(),
        replace: "Photo".into(),
        regex: true,
        template: "{name}-{n}{ext}".into(),
        start: 9,
        width: 3,
    };
    let plan = desktop::rename_plan(&sources, &options).unwrap();
    assert!(plan[0].1.ends_with("Photo-009.jpg"));
    desktop::batch_rename(&plan, &mut Task::default()).unwrap();
    assert!(plan.iter().all(|(s, t)| !s.exists() && t.exists()));
}

#[test]
fn batch_rename_supports_cycles_without_overwriting_contents() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    fs::write(&a, b"A").unwrap();
    fs::write(&b, b"B").unwrap();
    desktop::batch_rename(
        &[(a.clone(), b.clone()), (b.clone(), a.clone())],
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read(a).unwrap(), b"B");
    assert_eq!(fs::read(b).unwrap(), b"A");
}

#[test]
fn batch_rename_rejects_collisions_and_invalid_names() {
    let root = tempfile::tempdir().unwrap();
    let sources: Vec<_> = ["a", "b"].map(|name| root.path().join(name)).into();
    for path in &sources {
        fs::write(path, b"keep").unwrap();
    }
    for template in ["same", "../escape", "", "a/b"] {
        let options = RenameOptions {
            template: template.into(),
            ..Default::default()
        };
        assert!(desktop::rename_plan(&sources, &options).is_err());
    }
    assert!(sources.iter().all(|p| fs::read(p).unwrap() == b"keep"));
}

#[test]
fn batch_rename_cancellation_restores_originals() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    fs::write(&a, b"keep").unwrap();
    let mut task = Task::new(Arc::new(AtomicBool::new(true)), |_| {});
    assert!(desktop::batch_rename(&[(a.clone(), root.path().join("b"))], &mut task).is_err());
    assert_eq!(fs::read(a).unwrap(), b"keep");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn exclusive_rename_preserves_existing_destination() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    fs::write(&a, b"A").unwrap();
    fs::write(&b, b"B").unwrap();
    assert!(desktop::rename_exclusive(&a, &b).is_err());
    assert_eq!(fs::read(a).unwrap(), b"A");
    assert_eq!(fs::read(b).unwrap(), b"B");
}

#[test]
fn recursive_search_filters_names_content_hidden_and_does_not_follow_links() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(root.path().join("nested/notes.TXT"), b"needle").unwrap();
    fs::write(root.path().join("other.txt"), b"other").unwrap();
    fs::write(root.path().join(".secret.txt"), b"needle").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.path(), root.path().join("loop")).unwrap();
    let options = SearchOptions {
        root: root.path().into(),
        pattern: "*.txt".into(),
        content: "needle".into(),
        hidden: false,
        max_results: 100,
    };
    let result = desktop::search(&options, &mut Task::default()).unwrap();
    assert_eq!(result.paths.len(), 1);
    assert!(result.paths[0].ends_with("notes.TXT"));
    let result = desktop::search(
        &SearchOptions {
            hidden: true,
            ..options
        },
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(result.paths.len(), 2);
}

#[test]
fn split_and_combine_round_trip_every_chunk_boundary() {
    for chunk in [1, 3, 65536, 100_000] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let parts = root.path().join("parts");
        let bytes: Vec<_> = (0..if chunk < 100 { 17 } else { 150_017 })
            .map(|i| (i % 251) as u8)
            .collect();
        fs::write(&source, &bytes).unwrap();
        desktop::split(&source, &parts, chunk, &mut Task::default()).unwrap();
        let mut sources: Vec<_> = fs::read_dir(parts)
            .unwrap()
            .map(|i| i.unwrap().path())
            .collect();
        sources.sort();
        assert!(
            sources
                .iter()
                .all(|p| fs::metadata(p).unwrap().len() <= chunk)
        );
        let output = root.path().join("combined");
        desktop::combine(&sources, &output, &mut Task::default()).unwrap();
        assert_eq!(fs::read(output).unwrap(), bytes);
    }
}

#[test]
fn folder_comparison_uses_file_contents() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let b = root.path().join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    fs::write(a.join("equal"), b"same").unwrap();
    fs::write(b.join("equal"), b"same").unwrap();
    fs::write(a.join("changed"), b"AAAA").unwrap();
    fs::write(b.join("changed"), b"BBBB").unwrap();
    fs::write(a.join("only"), b"one").unwrap();
    let result = desktop::compare(&a, &b, &mut Task::default()).unwrap();
    for (name, status) in [
        ("equal", "Equal"),
        ("changed", "Different"),
        ("only", "Only left"),
    ] {
        assert_eq!(
            result.iter().find(|r| r.name == name).unwrap().status,
            status
        );
    }
}

#[test]
fn additional_archives_round_trip_nested_content() {
    for extension in ["7z", "tar.bz2", "tbz2", "tar.xz", "txz"] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir_all(source.join("nested/empty")).unwrap();
        fs::write(source.join("nested/a"), b"contents").unwrap();
        let packed = root.path().join(format!("test.{extension}"));
        archive::pack(&[source], &packed, &mut Task::default()).unwrap();
        assert_eq!(
            archive::list(&packed, Path::new("source/nested"))
                .unwrap()
                .len(),
            2
        );
        let output = root.path().join("out");
        archive::extract(&packed, &[], Path::new(""), &output, &mut Task::default()).unwrap();
        assert_eq!(
            fs::read(output.join("source/nested/a")).unwrap(),
            b"contents"
        );
    }
}

#[test]
fn stream_compression_round_trips() {
    for extension in ["gz", "bz2", "xz"] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("data");
        fs::write(&source, b"contents").unwrap();
        let packed = root.path().join(format!("data.{extension}"));
        archive::pack(&[source], &packed, &mut Task::default()).unwrap();
        let output = root.path().join("out");
        archive::extract(&packed, &[], Path::new(""), &output, &mut Task::default()).unwrap();
        assert_eq!(fs::read(output.join("data")).unwrap(), b"contents");
    }
}

#[test]
fn sevenz_traversal_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let packed = root.path().join("bad.7z");
    let mut writer = sevenz_rust::SevenZWriter::create(&packed).unwrap();
    let mut entry = sevenz_rust::SevenZArchiveEntry::new();
    entry.name = "../escape".into();
    writer
        .push_archive_entry(entry, Some(std::io::Cursor::new(b"bad")))
        .unwrap();
    writer.finish().unwrap();
    let output = root.path().join("out");
    assert!(archive::extract(&packed, &[], Path::new(""), &output, &mut Task::default()).is_err());
    assert!(!output.exists());
}

fn filesystem_connection(root: &Path) -> Connection {
    let connection = Connection {
        service: "fs".into(),
        root: root.to_string_lossy().into_owned(),
        ..Default::default()
    };
    remote::connect(&connection, Secrets::default(), false).unwrap();
    connection
}

#[test]
fn remote_backend_contract_upload_download_list_and_conflicts() {
    let root = tempfile::tempdir().unwrap();
    let storage = root.path().join("storage");
    fs::create_dir(&storage).unwrap();
    let local = root.path().join("local");
    fs::create_dir_all(local.join("folder/empty")).unwrap();
    fs::write(local.join("folder/a"), b"hello").unwrap();
    let connection = filesystem_connection(&storage);
    let from = Location::Local(local);
    let to = Location::Remote {
        connection: connection.id.clone(),
        path: PathBuf::new(),
    };
    let entries = filesystem::list(&from).unwrap();
    remote::transfer(&from, &entries, &to, false, &mut Task::default()).unwrap();
    assert_eq!(fs::read(storage.join("folder/a")).unwrap(), b"hello");
    assert!(remote::transfer(&from, &entries, &to, false, &mut Task::default()).is_err());
    let files = remote::list(&connection.id, Path::new("folder")).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(
        remote::read_bytes(&connection.id, Path::new("folder/a"), 32).unwrap(),
        b"hello"
    );
    let download = root.path().join("download");
    fs::create_dir(&download).unwrap();
    let entries = remote::list(&connection.id, Path::new("")).unwrap();
    remote::transfer(
        &to,
        &entries,
        &Location::Local(download.clone()),
        false,
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read(download.join("folder/a")).unwrap(), b"hello");
    assert!(download.join("folder/empty").is_dir());
}

#[test]
fn connection_configuration_contains_no_secret_fields() {
    let connection = Connection {
        endpoint: "https://user:secret@example.com".into(),
        ..Default::default()
    };
    assert!(remote::validate(&connection).is_err());
    let json = serde_json::to_string(&Connection::default()).unwrap();
    for key in ["password", "access_token", "secret_access_key"] {
        assert!(!json.contains(key));
    }
}

#[test]
fn remote_locations_round_trip_spaces_and_reject_traversal() {
    let location = Location::Remote {
        connection: "profile".into(),
        path: "folder/a b".into(),
    };
    assert_eq!(
        filesystem::parse_location(&location.label(), &Location::Local("/".into())).unwrap(),
        location
    );
    assert!(filesystem::parse_location("remote://profile/%2e%2e%2fescape", &location).is_err());
}

#[test]
fn image_and_hex_viewers_load_real_files() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("binary");
    fs::write(&binary, [0, 1, 65, 255]).unwrap();
    assert!(matches!(
        viewer::load(&Location::Local(root.path().into()), &binary).unwrap(),
        viewer::Content::Hex { .. }
    ));
    assert!(viewer::hex_dump(&[0, 65]).contains("00 41"));
    let image = root.path().join("image.png");
    image::RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255]))
        .save(&image)
        .unwrap();
    let viewer::Content::Image(image) =
        viewer::load(&Location::Local(root.path().into()), &image).unwrap()
    else {
        panic!("Expected image")
    };
    assert_eq!(image.size, [4, 3]);
}

#[test]
fn preferences_validate_custom_shortcuts_and_detect_collisions() {
    let mut settings = preferences::Preferences::default();
    preferences::validate(&settings).unwrap();
    settings
        .shortcuts
        .insert("Copy".into(), "Command+Shift+K".into());
    preferences::validate(&settings).unwrap();
    settings
        .shortcuts
        .insert("Move".into(), "Command+Shift+K".into());
    assert!(preferences::validate(&settings).is_err());
    assert!(preferences::shortcut("Command+Bogus").is_err());
}

#[test]
fn all_checksum_algorithms_match_known_values() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("abc");
    fs::write(&file, b"abc").unwrap();
    let text = operations::run(Operation::Checksum(file), &mut Task::default()).unwrap();
    assert!(text.contains("900150983cd24fb0d6963f7d28e17f72"));
    assert!(text.contains("a9993e364706816aba3e25717850c26c9cd0d89d"));
    assert!(text.contains("ba7816bf"));
}

#[test]
fn pdf_viewer_renders_and_navigates_pages() {
    let bytes = include_bytes!("fixtures/two-pages.pdf");
    let (first, count) = viewer::render_pdf(bytes, 0).unwrap();
    let (second, _) = viewer::render_pdf(bytes, 1).unwrap();
    assert_eq!(count, 2);
    assert!(first.size[0] > 0 && first.size[1] > 0);
    assert_ne!(first.pixels, second.pixels);
    assert!(viewer::render_pdf(bytes, 2).is_err());
    assert!(viewer::render_pdf(b"not a PDF", 0).is_err());
}

#[test]
fn remote_service_choices_include_cross_platform_sftp() {
    use rust_commander::remote::{self, Connection};
    assert!(remote::SERVICES.contains(&Connection::default().service.as_str()));
    let connection = Connection {
        service: "sftp".into(),
        ..Default::default()
    };
    assert!(remote::SERVICES.contains(&"sftp"));
    remote::validate(&connection).unwrap();
}
