use rust_commander::{
    config_import::{self, ImportedCommand},
    filesystem::{self, Location},
    history,
    model::Tab,
    navigation,
    operations::{self, Operation, Task},
    preferences::Preferences,
    settings::Settings,
    text_document::{Document, Encoding, Page},
    ui_signal::UiSignal,
};
use std::{fs, path::Path};
fn tab(root: &Path) -> Tab {
    let mut tab = Tab::new(Location::Local(root.into()), &UiSignal::default());
    tab.entries = filesystem::list(&tab.location).unwrap();
    filesystem::sort_entries(&mut tab.entries, Default::default(), false);
    tab.loading = false;
    tab
}
#[test]
fn quick_find_cycles_unicode_names_without_changing_filter_or_marks() {
    let root = tempfile::tempdir().unwrap();
    for name in ["Äpfel.txt", "ärger.txt", "beta.txt", ".ä-hidden"] {
        fs::write(root.path().join(name), name).unwrap();
    }
    let mut tab = tab(root.path());
    tab.cursor = Some(root.path().join("beta.txt"));
    tab.selected.insert(root.path().join("beta.txt"));
    assert!(navigation::quick_find(&mut tab, "Ä", false, false, false));
    let first = tab.cursor.clone();
    assert!(navigation::quick_find(&mut tab, "ä", false, true, false));
    assert_ne!(tab.cursor, first);
    assert!(navigation::quick_find(&mut tab, "ä", true, true, false));
    assert_eq!(tab.cursor, first);
    let cursor = tab.cursor.clone();
    assert!(!navigation::quick_find(
        &mut tab, "missing", false, false, false
    ));
    assert_eq!(tab.cursor, cursor);
    assert_eq!(tab.selected.len(), 1);
    assert!(tab.filter.is_empty());
}
#[test]
fn pattern_selection_is_atomic_and_excludes_filtered_hidden_files() {
    let root = tempfile::tempdir().unwrap();
    for name in ["one.TXT", "two.txt", "image.png", ".secret.txt"] {
        fs::write(root.path().join(name), name).unwrap();
    }
    let mut tab = tab(root.path());
    assert_eq!(
        navigation::select_pattern(&mut tab, "*.txt;*.png", true, false, false).unwrap(),
        3
    );
    let previous = tab.selected.clone();
    assert!(navigation::select_pattern(&mut tab, "[", false, false, false).is_err());
    assert_eq!(tab.selected, previous);
    tab.filter = "one".into();
    assert_eq!(
        navigation::select_pattern(&mut tab, "*", false, true, false).unwrap(),
        1
    );
    assert!(tab.selected.contains(&root.path().join("two.txt")));
    assert!(!tab.selected.contains(&root.path().join(".secret.txt")));
}
#[test]
fn keyboard_range_selection_marks_every_intermediate_row() {
    let root = tempfile::tempdir().unwrap();
    for i in 0..20 {
        fs::write(root.path().join(format!("{i:02}")), b"").unwrap();
    }
    let mut tab = tab(root.path());
    tab.cursor = Some(root.path().join("00"));
    tab.move_cursor(15, true, false);
    assert_eq!(tab.selected.len(), 16);
    tab.move_cursor(-15, true, false);
    assert_eq!(tab.selected.len(), 16);
}
#[test]
fn locked_tabs_request_an_unlocked_navigation_and_metadata_survives_settings() {
    let root = tempfile::tempdir().unwrap();
    let mut tab = tab(root.path());
    tab.state.title = "Pinned work".into();
    tab.state.locked = Some(tab.location.clone());
    let original = tab.location.clone();
    let target = Location::Local(root.path().join("other"));
    tab.navigate(target.clone(), &UiSignal::default());
    assert_eq!(tab.location, original);
    assert_eq!(tab.requested_navigation(), Some(&target));
    let mut settings = Settings::default();
    settings.tab_states[0].push(tab.state.clone());
    settings.preferences.layout.orientation = navigation::Orientation::Stacked;
    settings.preferences.layout.set_ratio(0.7);
    let decoded: Settings =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(decoded.tab_states[0][0], tab.state);
    assert_eq!(
        decoded.preferences.layout.orientation,
        navigation::Orientation::Stacked
    );
    assert_eq!(decoded.preferences.layout.ratio, 0.7);
}
#[test]
fn recent_locations_are_bounded_deduplicated_and_newest_first() {
    let mut recent = Vec::new();
    for i in 0..60 {
        navigation::remember(
            &mut recent,
            i.to_string(),
            Location::Local(format!("/folder/{i}").into()),
        );
    }
    assert_eq!(recent.len(), 50);
    navigation::remember(
        &mut recent,
        "Again".into(),
        Location::Local("/folder/15".into()),
    );
    assert_eq!(recent.len(), 50);
    assert_eq!(recent[0].title, "Again");
    assert_eq!(
        recent
            .iter()
            .filter(|i| i.location == Location::Local("/folder/15".into()))
            .count(),
        1
    );
}
#[test]
fn command_bar_validation_and_legacy_upgrade_preserve_custom_bindings() {
    let mut preferences = Preferences {
        command_actions: vec!["NewWindow".into(), "SelectPattern".into(), "Stacked".into()],
        ..Default::default()
    };
    rust_commander::preferences::validate(&preferences).unwrap();
    preferences.command_actions.push("NotACommand".into());
    assert!(rust_commander::preferences::validate(&preferences).is_err());
    preferences.shortcuts.clear();
    preferences
        .shortcuts
        .insert("Copy".into(), "Command+L".into());
    preferences.upgrade();
    assert_eq!(preferences.shortcuts["Copy"], "Command+L");
    assert!(preferences.shortcuts["FocusAddress"].is_empty());
}
#[cfg(unix)]
#[test]
fn ownership_columns_and_permission_sort_use_local_metadata() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a"), b"").unwrap();
    fs::write(root.path().join("z"), b"").unwrap();
    fs::set_permissions(root.path().join("a"), fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(root.path().join("z"), fs::Permissions::from_mode(0o400)).unwrap();
    let mut entries = filesystem::list(&Location::Local(root.path().into())).unwrap();
    filesystem::sort_entries(&mut entries, filesystem::Sort::Permissions, false);
    assert_eq!(entries[0].name, "z");
    assert!(filesystem::ownership(&entries[0].path).0.is_some());
    assert_eq!(
        filesystem::ownership(Path::new("remote-relative")),
        (None, None, None)
    );
    fs::set_permissions(root.path().join("z"), fs::Permissions::from_mode(0o600)).unwrap();
}
#[test]
fn encoded_edit_preserves_bom_crlf_and_undo_restores_exact_original_bytes() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("utf16.txt");
    let original = [
        vec![0xff, 0xfe],
        "Hello\r\n世界\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect(),
    ]
    .concat();
    fs::write(&file, &original).unwrap();
    let document = Document::detect(original.clone()).unwrap();
    assert_eq!(document.encoding, Encoding::Utf16Le);
    assert_eq!(document.encode(&document.text).unwrap(), original);
    let bytes = document.encode("Changed\n世界\n").unwrap();
    let record = history::execute_in(
        Operation::SaveBytes {
            path: file.clone(),
            original: original.clone(),
            bytes: bytes.clone(),
        },
        &mut Task::default(),
        &root.path().join("history"),
    )
    .unwrap();
    assert!(record.success, "{}", record.result);
    assert_eq!(fs::read(&file).unwrap(), bytes);
    history::undo(&record, &mut Task::default()).unwrap();
    assert_eq!(fs::read(&file).unwrap(), original);
    fs::write(&file, b"external").unwrap();
    assert!(operations::save_bytes(&file, &original, &bytes).is_err());
    assert_eq!(fs::read(&file).unwrap(), b"external");
}
#[test]
fn legacy_encodings_round_trip_and_refuse_unrepresentable_characters() {
    let document = Document::decode(vec![0x80, 0x20, 0xe9], Encoding::Windows1252).unwrap();
    assert_eq!(document.text, "€ é");
    assert_eq!(document.encode(&document.text).unwrap(), document.original);
    assert!(document.encode("世界").is_err());
    let latin = Document::decode(vec![0x80, 0xff], Encoding::Latin1).unwrap();
    assert_eq!(latin.encode(&latin.text).unwrap(), vec![0x80, 0xff]);
    assert!(Document::decode(vec![0xff, 0xfe, 0x41], Encoding::Utf16Le).is_err());
}
#[test]
fn large_unicode_file_pages_are_bounded_and_reassemble_without_splitting_characters() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("large.txt");
    let text = "ab🌍世界\n".repeat(240_000);
    fs::write(&file, text.as_bytes()).unwrap();
    let mut offset = 0;
    let mut actual = String::new();
    while offset < text.len() as u64 {
        let page = Page::read(&file, offset, Encoding::Utf8).unwrap();
        assert!(page.next > offset);
        assert!(page.document.original.len() <= rust_commander::text_document::PAGE_SIZE as usize);
        actual.push_str(&page.document.text);
        offset = page.next;
    }
    assert_eq!(actual, text);
    assert!(matches!(
        rust_commander::viewer::load(&Location::Local(root.path().into()), &file).unwrap(),
        rust_commander::viewer::Content::LargeText(_)
    ));
}
#[test]
fn mucommander_xml_preview_merges_supported_content_without_touching_sources() {
    let root = tempfile::tempdir().unwrap();
    let bookmarks = r#"<bookmarks><bookmark><name>Work &amp; café</name><location>file:///tmp/work%20space</location></bookmark><bookmark><name>Server</name><location>sftp://user:secret@host/path</location></bookmark></bookmarks>"#;
    fs::write(root.path().join("bookmarks.xml"), bookmarks).unwrap();
    fs::write(root.path().join("commands.xml"),r#"<commands><command alias="open" value="open $f" type="system"/><command alias="pipe" value="cat $f | less" type="system"/></commands>"#).unwrap();
    fs::write(root.path().join("action_keymap.xml"),r#"<keymap><action id="Copy" keystroke="ctrl pressed F9"/><action id="Unported" keystroke="F10"/><action id="SplitHorizontally" keystroke="alt pressed F11"/><action id="SplitVertically" keystroke="alt pressed F12"/></keymap>"#).unwrap();
    fs::write(root.path().join("preferences.xml"),"<prefs><ui><toolbar><visible>false</visible></toolbar></ui><show_hidden_files>true</show_hidden_files><unknown>value</unknown></prefs>").unwrap();
    let preview = config_import::preview(root.path()).unwrap();
    assert_eq!(preview.bookmarks.len(), 1);
    assert_eq!(preview.bookmarks[0].name, "Work & café");
    assert_eq!(preview.commands.len(), 1);
    assert_eq!(preview.shortcuts["Copy"], "Command+F9");
    assert_eq!(preview.shortcuts["Stacked"], "Alt+F11");
    assert_eq!(preview.shortcuts["SideBySide"], "Alt+F12");
    assert!(preview.unsupported.len() >= 4);
    let mut settings = Settings::default();
    preview.apply(&mut settings, true).unwrap();
    assert_eq!(settings.bookmarks.len(), 1);
    assert_eq!(settings.preferences.shortcuts["Copy"], "Command+F9");
    assert!(!settings.preferences.toolbar);
    assert!(settings.show_hidden);
    preview.apply(&mut settings, false).unwrap();
    assert_eq!(settings.bookmarks.len(), 1);
    assert_eq!(settings.imported_commands.len(), 1);
    assert_eq!(
        fs::read_to_string(root.path().join("bookmarks.xml")).unwrap(),
        bookmarks
    );
}
#[test]
fn import_rejects_external_entities_and_commands_preserve_filename_arguments() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("bookmarks.xml"),
        r#"<!DOCTYPE root [<!ENTITY x SYSTEM "file:///etc/passwd">]><root>&x;</root>"#,
    )
    .unwrap();
    assert!(config_import::preview(root.path()).is_err());
    let command = ImportedCommand::parse("test", "/bin/echo \"name=$n\" $f $d").unwrap();
    let files = vec![
        root.path().join("a $d; $(echo pwned).txt"),
        root.path().join("two words"),
    ];
    let arguments = command.expand(&files, root.path()).unwrap();
    assert_eq!(arguments[1], "name=a $d; $(echo pwned).txt");
    assert_eq!(arguments[2], files[0].display().to_string());
    assert_eq!(arguments[3], files[1].display().to_string());
    assert_eq!(arguments.len(), 5);
    assert!(ImportedCommand::parse("bad", "echo $(secret)").is_err());
}

#[test]
fn reviewed_import_backups_are_exact_and_external_changes_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.json");
    let settings = Settings::default();
    let original = serde_json::to_vec_pretty(&settings).unwrap();
    fs::write(&path, &original).unwrap();
    let preview = config_import::Preview {
        bookmarks: vec![rust_commander::settings::Bookmark {
            name: "Imported".into(),
            location: Location::Local("/tmp/imported".into()),
        }],
        ..Default::default()
    };
    let (updated, backup) =
        config_import::commit(&path, &settings, &preview, false, Some(&original)).unwrap();
    assert_eq!(updated.bookmarks.len(), 1);
    assert_eq!(fs::read(backup.unwrap()).unwrap(), original);
    let external = b"{\"show_hidden\":true}";
    fs::write(&path, external).unwrap();
    assert!(config_import::commit(&path, &settings, &preview, false, Some(&original)).is_err());
    assert_eq!(fs::read(&path).unwrap(), external);
}

#[test]
fn quick_find_normalizes_mac_decomposed_unicode_names() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("cafe\u{301}.txt"), b"text").unwrap();
    let mut tab = tab(root.path());
    assert!(navigation::quick_find(
        &mut tab, "CAFÉ", false, false, false
    ));
}

#[test]
fn upstream_alternate_shortcuts_are_imported_and_custom_bindings_are_preserved() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("action_keymap.xml"),
        r#"<keymap><action id="Copy" keystroke="F9" alt_keystroke="alt pressed F9"/></keymap>"#,
    )
    .unwrap();
    let preview = config_import::preview(root.path()).unwrap();
    assert_eq!(preview.alternate_shortcuts["Copy"], "Alt+F9");
    let mut settings = Settings::default();
    settings
        .preferences
        .shortcuts
        .insert("Copy".into(), "Command+K".into());
    preview.apply(&mut settings, false).unwrap();
    assert_eq!(settings.preferences.shortcuts["Copy"], "Command+K");
    assert_eq!(settings.preferences.alternate_shortcuts["Copy"], "Alt+F9");
}

#[test]
fn utf16_pages_keep_surrogates_intact_in_both_directions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("utf16.txt");
    let text = "x🌍世界\n".repeat(60_000);
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        let bytes = text
            .encode_utf16()
            .flat_map(|u| {
                if encoding == Encoding::Utf16Le {
                    u.to_le_bytes()
                } else {
                    u.to_be_bytes()
                }
            })
            .collect::<Vec<_>>();
        fs::write(&path, &bytes).unwrap();
        let mut offset = 0;
        let mut actual = String::new();
        while offset < bytes.len() as u64 {
            let page = Page::read(&path, offset, encoding).unwrap();
            actual.push_str(&page.document.text);
            if page.offset > 0 {
                Page::read(
                    &path,
                    page.offset
                        .saturating_sub(rust_commander::text_document::PAGE_SIZE),
                    encoding,
                )
                .unwrap();
            }
            offset = page.next;
        }
        assert_eq!(actual, text);
        // An arbitrary position at the low half of a surrogate must remain decodable.
        let page = Page::read(&path, 4, encoding).unwrap();
        assert_eq!(page.offset, 6);
    }
}
