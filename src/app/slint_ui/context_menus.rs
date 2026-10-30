//! Use physical pointer events: directly invoking actions misses popup teardown.
use super::*;
use i_slint_backend_testing::ElementHandle;
use slint::platform::PointerEventButton::{Left, Right};

fn setup() -> (
    tempfile::TempDir,
    CommanderWindow,
    Rc<RefCell<Adapter>>,
    PathBuf,
    PathBuf,
) {
    i_slint_backend_testing::init_no_event_loop();
    let (root, mut adapter, left, right) = tests::fixture();
    std::fs::write(right.join("right.txt"), b"right").unwrap();
    adapter.app.panes[1].tab_mut().entries =
        filesystem::list(&Location::Local(right.clone())).unwrap();
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.window().set_size(slint::LogicalSize::new(1240.0, 780.0));
    ui.show().unwrap();
    slint::platform::update_timers_and_animations();
    (root, ui, state, left, right)
}

fn element(ui: &CommanderWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(ui, label)
        .next()
        .unwrap_or_else(|| panic!("Missing visible element: {label}"))
}
fn open(ui: &CommanderWindow, label: &str) {
    element(ui, label).mock_single_click(Right);
    slint::platform::update_timers_and_animations();
}
fn choose(ui: &CommanderWindow, id: &str) {
    let command = ui
        .get_menu_items()
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("Missing menu action: {id}"));
    assert!(command.enabled, "Disabled action: {id}");
    // Long menus need the same wheel scrolling as a physical user. Searching
    // visible accessibility elements must not assume every row fits at once.
    for _ in 0..20 {
        if ElementHandle::find_by_accessible_label(ui, &command.title)
            .next()
            .is_some()
        {
            break;
        }
        let visible = ui
            .get_menu_items()
            .iter()
            .find_map(|item| ElementHandle::find_by_accessible_label(ui, &item.title).next())
            .expect("No visible menu rows to scroll");
        visible.scroll(0.0, -140.0);
        slint::platform::update_timers_and_animations();
    }
    element(ui, &command.title).mock_single_click(Left);
}

#[test]
fn right_click_trash_confirms_clicked_pane_and_preserves_multi_selection() {
    let (_root, ui, state, left, right) = setup();
    {
        let mut a = state.borrow_mut();
        a.app.panes[0]
            .tab_mut()
            .selected
            .extend([left.join("alpha.txt"), left.join("café.txt")]);
        a.sync(&ui);
    }
    open(&ui, "alpha.txt");
    choose(&ui, "Trash");
    assert_eq!(ui.get_dialog_title(), "Move to Trash");
    assert!(
        matches!(&state.borrow().app.prompt.as_ref().unwrap().kind, PromptKind::Trash(paths)
        if *paths == vec![left.join("alpha.txt"), left.join("café.txt")])
    );
    ui.invoke_dialog_action("cancel".into());
    open(&ui, "right.txt");
    choose(&ui, "Trash");
    assert_eq!(state.borrow().app.active, 1);
    assert!(
        matches!(&state.borrow().app.prompt.as_ref().unwrap().kind, PromptKind::Trash(paths) if *paths == vec![right.join("right.txt")])
    );
    ui.invoke_dialog_action("cancel".into());
    assert!(
        left.join("alpha.txt").exists()
            && left.join("café.txt").exists()
            && right.join("right.txt").exists()
    );
}

#[test]
fn every_file_context_action_dispatches_once_from_either_pane() {
    let (_root, ui, state, _left, _right) = setup();
    // Capture dispatch for OS-facing and destructive commands. The actual Trash
    // confirmation is tested separately; this loop must not launch other apps,
    // empty system Trash, or modify credential storage.
    let dispatched = Rc::new(RefCell::new(Vec::new()));
    let calls = dispatched.clone();
    ui.on_action(move |id| calls.borrow_mut().push(id.to_string()));
    let expected: Vec<_> = action_menus()
        .into_iter()
        .find(|(name, _)| *name == "File")
        .unwrap()
        .1
        .into_iter()
        .map(|(_, action)| format!("{action:?}"))
        .collect();
    for (side, row) in [(0, "alpha.txt"), (1, "right.txt")] {
        for id in &expected {
            open(&ui, row);
            assert_eq!(state.borrow().app.active, side);
            let before = dispatched.borrow().len();
            choose(&ui, id);
            assert_eq!(
                &dispatched.borrow()[before..],
                std::slice::from_ref(id),
                "{id} in pane {side}"
            );
        }
    }
}

#[test]
fn places_context_actions_dispatch_and_edit_the_selected_items() {
    let (_root, ui, state, left, right) = setup();
    {
        let mut a = state.borrow_mut();
        a.app.bookmarks = vec![
            settings::Bookmark {
                name: "First bookmark".into(),
                location: Location::Local(left),
            },
            settings::Bookmark {
                name: "Second bookmark".into(),
                location: Location::Local(right),
            },
        ];
        a.app.connections = ["First connection", "Second connection"]
            .map(|name| crate::remote::Connection {
                name: name.into(),
                service: "webdav".into(),
                ..Default::default()
            })
            .to_vec();
        a.place_sections_expanded[2] = false; // Keep connection rows on screen.
        a.sync(&ui);
    }
    open(&ui, "Second bookmark");
    choose(&ui, "RemoveBookmark:1");
    assert_eq!(state.borrow().app.bookmarks.len(), 1);
    assert_eq!(state.borrow().app.bookmarks[0].name, "First bookmark");
    open(&ui, "Second connection");
    choose(&ui, "EditConnection:1");
    assert_eq!(
        state
            .borrow()
            .app
            .connection_form
            .as_ref()
            .unwrap()
            .connection
            .name,
        "Second connection"
    );
    ui.invoke_dialog_action("cancel".into());
    let dispatched = Rc::new(RefCell::new(Vec::new()));
    let calls = dispatched.clone();
    ui.on_action(move |id| calls.borrow_mut().push(id.to_string()));
    for (row, id) in [
        ("First bookmark", "RemoveBookmark:0"),
        ("First connection", "EditConnection:0"),
        ("First connection", "ForgetCredentials:0"),
        ("Second connection", "EditConnection:1"),
        ("Second connection", "ForgetCredentials:1"),
    ] {
        open(&ui, row);
        let before = dispatched.borrow().len();
        choose(&ui, id);
        assert_eq!(&dispatched.borrow()[before..], &[id.to_owned()]);
    }
}

#[test]
fn disabled_read_only_actions_and_clicking_outside_do_not_dispatch() {
    let (_root, ui, state, _left, _right) = setup();
    let connection = crate::remote::Connection {
        service: "http".into(),
        endpoint: "https://example.test".into(),
        ..Default::default()
    };
    crate::remote::register_profiles(std::slice::from_ref(&connection));
    {
        let mut a = state.borrow_mut();
        a.app.panes[0].tab_mut().location = Location::Remote {
            connection: connection.id,
            path: PathBuf::new(),
        };
        a.sync(&ui);
    }
    let dispatched = Rc::new(RefCell::new(Vec::new()));
    let calls = dispatched.clone();
    ui.on_action(move |id| calls.borrow_mut().push(id.to_string()));
    open(&ui, "alpha.txt");
    for id in ["Trash", "Rename", "Mkdir", "ExternalEdit"] {
        let command = ui.get_menu_items().iter().find(|c| c.id == id).unwrap();
        assert!(!command.enabled, "{id}");
        element(&ui, &command.title).mock_single_click(Left);
        assert!(dispatched.borrow().is_empty());
        open(&ui, "alpha.txt");
    }
    let position = slint::LogicalPosition::new(5.0, 5.0);
    for pressed in [true, false] {
        ui.window().dispatch_event(if pressed {
            slint::platform::WindowEvent::PointerPressed {
                position,
                button: Left,
            }
        } else {
            slint::platform::WindowEvent::PointerReleased {
                position,
                button: Left,
            }
        });
    }
    assert!(dispatched.borrow().is_empty());
    let title = ui
        .get_menu_items()
        .iter()
        .find(|c| c.id == "Trash")
        .unwrap()
        .title;
    assert!(
        ElementHandle::find_by_accessible_label(&ui, &title)
            .next()
            .is_none()
    );
}

#[test]
fn file_context_dialog_actions_open_and_cancel_without_modifying_files() {
    let (_root, ui, state, left, _right) = setup();
    for (id, dialog) in [
        ("Connections", "connections"),
        ("ArchiveTools", "archive"),
        ("Mkfile", "prompt"),
        ("BatchRename", "rename"),
        ("Permissions", "prompt"),
        ("Modified", "prompt"),
        ("OpenWith", "open-with"),
        ("OpenAs", "open-as"),
        ("Associations", "associations"),
        ("Rename", "prompt"),
        ("Mkdir", "prompt"),
        ("Trash", "prompt"),
        ("PermanentDelete", "prompt"),
        ("EmptyTrash", "empty-trash"),
        ("ImportSettings", "import"),
        ("ImportedCommands", "quick-list"),
    ] {
        open(&ui, "alpha.txt");
        choose(&ui, id);
        assert_eq!(state.borrow().dialog_kind, dialog, "{id}");
        if id == "Rename" {
            for character in "renamed.txt".chars() {
                let text: SharedString = character.to_string().into();
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                        text: text.clone(),
                    });
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
            }
            assert_eq!(
                state.borrow().app.prompt.as_ref().unwrap().value,
                "renamed.txt",
                "The new dialog must receive keyboard focus"
            );
        }
        ui.invoke_dialog_action("cancel".into());
        assert!(!state.borrow().blocked(), "{id} must dismiss completely");
    }
    assert_eq!(std::fs::read(left.join("alpha.txt")).unwrap(), b"alpha");
}

#[test]
fn address_context_menu_select_copy_cut_undo_redo_and_paste_work() {
    let (_root, ui, _state, _left, _right) = setup();
    let original = ui.get_left_address();
    for action in ["Select All", "Copy", "Cut", "Undo", "Redo", "Paste"] {
        open(&ui, "Left location");
        element(&ui, action).mock_single_click(Left);
        let expected = if matches!(action, "Cut" | "Redo") {
            SharedString::default()
        } else {
            original.clone()
        };
        assert_eq!(ui.get_left_address(), expected, "{action}");
    }
}
