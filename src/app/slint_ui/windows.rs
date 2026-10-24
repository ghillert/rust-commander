//! One event loop and one operation runtime for independent desktop windows.
use super::*;
use anyhow::Result;
#[derive(Clone)]
pub(super) enum Request {
    New,
    Quit,
    ConfirmQuit,
    Cycle(bool),
    Raise(usize),
}
struct Runtime {
    queue: crate::queue::Queue,
    job: Option<Job>,
    history: Vec<crate::history::Record>,
    pending_save: Option<(String, String)>,
    settings: Settings,
    settings_valid: bool,
    #[cfg(target_os = "macos")]
    menu: Option<crate::native_menu::NativeMenu>,
}
impl Runtime {
    fn swap(&mut self, app: &mut Commander) {
        std::mem::swap(&mut self.queue, &mut app.queue);
        std::mem::swap(&mut self.job, &mut app.job);
        std::mem::swap(&mut self.history, &mut app.history);
        std::mem::swap(&mut self.pending_save, &mut app.pending_save);
        #[cfg(target_os = "macos")]
        std::mem::swap(&mut self.menu, &mut app.native_menu);
    }
    fn load_globals(&self, app: &mut Commander) {
        app.settings_valid &= self.settings_valid;
        app.saved_windows = self.settings.window_sessions.clone();
        let layout = app.preferences.layout.clone();
        let single_pane = app.preferences.single_pane;
        app.preferences = self.settings.preferences.clone();
        app.preferences.layout = layout;
        app.preferences.single_pane = single_pane;
        app.bookmarks = self.settings.bookmarks.clone();
        app.connections = self.settings.connections.clone();
        app.workspaces = self.settings.workspaces.clone();
        app.recent_locations = self.settings.recent_locations.clone();
        app.recent_files = self.settings.recent_files.clone();
        app.imported_commands = self.settings.imported_commands.clone();
        app.show_hidden = self.settings.show_hidden;
    }
}
struct Session {
    id: usize,
    title: String,
    ui: CommanderWindow,
    state: Rc<RefCell<Adapter>>,
}
thread_local! {
    static RUNTIME:RefCell<Option<Runtime>>=const {RefCell::new(None)};
    static SESSIONS:RefCell<Vec<Session>>=const {RefCell::new(Vec::new())};
    static NEXT_ID:std::cell::Cell<usize>=const {std::cell::Cell::new(1)};
}
/// UI callbacks are serialized by the event loop. Swapping preserves existing
/// controller guards while giving every window the same queue/job/history.
pub(super) fn with_state<T>(state: &Rc<RefCell<Adapter>>, f: impl FnOnce(&mut Adapter) -> T) -> T {
    RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        let mut adapter = state.borrow_mut();
        if let Some(runtime) = runtime.as_mut() {
            runtime.load_globals(&mut adapter.app);
            runtime.swap(&mut adapter.app);
            let result = f(&mut adapter);
            runtime.settings = adapter.app.snapshot_settings();
            runtime.swap(&mut adapter.app);
            result
        } else {
            f(&mut adapter)
        }
    })
}
#[cfg(target_os = "linux")]
pub(super) fn x11_windows() -> Vec<u32> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .iter()
            .filter_map(|session| {
                match session
                    .ui
                    .window()
                    .window_handle()
                    .window_handle()
                    .ok()?
                    .as_raw()
                {
                    RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok(),
                    RawWindowHandle::Xcb(h) => Some(h.window.get()),
                    _ => None,
                }
            })
            .collect()
    })
}
pub(super) fn entries() -> Vec<(usize, String)> {
    SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .iter()
            .map(|s| (s.id, format!("Window {} · {}", s.id, s.title)))
            .collect()
    })
}
pub(super) fn open(paths: &[PathBuf], loaded: Result<Settings>) -> Result<CommanderWindow> {
    anyhow::ensure!(
        SESSIONS.with(|s| s.borrow().len()) < 16,
        "At most sixteen windows can be open"
    );
    let ui = CommanderWindow::new()?;
    let mut adapter = Adapter::new(paths, loaded);
    let id = NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    adapter.app.window_id = id;
    RUNTIME.with(|runtime| {
        if runtime.borrow().is_none() {
            let settings = adapter.app.snapshot_settings();
            let mut value = Runtime {
                queue: Default::default(),
                job: None,
                history: Vec::new(),
                pending_save: None,
                settings,
                settings_valid: adapter.app.settings_valid,
                #[cfg(target_os = "macos")]
                menu: None,
            };
            value.swap(&mut adapter.app);
            *runtime.borrow_mut() = Some(value);
        } else {
            adapter.app.queue = Default::default();
            adapter.app.history.clear();
        }
    });
    let state = Rc::new(RefCell::new(adapter));
    wire(&ui, state.clone());
    with_state(&state, |a| a.sync(&ui));
    ui.show()?;
    ui.set_window_title(
        format!(
            "Rust Commander — {} — Window {id}",
            parity::tab_label(state.borrow().app.tab())
        )
        .into(),
    );
    ui.invoke_focus_files(state.borrow().app.active as i32);
    SESSIONS.with(|sessions| {
        sessions.borrow_mut().push(Session {
            id,
            title: parity::tab_label(state.borrow().app.tab()),
            ui: ui.clone_strong(),
            state: state.clone(),
        })
    });
    Ok(ui)
}
pub(super) fn restore(paths: &[PathBuf], loaded: Result<Settings>) -> Result<CommanderWindow> {
    let saved = loaded
        .as_ref()
        .ok()
        .filter(|_| paths.is_empty())
        .map(|s| s.window_sessions.clone())
        .unwrap_or_default();
    if saved.is_empty() {
        return open(paths, loaded);
    }
    let settings = loaded?;
    anyhow::ensure!(
        saved.len() <= 16,
        "The saved session exceeds sixteen windows; settings have been preserved"
    );
    let mut first = None;
    for session in saved.iter().take(16) {
        let ui = open(&[], Ok(session.settings(&settings)))?;
        if session.width.is_finite() && session.height.is_finite() {
            ui.window().set_size(slint::LogicalSize::new(
                session.width.clamp(900.0, 2400.0),
                session.height.clamp(600.0, 1600.0),
            ));
        }
        if let Some(position) = session.position {
            use slint::winit_030::WinitWindowAccessor;
            let visible = ui
                .window()
                .with_winit_window(|w| {
                    w.available_monitors().any(|monitor| {
                        let p = monitor.position();
                        let size = monitor.size();
                        position[0] >= p.x
                            && position[1] >= p.y
                            && position[0] < (p.x + size.width as i32 - 100)
                            && position[1] < (p.y + size.height as i32 - 40)
                    })
                })
                .unwrap_or(false);
            if visible {
                ui.window()
                    .set_position(slint::PhysicalPosition::new(position[0], position[1]));
            }
        }
        if first.is_none() {
            first = Some(ui);
        }
    }
    Ok(first.unwrap())
}
fn persist_sessions() {
    let sessions = SESSIONS.with(|s| {
        s.borrow()
            .iter()
            .map(|s| (s.ui.clone_strong(), s.state.clone()))
            .collect::<Vec<_>>()
    });
    if sessions.is_empty() {
        return;
    }
    let all_exiting = sessions.iter().all(|(_, s)| s.borrow().app.exiting);
    let windows = sessions
        .iter()
        .filter(|(_, s)| all_exiting || !s.borrow().app.exiting)
        .map(|(ui, state)| {
            let app = &state.borrow().app;
            let settings = app.snapshot_settings();
            let size = ui.window().size();
            let scale = ui.window().scale_factor();
            crate::settings::WindowSession {
                workspace: crate::workspaces::Workspace {
                    name: String::new(),
                    tabs: settings.tabs,
                    tab_states: settings.tab_states,
                    active_tabs: settings.active_tabs,
                    active_pane: settings.active_pane,
                    preferences: settings.preferences,
                },
                position: Some([ui.window().position().x, ui.window().position().y]),
                width: size.width as f32 / scale,
                height: size.height as f32 / scale,
            }
        })
        .collect::<Vec<_>>();
    for (_, state) in &sessions {
        state.borrow_mut().app.saved_windows = windows.clone();
    }
    RUNTIME.with(|r| {
        if let Some(r) = r.borrow_mut().as_mut() {
            let changed = serde_json::to_vec(&r.settings.window_sessions).ok()
                != serde_json::to_vec(&windows).ok();
            r.settings.window_sessions = windows;
            if changed
                && r.settings_valid
                && let Err(e) = settings::save(&r.settings)
            {
                eprintln!("Could not save window session: {e:#}");
            }
        }
    });
}
fn raise(id: usize) {
    use slint::winit_030::WinitWindowAccessor;
    SESSIONS.with(|sessions| {
        if let Some(s) = sessions.borrow().iter().find(|s| s.id == id) {
            s.ui.window().with_winit_window(|w| {
                w.set_minimized(false);
                w.focus_window();
            });
        }
    });
}
pub(super) fn tick() {
    let sessions = SESSIONS.with(|s| {
        s.borrow()
            .iter()
            .map(|s| (s.id, s.ui.clone_strong(), s.state.clone()))
            .collect::<Vec<_>>()
    });
    for (_, ui, state) in &sessions {
        with_state(state, |a| a.tick(ui));
        drag::dispatch(state, ui);
        dispatch_native_drag(state, ui);
    }
    SESSIONS.with(|all| {
        for session in all.borrow_mut().iter_mut() {
            let title = parity::tab_label(session.state.borrow().app.tab());
            if session.title != title {
                session.title = title;
                session.ui.set_window_title(
                    format!("Rust Commander — {} — Window {}", session.title, session.id).into(),
                );
            }
        }
    });
    let mut requests = Vec::new();
    for (id, _, state) in &sessions {
        if let Some(request) = state.borrow_mut().extra.window_request.take() {
            requests.push((*id, request));
        }
    }
    for (source, request) in requests {
        match request {
            Request::New => {
                let settings = RUNTIME.with(|r| r.borrow().as_ref().map(|r| r.settings.clone()));
                if let Some(settings) = settings
                    && let Err(error) = open(&[], Ok(settings))
                    && let Some((_, ui, state)) = sessions.iter().find(|(id, _, _)| *id == source)
                {
                    with_state(state, |a| {
                        a.app.fail(error);
                        a.sync(ui);
                    });
                }
            }
            Request::Cycle(backwards) => {
                let index = sessions
                    .iter()
                    .position(|(id, _, _)| *id == source)
                    .unwrap_or(0);
                if !sessions.is_empty() {
                    raise(
                        sessions[(index + if backwards { sessions.len() - 1 } else { 1 })
                            % sessions.len()]
                        .0,
                    );
                }
            }
            Request::Raise(id) => raise(id),
            Request::Quit => {
                let job = RUNTIME.with(|r| r.borrow().as_ref().is_some_and(|r| r.job.is_some()));
                let dirty = sessions.iter().any(|(_, _, s)| {
                    s.borrow().app.editor.as_ref().is_some_and(Editor::dirty)
                        || s.borrow().enhancements.external.is_some()
                        || s.borrow().drag.preparation.is_some()
                });
                if job || dirty {
                    if let Some((_, ui, state)) = sessions.iter().find(|(id, _, _)| *id == source) {
                        with_state(state, |a| {
                            a.extra.quit_all = true;
                            a.app.confirm_exit = true;
                            a.sync(ui);
                        });
                        raise(source);
                    }
                } else {
                    for (_, _, state) in &sessions {
                        with_state(state, |a| a.app.exiting = true);
                    }
                }
            }
            Request::ConfirmQuit => {
                for (_, _, state) in &sessions {
                    with_state(state, |a| {
                        if let Some(job) = &a.app.job {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                        if let Some(preparation) = &a.drag.preparation {
                            preparation.cancel.store(true, Ordering::Relaxed);
                        }
                        a.app.editor = None;
                        a.app.confirm_exit = false;
                        a.app.exiting = true;
                    });
                }
            }
        }
    }
    persist_sessions();
    let mut closed = Vec::new();
    for (id, ui, state) in sessions {
        let done = with_state(&state, |a| {
            a.app.exiting
                && a.app
                    .job
                    .as_ref()
                    .is_none_or(|j| j.owner != a.app.window_id)
                && a.drag.preparation.is_none()
        });
        if done {
            with_state(&state, |a| a.app.save_settings());
            let _ = ui.hide();
            closed.push(id);
        }
    }
    let empty = SESSIONS.with(|s| {
        s.borrow_mut().retain(|s| !closed.contains(&s.id));
        s.borrow().is_empty()
    });
    if empty {
        let _ = slint::quit_event_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            SESSIONS.with(|s| {
                for session in s.borrow().iter() {
                    session.state.borrow_mut().app.settings_valid = false;
                }
                s.borrow_mut().clear();
            });
            RUNTIME.with(|r| *r.borrow_mut() = None);
        }
    }
    #[test]
    fn windows_share_one_queue_and_preferences_while_preserving_independent_tabs() {
        i_slint_backend_testing::init_no_event_loop();
        let _reset = Reset;
        let root = tempfile::tempdir().unwrap();
        let left = root.path().join("first");
        let right = root.path().join("second");
        std::fs::create_dir(&left).unwrap();
        std::fs::create_dir(&right).unwrap();
        open(std::slice::from_ref(&left), Ok(Settings::default())).unwrap();
        RUNTIME.with(|r| {
            let mut r = r.borrow_mut();
            let r = r.as_mut().unwrap();
            r.settings_valid = false;
            r.queue = Default::default();
            r.history.clear();
        });
        open(std::slice::from_ref(&right), Ok(Settings::default())).unwrap();
        let states = SESSIONS.with(|s| {
            s.borrow()
                .iter()
                .map(|s| (s.id, s.state.clone()))
                .collect::<Vec<_>>()
        });
        with_state(&states[0].1, |a| {
            a.app.preferences.font_size = 19.0;
            a.app.preferences.file_icon_source = crate::preferences::FileIconSource::BuiltIn;
            a.app
                .queue
                .push_back(Operation::Mkfile(root.path().join("never-run")));
        });
        with_state(&states[1].1, |a| {
            assert_eq!(a.app.preferences.font_size, 19.0);
            assert_eq!(
                a.app.preferences.file_icon_source,
                crate::preferences::FileIconSource::BuiltIn
            );
            assert_eq!(a.app.queue.len(), 1);
            assert_eq!(a.app.tab().location, Location::Local(right.clone()));
        });
        with_state(&states[0].1, |a| {
            assert_eq!(a.app.tab().location, Location::Local(left))
        });
        assert_eq!(entries().len(), 2);
        assert!(!root.path().join("never-run").exists());
    }
    #[test]
    fn closing_another_window_does_not_cancel_the_job_owner() {
        i_slint_backend_testing::init_no_event_loop();
        let _reset = Reset;
        let root = tempfile::tempdir().unwrap();
        open(&[root.path().into()], Ok(Settings::default())).unwrap();
        RUNTIME.with(|r| {
            let mut r = r.borrow_mut();
            let r = r.as_mut().unwrap();
            r.settings_valid = false;
            r.queue = Default::default();
            r.history.clear();
        });
        open(&[root.path().into()], Ok(Settings::default())).unwrap();
        let states = SESSIONS.with(|s| {
            s.borrow()
                .iter()
                .map(|s| (s.id, s.state.clone()))
                .collect::<Vec<_>>()
        });
        let (_sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        RUNTIME.with(|r| {
            r.borrow_mut().as_mut().unwrap().job = Some(Job {
                owner: states[0].0,
                foreground: false,
                label: "Fixture job".into(),
                receiver,
                control: None,
                cancel: cancel.clone(),
                progress: Default::default(),
            })
        });
        with_state(&states[1].1, |a| {
            a.request_close();
            assert!(a.app.exiting);
            assert!(!a.app.confirm_exit);
        });
        assert!(!cancel.load(Ordering::Relaxed));
        with_state(&states[0].1, |a| {
            a.request_close();
            assert!(a.app.confirm_exit);
            assert!(!a.app.exiting);
        });
        assert!(!cancel.load(Ordering::Relaxed));
    }
    #[test]
    fn all_windows_restore_independent_locations_panes_and_layouts() {
        i_slint_backend_testing::init_no_event_loop();
        let _reset = Reset;
        let root = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        for (index, ratio) in [0.3, 0.7].into_iter().enumerate() {
            let path = root.path().join(format!("window-{index}"));
            std::fs::create_dir(&path).unwrap();
            let mut preferences = settings.preferences.clone();
            preferences.layout.ratio = ratio;
            settings
                .window_sessions
                .push(crate::settings::WindowSession {
                    workspace: crate::workspaces::Workspace {
                        name: String::new(),
                        tabs: [
                            vec![Location::Local(path.clone())],
                            vec![Location::Local(path)],
                        ],
                        tab_states: Default::default(),
                        active_tabs: [0, 0],
                        active_pane: index,
                        preferences,
                    },
                    width: 1200.0,
                    height: 800.0,
                    position: None,
                });
        }
        restore(&[], Ok(settings)).unwrap();
        RUNTIME.with(|r| r.borrow_mut().as_mut().unwrap().settings_valid = false);
        let states = SESSIONS.with(|s| {
            s.borrow()
                .iter()
                .map(|s| s.state.clone())
                .collect::<Vec<_>>()
        });
        assert_eq!(states.len(), 2);
        with_state(&states[0], |a| {
            assert_eq!(a.app.active, 0);
            assert_eq!(a.app.preferences.layout.ratio, 0.3);
            a.app.settings_valid = false;
        });
        with_state(&states[1], |a| {
            assert_eq!(a.app.active, 1);
            assert_eq!(a.app.preferences.layout.ratio, 0.7);
            a.app.settings_valid = false;
        });
        persist_sessions();
        RUNTIME.with(|r| {
            assert_eq!(
                r.borrow().as_ref().unwrap().settings.window_sessions.len(),
                2
            )
        });
    }
}
