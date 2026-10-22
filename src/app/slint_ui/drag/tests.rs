use super::*;
fn setup() -> (
    tempfile::TempDir,
    CommanderWindow,
    Rc<RefCell<Adapter>>,
    PathBuf,
    PathBuf,
) {
    i_slint_backend_testing::init_no_event_loop();
    let (root, adapter, left, right) = super::super::tests::fixture();
    let ui = CommanderWindow::new().unwrap();
    let state = Rc::new(RefCell::new(adapter));
    super::super::wire(&ui, state.clone());
    state.borrow_mut().sync(&ui);
    ui.window().set_size(slint::LogicalSize::new(1240.0, 780.0));
    ui.show().unwrap();
    (root, ui, state, left, right)
}
#[test]
fn payload_freezes_selection_through_navigation_and_owner_refresh() {
    let (_root, ui, state, left, right) = setup();
    let paths = vec![left.join("alpha.txt"), left.join("café.txt")];
    state
        .borrow_mut()
        .app
        .tab_mut()
        .selected
        .extend(paths.clone());
    state.borrow_mut().sync(&ui);
    let token = ui.get_left_selection_payload();
    let data = ui.global::<Transfers>().invoke_payload(token.clone());
    ui.invoke_navigate(0, right.display().to_string().into());
    state.borrow_mut().sync(&ui);
    let source = selection(&data).unwrap();
    assert_eq!(source.from, Location::Local(left));
    assert_eq!(source.entries.len(), 2);
    assert_eq!(
        data.file_paths()
            .unwrap()
            .map(Path::to_owned)
            .collect::<Vec<_>>(),
        paths
    );
    assert!(
        selection(&payload(&token)).is_some(),
        "DataTransfer owns its original source even after row handles disappear"
    );
    drop(source);
    drop(data);
    assert!(selection(&payload(&token)).is_none());
}
#[test]
fn folder_tab_breadcrumb_and_place_drops_capture_the_exact_destination() {
    let (_root, ui, state, left, right) = setup();
    let folder = left.join("folder");
    let source = Handle::new(
        Selection::local(&[right.join("incoming.txt")].map(|p| {
            std::fs::write(&p, b"source").unwrap();
            p
        }))
        .unwrap(),
    );
    let data = payload(&source.token);
    let target = format!("folder:{}", path_id(&folder));
    assert_eq!(
        ui.global::<Transfers>().invoke_can_drop(
            0,
            target.clone().into(),
            data.clone(),
            DragAction::Move
        ),
        DragAction::Move
    );
    assert!(
        ui.global::<Transfers>()
            .invoke_drop(0, target.into(), data.clone(), DragAction::Move)
    );
    let mut a = state.borrow_mut();
    match &a.app.prompt.as_ref().unwrap().kind {
        PromptKind::DropTransfer {
            to,
            moving,
            selection,
        } => {
            assert_eq!(*to, Location::Local(folder.clone()));
            assert!(*moving);
            assert_eq!(selection.entries.len(), 1);
        }
        _ => panic!("wrong confirmation"),
    }
    a.app.prompt = None;
    let crumb = a.breadcrumbs[0].len() - 1;
    assert_eq!(
        a.drop_destination(0, &format!("breadcrumb:{crumb}")),
        Some(Location::Local(left.clone()))
    );
    a.places
        .push(("test-destination".into(), Location::Local(folder.clone())));
    assert_eq!(
        a.drop_destination(0, "place:test-destination"),
        Some(Location::Local(folder.clone()))
    );
    let ctx = a.ctx.clone();
    a.app.panes[0]
        .tabs
        .push(Tab::new(Location::Local(folder.clone()), &ctx));
    let index = a.app.panes[0].tabs.len() - 1;
    assert_eq!(
        a.drop_destination(0, &format!("tab:{index}")),
        Some(Location::Local(folder))
    );
    assert!(a.drop_to(0, &format!("tab:{index}"), data, DragAction::Copy));
    assert_eq!(a.app.panes[0].active, index);
    assert!(
        !std::fs::read_dir(left)
            .unwrap()
            .any(|e| e.unwrap().file_name() == "incoming.txt"),
        "drop only opens confirmation"
    );
}
#[test]
fn cancellation_and_read_only_targets_never_start_a_transfer() {
    let (_root, ui, state, left, right) = setup();
    let source = Handle::new(Selection::local(&[left.join("alpha.txt")]).unwrap());
    let data = payload(&source.token);
    assert!(
        ui.global::<Transfers>()
            .invoke_drop(1, "pane".into(), data.clone(), DragAction::Copy)
    );
    ui.invoke_dialog_action("cancel".into());
    assert!(state.borrow().app.prompt.is_none());
    assert!(state.borrow().app.job.is_none());
    assert!(!right.join("alpha.txt").exists());
    let connection = crate::remote::Connection {
        service: "http".into(),
        endpoint: "https://example.test".into(),
        ..Default::default()
    };
    crate::remote::register_profiles(std::slice::from_ref(&connection));
    state.borrow_mut().app.panes[1].tab_mut().location = Location::Remote {
        connection: connection.id,
        path: PathBuf::new(),
    };
    assert_eq!(
        state.borrow().can_drop(1, "pane", &data, DragAction::Move),
        DragAction::None
    );
    assert_eq!(
        state.borrow().can_drop(-1, "pane", &data, DragAction::Copy),
        DragAction::None
    );
    assert_eq!(
        state
            .borrow()
            .can_drop(0, "folder:missing", &data, DragAction::Copy),
        DragAction::None
    );
}
#[test]
fn virtual_payloads_and_native_round_trips_preserve_original_source_only_during_the_drag() {
    let (_root, _ui, _state, left, _right) = setup();
    let original = Selection {
        from: Location::Archive {
            file: left.join("source.zip"),
            prefix: PathBuf::new(),
        },
        entries: vec![Entry {
            path: "alpha.txt".into(),
            ..filesystem::entry_from_path(&left.join("alpha.txt")).unwrap()
        }],
    };
    let handle = Handle::new(original.clone());
    let data = payload(&handle.token);
    assert!(!data.has_file_paths());
    assert_eq!(selection(&data).unwrap().from, original.from);
    let paths = vec![left.join("alpha.txt")];
    let ready = Ready {
        selection: original.clone(),
        paths: paths.clone(),
        prepared: std::time::Instant::now(),
    };
    remember(ready.clone());
    assert!(
        selection(&native_payload(paths.clone())).is_none(),
        "cached exports must not reinterpret an unrelated later native drag"
    );
    ACTIVE.with(|active| {
        *active.borrow_mut() = Some((ready, std::time::Instant::now() + Duration::from_secs(1)))
    });
    assert_eq!(
        selection(&native_payload(paths.clone())).unwrap().from,
        original.from
    );
    ACTIVE.with(|active| *active.borrow_mut() = None);
    assert!(selection(&native_payload(paths)).is_none());
}
#[test]
fn native_hit_feedback_survives_adapter_borrow_and_prioritizes_folder_rows() {
    let (_root, ui, state, left, right) = setup();
    let file = right.join("incoming.txt");
    std::fs::write(&file, b"incoming").unwrap();
    let mut a = state.borrow_mut();
    a.drag.hovered = vec![file.clone()];
    a.drag.position = Some((450.0, 280.0));
    *a.drag.targets.borrow_mut() = a.native_targets();
    let target = format!("folder:{}", path_id(&left.join("folder")));
    let data = native_payload(vec![file]);
    assert_eq!(
        ui.global::<Transfers>()
            .invoke_can_drop(0, target.clone().into(), data, DragAction::Copy),
        DragAction::Copy,
        "native bindings can evaluate while Adapter is mutably borrowed"
    );
    ui.global::<Transfers>()
        .invoke_hit(0, "pane".into(), 5, true);
    ui.global::<Transfers>()
        .invoke_hit(0, target.clone().into(), 50, true);
    ui.global::<Transfers>()
        .invoke_hit(0, "pane".into(), 101, true);
    ui.global::<Transfers>()
        .invoke_hit(0, "pane".into(), 101, false);
    a.drag_tick(&ui);
    assert_eq!(a.drag.last_hit, Some((0, target)));
}
#[test]
fn drag_preparation_cancel_and_close_wait_for_the_worker() {
    let (_root, ui, state, left, _right) = setup();
    let cancel = Arc::new(AtomicBool::new(false));
    let (_updates, updates) = mpsc::channel();
    let (sender, result) = mpsc::channel();
    let selection = Selection::local(&[left.join("alpha.txt")]).unwrap();
    state.borrow_mut().drag.preparation = Some(Preparation {
        cancel: cancel.clone(),
        progress: Default::default(),
        updates,
        result,
    });
    let mut escape = slint::language::KeyEvent::default();
    escape.text = slint::platform::Key::Escape.into();
    assert!(ui.invoke_global_key(escape));
    assert!(cancel.load(Ordering::Relaxed));
    sender
        .send(Ok(Ready {
            selection,
            paths: vec![left.join("alpha.txt")],
            prepared: std::time::Instant::now(),
        }))
        .unwrap();
    state.borrow_mut().drag_tick(&ui);
    assert!(state.borrow().drag.ready.is_none());
    assert!(state.borrow().drag.preparation.is_none());
    let (_updates, updates) = mpsc::channel();
    let (_sender, result) = mpsc::channel();
    state.borrow_mut().drag.preparation = Some(Preparation {
        cancel,
        progress: Default::default(),
        updates,
        result,
    });
    state.borrow_mut().request_close();
    assert!(state.borrow().app.confirm_exit);
    assert!(!state.borrow().app.exiting);
}

#[cfg(debug_assertions)]
#[test]
fn ordinary_pointer_drag_crosses_the_threshold_and_keeps_multiple_selected_files() {
    use i_slint_backend_testing::ElementHandle;
    use slint::platform::{PointerEventButton, WindowEvent};
    let (_root, ui, state, left, _right) = setup();
    state
        .borrow_mut()
        .app
        .tab_mut()
        .selected
        .extend([left.join("alpha.txt"), left.join("café.txt")]);
    state.borrow_mut().sync(&ui);
    slint::platform::update_timers_and_animations();
    let row = ElementHandle::find_by_accessible_label(&ui, "alpha.txt")
        .next()
        .unwrap();
    let origin = row.absolute_position();
    let position = slint::LogicalPosition::new(origin.x + 50.0, origin.y + 10.0);
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(200));
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(position.x + 50.0, position.y),
    });
    slint::platform::update_timers_and_animations();
    let pending = state
        .borrow()
        .drag
        .pending
        .clone()
        .expect("ordinary pointer movement begins a native file drag");
    assert_eq!(
        pending.entries.len(),
        2,
        "pressing an already selected row preserves multi-selection"
    );
    state.borrow_mut().drag.pressed = true;
    state.borrow_mut().prepare_native_drag();
    assert!(
        state.borrow().drag.pending.is_some(),
        "delivered button state keeps an internal drag alive between event-loop ticks"
    );
    let destination = ElementHandle::find_by_accessible_label(&ui, "Right files")
        .next()
        .unwrap()
        .absolute_position();
    let position = slint::LogicalPosition::new(destination.x + 80.0, destination.y + 80.0);
    state.borrow_mut().drag.cursor = Some(position);
    state.borrow_mut().drag.pending = None; // Drag-start notifications can be deferred by the native event loop.
    prime_release(&state, &ui);
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert!(
        state.borrow().app.prompt.is_some(),
        "a physical drag must reach the pane drop handler"
    );
    ui.window().dispatch_event(WindowEvent::PointerExited);
}

#[test]
fn dragging_and_abandoning_an_https_file_never_starts_a_download() {
    use std::net::TcpListener;
    let (_root, ui, state, left, _right) = setup();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    server.set_nonblocking(true).unwrap();
    let profile = crate::remote::Connection {
        service: "http".into(),
        endpoint: format!("http://{}", server.local_addr().unwrap()),
        ..Default::default()
    };
    crate::remote::register_profiles(std::slice::from_ref(&profile));
    let from = Location::Remote {
        connection: profile.id,
        path: "pub/data-repositories/WCVP".into(),
    };
    let entry = Entry {
        name: "wcvp.zip".into(),
        path: "pub/data-repositories/WCVP/wcvp.zip".into(),
        ..filesystem::entry_from_path(&left.join("alpha.txt")).unwrap()
    };
    let handle = Handle::new(Selection {
        from,
        entries: vec![entry],
    });
    let data = payload(&handle.token);
    assert!(
        !data.has_file_paths(),
        "a virtual payload contains metadata, not downloaded files"
    );
    assert_eq!(
        ui.global::<Transfers>()
            .invoke_can_drop(1, "pane".into(), data, DragAction::Copy),
        DragAction::Copy
    );
    ui.global::<Transfers>().invoke_begin(handle.token.into());
    {
        let mut a = state.borrow_mut();
        a.drag.outside = true;
        a.drag.pressed = true;
        for _ in 0..30 {
            a.prepare_native_drag();
        }
        assert!(
            a.drag.preparation.is_none(),
            "leaving the window without dropping must not authorize an export"
        );
        assert!(a.drag.ready.is_none());
        assert!(a.app.job.is_none());
        assert!(a.app.prompt.is_none());
        a.drag.pressed = false;
    }
    assert!(
        matches!(
            server.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        ),
        "no HTTP request may be sent by the gesture"
    );
}

#[test]
fn explicit_preparation_completion_does_not_start_an_unrelated_mouse_gesture() {
    let (_root, ui, state, left, _right) = setup();
    let (_updates, updates) = mpsc::channel();
    let (sender, result) = mpsc::channel();
    let source = Selection {
        from: Location::Archive {
            file: left.join("source.zip"),
            prefix: PathBuf::new(),
        },
        entries: vec![Entry {
            path: "alpha.txt".into(),
            ..filesystem::entry_from_path(&left.join("alpha.txt")).unwrap()
        }],
    };
    let mut a = state.borrow_mut();
    a.drag.pressed = true;
    a.drag.preparation = Some(Preparation {
        cancel: Arc::new(AtomicBool::new(false)),
        progress: Default::default(),
        updates,
        result,
    });
    sender
        .send(Ok(Ready {
            selection: source,
            paths: vec![left.join("alpha.txt")],
            prepared: std::time::Instant::now(),
        }))
        .unwrap();
    a.drag_tick(&ui);
    assert!(a.drag.preparation.is_none());
    assert!(
        a.drag.ready.is_none(),
        "preparing files must not launch a drag just because the mouse is held"
    );
    assert!(a.app.status.contains("Files ready to drag"));
}
