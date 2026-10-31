use super::*;

#[test]
fn extract_uses_the_shared_form_and_keeps_a_fallback_without_a_native_parent() {
    i_slint_backend_testing::init_no_event_loop();
    let (_root, mut adapter, left, right) = fixture();
    let archive = left.join("source.zip");
    crate::archive::pack(&[left.join("alpha.txt")], &archive, &mut Task::default()).unwrap();
    adapter.app.tab_mut().entries = filesystem::list(&Location::Local(left.clone())).unwrap();
    adapter.app.tab_mut().cursor = Some(archive);
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.invoke_action("Extract".into());
    assert_eq!(ui.get_dialog_title(), "Extract archive");
    assert!(state.borrow().blocked());
    #[cfg(target_os = "macos")]
    assert!(!state.borrow().native_extract.active());
    let fields = ui.get_dialog_fields();
    assert!(
        fields
            .iter()
            .any(|field| field.id == "prompt" && field.kind == 0)
    );
    assert!(
        fields
            .iter()
            .any(|field| field.id == "policy" && field.kind == 3)
    );
    assert!(
        fields
            .iter()
            .any(|field| field.id == "merge" && field.kind == 2)
    );
    ui.invoke_dialog_field("policy".into(), "Skip".into());
    ui.invoke_dialog_field("merge".into(), "true".into());
    assert_eq!(
        state
            .borrow()
            .app
            .prompt
            .as_ref()
            .unwrap()
            .transfer_options
            .conflicts,
        crate::transfer::ConflictPolicy::Skip
    );
    assert!(
        state
            .borrow()
            .app
            .prompt
            .as_ref()
            .unwrap()
            .transfer_options
            .merge_folders
    );
    #[cfg(target_os = "macos")]
    {
        let intent = dialogs::native_extract::Intent::capture(&state.borrow()).unwrap();
        state.borrow_mut().native_extract_accept = Some(intent);
        ui.invoke_dialog_field(
            "prompt".into(),
            right
                .join("changed destination")
                .display()
                .to_string()
                .into(),
        );
        assert!(
            state.borrow().native_extract_accept.is_none(),
            "Editing an input must cancel a deferred native submission"
        );
        assert!(state.borrow().app.job.is_none());
    }
    ui.invoke_dialog_action("cancel".into());
    assert!(state.borrow().app.prompt.is_none());
    assert!(!right.join("source").exists());
    assert!(left.join("alpha.txt").exists());
}

#[test]
fn dialog_fields_keep_control_height_and_standard_text_editing() {
    use i_slint_backend_testing::{AccessibleRole, ElementHandle};
    i_slint_backend_testing::init_no_event_loop();
    let (_root, mut adapter, left, _right) = fixture();
    adapter.app.tab_mut().cursor = Some(left.join("alpha.txt"));
    let ui = CommanderWindow::new().unwrap();
    ui.window().set_size(slint::LogicalSize::new(1240.0, 780.0));
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.show().unwrap();
    ui.invoke_action("Copy".into());
    slint::platform::update_timers_and_animations();
    let input = ElementHandle::find_by_accessible_label(&ui, "Destination")
        .find(|e| e.accessible_role() == Some(AccessibleRole::TextInput))
        .unwrap();
    assert!(
        input.size().height >= 20.0 && input.size().height <= 40.0,
        "Single-line fields must not fill the form: {:?}",
        input.size()
    );
    let choice = ElementHandle::find_by_accessible_label(&ui, "When a target exists")
        .find(|e| e.accessible_role() == Some(AccessibleRole::Combobox))
        .unwrap();
    assert!(
        choice.size().height >= 20.0 && choice.size().height <= 40.0,
        "Pop-up controls must keep their platform height: {:?}",
        choice.size()
    );
    assert!(ui.get_dialog_primary());
    let buttons = ui.get_dialog_buttons();
    let dismissal = if env!("RUST_COMMANDER_WIDGET_STYLE") == "fluent" {
        buttons.row_count() - 1
    } else {
        0
    };
    assert_eq!(buttons.row_data(dismissal).unwrap().id, "cancel");
    let button = ElementHandle::find_by_accessible_label(&ui, "Copy")
        .filter(|e| e.accessible_role() == Some(AccessibleRole::Button))
        .last()
        .unwrap();
    assert!(
        button.size().height <= 40.0,
        "Dialog button bounds: {:?} at {:?}",
        button.size(),
        button.absolute_position()
    );
    // Tab leaves the destination field for the next control; Space operates
    // that checkbox instead of leaking into the file pane's marking action.
    for text in [SharedString::from(slint::platform::Key::Tab), " ".into()] {
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    assert!(state.borrow().app.prompt.as_ref().unwrap().resumable);
    // The macOS face must not intercept clicks over the indicator or replace
    // the standard popup's keyboard navigation and selected-value callback.
    let bounds = choice.absolute_position();
    let position = slint::LogicalPosition::new(
        bounds.x + choice.size().width - 13.0,
        bounds.y + choice.size().height / 2.0,
    );
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
    for pressed in [true, false] {
        ui.window().dispatch_event(if pressed {
            slint::platform::WindowEvent::PointerPressed {
                position,
                button: slint::platform::PointerEventButton::Left,
            }
        } else {
            slint::platform::WindowEvent::PointerReleased {
                position,
                button: slint::platform::PointerEventButton::Left,
            }
        });
    }
    for key in [
        slint::platform::Key::DownArrow,
        slint::platform::Key::Return,
    ] {
        let text: SharedString = key.into();
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    assert_eq!(
        state
            .borrow()
            .app
            .prompt
            .as_ref()
            .unwrap()
            .transfer_options
            .conflicts,
        crate::transfer::ConflictPolicy::Skip
    );
    ui.invoke_dialog_action("cancel".into());
    ui.invoke_action("Rename".into());
    for character in "renamed.txt".chars() {
        let text: SharedString = character.to_string().into();
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    assert_eq!(
        state.borrow().app.prompt.as_ref().unwrap().value,
        "renamed.txt"
    );
    ui.invoke_dialog_action("cancel".into());
    ui.invoke_action("Trash".into());
    assert!(
        !ui.get_dialog_primary(),
        "A destructive action must not get default accent styling"
    );
    ui.invoke_dialog_action("cancel".into());
    assert_eq!(std::fs::read(left.join("alpha.txt")).unwrap(), b"alpha");
}

#[test]
fn macos_password_fields_hide_accessibility_values_and_respect_disabled_input() {
    use i_slint_backend_testing::{AccessibleRole, ElementHandle};
    i_slint_backend_testing::init_no_event_loop();
    let ui = CommanderWindow::new().unwrap();
    ui.global::<DialogTheme>().set_macos(true);
    ui.set_dialog_title("Credentials".into());
    ui.set_dialog_fields(
        Rc::new(VecModel::from(vec![FormField {
            id: "password".into(),
            title: "Password".into(),
            value: "disposable-secret".into(),
            kind: 1,
            ..Default::default()
        }]))
        .into(),
    );
    let edits = Rc::new(RefCell::new(Vec::new()));
    let captured = edits.clone();
    ui.on_dialog_field(move |id, value| {
        captured
            .borrow_mut()
            .push((id.to_string(), value.to_string()))
    });
    ui.show().unwrap();
    slint::platform::update_timers_and_animations();
    let password = ElementHandle::find_by_accessible_label(&ui, "Password")
        .find(|e| e.accessible_role() == Some(AccessibleRole::TextInput))
        .unwrap();
    assert_eq!(password.accessible_value().unwrap_or_default(), "");
    password.set_accessible_value("updated-secret");
    assert_eq!(
        &*edits.borrow(),
        &[("password".to_string(), "updated-secret".to_string())]
    );
    ui.set_dialog_input_locked(true);
    password.set_accessible_value("must-not-be-applied");
    assert_eq!(edits.borrow().len(), 1);
    assert_eq!(password.accessible_value().unwrap_or_default(), "");
}

#[test]
fn active_transfer_controls_work_with_queue_dialog_and_validate_live_settings() {
    i_slint_backend_testing::init_no_event_loop();
    let (_root, mut adapter, _left, _right) = fixture();
    let ui = CommanderWindow::new().unwrap();
    let (_sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let control = Arc::new(crate::transfer_control::Control::new(Default::default()));
    adapter.app.job = Some(Job {
        owner: adapter.app.window_id,
        foreground: false,
        label: "Checkpointed copy".into(),
        receiver,
        control: Some(control.clone()),
        cancel: cancel.clone(),
        progress: Default::default(),
    });
    adapter.app.show_history = true;
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    assert!(ui.get_transfer_controllable());
    assert!(ui.get_dialog_compact());
    ui.invoke_action("PauseTransfer".into());
    assert!(control.paused());
    assert!(ui.get_transfer_paused());
    assert!(ui.get_status().starts_with("Pausing"));
    ui.invoke_dialog_field("transfer-limit".into(), "128".into());
    assert_eq!(control.settings().kib_per_second, 128);
    ui.invoke_dialog_field("transfer-retries".into(), "9".into());
    assert!(ui.get_dialog_error().contains("0 to 8"));
    assert_eq!(control.settings().retries, 3);
    ui.invoke_dialog_field("transfer-retries".into(), "0".into());
    assert_eq!(control.settings().retries, 0);
    assert!(ui.get_dialog_error().is_empty());
    ui.invoke_dialog_action("pause-job".into());
    assert!(!control.paused());
    assert!(!ui.get_transfer_paused());
    ui.invoke_action("CancelJob".into());
    assert!(cancel.load(Ordering::Relaxed));
}

#[test]
fn local_folder_copy_dialog_offers_checkpoints_with_merge_and_replace() {
    i_slint_backend_testing::init_no_event_loop();
    let (_root, mut adapter, left, _right) = fixture();
    adapter.app.panes[0].tab_mut().selected =
        std::collections::BTreeSet::from([left.join("folder")]);
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.invoke_action("Copy".into());
    assert!(state.borrow().fields.iter().any(|f| f.id == "resumable"));
    ui.invoke_dialog_field("resumable".into(), "true".into());
    ui.invoke_dialog_field("policy".into(), "Replace".into());
    ui.invoke_dialog_field("merge".into(), "true".into());
    let borrowed = state.borrow();
    let prompt = borrowed.app.prompt.as_ref().unwrap();
    assert!(prompt.resumable);
    assert!(prompt.transfer_options.merge_folders);
    assert_eq!(
        prompt.transfer_options.conflicts,
        crate::transfer::ConflictPolicy::Replace
    );
}

#[test]
fn running_progress_shows_speed_beside_the_transferred_byte_count() {
    i_slint_backend_testing::init_no_event_loop();
    let (_root, mut adapter, _left, _right) = fixture();
    let ui = CommanderWindow::new().unwrap();
    let (_sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let mut task = Task::default();
    task.plan_total(Some(8 * 1024 * 1024));
    task.tick(Path::new("fixture.zip"), 4 * 1024 * 1024)
        .unwrap();
    adapter.app.job = Some(Job {
        owner: adapter.app.window_id,
        foreground: false,
        label: "Remote transfer".into(),
        receiver,
        control: None,
        cancel,
        progress: task.progress,
    });
    adapter.sync(&ui);
    let status = ui.get_status().to_string();
    assert!(status.contains("fixture.zip"), "{status}");
    assert!(status.ends_with("4.0 MiB"), "{status}");
    assert!(ui.get_transfer_speed().ends_with("/s"));
    assert!(!ui.get_progress_indeterminate());
    assert_eq!(ui.get_progress(), 0.5);
    assert_eq!(ui.get_progress_text(), "50%");
    adapter.app.job.as_mut().unwrap().progress.total_bytes = None;
    adapter.sync(&ui);
    assert!(ui.get_progress_indeterminate());
    assert!(ui.get_progress_text().is_empty());
    adapter.app.job.as_mut().unwrap().progress.total_bytes = Some(4 * 1024 * 1024);
    adapter.sync(&ui);
    assert_eq!(ui.get_progress_text(), "99%");
    adapter.app.job = None;
    adapter.sync(&ui);
    assert!(!ui.get_busy());
    assert!(ui.get_transfer_speed().is_empty());
    assert!(ui.get_progress_text().is_empty());
    assert_eq!(ui.get_progress(), 0.0);
}

#[test]
fn location_bar_toggle_apply_cancel_and_escape_preserve_the_address() {
    use crate::preferences::LocationBar;
    i_slint_backend_testing::init_no_event_loop();
    let (_root, adapter, left, _right) = fixture();
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    assert!(!ui.get_breadcrumbs_mode());
    ui.invoke_action("ToggleLocationBar".into());
    assert!(ui.get_breadcrumbs_mode());
    ui.invoke_action("Preferences".into());
    ui.invoke_dialog_field("location-bar".into(), "Editable address".into());
    ui.invoke_dialog_action("cancel".into());
    assert!(ui.get_breadcrumbs_mode());
    ui.invoke_action("Preferences".into());
    ui.invoke_dialog_field("location-bar".into(), "Editable address".into());
    ui.invoke_dialog_action("accept".into());
    assert!(!ui.get_breadcrumbs_mode());
    let snapshot = state.borrow().app.snapshot_settings();
    assert_eq!(snapshot.preferences.location_bar, LocationBar::Address);
    ui.invoke_address_edited(0, "data:text/plain,example".into());
    ui.invoke_navigate(0, "data:text/plain,example".into());
    assert_eq!(
        state.borrow().app.tab().location,
        Location::Local(left.clone())
    );
    assert_eq!(ui.get_left_address(), "data:text/plain,example");
    assert!(ui.get_left_error().contains("data:"));
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
    assert_eq!(state.borrow().app.tab().address, left.display().to_string());
}

#[test]
fn unnamed_uri_profiles_restore_tabs_and_save_explicitly() {
    use crate::remote::Connection;
    i_slint_backend_testing::init_no_event_loop();
    let (_root, mut adapter, _left, _right) = fixture();
    let c = Connection {
        service: "http".into(),
        endpoint: "https://example.test".into(),
        listed: false,
        ..Default::default()
    };
    crate::remote::register_profiles(std::slice::from_ref(&c));
    adapter.app.connections.push(c.clone());
    adapter.app.panes[0].tab_mut().location = Location::Remote {
        connection: c.id.clone(),
        path: "dir".into(),
    };
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    assert_eq!(ui.get_left_protocol(), "HTTPS");
    assert!(
        !state
            .borrow()
            .places
            .iter()
            .any(|(id, _)| id.starts_with("connection:"))
    );
    let saved = state.borrow().app.snapshot_settings();
    let restored: Settings = serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    assert!(
        restored
            .connections
            .iter()
            .any(|p| p.id == c.id && !p.listed)
    );
    assert_eq!(restored.tabs[0][0], saved.tabs[0][0]);
    assert!(!state.borrow().action_available("Mkdir"));
    assert!(!state.borrow().action_available("Move"));
    assert!(state.borrow().action_available("Copy"));
    ui.invoke_action("SaveConnection:0".into());
    ui.invoke_dialog_field("name".into(), "My downloads".into());
    ui.invoke_dialog_action("accept".into());
    assert!(
        state
            .borrow()
            .app
            .connections
            .iter()
            .any(|p| p.id == c.id && p.listed && p.name == "My downloads")
    );
    assert!(
        state
            .borrow()
            .places
            .iter()
            .any(|(id, _)| id.starts_with("connection:"))
    );
}

pub(super) fn fixture() -> (tempfile::TempDir, Adapter, PathBuf, PathBuf) {
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

fn wait_for_folder_tools(
    state: &Rc<RefCell<Adapter>>,
    ui: &CommanderWindow,
    done: impl Fn(&Adapter) -> bool,
) {
    for _ in 0..500 {
        {
            let mut adapter = state.borrow_mut();
            adapter.poll_dialogs();
            adapter.sync(ui);
            if done(&adapter) {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("Folder tool worker did not finish");
}

#[test]
fn advanced_search_dialog_validates_and_runs_the_shared_matcher() {
    i_slint_backend_testing::init_no_event_loop();
    let (_root, adapter, left, _right) = fixture();
    std::fs::write(left.join("alpha.txt"), "Error: 429\n").unwrap();
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.invoke_action("Search".into());
    ui.invoke_dialog_field("search-regex".into(), "true".into());
    ui.invoke_dialog_field("content".into(), "[".into());
    ui.invoke_dialog_action("accept".into());
    assert!(
        state
            .borrow()
            .app
            .tools
            .search
            .as_ref()
            .unwrap()
            .pending
            .is_none()
    );
    assert!(ui.get_dialog_error().contains("Invalid content pattern"));
    ui.invoke_dialog_field("content".into(), r"^error: \d{3}$".into());
    ui.invoke_dialog_field("search-content-case".into(), "false".into());
    ui.invoke_dialog_field("search-min-size".into(), "10".into());
    ui.invoke_dialog_action("accept".into());
    wait_for_folder_tools(&state, &ui, |a| {
        a.app.tools.search.as_ref().unwrap().pending.is_none()
    });
    let borrowed = state.borrow();
    let form = borrowed.app.tools.search.as_ref().unwrap();
    assert!(form.error.is_none(), "{:?}", form.error);
    assert_eq!(
        form.result.as_ref().unwrap().paths,
        vec![left.join("alpha.txt")]
    );
}

#[test]
fn comparison_dialog_shares_presets_with_sync_and_opens_read_only_aligned_diff() {
    i_slint_backend_testing::init_no_event_loop();
    let (_root, adapter, left, right) = fixture();
    std::fs::write(left.join("alpha.txt"), "alpha\nbeta\n").unwrap();
    std::fs::write(right.join("alpha.txt"), "alpha\ninsert\nbeta\n").unwrap();
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.invoke_action("Compare".into());
    assert_eq!(ui.get_dialog_title(), "Compare folders");
    ui.invoke_dialog_field("folder-include".into(), "*.txt".into());
    ui.invoke_dialog_field("folder-exclude".into(), "cache;.git".into());
    ui.invoke_dialog_field("folder-preset-name".into(), "Documents".into());
    ui.invoke_dialog_action("folder-preset-save".into());
    assert_eq!(state.borrow().app.preferences.folder_tool_presets.len(), 1);
    ui.invoke_dialog_action("accept".into());
    wait_for_folder_tools(&state, &ui, |a| {
        a.app
            .features
            .comparison
            .as_ref()
            .unwrap()
            .pending
            .is_none()
    });
    let index = state
        .borrow()
        .app
        .features
        .comparison
        .as_ref()
        .unwrap()
        .result
        .as_ref()
        .unwrap()
        .iter()
        .position(|r| r.name == "alpha.txt")
        .unwrap();
    ui.invoke_dialog_action(format!("comparison-diff-{index}").into());
    wait_for_folder_tools(&state, &ui, |a| {
        a.app
            .features
            .comparison
            .as_ref()
            .unwrap()
            .diff_pending
            .is_none()
    });
    assert!(ui.get_dialog_diff());
    assert!(
        ui.get_dialog_fields()
            .iter()
            .any(|f| f.kind == 7 && f.choices.row_data(0).unwrap().contains("insert"))
    );
    ui.invoke_dialog_action("cancel".into());
    assert_eq!(ui.get_dialog_title(), "Compare folders");
    ui.invoke_dialog_action("cancel".into());
    ui.invoke_action("Synchronize".into());
    ui.invoke_dialog_action("folder-preset-load-0".into());
    assert_eq!(
        state
            .borrow()
            .app
            .features
            .sync
            .as_ref()
            .unwrap()
            .rules
            .include,
        "*.txt"
    );
    ui.invoke_dialog_action("sync-preview".into());
    wait_for_folder_tools(&state, &ui, |a| {
        a.app.features.sync.as_ref().unwrap().pending.is_none()
    });
    assert!(
        state
            .borrow()
            .app
            .features
            .sync
            .as_ref()
            .unwrap()
            .plan
            .is_some()
    );
    ui.invoke_dialog_field("folder-exclude".into(), "cache;.git;*.txt".into());
    assert!(
        state
            .borrow()
            .app
            .features
            .sync
            .as_ref()
            .unwrap()
            .plan
            .is_none()
    );
}

#[cfg(debug_assertions)]
#[test]
fn sidebar_sections_collapse_independently() {
    use i_slint_backend_testing::{AccessibleRole, ElementHandle};

    fn heading(ui: &CommanderWindow, title: &str) -> ElementHandle {
        ElementHandle::find_by_accessible_label(ui, title)
            .find(|item| item.accessible_expandable() == Some(true))
            .unwrap()
    }

    i_slint_backend_testing::init_no_event_loop();
    let (_root, adapter, _left, _right) = fixture();
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.window().set_size(slint::LogicalSize::new(1240.0, 780.0));
    ui.show().unwrap();

    assert_eq!(heading(&ui, "Places").accessible_expanded(), Some(true));
    assert_eq!(heading(&ui, "Favorites").accessible_expanded(), Some(true));
    let favorites_top = heading(&ui, "Favorites").absolute_position().y;

    heading(&ui, "Places").invoke_accessible_default_action();
    assert_eq!(heading(&ui, "Places").accessible_expanded(), Some(false));
    assert_eq!(heading(&ui, "Favorites").accessible_expanded(), Some(true));
    assert!(heading(&ui, "Favorites").absolute_position().y < favorites_top - 100.0);
    state.borrow_mut().sync(&ui);
    assert_eq!(heading(&ui, "Places").accessible_expanded(), Some(false));
    assert_eq!(
        ElementHandle::find_by_accessible_label(&ui, "Home")
            .filter(|item| item.accessible_role() == Some(AccessibleRole::ListItem))
            .count(),
        0
    );

    heading(&ui, "Favorites").invoke_accessible_default_action();
    assert_eq!(heading(&ui, "Favorites").accessible_expanded(), Some(false));
    assert_eq!(heading(&ui, "Places").accessible_expanded(), Some(false));
    heading(&ui, "Places").invoke_accessible_default_action();
    assert_eq!(heading(&ui, "Places").accessible_expanded(), Some(true));
    assert!(
        ElementHandle::find_by_accessible_label(&ui, "Home")
            .filter(|item| item.accessible_role() == Some(AccessibleRole::ListItem))
            .any(|item| item.size().height > 0.0)
    );
}

#[test]
fn sidebar_toggle_hides_places_and_preserves_the_tree_setting() {
    use i_slint_backend_testing::ElementHandle;

    i_slint_backend_testing::init_no_event_loop();
    let (_root, adapter, _left, _right) = fixture();
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.window().set_size(slint::LogicalSize::new(1240.0, 780.0));
    ui.show().unwrap();

    ui.invoke_action("DirectoryTree".into());
    assert!(ui.get_sidebar_visible());
    assert!(ui.get_tree_visible());
    assert_eq!(ui.invoke_drop_side(50.0, 400.0), -1);

    ui.invoke_action("ToggleSidebar".into());
    assert!(!ui.get_sidebar_visible());
    assert!(!state.borrow().app.preferences.sidebar);
    assert!(
        ui.get_tree_visible(),
        "the tree setting should survive hiding the sidebar"
    );
    assert_eq!(
        ElementHandle::find_by_accessible_label(&ui, "Places").count(),
        0
    );

    ui.invoke_action("ToggleSidebar".into());
    assert!(ui.get_sidebar_visible());
    assert!(state.borrow().app.preferences.sidebar);
    assert!(ElementHandle::find_by_accessible_label(&ui, "Places").count() > 0);
    assert_eq!(ui.invoke_drop_side(50.0, 400.0), -1);
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
        navigation: viewer_tools::Navigation::new(
            Location::Local(filesystem::home()),
            PathBuf::new(),
        ),
        image_revision: 0,
        document: None,
        text_page: None,
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
        can_edit: true,
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
