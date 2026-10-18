use super::*;

fn fixture() -> (tempfile::TempDir, Adapter, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let left = root.path().join("left");
    let right = root.path().join("right");
    std::fs::create_dir_all(left.join("folder")).unwrap();
    std::fs::create_dir(&right).unwrap();
    std::fs::write(left.join("alpha.txt"), "alpha").unwrap();
    std::fs::write(left.join("café.txt"), "unicode").unwrap();
    let mut adapter = Adapter::new(&[left.clone(), right.clone()], Ok(Settings::default()));
    // UI tests never load or modify the user's operation queue or configuration.
    adapter.app.settings_valid = false;
    adapter.app.queue = Default::default();
    adapter.app.history.clear();
    for side in 0..2 {
        let location = adapter.app.panes[side].tab().location.clone();
        adapter.app.panes[side].tab_mut().entries = filesystem::list(&location).unwrap();
        adapter.app.panes[side].tab_mut().loading = false;
    }
    (root, adapter, left, right)
}

#[test]
fn slint_callbacks_preserve_navigation_selection_and_confirmation() {
    i_slint_backend_testing::init_no_event_loop();
    let (_root, adapter, left, right) = fixture();
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    assert_eq!(ui.get_left_rows().row_count(), 3);
    ui.invoke_selected(0, path_id(&left.join("café.txt")).into(), false, false);
    assert_eq!(
        state.borrow().app.tab().cursor.as_deref(),
        Some(left.join("café.txt").as_path())
    );
    ui.invoke_selected(0, path_id(&left.join("alpha.txt")).into(), true, false);
    assert!(
        state
            .borrow()
            .app
            .tab()
            .selected
            .contains(&left.join("alpha.txt"))
    );
    ui.invoke_filter(0, "alpha".into());
    assert_eq!(ui.get_left_rows().row_count(), 1);
    ui.invoke_filter(0, "".into());
    assert_eq!(ui.get_left_rows().row_count(), 3);
    ui.invoke_address_edited(0, "/a/path/being/typed".into());
    state.borrow_mut().sync(&ui);
    assert_eq!(ui.get_left_address(), "/a/path/being/typed");
    ui.invoke_address_edited(0, left.to_string_lossy().as_ref().into());
    ui.invoke_action("Copy".into());
    assert!(state.borrow().app.prompt.is_some());
    assert!(!right.join("alpha.txt").exists());
    let mut transfer = slint::DataTransfer::default();
    transfer.set_file_paths([left.join("alpha.txt")]);
    assert!(
        !ui.invoke_dropped(1, transfer.clone()),
        "a modal must block additional drops"
    );
    ui.invoke_dialog_action("cancel".into());
    assert!(state.borrow().app.prompt.is_none());
    assert!(ui.invoke_dropped(1, transfer));
    assert_eq!(state.borrow().app.active, 1);
    assert!(state.borrow().app.prompt.is_some());
    assert!(!right.join("alpha.txt").exists());
    ui.invoke_dialog_action("cancel".into());
    ui.invoke_action("Mkdir:1".into());
    ui.invoke_dialog_field("prompt".into(), "../escape".into());
    ui.invoke_dialog_action("accept".into());
    assert!(
        state
            .borrow()
            .app
            .prompt
            .as_ref()
            .is_some_and(|p| p.error.is_some())
    );
    assert!(!right.parent().unwrap().join("escape").exists());
    ui.invoke_dialog_action("cancel".into());
    ui.invoke_opened(0, path_id(&left.join("folder")).into());
    assert_eq!(
        state.borrow().app.panes[0].tab().location,
        Location::Local(left.join("folder"))
    );
    ui.invoke_action("Parent:0".into());
    assert_eq!(
        state.borrow().app.tab().location,
        Location::Local(left.clone())
    );
    ui.invoke_action("NewTab:0".into());
    assert_eq!(state.borrow().app.panes[0].tabs.len(), 2);
    ui.invoke_action("CloseTab:0".into());
    assert_eq!(state.borrow().app.panes[0].tabs.len(), 1);
    ui.window().set_size(slint::LogicalSize::new(1240.0, 780.0));
    ui.show().unwrap();
    assert_eq!(ui.invoke_drop_side(500.0, 400.0), 0);
    assert_eq!(ui.invoke_drop_side(1000.0, 400.0), 1);
    assert_eq!(ui.invoke_drop_side(50.0, 400.0), -1);
    state.borrow_mut().app.preferences.single_pane = true;
    state.borrow_mut().app.active = 1;
    state.borrow_mut().sync(&ui);
    assert_eq!(ui.get_active_pane(), 1);
    assert_eq!(ui.invoke_drop_side(500.0, 400.0), 1);
    state.borrow_mut().app.preferences.single_pane = false;
    state.borrow_mut().app.active = 0;
    state.borrow_mut().sync(&ui);
    let mut search = slint::language::KeyEvent::default();
    search.text = "f".into();
    search.modifiers.shift = true;
    search.modifiers.control = true;
    assert!(ui.invoke_global_key(search));
    assert!(state.borrow().app.tools.search.is_some());
    ui.invoke_dialog_action("cancel".into());
    let mut redo = slint::language::KeyEvent::default();
    redo.text = "z".into();
    redo.modifiers.shift = true;
    redo.modifiers.control = true;
    assert!(ui.invoke_global_key(redo));
    assert!(state.borrow().app.status_error);
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        state.borrow_mut().app.panes[0].tab_mut().poll(false);
        if !state.borrow().app.panes[0].tab().loading {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "directory worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    ui.invoke_selected(0, path_id(&left.join("alpha.txt")).into(), false, false);
    ui.invoke_action("CopyNames".into());
    assert_eq!(state.borrow().ctx.take_events().0, vec!["alpha.txt"]);
    ui.invoke_action("Preferences".into());
    ui.invoke_dialog_action("cancel".into());
    // Dispatch through the actual Slint focus tree after the modal disappears.
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::F7.into(),
        });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased {
            text: slint::platform::Key::F7.into(),
        });
    assert!(state.borrow().app.prompt.is_some());
    ui.invoke_dialog_action("cancel".into());
    // Exercise native modifier reconciliation through the real Slint focus tree.
    let modifiers = std::cell::Cell::new([false; 4]);
    use slint::winit_030::winit::keyboard::ModifiersState;
    sync_modifiers(
        &ui,
        &modifiers,
        if cfg!(target_os = "macos") {
            ModifiersState::SUPER
        } else {
            ModifiersState::CONTROL
        },
    );
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: "j".into() });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: "j".into() });
    sync_modifiers(&ui, &modifiers, Default::default());
    assert!(state.borrow().app.show_history);
    ui.invoke_dialog_action("cancel".into());
    state.borrow_mut().app.terminal_visible = true;
    state.borrow_mut().sync(&ui);
    state.borrow_mut().app.terminal_visible = false;
    state.borrow_mut().sync(&ui);
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::F7.into(),
        });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased {
            text: slint::platform::Key::F7.into(),
        });
    assert!(state.borrow().app.prompt.is_some());
    ui.invoke_dialog_action("cancel".into());
    // Flush the dialog's deferred focus restoration as the event loop does.
    slint::platform::update_timers_and_animations();
    // The compact breadcrumb bar must still expose a fully editable address.
    ui.invoke_focus_address(0);
    slint::platform::update_timers_and_animations();
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: right.to_string_lossy().as_ref().into(),
        });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased {
            text: right.to_string_lossy().as_ref().into(),
        });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Return.into(),
        });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased {
            text: slint::platform::Key::Return.into(),
        });
    assert_eq!(
        state.borrow().app.panes[0].tab().location,
        Location::Local(right)
    );
}

#[test]
fn invalid_numeric_fields_block_apply_until_corrected() {
    let (_root, mut adapter, _left, _right) = fixture();
    adapter.action("Preferences");
    adapter.dialog_kind = "preferences".into();
    adapter.edit_field("font", "not a number");
    adapter.edit_field("toolbar", "false");
    adapter.dialog_action("accept");
    assert!(adapter.app.preferences_form.is_some());
    assert!(adapter.app.preferences.toolbar);
    assert!(adapter.invalid_fields.contains_key("font"));
    adapter.edit_field("font", "16");
    adapter.dialog_action("accept");
    assert!(adapter.app.preferences_form.is_none());
    assert_eq!(adapter.app.preferences.font_size, 16.0);
    assert!(!adapter.app.preferences.toolbar);
}

#[test]
fn viewer_close_and_exit_preserve_unsaved_text_until_confirmed() {
    let (_root, mut adapter, left, _right) = fixture();
    adapter.app.editor = Some(Editor {
        pdf: None,
        image: None,
        zoom: 1.0,
        kind: "Text",
        title: "alpha.txt".into(),
        path: Some(left.join("alpha.txt")),
        remote: None,
        text: "changed".into(),
        original: "alpha".into(),
        editable: true,
        error: None,
        confirm_close: false,
    });
    adapter.viewer_action("close");
    assert!(adapter.app.editor.as_ref().unwrap().confirm_close);
    adapter.viewer_action("keep");
    assert!(!adapter.app.editor.as_ref().unwrap().confirm_close);
    adapter.request_close();
    assert!(adapter.app.confirm_exit);
    assert!(!adapter.app.exiting);
    adapter.dialog_kind = "exit".into();
    adapter.dialog_action("cancel");
    assert!(!adapter.app.confirm_exit);
    assert!(adapter.app.editor.is_some());
    adapter.viewer_action("discard");
    assert!(adapter.app.editor.is_none());
    assert_eq!(
        std::fs::read_to_string(left.join("alpha.txt")).unwrap(),
        "alpha"
    );
}

#[test]
fn native_drops_preserve_archive_and_remote_confirmation_contracts() {
    let (_root, mut adapter, left, right) = fixture();
    let mut data = slint::DataTransfer::default();
    data.set_file_paths([left.join("alpha.txt")]);
    adapter.app.panes[1].tab_mut().location = Location::Remote {
        connection: "test-profile".into(),
        path: PathBuf::from("folder"),
    };
    assert!(adapter.drop_files(1, data.clone()));
    assert!(matches!(
        adapter.app.prompt.as_ref().map(|p| &p.kind),
        Some(PromptKind::RemoteTransfer { .. })
    ));
    assert!(adapter.app.job.is_none());
    adapter.app.prompt = None;
    let archive = right.join("archive.zip");
    std::fs::write(&archive, "original archive sentinel").unwrap();
    adapter.app.panes[1].tab_mut().location = Location::Archive {
        file: archive.clone(),
        prefix: "inside".into(),
    };
    assert!(adapter.drop_files(1, data));
    assert!(
        adapter
            .app
            .archive_form
            .as_ref()
            .is_some_and(|form| form.prefix == *"inside" && form.additions.contains("alpha.txt"))
    );
    assert_eq!(
        std::fs::read_to_string(&archive).unwrap(),
        "original archive sentinel"
    );
    assert!(adapter.app.job.is_none());
    adapter.app.archive_form = None;
    let mut relative = slint::DataTransfer::default();
    relative.set_file_paths([PathBuf::from("relative.txt")]);
    assert!(!adapter.drop_files(0, relative));
}
