use super::context_menus::{choose, element, open, press, setup};
use super::*;
use i_slint_backend_testing::ElementHandle;
use slint::platform::{
    Key,
    PointerEventButton::{Left, Middle, Right},
};

fn sorted(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow) {
    let mut a = state.borrow_mut();
    filesystem::sort_entries(&mut a.app.tab_mut().entries, Default::default(), false);
    a.app.tab_mut().revision += 1;
    a.sync(ui);
}

#[test]
fn focused_commands_ignore_marks_while_copy_keeps_them() {
    let (_root, ui, state, left, right) = setup();
    {
        let mut a = state.borrow_mut();
        a.app.tab_mut().selected.insert(left.join("alpha.txt"));
        a.app.tab_mut().cursor = Some(left.join("café.txt"));
        a.sync(&ui);
    }
    ui.invoke_action("Rename".into());
    assert!(
        matches!(&state.borrow().app.prompt.as_ref().unwrap().kind,PromptKind::Rename(path) if path==&left.join("café.txt"))
    );
    ui.invoke_dialog_action("cancel".into());
    ui.invoke_action("Edit".into());
    for _ in 0..500 {
        {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app.poll(&ctx);
            a.sync(&ui);
            if a.app.editor.is_some() {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        state.borrow().app.editor.as_ref().unwrap().path,
        Some(left.join("café.txt"))
    );
    ui.invoke_viewer_action("close".into());
    ui.invoke_action("Copy".into());
    assert!(
        matches!(&state.borrow().app.prompt.as_ref().unwrap().kind,PromptKind::Transfer{sources,..} if *sources==vec![left.join("alpha.txt")])
    );
    ui.invoke_dialog_action("cancel".into());
    element(&ui, "folder").mock_single_click(Left);
    ui.invoke_action("OpenOther".into());
    assert_eq!(
        state.borrow().app.panes[1].tab().location,
        Location::Local(left.join("folder"))
    );
    assert_ne!(
        state.borrow().app.panes[1].tab().location,
        Location::Local(right)
    );
    press(&ui, Key::Return);
    assert_eq!(
        state.borrow().app.tab().location,
        Location::Local(left.join("folder"))
    );
}

#[test]
fn direct_file_navigation_reveals_the_target_outside_the_current_filter() {
    let (_root, ui, state, left, _right) = setup();
    ui.invoke_filter(0, "alpha".into());
    ui.invoke_navigate(0, left.join("café.txt").display().to_string().into());
    for _ in 0..500 {
        {
            let mut a = state.borrow_mut();
            let ctx = a.ctx.clone();
            a.app.poll(&ctx);
            a.sync(&ui);
            if !a.app.tab().loading {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(ui.get_left_filter().is_empty());
    assert_eq!(state.borrow().app.tab().cursor, Some(left.join("café.txt")));
    assert!(
        ui.get_left_rows()
            .iter()
            .any(|row| row.name == "café.txt" && row.cursor)
    );
}

#[test]
fn parent_keyboard_ranges_and_hidden_parent_preference_are_consistent() {
    let (_root, ui, state, left, _right) = setup();
    sorted(&state, &ui);
    element(&ui, "folder").mock_single_click(Left);
    press(&ui, Key::UpArrow);
    assert!(ui.get_left_parent_selected());
    press(&ui, Key::DownArrow);
    assert_eq!(state.borrow().app.tab().cursor, Some(left.join("folder")));
    press(&ui, Key::Home);
    assert!(ui.get_left_parent_selected());
    let mut event = slint::language::KeyEvent::default();
    event.text = Key::End.into();
    event.modifiers.shift = true;
    ui.invoke_pane_key(0, event);
    assert_eq!(state.borrow().app.tab().selected.len(), 3);
    assert!(
        state
            .borrow()
            .app
            .tab()
            .selected
            .iter()
            .all(|p| p.starts_with(&left))
    );
    state.borrow_mut().app.preferences.parent_entry = false;
    press(&ui, Key::Home);
    assert!(!ui.get_left_parent_selected());
    assert_eq!(state.borrow().app.tab().cursor, Some(left.join("folder")));
    press(&ui, Key::UpArrow);
    assert_eq!(state.borrow().app.tab().cursor, Some(left.join("folder")));
}

#[test]
fn middle_click_marks_without_opening_and_held_space_does_not_flap_last_row() {
    let (_root, ui, state, left, _right) = setup();
    sorted(&state, &ui);
    element(&ui, "alpha.txt").mock_single_click(Middle);
    assert!(
        state
            .borrow()
            .app
            .tab()
            .selected
            .contains(&left.join("alpha.txt"))
    );
    assert_eq!(
        state.borrow().app.tab().cursor,
        Some(left.join("alpha.txt"))
    );
    element(&ui, "café.txt").mock_single_click(Left);
    let mut event = slint::language::KeyEvent::default();
    event.text = " ".into();
    ui.invoke_pane_key(0, event.clone());
    event.repeat = true;
    for _ in 0..5 {
        ui.invoke_pane_key(0, event.clone());
    }
    assert!(
        state
            .borrow()
            .app
            .tab()
            .selected
            .contains(&left.join("café.txt"))
    );
    event.repeat = false;
    ui.invoke_pane_key(0, event);
    assert!(
        !state
            .borrow()
            .app
            .tab()
            .selected
            .contains(&left.join("café.txt"))
    );
}

#[test]
fn background_and_parent_contexts_have_safe_targets() {
    let (root, ui, state, left, right) = setup();
    element(&ui, "alpha.txt").mock_single_click(Left);
    let position = slint::LogicalPosition::new(1000., 600.);
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position,
            button: Right,
        });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position,
            button: Right,
        });
    assert_eq!(state.borrow().app.active, 1);
    assert_eq!(state.borrow().app.tab().location, Location::Local(right));
    assert!(
        ui.get_menu_items()
            .iter()
            .any(|item| item.id == "PasteClipboard")
    );
    assert!(
        !ui.get_menu_items()
            .iter()
            .any(|item| item.id == "Rename" || item.id == "Trash" || item.id == "Open")
    );
    press(&ui, Key::Escape);
    let parent = ElementHandle::find_by_accessible_label(&ui, "Parent directory row")
        .min_by(|a, b| a.absolute_position().x.total_cmp(&b.absolute_position().x))
        .unwrap();
    parent.mock_single_click(Left);
    parent.mock_single_click(Right);
    assert_eq!(state.borrow().app.tab().location, Location::Local(left));
    assert!(ui.get_left_parent_selected());
    assert!(
        !ui.get_menu_items()
            .iter()
            .any(|item| item.id == "Rename" || item.id == "Trash")
    );
    choose(&ui, "Open");
    assert_eq!(
        state.borrow().app.tab().location,
        Location::Local(root.path().into())
    );
}

#[test]
fn header_and_tab_contexts_use_clicked_pane_and_retain_tab_metadata() {
    let (_root, ui, state, _left, _right) = setup();
    let names: Vec<_> = ElementHandle::find_by_accessible_label(&ui, "Name").collect();
    names[1].mock_single_click(Right);
    assert_eq!(state.borrow().app.active, 1);
    assert!(
        !ui.get_menu_items()
            .iter()
            .any(|item| item.id == "Column:Name")
    );
    choose(&ui, "Column:Extension");
    assert!(
        state
            .borrow()
            .app
            .preferences
            .columns
            .iter()
            .any(|c| c.kind == crate::preferences::Column::Extension)
    );
    let names: Vec<_> = ElementHandle::find_by_accessible_label(&ui, "Name").collect();
    names[1].mock_single_click(Right);
    let extension = ui
        .get_menu_items()
        .iter()
        .find(|item| item.id == "Column:Extension")
        .unwrap();
    assert!(extension.checked);
    assert_eq!(
        element(&ui, &extension.title).accessible_checked(),
        Some(true)
    );
    choose(&ui, "Column:Extension");
    let title = ui.get_right_tabs().row_data(0).unwrap();
    open(&ui, &title);
    choose(&ui, "LockTab");
    let title = ui.get_right_tabs().row_data(0).unwrap();
    open(&ui, &title);
    choose(&ui, "DuplicateTab");
    assert_eq!(state.borrow().app.panes[1].tabs.len(), 2);
    assert!(
        state.borrow().app.panes[1]
            .tabs
            .iter()
            .all(|tab| tab.state.locked.is_some())
    );
    let active_id = state.borrow().app.tab().id;
    let title = ui.get_right_tabs().row_data(0).unwrap();
    ElementHandle::find_by_accessible_label(&ui, &title)
        .next()
        .unwrap()
        .mock_single_click(Middle);
    assert_eq!(state.borrow().app.panes[1].tabs.len(), 1);
    assert_eq!(state.borrow().app.tab().id, active_id);
    element(&ui, &ui.get_right_tabs().row_data(0).unwrap()).mock_single_click(Middle);
    assert_eq!(state.borrow().app.panes[1].tabs.len(), 1);
}

#[test]
fn keyboard_context_menu_anchors_to_cursor_navigates_scrolls_and_dispatches_once() {
    let (_root, ui, state, _left, _right) = setup();
    element(&ui, "café.txt").mock_single_click(Left);
    let row_y = element(&ui, "café.txt").absolute_position().y;
    let before = state.borrow().app.tab().cursor.clone();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let captured = calls.clone();
    ui.on_action(move |id| captured.borrow_mut().push(id.to_string()));
    let mut event = slint::language::KeyEvent::default();
    event.text = Key::F10.into();
    event.modifiers.shift = true;
    ui.invoke_pane_key(0, event);
    slint::platform::update_timers_and_animations();
    assert!((ui.get_menu_y() - row_y).abs() < 26.);
    let enabled: Vec<_> = ui
        .get_menu_items()
        .iter()
        .enumerate()
        .filter(|(_, c)| c.enabled)
        .collect();
    assert_eq!(ui.get_menu_index(), enabled[0].0 as i32);
    press(&ui, Key::DownArrow);
    assert_eq!(ui.get_menu_index(), enabled[1].0 as i32);
    assert_eq!(state.borrow().app.tab().cursor, before);
    press(&ui, Key::Home);
    for _ in 0..4 {
        press(&ui, Key::DownArrow);
    }
    assert_eq!(
        ui.get_menu_items()
            .row_data(ui.get_menu_index() as usize)
            .unwrap()
            .id,
        "View"
    );
    press(&ui, "r");
    assert_eq!(
        ui.get_menu_items()
            .row_data(ui.get_menu_index() as usize)
            .unwrap()
            .id,
        "Rename"
    );
    press(&ui, Key::End);
    let last = enabled.last().unwrap();
    assert_eq!(ui.get_menu_index(), last.0 as i32);
    assert!(
        ElementHandle::find_by_accessible_label(&ui, &last.1.title)
            .next()
            .is_some()
    );
    press(&ui, Key::Return);
    assert_eq!(*calls.borrow(), vec![last.1.id.to_string()]);
    assert_eq!(ui.get_focused_control(), 0);
}

#[test]
fn every_tab_creation_path_obeys_the_limit_and_moving_tabs_moves_focus() {
    let (_root, ui, state, _left, _right) = setup();
    ui.invoke_action("MoveTabOther".into());
    assert_eq!(state.borrow().app.active, 1);
    assert_eq!(ui.get_focused_control(), 3);
    press(&ui, Key::DownArrow);
    assert_eq!(state.borrow().app.active, 1);
    let mut a = state.borrow_mut();
    let ctx = a.ctx.clone();
    for side in 0..2 {
        while a.app.panes[side].tabs.len() < 32 {
            a.app.panes[side].add_tab(&ctx);
        }
    }
    for action in [
        Action::NewTab,
        Action::DuplicateTab,
        Action::CloneTab,
        Action::OpenNewTab,
    ] {
        a.app.act(action, &ctx);
        assert_eq!(a.app.panes[0].tabs.len(), 32);
        assert_eq!(a.app.panes[1].tabs.len(), 32);
        assert!(a.app.status_error);
        assert!(a.app.status.contains("32"));
    }
}

#[test]
fn scaled_panes_keep_pointer_keyboard_accessibility_and_context_targets_aligned() {
    use i_slint_backend_testing::AccessibleRole;
    let (_root, ui, state, left, _right) = setup();
    for scale_factor in [1.0, 1.25, 1.5, 2.0] {
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor });
        ui.window().set_size(slint::LogicalSize::new(1240.0, 780.0));
        slint::platform::update_timers_and_animations();
        let row = ElementHandle::find_by_accessible_label(&ui, "alpha.txt")
            .find(|e| e.accessible_role() == Some(AccessibleRole::ListItem))
            .unwrap();
        row.mock_single_click(Left);
        assert_eq!(
            state.borrow().app.tab().cursor,
            Some(left.join("alpha.txt"))
        );
        assert_eq!(row.accessible_item_selected(), Some(false));
        press(&ui, " ");
        assert_eq!(row.accessible_item_selected(), Some(true));
        assert!(
            state
                .borrow()
                .app
                .tab()
                .selected
                .contains(&left.join("alpha.txt"))
        );
        open(&ui, "café.txt");
        choose(&ui, "Rename");
        assert!(
            matches!(&state.borrow().app.prompt.as_ref().unwrap().kind,PromptKind::Rename(p) if p==&left.join("café.txt"))
        );
        press(&ui, Key::Escape);
        assert!(state.borrow().app.prompt.is_none());
        assert!(
            state
                .borrow()
                .app
                .tab()
                .selected
                .contains(&left.join("alpha.txt"))
        );
        state.borrow_mut().app.tab_mut().selected.clear();
    }
}

#[cfg(unix)]
#[test]
fn permission_fields_sync_octal_symbolic_and_each_checkbox() {
    let (_root, ui, state, _left, _right) = setup();
    element(&ui, "alpha.txt").mock_single_click(Left);
    ui.invoke_action("Permissions".into());
    ui.invoke_dialog_field("prompt".into(), "0640".into());
    ui.invoke_dialog_field("permission-bit-1000".into(), "true".into());
    ui.invoke_dialog_field("permission-bit-1".into(), "true".into());
    assert_eq!(state.borrow().app.prompt.as_ref().unwrap().value, "1641");
    assert!(
        ui.get_dialog_fields()
            .iter()
            .any(|f| f.title == "Symbolic permissions" && f.value == "rw-r----t")
    );
    ui.invoke_dialog_field("prompt".into(), "0755".into());
    assert_eq!(
        ui.get_dialog_fields()
            .iter()
            .find(|f| f.id == "permission-bit-1000")
            .unwrap()
            .value,
        "false"
    );
    ui.invoke_dialog_field("prompt".into(), "8888".into());
    ui.invoke_dialog_action("accept".into());
    assert!(state.borrow().app.job.is_none());
    assert!(ui.get_dialog_error().contains("octal"));
}

#[test]
fn alt_menu_navigation_moves_between_headings_and_restores_pane_focus() {
    let (_root, ui, state, _left, _right) = setup();
    ui.set_system_menubar(false);
    ui.invoke_focus_files(0);
    press(&ui, Key::F10);
    assert!(ui.get_menu_heading_active());
    assert_eq!(ui.get_menu_heading_index(), 0);
    press(&ui, Key::RightArrow);
    assert_eq!(ui.get_menu_heading_index(), 1);
    press(&ui, Key::DownArrow);
    assert!(ui.get_menu_items().iter().any(|i| i.id == "Copy"));
    press(&ui, Key::RightArrow);
    assert_eq!(ui.get_menu_heading_index(), 2);
    assert!(ui.get_menu_items().iter().any(|i| i.id == "Parent"));
    press(&ui, Key::Escape);
    assert!(ui.get_menu_heading_active());
    press(&ui, Key::Escape);
    assert!(!ui.get_menu_heading_active());
    assert!(state.borrow().app.prompt.is_none());
}
#[test]
fn ownership_acl_form_loads_async_and_retains_invalid_drafts() {
    let (_root, ui, state, left, _right) = setup();
    ui.invoke_selected(0, path_id(&left.join("alpha.txt")).into(), false, false);
    ui.invoke_action("OwnershipAcl".into());
    for _ in 0..1000 {
        let mut a = state.borrow_mut();
        a.security_tick();
        a.sync(&ui);
        if a.security_dialog().is_some_and(|dialog| !dialog.busy) {
            break;
        }
        drop(a);
        std::thread::sleep(Duration::from_millis(3));
    }
    assert_eq!(ui.get_dialog_title(), "Ownership and ACL");
    ui.invoke_dialog_field(
        "security-owner".into(),
        "rust-commander-no-such-identity".into(),
    );
    ui.invoke_dialog_action("security-apply".into());
    for _ in 0..1000 {
        let mut a = state.borrow_mut();
        let ctx = a.ctx.clone();
        a.app.poll(&ctx);
        a.security_tick();
        a.sync(&ui);
        if a.app.job.is_none() {
            break;
        }
        drop(a);
        std::thread::sleep(Duration::from_millis(3));
    }
    assert_eq!(ui.get_dialog_title(), "Ownership and ACL");
    assert!(!ui.get_dialog_error().is_empty());
    assert!(
        ui.get_dialog_fields()
            .iter()
            .any(|f| f.id == "security-owner" && f.value == "rust-commander-no-such-identity")
    );
    ui.invoke_dialog_action("cancel".into());
    assert!(left.join("alpha.txt").exists());
}

#[test]
fn screen_reader_selection_changes_when_space_marks_the_focused_last_file() {
    use i_slint_backend_testing::AccessibleRole;
    let (_root, ui, state, left, _) = setup();
    sorted(&state, &ui);
    let row = ElementHandle::find_by_accessible_label(&ui, "café.txt")
        .find(|e| e.accessible_role() == Some(AccessibleRole::ListItem))
        .unwrap();
    row.mock_single_click(Left);
    assert_eq!(row.accessible_item_selected(), Some(false));
    press(&ui, " ");
    assert_eq!(row.accessible_item_selected(), Some(true));
    row.mock_single_click(Left);
    press(&ui, " ");
    assert_eq!(row.accessible_item_selected(), Some(false));
    assert_eq!(state.borrow().app.tab().cursor, Some(left.join("café.txt")));
}
