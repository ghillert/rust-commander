use super::*;
use i_slint_backend_testing::ElementHandle;

slint::slint! {
    export component SourceClipboardProbe inherits Window {
        in-out property <string> text;
        input := TextInput { text <=> root.text; single-line: false; }
        public function paste() { input.paste(); }
    }
}

fn setup(
    name: &str,
    contents: &[u8],
) -> (
    tempfile::TempDir,
    CommanderWindow,
    Rc<RefCell<Adapter>>,
    PathBuf,
) {
    i_slint_backend_testing::init_no_event_loop();
    let (root, mut a, left, _) = tests::fixture();
    let path = left.join(name);
    std::fs::write(&path, contents).unwrap();
    a.app.tab_mut().entries = filesystem::list(&Location::Local(left)).unwrap();
    a.app.tab_mut().cursor = Some(path.clone());
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(a));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.window().set_size(slint::LogicalSize::new(1240.0, 780.0));
    ui.show().unwrap();
    slint::platform::update_timers_and_animations();
    (root, ui, state, path)
}
fn wait(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow) {
    for _ in 0..1500 {
        let ready = {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app.poll(&ctx);
            a.viewer_tools_tick();
            a.sync(ui);
            a.app.pending_view.is_none()
                && a.app.pending_pdf.is_none()
                && a.app.pending_archive.is_none()
                && a.app.editor.as_ref().is_none_or(|e| {
                    e.navigation.pending.is_none() && !e.navigation.presentation.busy()
                })
                && a.app
                    .editor
                    .as_ref()
                    .is_none_or(|e| e.pdf.as_ref().is_none_or(|p| p.search_pending.is_none()))
        };
        if ready {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("Viewer worker did not finish");
}
fn open(ui: &CommanderWindow, mode: &str) {
    ui.invoke_action("OpenAs".into());
    assert_eq!(ui.get_dialog_title(), "Open As");
    assert!(ui.get_dialog_compact());
    assert!(ui.get_dialog_preferred_height() <= 300.0);
    ui.invoke_dialog_field("open-format".into(), mode.into());
    ui.invoke_dialog_action("accept".into());
}

#[test]
fn open_as_encrypted_archive_uses_the_password_without_renaming_the_source() {
    let (_root, ui, state, path) = setup("protected.bin", b"placeholder");
    let source = path.with_file_name("payload.txt");
    std::fs::write(&source, b"protected payload").unwrap();
    let archive = path.with_extension("7z");
    crate::archive::pack_with_password(&[source], &archive, "archive-test", &mut Task::default())
        .unwrap();
    std::fs::rename(archive, &path).unwrap();
    ui.invoke_action("OpenAs".into());
    ui.invoke_dialog_field("open-format".into(), "7z archive".into());
    let password = ui
        .get_dialog_fields()
        .iter()
        .find(|f| f.id == "open-password")
        .unwrap();
    assert_eq!(password.kind, 1);
    ui.invoke_dialog_field("open-password".into(), "archive-test".into());
    ui.invoke_dialog_action("accept".into());
    wait(&state, &ui);
    let a = state.borrow();
    let Location::Archive { file, .. } = &a.app.tab().location else {
        panic!("Archive did not open: {:?}", a.app.message);
    };
    assert_eq!(
        crate::archive::read_bytes(file, Path::new("payload.txt"), 1024).unwrap(),
        b"protected payload"
    );
    assert!(!crate::archive_tools::editable(file));
    assert!(path.exists());
}
#[test]
fn open_as_keeps_the_selected_source_when_focus_changes() {
    let (_root, ui, state, path) = setup("misleading.pdf", b"plain text");
    ui.invoke_action("OpenAs".into());
    state.borrow_mut().app.active = 1;
    ui.invoke_dialog_field("open-format".into(), "Hexadecimal".into());
    ui.invoke_dialog_action("accept".into());
    wait(&state, &ui);
    assert!(ui.get_viewer_visible());
    assert!(ui.get_viewer_text().starts_with("00000000"));
    assert_eq!(
        state.borrow().app.editor.as_ref().unwrap().path.as_ref(),
        Some(&path)
    );
    assert!(!ui.get_viewer_editable());
    assert_eq!(std::fs::read(path).unwrap(), b"plain text");
}
#[test]
fn pdf_search_selects_visible_text_and_survives_rotation_and_text_view() {
    let (_root, ui, state, _) = setup(
        "document.bin",
        include_bytes!("../../../tests/fixtures/two-pages.pdf"),
    );
    open(&ui, "PDF");
    wait(&state, &ui);
    assert!(ui.get_viewer_pdf_document());
    ui.invoke_viewer_action("pdf-find:two".into());
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_page(), "2 / 2");
    assert!(ui.get_viewer_pdf_copy());
    assert!(ui.get_viewer_pdf_marks().row_count() > 0);
    {
        let a = state.borrow();
        let pdf = a.app.editor.as_ref().unwrap().pdf.as_ref().unwrap();
        assert_eq!(&pdf.text.text[pdf.selection.clone().unwrap()], "two");
    }
    ui.invoke_viewer_action("rotate".into());
    wait(&state, &ui);
    assert_eq!(
        state
            .borrow()
            .app
            .editor
            .as_ref()
            .unwrap()
            .pdf
            .as_ref()
            .unwrap()
            .rotation,
        1
    );
    assert!(ui.get_viewer_pdf_copy());
    ui.invoke_viewer_action("pdf-text".into());
    assert!(ui.get_viewer_pdf_text());
    assert_eq!(ui.get_viewer_image().size().width, 0);
    assert_eq!(ui.get_viewer_text(), "Page two");
    assert!(!ui.get_viewer_editable());
    ui.invoke_viewer_action("pdf-find-previous:Page".into());
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_page(), "2 / 2");
    ui.invoke_viewer_action("previous".into());
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_text(), "Page one");
    assert!(!ui.get_viewer_pdf_copy());
}
#[test]
fn pdf_pointer_selection_uses_page_coordinates_at_fit_width() {
    let (_root, ui, state, _) = setup(
        "document.pdf",
        include_bytes!("../../../tests/fixtures/two-pages.pdf"),
    );
    open(&ui, "PDF");
    wait(&state, &ui);
    ui.invoke_viewer_action("fit-width".into());
    slint::platform::update_timers_and_animations();
    let page = ElementHandle::find_by_accessible_label(&ui, "PDF page")
        .next()
        .unwrap();
    let position = page.absolute_position();
    let size = page.size();
    let (first, last) = {
        let a = state.borrow();
        let text = &a.app.editor.as_ref().unwrap().pdf.as_ref().unwrap().text;
        (text.glyphs[0].bounds, text.glyphs[3].bounds)
    };
    let point = |b: [f32; 4]| {
        slint::LogicalPosition::new(
            position.x + (b[0] + b[2]) / 2.0 * size.width,
            position.y + (b[1] + b[3]) / 2.0 * size.height,
        )
    };
    let first = point(first);
    let last = point(last);
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position: first,
            button: slint::platform::PointerEventButton::Left,
        });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved { position: last });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position: last,
            button: slint::platform::PointerEventButton::Left,
        });
    let a = state.borrow();
    let pdf = a.app.editor.as_ref().unwrap().pdf.as_ref().unwrap();
    assert_eq!(&pdf.text.text[pdf.selection.clone().unwrap()], "Page");
}
#[test]
fn encrypted_pdf_unlock_and_search_share_the_password() {
    let (_root, ui, state, _) = setup(
        "document.bin",
        include_bytes!("../../../tests/fixtures/two-pages-protected.pdf"),
    );
    open(&ui, "PDF");
    wait(&state, &ui);
    assert!(ui.get_viewer_locked());
    ui.invoke_viewer_action("unlock:wrong".into());
    wait(&state, &ui);
    assert!(ui.get_viewer_locked());
    assert!(!ui.get_viewer_error().is_empty());
    ui.invoke_viewer_action("unlock:commander-test".into());
    wait(&state, &ui);
    assert!(!ui.get_viewer_locked());
    ui.invoke_viewer_action("pdf-find:two".into());
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_page(), "2 / 2");
    assert!(ui.get_viewer_pdf_copy());
    let cancel = state
        .borrow()
        .app
        .editor
        .as_ref()
        .unwrap()
        .pdf
        .as_ref()
        .unwrap()
        .search_cancel
        .clone();
    ui.invoke_viewer_action("close".into());
    assert!(cancel.load(Ordering::Relaxed));
}

#[test]
fn source_modes_preserve_original_bytes_and_search_the_displayed_text() {
    let original = "{\r\n\t\"snow\": \"雪\", \"b\": [1,2]\r\n}\r\n";
    let (_root, ui, state, path) = setup("source.json", original.as_bytes());
    open(&ui, "Text");
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_source_mode(), "Highlighted original");
    assert!(ui.get_viewer_source_active());
    assert!(
        ui.get_viewer_source_groups()
            .iter()
            .any(|group| group.lines.row_count() > 0)
    );
    assert_eq!(ui.get_viewer_text(), original);
    assert!(!ui.get_viewer_dirty());
    ui.invoke_select_all_viewer();
    ui.invoke_copy_viewer();
    let original_probe = SourceClipboardProbe::new().unwrap();
    original_probe.invoke_paste();
    assert_eq!(original_probe.get_text(), original);
    ui.invoke_viewer_action("source-mode:Pretty view".into());
    wait(&state, &ui);
    let pretty = ui.get_viewer_text().to_string();
    assert!(pretty.contains("\n  \"b\": [\n    1,\n    2\n  ]"));
    assert!(!ui.get_viewer_editable());
    ui.invoke_viewer_action("find:\n  \"b\"".into());
    assert_eq!(
        state.borrow().app.editor.as_ref().unwrap().navigation.last,
        Some(pretty.find("\n  \"b\"").unwrap() as u64)
    );
    ui.invoke_select_all_viewer();
    ui.invoke_copy_viewer();
    let probe = SourceClipboardProbe::new().unwrap();
    probe.invoke_paste();
    assert_eq!(probe.get_text(), pretty);
    let start = pretty.find("雪").unwrap();
    let end = pretty.find("2\n").unwrap() + 1;
    ui.invoke_select_viewer(start as i32, end as i32);
    ui.invoke_copy_viewer();
    probe.set_text("".into());
    probe.invoke_paste();
    assert_eq!(probe.get_text(), &pretty[start..end]);
    ui.invoke_viewer_action("source-mode:Plain original".into());
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_text(), original);
    assert!(!ui.get_viewer_source_active());
    assert!(!ui.get_viewer_dirty());
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
    ui.invoke_viewer_action("source-mode:Highlighted original".into());
    ui.invoke_viewer_action("find:雪".into());
    wait(&state, &ui);
    ui.invoke_copy_viewer();
    probe.set_text("".into());
    probe.invoke_paste();
    assert_eq!(probe.get_text(), "雪");
    ui.invoke_viewer_action("hex-find:7b".into());
    wait(&state, &ui);
    assert_eq!(
        state
            .borrow()
            .app
            .editor
            .as_ref()
            .unwrap()
            .navigation
            .presentation
            .mode,
        crate::source_view::Mode::Plain
    );
    assert!(ui.get_viewer_text().starts_with("00000000"));
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
}

#[test]
fn native_viewer_events_search_the_display_and_reject_previous_documents() {
    let original = br#"{"snow":"value"}"#;
    let (_root, ui, state, path) = setup("native.json", original);
    open(&ui, "Text");
    wait(&state, &ui);
    let document = state.borrow().app.editor.as_ref().unwrap().navigation.id;
    state
        .borrow()
        .native_viewer
        .test_event(document, "mode", Some("Pretty view"));
    wait(&state, &ui);
    let query = "\n  \"snow\"";
    state
        .borrow()
        .native_viewer
        .test_event(document, "query", Some(query));
    state
        .borrow()
        .native_viewer
        .test_event(document, "find-next", None);
    wait(&state, &ui);
    assert_eq!(
        state.borrow().app.editor.as_ref().unwrap().navigation.last,
        Some(ui.get_viewer_text().find(query).unwrap() as u64)
    );
    ui.invoke_viewer_action("close".into());
    assert!(state.borrow().app.editor.is_none());
    {
        let entry = filesystem::list(&Location::Local(path.parent().unwrap().to_path_buf()))
            .unwrap()
            .into_iter()
            .find(|e| e.path == path)
            .unwrap();
        let mut a = state.borrow_mut();
        let ctx = a.ctx.clone();
        a.app.view(entry, false, &ctx);
    }
    wait(&state, &ui);
    let reopened = state.borrow().app.editor.as_ref().unwrap().navigation.id;
    assert_ne!(document, reopened);
    state
        .borrow()
        .native_viewer
        .test_event(document, "close", None);
    state
        .borrow()
        .native_viewer
        .test_event(document, "mode", Some("Plain original"));
    wait(&state, &ui);
    assert_eq!(
        state.borrow().app.editor.as_ref().unwrap().navigation.id,
        reopened
    );
    assert_eq!(ui.get_viewer_source_mode(), "Highlighted original");
    assert!(!ui.get_viewer_dirty());
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn viewer_toolbar_keeps_original_accessible_and_find_dismisses_before_closing() {
    let original = br#"{"value":1}"#;
    let (_root, ui, state, path) = setup("toolbar.json", original);
    open(&ui, "Text");
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_title(), "toolbar.json");
    assert!(ui.get_viewer_path().ends_with("toolbar.json"));
    assert!(!ui.get_viewer_find_visible());

    ElementHandle::find_by_accessible_label(&ui, "Plain original")
        .next()
        .unwrap()
        .mock_single_click(slint::platform::PointerEventButton::Left);
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_source_mode(), "Plain original");
    assert_eq!(ui.get_viewer_text(), String::from_utf8_lossy(original));

    let mut find = slint::language::KeyEvent::default();
    find.text = "f".into();
    find.modifiers.control = true;
    assert!(ui.invoke_global_key(find));
    assert!(ui.get_viewer_find_visible());
    let mut escape = slint::language::KeyEvent::default();
    escape.text = slint::platform::Key::Escape.into();
    assert!(ui.invoke_global_key(escape.clone()));
    assert!(!ui.get_viewer_find_visible());
    assert!(ui.get_viewer_visible());
    assert!(ui.invoke_global_key(escape));
    assert!(!ui.get_viewer_visible());
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn viewer_tools_are_transient_and_do_not_modify_the_source() {
    let original = br#"{"value":1}"#;
    let (_root, ui, state, path) = setup("tools.json", original);
    open(&ui, "Text");
    wait(&state, &ui);
    ui.invoke_show_viewer_tools();
    assert!(ui.get_viewer_tools_visible());
    assert!(ui.invoke_dismiss_viewer_tools());
    assert!(!ui.get_viewer_tools_visible());
    ui.invoke_show_viewer_tools();
    ui.invoke_focus_viewer_search();
    assert!(!ui.get_viewer_tools_visible());
    assert!(ui.get_viewer_find_visible());
    assert_eq!(ui.get_viewer_source_mode(), "Highlighted original");
    assert!(!ui.get_viewer_dirty());
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn editing_and_saving_never_use_the_pretty_buffer() {
    let original = r#"{"z":1,"z":2}"#;
    let edited = r#"{"z":1,"z":2,"unsaved":true}"#;
    let (_root, ui, state, path) = setup("edit.json", original.as_bytes());
    {
        let mut a = state.borrow_mut();
        let entry = a
            .app
            .tab()
            .entries
            .iter()
            .find(|e| e.path == path)
            .unwrap()
            .clone();
        let ctx = a.ctx.clone();
        a.app.view(entry, true, &ctx);
    }
    wait(&state, &ui);
    assert!(ui.get_viewer_editable());
    ui.invoke_viewer_edit(edited.into());
    assert!(ui.get_viewer_dirty());
    let revision = state
        .borrow()
        .app
        .editor
        .as_ref()
        .unwrap()
        .navigation
        .draft_revision;
    for mode in [
        "Highlighted original",
        "Pretty view",
        "Plain original",
        "Pretty view",
    ] {
        ui.invoke_viewer_action(format!("source-mode:{mode}").into());
        wait(&state, &ui);
        let a = state.borrow();
        let editor = a.app.editor.as_ref().unwrap();
        assert_eq!(editor.text, edited);
        assert!(editor.dirty());
        assert_eq!(editor.navigation.draft_revision, revision);
        assert_eq!(
            editor.document.as_ref().unwrap().original,
            original.as_bytes()
        );
    }
    assert!(!ui.get_viewer_editable());
    assert!(ui.get_viewer_dirty());
    assert!(ui.get_viewer_can_save());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    ui.invoke_focus_viewer_search();
    ui.invoke_viewer_action("close".into());
    assert!(ui.get_viewer_confirm_close());
    let mut escape = slint::language::KeyEvent::default();
    escape.text = slint::platform::Key::Escape.into();
    assert!(ui.invoke_global_key(escape));
    assert!(!ui.get_viewer_confirm_close());
    assert!(ui.get_viewer_find_visible());
    assert!(ui.get_viewer_dirty());
    // Even the Save shortcut from a read-only presentation saves only the source.
    ui.invoke_viewer_action("save".into());
    for _ in 0..1500 {
        let done = {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app.poll(&ctx);
            a.sync(&ui);
            a.app.job.is_none() && a.app.pending_save.is_none()
        };
        if done {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    assert!(!state.borrow().app.editor.as_ref().unwrap().dirty());
}

fn wait_preview(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow) {
    for _ in 0..1500 {
        let done = {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app.preview.poll(&ctx);
            a.sync(ui);
            a.app.preview.source_text
                && !a.app.preview.loading
                && !a.app.preview.presentation.busy()
        };
        if done {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("Preview preparation did not finish");
}

#[test]
fn preview_modes_reset_for_new_files_and_do_not_format_truncated_documents() {
    let original = r#"{"preview":[1,2]}"#;
    let (_root, ui, state, path) = setup("preview.json", original.as_bytes());
    let location = state.borrow().app.tab().location.clone();
    let entry = state
        .borrow()
        .app
        .tab()
        .entries
        .iter()
        .find(|e| e.path == path)
        .unwrap()
        .clone();
    {
        let mut a = state.borrow_mut();
        a.app.preferences.preview = true;
        a.app.preview.select(location.clone(), Some(entry.clone()));
    }
    wait_preview(&state, &ui);
    assert_eq!(ui.get_preview_source_mode(), "Highlighted original");
    assert_eq!(ui.get_preview_text(), original);
    ui.invoke_preview_source_mode_selected("Pretty view".into());
    wait_preview(&state, &ui);
    assert!(ui.get_preview_text().contains("\n"));
    ui.invoke_preview_source_mode_selected("Plain original".into());
    wait_preview(&state, &ui);
    assert_eq!(ui.get_preview_text(), original);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
    // Retire an in-flight formatting worker while the file selection changes.
    ui.invoke_preview_source_mode_selected("Pretty view".into());
    let next = path.with_file_name("next.xml");
    std::fs::write(&next, "<root><item/></root>").unwrap();
    let next_entry = filesystem::list(&location)
        .unwrap()
        .into_iter()
        .find(|e| e.path == next)
        .unwrap();
    state
        .borrow_mut()
        .app
        .preview
        .select(location, Some(next_entry));
    wait_preview(&state, &ui);
    assert_eq!(ui.get_preview_source_mode(), "Highlighted original");
    assert_eq!(ui.get_preview_text(), "<root><item/></root>");
    {
        let mut a = state.borrow_mut();
        a.app.preview.complete = false;
        a.app.preview.presentation.mode = crate::source_view::Mode::Pretty;
        a.sync(&ui);
    }
    assert_eq!(ui.get_preview_source_mode(), "Plain original");
    assert!(ui.get_preview_source_notice().contains("complete document"));
    assert_eq!(ui.get_preview_text(), "<root><item/></root>");
}

#[test]
fn utf16_source_views_preserve_the_bom_encoding_and_line_endings() {
    let source = "<root>\r\n\t<name>café 雪</name>\r\n</root>\r\n";
    let bytes: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(source.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let (_root, ui, state, path) = setup("utf16.xml", &bytes);
    open(&ui, "Text");
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_text(), source);
    assert_eq!(ui.get_viewer_encoding(), "UTF-16 LE");
    for mode in ["Pretty view", "Plain original", "Highlighted original"] {
        ui.invoke_viewer_action(format!("source-mode:{mode}").into());
        wait(&state, &ui);
        let a = state.borrow();
        let editor = a.app.editor.as_ref().unwrap();
        assert_eq!(editor.text, source);
        assert_eq!(editor.document.as_ref().unwrap().original, bytes);
        assert!(!editor.dirty());
    }
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn archive_source_highlighting_never_reopens_the_member_as_a_local_file() {
    let original = r#"{"archive":{"value":true}}"#;
    let (_root, ui, state, path) = setup("member.json", original.as_bytes());
    let archive = path.with_file_name("source.zip");
    crate::archive::pack(std::slice::from_ref(&path), &archive, &mut Task::default()).unwrap();
    let location = Location::Archive {
        file: archive.clone(),
        prefix: PathBuf::new(),
    };
    let entry = crate::archive::list(&archive, Path::new(""))
        .unwrap()
        .into_iter()
        .find(|e| e.name == "member.json")
        .unwrap();
    {
        let mut a = state.borrow_mut();
        let ctx = a.ctx.clone();
        a.app
            .view_as(entry, false, location, crate::viewer::Mode::Auto, &ctx);
    }
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_source_language(), "JSON");
    assert!(ui.get_viewer_source_active());
    ui.invoke_viewer_action("source-mode:Pretty view".into());
    wait(&state, &ui);
    assert!(ui.get_viewer_text().contains("\n"));
    ui.invoke_viewer_action("source-mode:Plain original".into());
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_text(), original);
    assert_eq!(
        crate::archive::read_bytes(&archive, Path::new("member.json"), 1024).unwrap(),
        original.as_bytes()
    );
}

#[test]
fn remote_source_views_use_the_loaded_snapshot_and_keep_the_original() {
    let original = r#"{"remote":[true,false]}"#;
    let (_root, ui, state, path) = setup("remote-source.json", original.as_bytes());
    // Exercise the remote reader with an isolated filesystem-backed session.
    let profile = crate::remote::Connection {
        service: "fs".into(),
        root: path.parent().unwrap().to_string_lossy().into_owned(),
        ..Default::default()
    };
    crate::remote::connect(&profile, Default::default(), false).unwrap();
    let location = Location::Remote {
        connection: profile.id.clone(),
        path: PathBuf::new(),
    };
    let entry = filesystem::list(&location)
        .unwrap()
        .into_iter()
        .find(|e| e.name == "remote-source.json")
        .unwrap();
    {
        let mut a = state.borrow_mut();
        let ctx = a.ctx.clone();
        a.app
            .view_as(entry, false, location, crate::viewer::Mode::Auto, &ctx);
    }
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_source_language(), "JSON");
    assert_eq!(ui.get_viewer_text(), original);
    ui.invoke_viewer_action("source-mode:Pretty view".into());
    wait(&state, &ui);
    assert!(ui.get_viewer_text().contains("\n"));
    ui.invoke_viewer_action("source-mode:Plain original".into());
    wait(&state, &ui);
    assert_eq!(ui.get_viewer_text(), original);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
}
