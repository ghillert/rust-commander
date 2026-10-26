use rust_commander::{
    archive, archive_tools,
    filesystem::Location,
    operations::Task,
    pdf_text,
    viewer::{self, Content, Mode},
};
use std::{
    fs,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

#[test]
fn explicit_viewers_override_extensions_without_changing_sources() {
    let root = tempfile::tempdir().unwrap();
    let location = Location::Local(root.path().into());
    let file = root.path().join("misleading.pdf");
    fs::write(&file, "ordinary UTF-8 text — café").unwrap();
    assert!(viewer::load(&location, &file).is_err());
    assert!(
        matches!(viewer::load_as(&location,&file,Mode::Text).unwrap(),Content::EncodedText(doc) if doc.text == "ordinary UTF-8 text — café")
    );
    assert!(
        matches!(viewer::load_as(&location,&file,Mode::Hex).unwrap(),Content::Hex {text,truncated:false} if text.starts_with("00000000"))
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "ordinary UTF-8 text — café"
    );
    let pdf = root.path().join("no-extension");
    fs::write(&pdf, include_bytes!("fixtures/two-pages.pdf")).unwrap();
    assert!(
        matches!(viewer::load_as(&location,&pdf,Mode::Pdf).unwrap(),Content::Pdf{pages:2,text,..} if text.text == "Page one")
    );
    let png = root.path().join("picture.txt");
    image::RgbaImage::from_pixel(2, 3, image::Rgba([255, 0, 0, 255]))
        .save_with_format(&png, image::ImageFormat::Png)
        .unwrap();
    assert!(
        matches!(viewer::load_as(&location,&png,Mode::Image).unwrap(),Content::Image(image) if image.size == [2,3])
    );
    assert!(viewer::load_as(&location, &file, Mode::Image).is_err());
}

#[test]
fn forced_hex_is_bounded_and_forced_large_text_uses_pages() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("large.pdf");
    fs::write(&file, b"text line\n".repeat(300_000)).unwrap();
    let location = Location::Local(root.path().into());
    assert!(matches!(
        viewer::load_as(&location, &file, Mode::Hex).unwrap(),
        Content::Hex {
            truncated: true,
            ..
        }
    ));
    assert!(
        matches!(viewer::load_as(&location,&file,Mode::Text).unwrap(),Content::LargeText(page) if page.next < page.total)
    );
}

#[test]
fn forced_archive_is_a_read_only_cached_interpretation_with_original_parent() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.txt");
    fs::write(&source, b"unchanged payload").unwrap();
    let zip = root.path().join("original.zip");
    archive::pack(&[source], &zip, &mut Task::default()).unwrap();
    let disguised = root.path().join("archive.bin");
    fs::rename(&zip, &disguised).unwrap();
    let original = fs::read(&disguised).unwrap();
    let location = Location::Local(root.path().into());
    let cached =
        archive_tools::materialize_as(&location, &disguised, Some(archive::Format::Zip)).unwrap();
    assert_eq!(
        archive::list(&cached, Path::new("")).unwrap()[0].name,
        "source.txt"
    );
    assert!(!archive_tools::editable(&cached));
    assert_eq!(archive_tools::origin(&cached), Some(location));
    assert_eq!(fs::read(&disguised).unwrap(), original);
    assert!(
        archive_tools::materialize_as(
            &Location::Local(root.path().into()),
            &disguised,
            Some(archive::Format::SevenZ)
        )
        .is_err()
    );
    let outer = root.path().join("outer.zip");
    archive::pack(&[disguised], &outer, &mut Task::default()).unwrap();
    let nested = archive_tools::materialize_as(
        &Location::Archive {
            file: outer,
            prefix: Default::default(),
        },
        Path::new("archive.bin"),
        Some(archive::Format::Zip),
    )
    .unwrap();
    assert_eq!(
        archive::read_bytes(&nested, Path::new("source.txt"), 1024).unwrap(),
        b"unchanged payload"
    );
}

#[test]
fn forced_encrypted_archive_validates_passwords_before_publishing_the_copy() {
    for extension in ["zip", "7z"] {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("source.txt");
        fs::write(&file, b"encrypted contents").unwrap();
        let archive = root.path().join(format!("archive.{extension}"));
        archive::pack_with_password(&[file], &archive, "test-password", &mut Task::default())
            .unwrap();
        let kind = archive::format(&archive).unwrap();
        let source = root.path().join("misnamed.bin");
        fs::rename(&archive, &source).unwrap();
        let location = Location::Local(root.path().into());
        assert!(
            archive_tools::materialize_as_with_password(&location, &source, kind, "wrong").is_err()
        );
        let cached =
            archive_tools::materialize_as_with_password(&location, &source, kind, "test-password")
                .unwrap();
        assert_eq!(
            archive::read_bytes(&cached, Path::new("source.txt"), 1024).unwrap(),
            b"encrypted contents"
        );
        assert!(source.exists());
        assert!(!archive_tools::editable(&cached));
    }
}

#[test]
fn pdf_search_crosses_pages_wraps_and_searches_backwards() {
    let bytes = include_bytes!("fixtures/two-pages.pdf");
    let task = Task::default();
    let first = pdf_text::find(bytes, "", "PAGE", (0, 0), false, &task)
        .unwrap()
        .unwrap();
    assert_eq!((first.page, first.range.clone()), (0, 0..4));
    let second = pdf_text::find(
        bytes,
        "",
        "page",
        (first.page, first.range.end),
        false,
        &task,
    )
    .unwrap()
    .unwrap();
    assert_eq!(second.page, 1);
    assert!(!second.wrapped);
    let wrapped = pdf_text::find(
        bytes,
        "",
        "page",
        (second.page, second.range.end),
        false,
        &task,
    )
    .unwrap()
    .unwrap();
    assert_eq!(wrapped.page, 0);
    assert!(wrapped.wrapped);
    let previous = pdf_text::find(bytes, "", "page", (0, 0), true, &task)
        .unwrap()
        .unwrap();
    assert_eq!(previous.page, 1);
    assert!(previous.wrapped);
    assert!(
        pdf_text::find(bytes, "", "missing", (0, 0), false, &task)
            .unwrap()
            .is_none()
    );
    assert!(pdf_text::find(bytes, "", "", (0, 0), false, &task).is_err());
    let cancel = Task::new(Arc::new(AtomicBool::new(true)), |_| {});
    assert!(pdf_text::find(bytes, "", "page", (0, 0), false, &cancel).is_err());
}

#[test]
fn pdf_selection_tracks_page_rotation_and_encrypted_text() {
    let bytes = include_bytes!("fixtures/two-pages-protected.pdf");
    assert!(pdf_text::load_page(bytes, "wrong", 0, &Task::default()).is_err());
    let page = pdf_text::load_page(bytes, "commander-test", 0, &Task::default()).unwrap();
    assert_eq!(page.text, "Page one");
    for rotation in 0..4 {
        let first = pdf_text::rotate_bounds(page.glyphs[0].bounds, rotation);
        let last = pdf_text::rotate_bounds(page.glyphs[3].bounds, rotation);
        let start = page
            .nearest(
                (first[0] + first[2]) / 2.0,
                (first[1] + first[3]) / 2.0,
                rotation,
            )
            .unwrap();
        let end = page
            .nearest(
                (last[0] + last[2]) / 2.0,
                (last[1] + last[3]) / 2.0,
                rotation,
            )
            .unwrap();
        let range = page.selection(end, start).unwrap();
        assert_eq!(&page.text[range.clone()], "Page");
        assert_eq!(page.rectangles(&range, rotation).len(), 4);
    }
    assert!(
        pdf_text::find(
            bytes,
            "commander-test",
            "two",
            (0, 0),
            false,
            &Task::default()
        )
        .unwrap()
        .is_some_and(|hit| hit.page == 1)
    );
}

fn unicode_pdf() -> Vec<u8> {
    let cmap =
        "begincmap 3 beginbfchar <01> <00E9> <02> <00660069> <03> <D83DDE00> endbfchar endcmap";
    let content = "BT /F1 16 Tf 20 100 Td <010203> Tj ET";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".into(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream",content.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>".into(),
        format!("<< /Length {} >>\nstream\n{cmap}\nendstream",cmap.len()),
    ];
    let mut pdf = "%PDF-1.4\n".to_owned();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
    }
    let xref = pdf.len();
    pdf.push_str("xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        pdf.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ));
    pdf.into_bytes()
}
#[test]
fn pdf_unicode_ligatures_and_surrogates_keep_utf8_selection_boundaries() {
    let bytes = unicode_pdf();
    let page = pdf_text::load_page(&bytes, "", 0, &Task::default()).unwrap();
    assert_eq!(page.text, "éfi😀");
    let hit = pdf_text::find(&bytes, "", "😀", (0, 0), false, &Task::default())
        .unwrap()
        .unwrap();
    assert_eq!(&hit.text.text[hit.range.clone()], "😀");
    assert_eq!(&page.text[page.selection(0, 1).unwrap()], "éfi");
}
