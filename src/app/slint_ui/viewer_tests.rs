use super::*;
use i_slint_backend_testing::ElementHandle;

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
