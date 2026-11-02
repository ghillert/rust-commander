use super::*;
fn setup() -> (
    tempfile::TempDir,
    CommanderWindow,
    Rc<RefCell<Adapter>>,
    PathBuf,
    PathBuf,
) {
    i_slint_backend_testing::init_no_event_loop();
    let (root, adapter, left, right) = super::tests::fixture();
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.window().set_size(slint::LogicalSize::new(1240.0, 780.0));
    ui.show().unwrap();
    (root, ui, state, left, right)
}
fn key(text: impl Into<SharedString>) -> slint::language::KeyEvent {
    let mut event = slint::language::KeyEvent::default();
    event.text = text.into();
    event
}
#[test]
fn sort_indicators_follow_each_pane_tab_and_configured_columns() {
    use i_slint_backend_testing::{AccessibleRole, ElementHandle};
    let (_root, ui, state, _left, _right) = setup();
    let headers = |title| {
        let mut elements: Vec<_> = ElementHandle::find_by_accessible_label(&ui, title)
            .filter(|element| element.accessible_role() == Some(AccessibleRole::Button))
            .collect();
        elements.sort_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x));
        elements
    };
    assert_eq!(ui.get_left_sort_column(), 0);
    assert_eq!(ui.get_right_sort_column(), 0);
    assert!(!ui.get_left_sort_descending());
    assert!(!ui.get_right_sort_descending());
    assert_eq!(
        headers("Name")[0].accessible_description().unwrap(),
        "Sort ascending"
    );
    headers("Name")[0].mock_single_click(slint::platform::PointerEventButton::Left);
    assert!(ui.get_left_sort_descending());
    assert!(!ui.get_right_sort_descending());
    assert_eq!(
        headers("Name")[0].accessible_description().unwrap(),
        "Sort descending"
    );
    let file_names: Vec<_> = state.borrow().app.panes[0]
        .tab()
        .entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| entry.name.clone())
        .collect();
    assert_eq!(file_names, ["café.txt", "alpha.txt"]);
    headers("Size")[1].mock_single_click(slint::platform::PointerEventButton::Left);
    assert_eq!(state.borrow().app.active, 1);
    assert_eq!(ui.get_right_sort_column(), 1);
    assert!(!ui.get_right_sort_descending());
    headers("Size")[1].invoke_accessible_default_action();
    assert!(ui.get_right_sort_descending());
    assert_eq!(
        headers("Name")[1]
            .accessible_description()
            .unwrap_or_default(),
        ""
    );
    {
        let mut a = state.borrow_mut();
        a.app.preferences.columns.swap(0, 2);
        a.sync(&ui);
    }
    assert_eq!(ui.get_left_sort_column(), 2);
    assert_eq!(ui.get_right_sort_column(), 1);
    ui.invoke_tab(0, 0);
    ui.invoke_action("NewTab".into());
    assert_eq!(ui.get_left_sort_column(), 2);
    assert!(!ui.get_left_sort_descending());
    ui.invoke_tab(0, 0);
    assert!(ui.get_left_sort_descending());
    ui.invoke_action("Swap".into());
    assert_eq!(ui.get_left_sort_column(), 1);
    assert_eq!(ui.get_right_sort_column(), 2);
    assert!(ui.get_left_sort_descending());
    assert!(ui.get_right_sort_descending());
    {
        let mut a = state.borrow_mut();
        a.app
            .preferences
            .columns
            .retain(|column| column.kind != crate::preferences::Column::Size);
        a.sync(&ui);
    }
    assert_eq!(ui.get_left_sort_column(), -1);
    assert_eq!(ui.get_right_sort_column(), 1);
}
#[test]
fn parent_rows_highlight_before_opening_for_separate_and_double_clicks() {
    use i_slint_backend_testing::ElementHandle;
    let (root, ui, state, left, right) = setup();
    for delay in [1000, 0] {
        {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app.panes[1]
                .tab_mut()
                .navigate(Location::Local(right.clone()), &ctx);
            a.sync(&ui);
        }
        let mut parents: Vec<_> =
            ElementHandle::find_by_accessible_label(&ui, "Parent directory row").collect();
        parents.sort_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x));
        assert_eq!(parents.len(), 2);
        assert!(
            parents.iter().all(|parent| parent.size().width > 400.0),
            "The parent entry must span the full file-list row"
        );
        parents[1].mock_single_click(slint::platform::PointerEventButton::Left);
        assert_eq!(state.borrow().app.active, 1);
        assert!(ui.get_right_parent_selected());
        assert_eq!(parents[1].accessible_item_selected(), Some(true));
        assert_eq!(
            state.borrow().app.panes[1].tab().location,
            Location::Local(right.clone())
        );
        assert!(state.borrow().app.panes[1].tab().chosen(false).is_empty());
        assert_eq!(ui.get_right_cursor(), -1);
        i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(delay));
        parents[1].mock_single_click(slint::platform::PointerEventButton::Left);
        assert_eq!(
            state.borrow().app.panes[0].tab().location,
            Location::Local(left.clone())
        );
        assert_eq!(
            state.borrow().app.panes[1].tab().location,
            Location::Local(root.path().to_path_buf())
        );
        assert!(!ui.get_right_parent_selected());
    }
}
#[test]
fn parent_row_preserves_marks_and_supports_file_focus_and_keyboard_navigation() {
    use i_slint_backend_testing::ElementHandle;
    let (root, ui, state, left, _right) = setup();
    {
        let mut a = state.borrow_mut();
        filesystem::sort_entries(&mut a.app.tab_mut().entries, Default::default(), false);
        a.app.tab_mut().selected.insert(left.join("alpha.txt"));
        a.app.tab_mut().cursor = Some(left.join("alpha.txt"));
        a.sync(&ui);
    }
    let parent = || {
        ElementHandle::find_by_accessible_label(&ui, "Parent directory row")
            .min_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x))
            .unwrap()
    };
    parent().mock_single_click(slint::platform::PointerEventButton::Left);
    assert!(ui.get_left_parent_selected());
    assert!(
        state
            .borrow()
            .app
            .tab()
            .selected
            .contains(&left.join("alpha.txt"))
    );
    assert!(ui.get_left_rows().iter().all(|row| !row.cursor));
    ui.invoke_pane_key(0, key(SharedString::from(slint::platform::Key::DownArrow)));
    assert!(!ui.get_left_parent_selected());
    assert_eq!(state.borrow().app.tab().cursor, Some(left.join("folder")));
    parent().mock_single_click(slint::platform::PointerEventButton::Left);
    press(&ui, " ");
    assert!(!ui.get_left_parent_selected());
    assert_eq!(state.borrow().app.tab().cursor, Some(left.join("folder")));
    assert_eq!(state.borrow().app.tab().selected.len(), 1);
    parent().mock_single_click(slint::platform::PointerEventButton::Left);
    ElementHandle::find_by_accessible_label(&ui, "café.txt")
        .next()
        .unwrap()
        .mock_single_click(slint::platform::PointerEventButton::Left);
    assert!(!ui.get_left_parent_selected());
    parent().mock_single_click(slint::platform::PointerEventButton::Left);
    // Enter opens the parent even when other files remain marked.
    press(&ui, slint::platform::Key::Return);
    assert_eq!(
        state.borrow().app.tab().location,
        Location::Local(root.path().to_path_buf())
    );
    assert!(!ui.get_left_parent_selected());
}
#[test]
fn quick_find_and_pattern_selection_use_the_real_slint_callbacks() {
    let (_root, ui, state, left, _right) = setup();
    slint::platform::update_timers_and_animations();
    ui.invoke_focus_files(0);
    for character in "CAFÉ".chars() {
        let text: SharedString = character.to_string().into();
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
    }
    assert_eq!(state.borrow().app.tab().cursor, Some(left.join("café.txt")));
    assert_eq!(ui.get_left_quick_find(), "CAFÉ");
    ui.invoke_pane_key(0, key(SharedString::from(slint::platform::Key::Escape)));
    assert!(ui.get_left_quick_find().is_empty());
    ui.invoke_action("SelectPattern".into());
    ui.invoke_dialog_field("pattern".into(), "*.txt".into());
    ui.invoke_dialog_action("accept".into());
    assert_eq!(state.borrow().app.tab().selected.len(), 2);
    assert!(ui.get_dialog_title().is_empty());
}
#[test]
fn locked_named_tabs_move_between_panes_and_defer_navigation_to_a_new_tab() {
    let (_root, ui, state, left, right) = setup();
    ui.invoke_action("LockTab".into());
    ui.invoke_action("TabTitle".into());
    ui.invoke_dialog_field("title".into(), "Pinned work".into());
    ui.invoke_dialog_action("accept".into());
    assert!(
        ui.get_left_tabs()
            .row_data(0)
            .unwrap()
            .contains("Pinned work")
    );
    ui.invoke_navigate(0, right.display().to_string().into());
    assert_eq!(
        state.borrow().app.tab().location,
        Location::Local(left.clone())
    );
    let ctx = state.borrow().ctx.clone();
    state.borrow_mut().app.poll(&ctx);
    state.borrow_mut().sync(&ui);
    assert_eq!(state.borrow().app.panes[0].tabs.len(), 2);
    assert!(state.borrow().app.tab().state.locked.is_none());
    ui.invoke_tab(0, 0);
    ui.invoke_action("MoveTabOther".into());
    assert_eq!(state.borrow().app.active, 1);
    assert_eq!(state.borrow().app.tab().state.title, "Pinned work");
    assert_eq!(
        state.borrow().app.tab().state.locked,
        Some(Location::Local(left))
    );
}
#[test]
fn pane_orientation_and_divider_preserve_drop_target_routing() {
    let (_root, ui, state, _left, _right) = setup();
    ui.invoke_action("Stacked".into());
    assert!(ui.get_stacked());
    assert_eq!(ui.invoke_drop_side(500.0, 150.0), 0);
    assert_eq!(ui.invoke_drop_side(500.0, 650.0), 1);
    ui.invoke_resize_panes(0.3, true);
    assert_eq!(state.borrow().app.preferences.layout.ratio, 0.3);
    assert_eq!(ui.invoke_drop_side(500.0, 650.0), 1);
    ui.invoke_action("EqualPanes".into());
    assert_eq!(ui.get_pane_ratio(), 0.5);
    ui.invoke_action("SideBySide".into());
    assert_eq!(ui.invoke_drop_side(1000.0, 400.0), 1);
}
#[test]
fn searchable_quick_lists_and_cross_pane_commands_navigate_only_the_requested_panes() {
    let (_root, ui, state, left, right) = setup();
    state.borrow_mut().app.bookmarks = vec![
        Bookmark {
            name: "Here".into(),
            location: Location::Local(left.clone()),
        },
        Bookmark {
            name: "There".into(),
            location: Location::Local(right.clone()),
        },
    ];
    ui.invoke_action("BookmarksList".into());
    ui.invoke_dialog_field("filter".into(), "There".into());
    ui.invoke_dialog_action("accept".into());
    assert_eq!(
        state.borrow().app.tab().location,
        Location::Local(right.clone())
    );
    ui.invoke_action("ParentOther".into());
    assert_eq!(
        state.borrow().app.panes[1].tab().location,
        Location::Local(right.parent().unwrap().into())
    );
    assert_eq!(
        state.borrow().app.panes[0].tab().location,
        Location::Local(right)
    );
    ui.invoke_navigate(0, left.display().to_string().into());
    let ctx = state.borrow().ctx.clone();
    state.borrow_mut().app.poll(&ctx);
    state.borrow_mut().app.tab_mut().entries =
        filesystem::list(&Location::Local(left.clone())).unwrap();
    state.borrow_mut().app.tab_mut().cursor = Some(left.join("folder"));
    ui.invoke_action("OpenBoth".into());
    assert!(
        state
            .borrow()
            .app
            .panes
            .iter()
            .all(|p| p.tab().location == Location::Local(left.join("folder")))
    );
}
#[test]
fn command_bar_and_shortcut_labels_follow_custom_bindings() {
    let (_root, ui, state, left, _right) = setup();
    assert!(
        ui.get_command_bar_visible(),
        "Commander action buttons should be visible by default on every OS"
    );
    state.borrow_mut().app.preferences.command_actions = vec!["Copy".into(), "Stacked".into()];
    state
        .borrow_mut()
        .app
        .preferences
        .shortcuts
        .insert("Copy".into(), "Command+K".into());
    state.borrow_mut().app.tab_mut().cursor = Some(left.join("alpha.txt"));
    state.borrow_mut().sync(&ui);
    assert_eq!(ui.get_command_buttons().row_count(), 2);
    assert!(
        ui.get_command_buttons()
            .row_data(0)
            .unwrap()
            .key
            .contains('K')
    );
    let mut shortcut = key("k");
    shortcut.modifiers.control = true;
    assert!(ui.invoke_global_key(shortcut));
    assert!(state.borrow().app.prompt.is_some());
    ui.invoke_dialog_action("cancel".into());
    assert!(!ui.invoke_global_key(key(SharedString::from(slint::platform::Key::F5))));
    ui.invoke_menu("Operations".into());
    assert!(
        ui.get_menu_items()
            .iter()
            .any(|c| c.id == "Copy" && c.title.contains('K'))
    );
}

fn settings_element(
    ui: &CommanderWindow,
    label: &str,
    role: i_slint_backend_testing::AccessibleRole,
) -> i_slint_backend_testing::ElementHandle {
    i_slint_backend_testing::ElementHandle::find_by_accessible_label(ui, label)
        .find(|element| element.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("Missing settings control: {label}"))
}

#[test]
fn settings_tabs_preserve_drafts_and_distinguish_apply_ok_and_cancel() {
    use i_slint_backend_testing::AccessibleRole::{Tab, TextInput};
    use slint::platform::PointerEventButton::Left;
    let (_root, ui, state, _left, _right) = setup();
    ui.invoke_action("Preferences".into());
    assert!(ui.get_dialog_preferences());
    settings_element(&ui, "Appearance", Tab).mock_single_click(Left);
    assert_eq!(ui.get_settings_tab(), 2);
    settings_element(&ui, "Font size (10–24 px)", TextInput).set_accessible_value("16");
    settings_element(&ui, "Folders", Tab).mock_single_click(Left);
    assert_eq!(ui.get_settings_tab(), 1);
    ui.invoke_dialog_field("parent-entry".into(), "false".into());
    assert_eq!(state.borrow().app.preferences.font_size, 14.0);
    assert!(state.borrow().app.preferences.parent_entry);
    settings_element(&ui, "Appearance", Tab).mock_single_click(Left);
    assert_eq!(
        settings_element(&ui, "Font size (10–24 px)", TextInput)
            .accessible_value()
            .unwrap(),
        "16"
    );
    ui.invoke_dialog_action("apply".into());
    assert!(state.borrow().app.preferences_form.is_some());
    assert_eq!(state.borrow().app.preferences.font_size, 16.0);
    assert!(!state.borrow().app.preferences.parent_entry);
    ui.invoke_dialog_field("font".into(), "18".into());
    ui.invoke_dialog_action("cancel".into());
    assert_eq!(state.borrow().app.preferences.font_size, 16.0);
    ui.invoke_action("Preferences".into());
    assert_eq!(ui.get_settings_tab(), 0);
    ui.invoke_dialog_field("font".into(), "17".into());
    ui.invoke_dialog_action("accept".into());
    assert!(state.borrow().app.preferences_form.is_none());
    assert_eq!(state.borrow().app.preferences.font_size, 17.0);
}

#[test]
fn settings_validation_survives_tab_switches_and_defaults_remain_a_draft() {
    let (_root, ui, state, _left, _right) = setup();
    ui.invoke_action("Preferences".into());
    ui.invoke_show_settings_page(2);
    ui.invoke_dialog_field("font".into(), "invalid size".into());
    ui.invoke_show_settings_page(4);
    ui.invoke_dialog_field("mail-host".into(), "smtp.example.org".into());
    ui.invoke_dialog_field("mail-sender".into(), "sender@example.org".into());
    ui.invoke_dialog_field("mail-tls".into(), "true".into());
    ui.invoke_dialog_action("apply".into());
    assert!(state.borrow().app.preferences.mail.host.is_empty());
    assert_eq!(ui.get_settings_error_page(), 2);
    ui.invoke_show_settings_page(ui.get_settings_error_page());
    assert_eq!(ui.get_settings_tab(), 2);
    assert!(
        ui.get_dialog_fields()
            .iter()
            .any(|field| field.id == "font" && field.value == "invalid size")
    );
    ui.invoke_dialog_field("font".into(), "18".into());
    ui.invoke_dialog_action("apply".into());
    assert_eq!(state.borrow().app.preferences.mail.port, 465);
    assert_eq!(state.borrow().app.preferences.font_size, 18.0);
    ui.invoke_dialog_action("defaults".into());
    assert_eq!(
        state
            .borrow()
            .app
            .preferences_form
            .as_ref()
            .unwrap()
            .font_size,
        14.0
    );
    assert_eq!(state.borrow().app.preferences.font_size, 18.0);
    ui.invoke_dialog_action("cancel".into());
    assert_eq!(state.borrow().app.preferences.font_size, 18.0);
}

#[test]
fn settings_shortcut_search_and_reordering_keep_unseen_edits() {
    use i_slint_backend_testing::AccessibleRole::TextInput;
    use slint::platform::{PointerEventButton::Left, WindowEvent};
    let (_root, ui, state, _left, _right) = setup();
    ui.invoke_action("Preferences".into());
    ui.invoke_show_settings_page(3);
    settings_element(&ui, "Find a command", TextInput).mock_single_click(Left);
    let mut query = String::new();
    for letter in ["c", "o", "p", "y"] {
        ui.window().dispatch_event(WindowEvent::KeyPressed {
            text: letter.into(),
        });
        ui.window().dispatch_event(WindowEvent::KeyReleased {
            text: letter.into(),
        });
        query.push_str(letter);
        assert_eq!(
            state.borrow().extra.shortcut_filter,
            query,
            "Search must keep focus as matches disappear"
        );
    }
    ui.invoke_show_settings_page(8);
    assert_eq!(ui.get_settings_tab(), 1);
    assert_eq!(ui.get_settings_folder_tab(), 1);
    ui.invoke_dialog_action("column-down-1".into());
    ui.invoke_dialog_field("width-1".into(), "210".into());
    ui.invoke_show_settings_page(6);
    assert_eq!(ui.get_settings_appearance_tab(), 1);
    ui.invoke_dialog_field("toolbar-Search".into(), "true".into());
    ui.invoke_show_settings_page(3);
    ui.invoke_dialog_field("shortcut-filter".into(), "Copy".into());
    ui.invoke_dialog_field("alternate-Copy".into(), "Control+Alt+Y".into());
    ui.invoke_dialog_field("shortcut-filter".into(), "no-such-command-123".into());
    assert!(
        !ui.get_settings_pages()
            .row_data(3)
            .unwrap()
            .fields
            .iter()
            .any(|field| field.kind == 11)
    );
    ui.invoke_dialog_field("shortcut-filter".into(), "Copy".into());
    assert!(
        ui.get_settings_pages()
            .row_data(3)
            .unwrap()
            .fields
            .iter()
            .any(|field| field.id == "shortcut-Copy"
                && field.choices.row_data(0).unwrap() == "Control+Alt+Y")
    );
    ui.invoke_dialog_action("accept".into());
    let a = state.borrow();
    assert_eq!(
        a.app.preferences.columns[0].kind,
        crate::preferences::Column::Name
    );
    assert_eq!(
        a.app.preferences.columns[1].kind,
        crate::preferences::Column::Modified
    );
    assert_eq!(a.app.preferences.columns[1].width, 210.0);
    assert!(
        a.app
            .preferences
            .toolbar_actions
            .contains(&crate::preferences::ToolbarAction::Search)
    );
    assert_eq!(
        a.app.preferences.alternate_shortcuts["Copy"],
        "Control+Alt+Y"
    );
}

#[test]
fn settings_tab_headers_support_keyboard_selection_without_file_actions() {
    use slint::platform::{Key, WindowEvent};
    let (_root, ui, state, _left, _right) = setup();
    ui.invoke_action("Preferences".into());
    for key in [Key::Tab, Key::RightArrow, Key::Return] {
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    }
    assert_eq!(ui.get_settings_tab(), 1);
    assert!(state.borrow().app.prompt.is_none());
    assert!(state.borrow().app.tab().selected.is_empty());
}

#[test]
fn settings_footer_stays_visible_in_small_windows_with_large_text() {
    use i_slint_backend_testing::AccessibleRole::Button;
    let (_root, ui, state, _left, _right) = setup();
    state.borrow_mut().app.preferences.font_size = 24.0;
    state.borrow_mut().sync(&ui);
    ui.window().set_size(slint::LogicalSize::new(840.0, 480.0));
    ui.invoke_action("Preferences".into());
    for page in 0..9 {
        ui.invoke_show_settings_page(page);
        slint::platform::update_timers_and_animations();
        for label in ["Restore defaults", "Cancel", "Apply", "OK"] {
            let control = settings_element(&ui, label, Button);
            let p = control.absolute_position();
            let size = control.size();
            assert!(
                p.x >= 0.0
                    && p.y >= 0.0
                    && size.height > 20.0
                    && p.x + size.width <= 840.5
                    && p.y + size.height <= 480.5,
                "Page {page}: {label} clipped at {p:?}, {size:?}"
            );
        }
    }
}
#[test]
fn trash_emptying_requires_confirmation_and_cannot_resume_unattended() {
    let (_root, ui, state, _left, _right) = setup();
    ui.invoke_action("EmptyTrash".into());
    ui.invoke_dialog_field("confirm".into(), "EMPT".into());
    ui.invoke_dialog_action("accept".into());
    assert!(state.borrow().app.job.is_none());
    assert!(!ui.get_dialog_error().is_empty());
    assert!(crate::queue::Queue::requires_input(&Operation::EmptyTrash));
    ui.invoke_dialog_action("cancel".into());
}
#[test]
fn legacy_text_can_be_decoded_edited_and_converted_without_losing_original_bytes() {
    let (_root, ui, state, left, _right) = setup();
    let path = left.join("legacy.txt");
    std::fs::write(&path, [0xe9, 0x20, 0x80]).unwrap();
    state.borrow_mut().app.tab_mut().entries = filesystem::list(&Location::Local(left)).unwrap();
    let entry = state
        .borrow()
        .app
        .tab()
        .entries
        .iter()
        .find(|e| e.path == path)
        .unwrap()
        .clone();
    let ctx = state.borrow().ctx.clone();
    state.borrow_mut().app.view(entry, true, &ctx);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while state.borrow().app.editor.is_none() {
        assert!(std::time::Instant::now() < deadline);
        state.borrow_mut().app.poll(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    state.borrow_mut().sync(&ui);
    let source = std::sync::Arc::downgrade(
        state
            .borrow()
            .app
            .editor
            .as_ref()
            .unwrap()
            .navigation
            .source
            .as_ref()
            .expect("Invalid UTF-8 opens as hex with a pinned source"),
    );
    ui.invoke_viewer_action("encoding:Windows-1252".into());
    assert!(
        source.upgrade().is_none(),
        "Editing must release the read-only source before replacing the file"
    );
    assert!(ui.get_viewer_editable());
    assert_eq!(ui.get_viewer_text(), "é €");
    ui.invoke_viewer_action("save-encoding:UTF-8".into());
    assert!(ui.get_viewer_dirty());
    let adapter = state.borrow();
    let document = adapter
        .app
        .editor
        .as_ref()
        .unwrap()
        .document
        .as_ref()
        .unwrap();
    assert_eq!(document.original, [0xe9, 0x20, 0x80]);
    assert_eq!(document.encode("é €").unwrap(), "é €".as_bytes());
    assert_eq!(std::fs::read(&path).unwrap(), [0xe9, 0x20, 0x80]);
    drop(adapter);
    state
        .borrow_mut()
        .app
        .preferences
        .shortcuts
        .insert("SaveText".into(), "Command+K".into());
    state.borrow_mut().sync(&ui);
    let mut save = key("k");
    save.modifiers.control = true;
    assert!(ui.invoke_global_key(save));
    assert!(state.borrow().app.job.is_some());
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while state.borrow().app.job.is_some() {
        assert!(std::time::Instant::now() < deadline);
        state.borrow_mut().app.poll(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !state.borrow().app.status_error,
        "{}",
        state.borrow().app.status
    );
    assert_eq!(std::fs::read(path).unwrap(), "é €".as_bytes());
    assert!(!state.borrow().app.editor.as_ref().unwrap().dirty());
}

#[test]
fn five_row_block_commands_match_upstream_increments() {
    let (_root, ui, state, left, _right) = setup();
    for i in 0..20 {
        std::fs::write(left.join(format!("{i:02}")), b"").unwrap();
    }
    let mut entries = filesystem::list(&Location::Local(left)).unwrap();
    filesystem::sort_entries(&mut entries, Default::default(), false);
    let first = entries[0].path.clone();
    let fifth = entries[5].path.clone();
    let fourth = entries[4].path.clone();
    let tenth = entries[10].path.clone();
    {
        let mut a = state.borrow_mut();
        a.app.tab_mut().entries = entries;
        a.app.tab_mut().cursor = Some(first);
        a.app.tab_mut().revision += 1;
        a.sync(&ui);
    }
    ui.invoke_action("SelectNextBlock".into());
    assert_eq!(state.borrow().app.tab().cursor, Some(fifth));
    assert!(state.borrow().app.tab().selected.is_empty());
    ui.invoke_action("MarkNextBlock".into());
    assert_eq!(state.borrow().app.tab().cursor, Some(tenth));
    assert_eq!(state.borrow().app.tab().selected.len(), 5);
    ui.invoke_action("UnmarkAll".into());
    ui.invoke_action("SelectFirstRow".into());
    assert!(ui.get_left_parent_selected());
    ui.invoke_action("MarkNextBlock".into());
    assert_eq!(state.borrow().app.tab().selected.len(), 4);
    assert_eq!(state.borrow().app.tab().cursor, Some(fourth));
}

fn press(ui: &CommanderWindow, text: impl Into<SharedString>) {
    let text = text.into();
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
}

#[test]
fn space_marks_and_advances_after_quick_find_but_types_in_text_fields() {
    let (_root, ui, state, left, _right) = setup();
    {
        let mut a = state.borrow_mut();
        filesystem::sort_entries(&mut a.app.tab_mut().entries, Default::default(), false);
        a.app.tab_mut().revision += 1;
        a.sync(&ui);
    }
    slint::platform::update_timers_and_animations();
    ui.invoke_focus_files(0);
    press(&ui, "a");
    assert_eq!(
        state.borrow().app.tab().cursor,
        Some(left.join("alpha.txt"))
    );
    press(&ui, " ");
    assert!(
        state
            .borrow()
            .app
            .tab()
            .selected
            .contains(&left.join("alpha.txt"))
    );
    assert_eq!(state.borrow().app.tab().cursor, Some(left.join("café.txt")));
    press(&ui, SharedString::from(slint::platform::Key::Insert));
    assert_eq!(state.borrow().app.tab().selected.len(), 2);
    assert_eq!(state.borrow().app.tab().cursor, Some(left.join("café.txt")));
    press(&ui, " ");
    assert_eq!(state.borrow().app.tab().selected.len(), 1);
    let marks = state.borrow().app.tab().selected.clone();
    ui.invoke_focus_filter(0);
    press(&ui, " ");
    assert_eq!(ui.get_left_filter(), " ");
    assert_eq!(state.borrow().app.tab().selected, marks);
    ui.invoke_focus_address(0);
    press(&ui, " ");
    assert_eq!(ui.get_left_address(), " ");
    assert_eq!(state.borrow().app.tab().selected, marks);
}

#[cfg(debug_assertions)]
#[test]
fn visible_swap_button_moves_tabs_marks_and_keyboard_focus_together() {
    use i_slint_backend_testing::ElementHandle;
    let (_root, ui, state, left, right) = setup();
    ui.invoke_action("NewTab".into());
    let left_ids: Vec<_> = state.borrow().app.panes[0]
        .tabs
        .iter()
        .map(|t| t.id)
        .collect();
    {
        let mut a = state.borrow_mut();
        a.app.tab_mut().entries = filesystem::list(&Location::Local(left.clone())).unwrap();
        filesystem::sort_entries(&mut a.app.tab_mut().entries, Default::default(), false);
        a.app.tab_mut().cursor = Some(left.join("alpha.txt"));
        a.app.tab_mut().selected.insert(left.join("alpha.txt"));
        a.app.tab_mut().revision += 1;
        a.sync(&ui);
    }
    ui.invoke_focus_files(0);
    for (orientation, single) in [
        ("SideBySide", false),
        ("Stacked", false),
        ("SideBySide", true),
    ] {
        ui.invoke_action(orientation.into());
        state.borrow_mut().app.preferences.single_pane = single;
        state.borrow_mut().sync(&ui);
        slint::platform::update_timers_and_animations();
        ElementHandle::find_by_accessible_label(&ui, "Swap panes")
            .next()
            .expect("Visible Swap panes button")
            .mock_single_click(slint::platform::PointerEventButton::Left);
        assert_eq!(state.borrow().app.active, 1);
        assert_eq!(
            state.borrow().app.panes[0].tab().location,
            Location::Local(right.clone())
        );
        assert_eq!(
            state.borrow().app.panes[1]
                .tabs
                .iter()
                .map(|t| t.id)
                .collect::<Vec<_>>(),
            left_ids
        );
        press(&ui, " ");
        assert_eq!(
            state.borrow().app.active,
            1,
            "Keyboard focus follows the swapped content"
        );
        assert!(
            !state
                .borrow()
                .app
                .tab()
                .selected
                .contains(&left.join("alpha.txt"))
        );
        assert_eq!(state.borrow().app.tab().cursor, Some(left.join("café.txt")));
        ui.invoke_action("Swap".into());
        {
            let mut a = state.borrow_mut();
            a.app.tab_mut().cursor = Some(left.join("alpha.txt"));
            a.app.tab_mut().selected.insert(left.join("alpha.txt"));
            a.sync(&ui);
        }
    }
}

#[cfg(debug_assertions)]
#[test]
fn mouse_clicks_move_cursor_without_clearing_space_marks() {
    use i_slint_backend_testing::ElementHandle;
    let (_root, ui, state, left, right) = setup();
    std::fs::write(right.join("right.txt"), b"right").unwrap();
    {
        let mut a = state.borrow_mut();
        filesystem::sort_entries(&mut a.app.tab_mut().entries, Default::default(), false);
        a.app.tab_mut().cursor = Some(left.join("alpha.txt"));
        a.app.tab_mut().revision += 1;
        a.app.panes[1].tab_mut().entries =
            filesystem::list(&Location::Local(right.clone())).unwrap();
        a.app.panes[1].tab_mut().revision += 1;
        a.sync(&ui);
    }
    slint::platform::update_timers_and_animations();
    ui.invoke_focus_files(0);
    press(&ui, " ");
    press(&ui, " ");
    let marks = state.borrow().app.tab().selected.clone();
    assert_eq!(marks.len(), 2);
    // Exercise both mouse-down selection and mouse-up click handling, including
    // an unmarked row and a trip to the other pane.
    for (label, side, path) in [
        ("alpha.txt", 0, left.join("alpha.txt")),
        ("folder", 0, left.join("folder")),
        ("right.txt", 1, right.join("right.txt")),
        ("café.txt", 0, left.join("café.txt")),
    ] {
        ElementHandle::find_by_accessible_label(&ui, label)
            .next()
            .expect("Visible file row")
            .mock_single_click(slint::platform::PointerEventButton::Left);
        let a = state.borrow();
        assert_eq!(a.app.active, side);
        assert_eq!(a.app.panes[side].tab().cursor, Some(path));
        assert_eq!(
            a.app.panes[0].tab().selected,
            marks,
            "Clicking {label} must preserve marks"
        );
        assert_eq!(
            a.app.panes[0]
                .tab()
                .chosen(false)
                .iter()
                .map(|e| e.path.clone())
                .collect::<std::collections::BTreeSet<_>>(),
            marks
        );
    }
    press(&ui, " ");
    assert_eq!(state.borrow().app.tab().selected.len(), 1);
    ui.invoke_action("ClearSelection".into());
    assert!(state.borrow().app.tab().selected.is_empty());
}
