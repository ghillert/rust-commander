use rust_commander::{
    archive,
    archive_tools::{self, ZipEdit},
    operations::Task,
};
use std::{fs, fs::File, path::Path};

fn member_names(file: &Path, extension: &str) -> Vec<String> {
    match extension {
        "zip" => {
            let mut reader = zip::ZipArchive::new(File::open(file).unwrap()).unwrap();
            (0..reader.len())
                .map(|i| reader.by_index_raw(i).unwrap().name().to_owned())
                .collect()
        }
        "tar" => tar::Archive::new(File::open(file).unwrap())
            .entries()
            .unwrap()
            .map(|e| String::from_utf8(e.unwrap().path_bytes().into_owned()).unwrap())
            .collect(),
        "7z" => {
            let mut names = Vec::new();
            sevenz_rust::SevenZReader::open(file, sevenz_rust::Password::empty())
                .unwrap()
                .for_each_entries(|entry, _| {
                    names.push(entry.name.clone());
                    Ok(true)
                })
                .unwrap();
            names
        }
        _ => unreachable!(),
    }
}

#[test]
fn native_nested_paths_work_for_archive_creation_selection_and_edits() {
    for extension in ["zip", "tar", "7z"] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir_all(source.join("nested").join("empty")).unwrap();
        fs::write(source.join("nested").join("café.txt"), "contents").unwrap();
        let packed = root.path().join(format!("test.{extension}"));
        archive::pack(&[source], &packed, &mut Task::default()).unwrap();
        let names = member_names(&packed, extension);
        assert!(names.iter().any(|n| n == "source/nested/café.txt"));
        assert!(names.iter().all(|n| !n.contains('\\')));

        // These are native PathBufs from navigation/listing, not raw archive headers.
        let prefix = Path::new("source").join("nested");
        let listing = archive::list(&packed, &prefix).unwrap();
        let chosen = listing.iter().find(|e| e.name == "café.txt").unwrap();
        assert_eq!(
            archive::read_text(&packed, &chosen.path, 100).unwrap(),
            "contents"
        );
        let extracted = root.path().join("selected");
        archive::extract(
            &packed,
            std::slice::from_ref(&chosen.path),
            &prefix,
            &extracted,
            &mut Task::default(),
        )
        .unwrap();
        assert_eq!(fs::read(extracted.join("café.txt")).unwrap(), b"contents");
        assert!(!extracted.join("empty").exists());

        let addition = root.path().join("addition");
        fs::create_dir_all(addition.join("child")).unwrap();
        fs::write(addition.join("child").join("new.txt"), "new").unwrap();
        archive_tools::edit_archive(
            &packed,
            &ZipEdit::Add {
                sources: vec![addition],
                prefix: prefix.clone(),
            },
            &mut Task::default(),
        )
        .unwrap();
        let renamed = prefix.join("renamed");
        archive_tools::edit_archive(
            &packed,
            &ZipEdit::Rename {
                source: prefix.join("addition"),
                target: renamed.clone(),
            },
            &mut Task::default(),
        )
        .unwrap();
        assert_eq!(
            archive::read_text(&packed, &renamed.join("child").join("new.txt"), 100).unwrap(),
            "new"
        );
        assert!(
            member_names(&packed, extension)
                .iter()
                .all(|n| !n.contains('\\'))
        );
        archive_tools::edit_archive(
            &packed,
            &ZipEdit::Remove(vec![renamed]),
            &mut Task::default(),
        )
        .unwrap();
        assert_eq!(archive::list(&packed, &prefix).unwrap().len(), 2);
    }
}

#[test]
fn raw_tar_and_sevenz_members_keep_windows_separators_and_traversal_rejected() {
    for name in [
        "../escape",
        r"nested\file",
        r"C:\escape",
        r"\\server\share",
        "name:stream",
    ] {
        for extension in ["tar", "7z"] {
            let root = tempfile::tempdir().unwrap();
            let file = root.path().join(format!("unsafe.{extension}"));
            if extension == "tar" {
                // Write the raw header directly so the fixture writer does not sanitize it.
                let mut writer = tar::Builder::new(File::create(&file).unwrap());
                let mut header = tar::Header::new_gnu();
                header.set_size(3);
                header.set_mode(0o644);
                header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
                header.set_cksum();
                writer.append(&header, &b"bad"[..]).unwrap();
                writer.finish().unwrap();
            } else {
                let mut writer = sevenz_rust::SevenZWriter::create(&file).unwrap();
                let mut entry = sevenz_rust::SevenZArchiveEntry::new();
                entry.name = name.into();
                writer
                    .push_archive_entry(entry, Some(std::io::Cursor::new(b"bad")))
                    .unwrap();
                writer.finish().unwrap();
            }
            let destination = root.path().join("extracted");
            assert!(
                archive::list(&file, Path::new("")).is_err(),
                "{extension}: {name}"
            );
            assert!(
                archive::extract(
                    &file,
                    &[],
                    Path::new(""),
                    &destination,
                    &mut Task::default()
                )
                .is_err()
            );
            assert!(!destination.exists());
        }
    }
}

#[cfg(windows)]
#[test]
fn zip_edit_preserves_original_when_an_external_reader_blocks_replacement() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("first.txt");
    fs::write(&source, "original content").unwrap();
    let packed = root.path().join("test.zip");
    archive::pack(&[source], &packed, &mut Task::default()).unwrap();
    let original = fs::read(&packed).unwrap();
    let addition = root.path().join("second.txt");
    fs::write(&addition, "new content").unwrap();
    let edit = ZipEdit::Add {
        sources: vec![addition],
        prefix: Path::new("nested").to_owned(),
    };
    let external_reader = File::open(&packed).unwrap();
    assert!(archive_tools::edit_archive(&packed, &edit, &mut Task::default()).is_err());
    assert_eq!(fs::read(&packed).unwrap(), original);
    drop(external_reader);
    archive_tools::edit_archive(&packed, &edit, &mut Task::default()).unwrap();
    assert_eq!(
        archive::read_text(&packed, Path::new("first.txt"), 100).unwrap(),
        "original content"
    );
    assert_eq!(
        archive::read_text(&packed, &Path::new("nested").join("second.txt"), 100).unwrap(),
        "new content"
    );
}
