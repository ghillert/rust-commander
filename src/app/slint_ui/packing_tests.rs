use super::*;

fn setup() -> (
    tempfile::TempDir,
    CommanderWindow,
    Rc<RefCell<Adapter>>,
    PathBuf,
    PathBuf,
) {
    i_slint_backend_testing::init_no_event_loop();
    let (root, mut adapter, left, right) = tests::fixture();
    adapter
        .app
        .tab_mut()
        .selected
        .extend([left.join("alpha.txt"), left.join("café.txt")]);
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.window().set_size(slint::LogicalSize::new(1240.0, 780.0));
    ui.show().unwrap();
    slint::platform::update_timers_and_animations();
    (root, ui, state, left, right)
}
fn field(ui: &CommanderWindow, id: &str) -> FormField {
    ui.get_dialog_fields()
        .iter()
        .find(|f| f.id == id)
        .unwrap_or_else(|| panic!("Missing field {id}"))
}
fn wait_for_job(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow) {
    for _ in 0..2000 {
        {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app.poll(&ctx);
            a.sync(ui);
            if a.app.job.is_none() {
                assert!(!a.app.status_error, "{}", a.app.status);
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("Packing worker did not complete");
}

#[test]
fn pack_dialog_tracks_format_comment_selection_and_validation() {
    let (_root, ui, state, left, right) = setup();
    ui.invoke_action("Pack".into());
    assert_eq!(ui.get_dialog_title(), "Pack files");
    assert_eq!(
        field(&ui, "prompt").value,
        right.join("left.zip").display().to_string()
    );
    assert_eq!(field(&ui, "pack-format").value, "ZIP");
    assert!(field(&ui, "pack-format").choices.iter().any(|s| s == "7z"));
    assert!(!ui.get_dialog_fields().iter().any(|f| f.kind == 9));
    ui.invoke_dialog_action("pack-files".into());
    let shown = ui
        .get_dialog_fields()
        .iter()
        .filter(|f| f.kind == 9)
        .map(|f| f.value.to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        vec![
            left.join("alpha.txt").display().to_string(),
            left.join("café.txt").display().to_string()
        ]
    );
    ui.invoke_dialog_field("pack-comment".into(), "My ZIP comment".into());
    ui.invoke_dialog_field("pack-format".into(), "7z".into());
    assert!(field(&ui, "prompt").value.ends_with("left.7z"));
    assert!(
        !ui.get_dialog_fields()
            .iter()
            .any(|f| f.id == "pack-comment")
    );
    ui.invoke_dialog_field("pack-format".into(), "TAR.GZ".into());
    ui.invoke_dialog_field("pack-format".into(), "ZIP".into());
    assert!(field(&ui, "prompt").value.ends_with("left.zip"));
    assert_eq!(field(&ui, "pack-comment").value, "My ZIP comment");
    ui.invoke_dialog_field("pack-format".into(), "GZip (one file)".into());
    ui.invoke_dialog_action("accept".into());
    assert!(ui.get_dialog_error().contains("one regular file"));
    assert!(state.borrow().app.job.is_none());
    ui.invoke_dialog_field("pack-format".into(), "ZIP".into());
    ui.invoke_dialog_field("prompt".into(), "".into());
    ui.invoke_dialog_action("accept".into());
    assert!(ui.get_dialog_error().contains("destination"));
    assert!(state.borrow().app.job.is_none());
    ui.invoke_dialog_action("cancel".into());
    assert!(ui.get_dialog_title().is_empty());
    assert!(std::fs::read_dir(right).unwrap().next().is_none());
}

#[cfg(debug_assertions)]
#[test]
fn right_click_pack_and_selection_disclosure_use_physical_clicks() {
    use i_slint_backend_testing::ElementHandle;
    use slint::platform::PointerEventButton::{Left, Right};
    let (_root, ui, state, _left, _right) = setup();
    ElementHandle::find_by_accessible_label(&ui, "alpha.txt")
        .next()
        .unwrap()
        .mock_single_click(Right);
    let title = ui
        .get_menu_items()
        .iter()
        .find(|c| c.id == "Pack")
        .unwrap()
        .title;
    ElementHandle::find_by_accessible_label(&ui, &title)
        .next()
        .unwrap()
        .mock_single_click(Left);
    assert_eq!(ui.get_dialog_title(), "Pack files");
    slint::platform::update_timers_and_animations();
    ElementHandle::find_by_accessible_label(&ui, "2 file(s)")
        .next()
        .unwrap()
        .mock_single_click(Left);
    assert_eq!(
        ui.get_dialog_fields()
            .iter()
            .filter(|f| f.kind == 9)
            .count(),
        2
    );
    slint::platform::update_timers_and_animations();
    ElementHandle::find_by_accessible_label(&ui, "2 file(s)")
        .next()
        .unwrap()
        .mock_single_click(Left);
    assert!(!ui.get_dialog_fields().iter().any(|f| f.kind == 9));
    ElementHandle::find_by_accessible_label(&ui, "Comment")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::TextInput))
        .expect("Comment text input")
        .mock_single_click(Left);
    for ch in "Release note".chars() {
        let text: SharedString = ch.to_string().into();
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    assert_eq!(field(&ui, "pack-comment").value, "Release note");
    ElementHandle::find_by_accessible_label(&ui, "Run in background")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Checkbox))
        .expect("Background checkbox")
        .mock_single_click(Left);
    assert_eq!(field(&ui, "pack-background").value, "true");
    ElementHandle::find_by_accessible_label(&ui, "Archive format")
        .find(|e| e.accessible_role() == Some(i_slint_backend_testing::AccessibleRole::Combobox))
        .unwrap()
        .mock_single_click(Left);
    slint::platform::update_timers_and_animations();
    ElementHandle::find_by_accessible_label(&ui, "7z")
        .next()
        .expect("7z format option")
        .mock_single_click(Left);
    assert_eq!(field(&ui, "pack-format").value, "7z");
    assert!(field(&ui, "prompt").value.ends_with(".7z"));
    ui.invoke_dialog_action("cancel".into());
    assert_eq!(state.borrow().app.tab().selected.len(), 2);
}

#[test]
fn zip_and_sevenz_dialogs_create_the_marked_selection_with_real_workers() {
    let (_root, ui, state, left, right) = setup();
    for (format, extension, background) in [("ZIP", "zip", false), ("7z", "7z", true)] {
        // Refresh can briefly leave the row list loading; the selection belongs
        // to the source tab and is deliberately independent of its cursor.
        {
            let mut a = state.borrow_mut();
            a.app.tab_mut().entries = filesystem::list(&Location::Local(left.clone())).unwrap();
            a.app.tab_mut().loading = false;
            a.app
                .tab_mut()
                .selected
                .extend([left.join("alpha.txt"), left.join("café.txt")]);
            a.app.tab_mut().revision += 1;
            a.sync(&ui);
        }
        ui.invoke_action("Pack".into());
        ui.invoke_dialog_field("pack-comment".into(), "A useful ZIP comment".into());
        ui.invoke_dialog_field("pack-format".into(), format.into());
        ui.invoke_dialog_field("pack-background".into(), background.to_string().into());
        ui.invoke_dialog_action("accept".into());
        assert!(state.borrow().app.prompt.is_none());
        assert_eq!(
            state.borrow().app.job.as_ref().unwrap().foreground,
            !background
        );
        assert_eq!(state.borrow().blocked(), !background);
        if !background {
            assert_eq!(ui.get_dialog_title(), "Packing files");
            ui.invoke_dialog_action("pack-background".into());
            assert!(!state.borrow().blocked());
        }
        wait_for_job(&state, &ui);
        let archive = right.join(format!("left.{extension}"));
        let mut names = crate::archive::list(&archive, Path::new(""))
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["alpha.txt", "café.txt"]);
        if format == "ZIP" {
            let zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
            assert_eq!(zip.comment(), b"A useful ZIP comment");
        }
        let output = right.join(format!("extracted-{extension}"));
        crate::archive::extract(&archive, &[], Path::new(""), &output, &mut Task::default())
            .unwrap();
        assert_eq!(std::fs::read(output.join("alpha.txt")).unwrap(), b"alpha");
        assert_eq!(
            std::fs::read(output.join("café.txt")).unwrap(),
            "unicode".as_bytes()
        );
    }
}

#[test]
fn foreground_progress_cancels_the_worker_and_can_be_sent_to_background() {
    let (_root, ui, state, _left, _right) = setup();
    let (_sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut a = state.borrow_mut();
        a.app.job = Some(Job {
            owner: a.app.window_id,
            foreground: true,
            label: "Create archive".into(),
            receiver,
            control: None,
            cancel: cancel.clone(),
            progress: Default::default(),
        });
        a.sync(&ui);
    }
    assert_eq!(ui.get_dialog_title(), "Packing files");
    assert!(state.borrow().blocked());
    let mut escape = slint::language::KeyEvent::default();
    escape.text = slint::platform::Key::Escape.into();
    assert!(ui.invoke_global_key(escape));
    assert!(
        cancel.load(Ordering::Relaxed),
        "Escape cancels foreground packing"
    );
    cancel.store(false, Ordering::Relaxed);
    ui.invoke_dialog_action("cancel-pack".into());
    assert!(cancel.load(Ordering::Relaxed));
    assert!(ui.get_dialog_description().contains("Cancelling"));
    ui.invoke_dialog_action("pack-background".into());
    assert!(!state.borrow().blocked());
    assert!(ui.get_dialog_title().is_empty());
}
