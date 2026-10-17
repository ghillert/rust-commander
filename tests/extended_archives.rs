use mucommander_rust::{archive, operations::Task};
use std::{fs, io::Write, path::Path};

#[test]
fn rar_and_iso_can_be_browsed_and_extracted() {
    for name in ["tree.rar", "sample.iso"] {
        let file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        assert!(!archive::list(&file, Path::new("")).unwrap().is_empty());
        let temporary = tempfile::tempdir().unwrap();
        let output = temporary.path().join("out");
        archive::extract(&file, &[], Path::new(""), &output, &mut Task::default()).unwrap();
        if name.ends_with("iso") {
            assert_eq!(
                fs::read(output.join("nested/hello.txt")).unwrap(),
                b"ISO fixture contents\n"
            );
        } else {
            assert!(fs::read_dir(output).unwrap().next().is_some());
        }
    }
}

#[test]
fn ar_and_deb_members_are_streamed() {
    for extension in ["ar", "deb"] {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join(format!("test.{extension}"));
        let mut archive = fs::File::create(&file).unwrap();
        archive.write_all(b"!<arch>\n").unwrap();
        let header = format!(
            "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            "hello.txt/", 0, 0, 0, "100644", 5
        );
        assert_eq!(header.len(), 60);
        archive.write_all(header.as_bytes()).unwrap();
        archive.write_all(b"hello\n").unwrap();
        drop(archive);
        let output = root.path().join("out");
        assert_eq!(
            archive::list(&file, Path::new("")).unwrap()[0].name,
            "hello.txt"
        );
        archive::extract(&file, &[], Path::new(""), &output, &mut Task::default()).unwrap();
        assert_eq!(fs::read(output.join("hello.txt")).unwrap(), b"hello");
    }
}

#[test]
fn extended_reader_rejects_traversal_paths() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("bad.ar");
    let mut archive = fs::File::create(&file).unwrap();
    archive.write_all(b"!<arch>\n").unwrap();
    archive
        .write_all(
            format!(
                "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                "../escape/", 0, 0, 0, "100644", 3
            )
            .as_bytes(),
        )
        .unwrap();
    archive.write_all(b"bad\n").unwrap();
    drop(archive);
    let output = root.path().join("out");
    assert!(archive::extract(&file, &[], Path::new(""), &output, &mut Task::default()).is_err());
    assert!(!output.exists());
}
