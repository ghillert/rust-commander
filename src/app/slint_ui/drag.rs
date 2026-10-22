//! UI drag payload ownership and destination resolution; no filesystem I/O on hover.
use super::*;
use crate::drag_drop::Selection;
use slint::private_unstable_api::re_exports::DragAction;
use std::{collections::BTreeMap, rc::Weak};

pub(super) fn trace(event: &str) {
    #[cfg(feature = "screenshot")]
    if std::env::var_os("RUST_COMMANDER_DND_TRACE").is_some() {
        eprintln!("Drag trace: {event}");
    }
    let _ = event;
}
thread_local! {
    static SOURCES: RefCell<BTreeMap<String,Weak<Selection>>> = const {RefCell::new(BTreeMap::new())};
}
pub(super) struct Handle {
    pub token: String,
    _selection: Rc<Selection>,
}
impl Handle {
    pub fn new(selection: Selection) -> Self {
        let token = format!("{:032x}", rand::random::<u128>());
        let selection = Rc::new(selection);
        SOURCES.with(|sources| {
            let mut sources = sources.borrow_mut();
            if sources.len().is_multiple_of(512) {
                sources.retain(|_, source| source.strong_count() > 0);
            }
            sources.insert(token.clone(), Rc::downgrade(&selection));
        });
        Self {
            token,
            _selection: selection,
        }
    }
}
pub(super) fn payload(token: &str) -> slint::DataTransfer {
    let mut data = slint::DataTransfer::default();
    if let Some(selection) =
        SOURCES.with(|sources| sources.borrow().get(token).and_then(Weak::upgrade))
    {
        if selection.from.local().is_some() {
            data.set_file_paths(selection.entries.iter().map(|e| &e.path));
        }
        data.set_user_data(selection);
    }
    data
}
pub(super) fn selection(data: &slint::DataTransfer) -> Option<Rc<Selection>> {
    data.user_data()?.downcast::<Selection>().ok()
}
type Hits = Rc<RefCell<BTreeMap<(i32, String, i32), ()>>>;
type Targets = Rc<RefCell<BTreeMap<(i32, String), Location>>>;
#[derive(Default)]
pub(super) struct State {
    pub rows: [Vec<Handle>; 2],
    pub selected: [Option<Handle>; 2],
    pub pending: Option<Rc<Selection>>,
    pub hits: Hits,
    pub hovered: Vec<PathBuf>,
    pub preparation: Option<Preparation>,
    pub ready: Option<Ready>,
    pub position: Option<(f32, f32)>,
    pub last_hit: Option<(i32, String)>,
    pub drop_action: Option<DragAction>,
    pub finished: Option<Receiver<()>>,
    #[cfg(target_os = "linux")]
    pub linux: Option<crate::native_drag::Linux>,
    pub move_modifier: bool,
    pub outside: bool,
    pub pressed: bool,
    pub cursor: Option<slint::LogicalPosition>,
    pub prepare_requested: bool,
    pub deferred: Option<Selection>,
    pub incoming: Option<slint::DataTransfer>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    native_target: Option<NativeTarget>,
    targets: Targets,
}
impl Adapter {
    pub(super) fn drop_destination(&self, side: usize, target: &str) -> Option<Location> {
        let pane = self.app.panes.get(side)?;
        if target == "pane" {
            return Some(pane.tab().location.clone());
        }
        let (kind, id) = target.split_once(':')?;
        match kind {
            "folder" => {
                let entry = self.entry(side, id)?;
                if entry.is_dir {
                    Some(crate::directory_tree::at(&pane.tab().location, &entry.path))
                } else if pane.tab().location.local().is_some()
                    && crate::archive_tools::editable(&entry.path)
                {
                    Some(Location::Archive {
                        file: entry.path,
                        prefix: PathBuf::new(),
                    })
                } else {
                    None
                }
            }
            "tab" => Some(pane.tabs.get(id.parse::<usize>().ok()?)?.location.clone()),
            "breadcrumb" => self.breadcrumbs[side]
                .get(id.parse::<usize>().ok()?)
                .cloned(),
            "place" => self
                .places
                .iter()
                .find(|(key, _)| key == id)
                .map(|(_, location)| location.clone()),
            "tree" => self.enhancements.drop_location(id.parse().ok()?),
            _ => None,
        }
    }
    pub(super) fn can_drop(
        &self,
        side: i32,
        target: &str,
        data: &slint::DataTransfer,
        proposed: DragAction,
    ) -> DragAction {
        if self.blocked() || self.app.job.is_some() || !(0..2).contains(&side) {
            return DragAction::None;
        }
        let Some(to) = self.drop_destination(side as usize, target) else {
            return DragAction::None;
        };
        accepted(&to, data, proposed)
    }
    fn native_targets(&self) -> BTreeMap<(i32, String), Location> {
        let mut targets = BTreeMap::new();
        if self.blocked() || self.app.job.is_some() {
            return targets;
        }
        for side in 0..2 {
            let mut ids = vec!["pane".to_owned()];
            ids.extend(
                self.row_cache[side]
                    .iter()
                    .filter(|r| r.directory || crate::archive_tools::editable(&r.path))
                    .map(|r| format!("folder:{}", r.id)),
            );
            ids.extend((0..self.app.panes[side].tabs.len()).map(|i| format!("tab:{i}")));
            ids.extend((0..self.breadcrumbs[side].len()).map(|i| format!("breadcrumb:{i}")));
            ids.extend(self.places.iter().map(|(id, _)| format!("place:{id}")));
            ids.extend((0..self.enhancements.drop_count()).map(|i| format!("tree:{i}")));
            for id in ids {
                if let Some(to) = self.drop_destination(side, &id) {
                    targets.insert((side as i32, id), to);
                }
            }
        }
        targets
    }
    pub(super) fn drop_to(
        &mut self,
        side: i32,
        target: &str,
        data: slint::DataTransfer,
        action: DragAction,
    ) -> bool {
        if self.can_drop(side, target, &data, action) == DragAction::None {
            return false;
        }
        trace("drop confirmation");
        let Some(to) = self.drop_destination(side as usize, target) else {
            return false;
        };
        let selection = match selection(&data) {
            Some(selection) => Ok((*selection).clone()),
            None => data
                .file_paths()
                .map_err(anyhow::Error::from)
                .and_then(|paths| {
                    crate::drag_drop::Selection::local(
                        &paths.map(Path::to_owned).collect::<Vec<_>>(),
                    )
                }),
        };
        if let Some(index) = target
            .strip_prefix("tab:")
            .and_then(|i| i.parse::<usize>().ok())
        {
            self.app.panes[side as usize].active = index;
        }
        let result = selection.and_then(|selection| {
            let moving = action == DragAction::Move && selection.can_move();
            self.app
                .drop_selection(side as usize, selection, to, moving)
        });
        if let Err(error) = result {
            self.app.fail(error);
            return false;
        }
        true
    }
    pub(super) fn hover_drop(&mut self, side: i32, target: &str) {
        if self.blocked()
            || self.app.job.is_some()
            || !self.pending_drops.is_empty()
            || !(0..2).contains(&side)
        {
            return;
        }
        if let Some(index) = target
            .strip_prefix("tab:")
            .and_then(|id| id.parse::<usize>().ok())
        {
            if index < self.app.panes[side as usize].tabs.len() {
                self.app.panes[side as usize].active = index;
            }
        } else if let Some(index) = target
            .strip_prefix("tree:")
            .and_then(|id| id.parse::<usize>().ok())
        {
            self.expand_drop_tree(index);
        } else if (target.starts_with("folder:")
            || target.starts_with("breadcrumb:")
            || target.starts_with("place:"))
            && let Some(location) = self.drop_destination(side as usize, target)
        {
            self.app.active = side as usize;
            // Locked tabs open a new tab through the normal navigation path.
            self.app.tab_mut().navigate(location, &self.ctx);
        }
    }
}
fn accepted(to: &Location, data: &slint::DataTransfer, proposed: DragAction) -> DragAction {
    if !crate::drag_drop::writable(to) {
        return DragAction::None;
    }
    if let Some(selection) = selection(data) {
        let moving = proposed == DragAction::Move && selection.can_move();
        if selection.check_target(to, moving).is_err() {
            return DragAction::None;
        }
        if moving {
            DragAction::Move
        } else {
            DragAction::Copy
        }
    } else if data.has_file_paths() {
        DragAction::Copy
    } else {
        DragAction::None
    }
}
pub(super) fn wire(ui: &CommanderWindow, state: Rc<RefCell<Adapter>>) {
    ui.global::<Transfers>().on_payload(|token| payload(&token));
    ui.global::<Transfers>()
        .on_accepts(|data| selection(&data).is_some() || data.has_file_paths());
    ui.global::<Transfers>()
        .on_movable(|data| selection(&data).is_some_and(|selection| selection.can_move()));
    let checked = state.clone();
    let targets = state.borrow().drag.targets.clone();
    ui.global::<Transfers>()
        .on_can_drop(move |side, target, data, action| {
            if let Ok(a) = checked.try_borrow() {
                a.can_drop(side, &target, &data, action)
            } else {
                targets
                    .borrow()
                    .get(&(side, target.to_string()))
                    .map(|to| accepted(to, &data, action))
                    .unwrap_or(DragAction::None)
            }
        });
    let dropped = state.clone();
    let weak = ui.as_weak();
    ui.global::<Transfers>()
        .on_drop(move |side, target, data, action| {
            windows::with_state(&dropped, |a| {
                let result = a.drop_to(side, &target, data, action);
                if let Some(ui) = weak.upgrade() {
                    a.sync(&ui);
                }
                result
            })
        });
    let hover = state.clone();
    let weak = ui.as_weak();
    ui.global::<Transfers>().on_hover(move |side, target| {
        windows::with_state(&hover, |a| {
            a.hover_drop(side, &target);
            if let Some(ui) = weak.upgrade() {
                a.sync(&ui);
            }
        });
    });
    let begin = state.clone();
    ui.global::<Transfers>().on_begin(move |token| {
        trace("threshold crossed");
        let data = payload(&token);
        if let Some(selection) = selection(&data) {
            begin.borrow_mut().drag.pending = Some(selection);
        }
    });
    let hits = state.borrow().drag.hits.clone();
    ui.global::<Transfers>()
        .on_hit(move |side, target, priority, over| {
            let key = (side, target.to_string(), priority);
            if over {
                hits.borrow_mut().insert(key, ());
            } else {
                hits.borrow_mut().remove(&key);
            }
        });
}

#[derive(Clone)]
pub(super) struct Ready {
    selection: Selection,
    paths: Vec<PathBuf>,
    prepared: std::time::Instant,
}
thread_local! {static ACTIVE:RefCell<Option<(Ready,std::time::Instant)>>=const{RefCell::new(None)};}
thread_local! {static READY:RefCell<std::collections::VecDeque<Ready>>=const{RefCell::new(std::collections::VecDeque::new())};}
#[cfg(any(target_os = "macos", target_os = "windows"))]
struct NativeTarget {
    _registration: ::drag::DropTarget,
    events: Receiver<::drag::DropEvent>,
}
struct PrivateOffer {
    session: Arc<PromiseSession>,
    until: std::time::Instant,
}
thread_local! { static PRIVATE: RefCell<BTreeMap<String,PrivateOffer>> = const {RefCell::new(BTreeMap::new())}; }
fn private_payload(token: &str, claim: bool) -> Option<slint::DataTransfer> {
    let session = PRIVATE.with(|offers| {
        let mut offers = offers.borrow_mut();
        offers
            .retain(|_, offer| offer.until > std::time::Instant::now() && offer.session.routable());
        let session = offers.get(token)?.session.clone();
        if claim {
            offers.remove(token);
        }
        Some(session)
    })?;
    let mut data = slint::DataTransfer::default();
    data.set_user_data(Rc::new(session.selection.clone()));
    if claim {
        session.internal_drop();
    }
    Some(data)
}
fn marker(token: &str) -> PathBuf {
    std::env::temp_dir().join(format!(".rust-commander-offer-{token}"))
}
fn marker_token(paths: &[PathBuf]) -> Option<String> {
    if paths.len() != 1 {
        return None;
    }
    let token = paths[0]
        .file_name()?
        .to_str()?
        .strip_prefix(".rust-commander-offer-")?;
    (token.len() == 32 && token.bytes().all(|c| c.is_ascii_hexdigit()) && paths[0] == marker(token))
        .then(|| token.to_owned())
}
pub(super) fn dropped_payload(paths: Vec<PathBuf>) -> slint::DataTransfer {
    if let Some(data) = marker_token(&paths).and_then(|token| private_payload(&token, true)) {
        return data;
    }
    native_payload(paths)
}
pub(super) struct Preparation {
    pub cancel: Arc<AtomicBool>,
    pub progress: Progress,
    updates: Receiver<Progress>,
    result: Receiver<Result<Ready, String>>,
    pub external: Option<Arc<PromiseSession>>,
    pub waiting_since: std::time::Instant,
}
fn same(selection: &Selection, other: &Selection) -> bool {
    selection.from == other.from
        && selection.entries.len() == other.entries.len()
        && selection
            .entries
            .iter()
            .zip(&other.entries)
            .all(|(a, b)| a.path == b.path && a.size == b.size && a.modified == b.modified)
}
fn remember(ready: Ready) {
    READY.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.push_front(ready);
        while cache.len() > 64
            || cache
                .iter()
                .map(|r| r.selection.entries.len())
                .sum::<usize>()
                > 100_000
        {
            cache.pop_back();
        }
    });
}
pub(super) fn native_payload(paths: Vec<PathBuf>) -> slint::DataTransfer {
    if let Some(data) = marker_token(&paths).and_then(|token| private_payload(&token, false)) {
        return data;
    }
    let mut data = slint::DataTransfer::default();
    data.set_file_paths(&paths);
    let selection = ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .filter(|(ready, until)| {
                std::time::Instant::now() < *until
                    && ready.paths.len() == paths.len()
                    && ready.paths.iter().all(|p| paths.contains(p))
            })
            .map(|(ready, _)| ready.selection.clone())
    });
    if let Some(selection) = selection {
        data.set_user_data(Rc::new(selection));
    }
    data
}
use crate::drag_drop::promises::{PromiseSession, export_cache, retain_export};
fn discard_export(ready: Ready) {
    if ready.selection.from.local().is_some() {
        return;
    }
    let directory = ready
        .paths
        .first()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .map(Path::to_owned);
    if let Some(directory) = directory.filter(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("export-"))
            && path.parent().and_then(Path::file_name) == Some(std::ffi::OsStr::new("drag-exports"))
    }) {
        let _ = std::thread::Builder::new()
            .name("rust-commander-drag-cleanup".into())
            .spawn(move || {
                let _ = std::fs::remove_dir_all(directory);
            });
    }
}
impl State {
    pub(super) fn buttons(&self) -> (bool, bool) {
        #[cfg(target_os = "linux")]
        {
            (
                self.pressed || self.linux.as_ref().is_some_and(|backend| backend.pressed()),
                self.move_modifier,
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            let (pressed, moving) = crate::native_files::drag_buttons();
            (self.pressed || pressed, moving || self.move_modifier)
        }
    }
}
impl Adapter {
    pub(super) fn drag_tick(&mut self, ui: &CommanderWindow) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            use raw_window_handle::HasWindowHandle;
            if self.drag.native_target.is_none()
                && let Ok(handle) = ui.window().window_handle().window_handle()
            {
                let (sender, events) = mpsc::channel();
                let ctx = self.ctx.clone();
                if let Ok(registration) = ::drag::attach_drop_target(
                    &crate::native_files::Window(handle.as_raw()),
                    Arc::new(move |event| {
                        let _ = sender.send(event);
                        ctx.request_repaint();
                    }),
                ) {
                    self.drag.native_target = Some(NativeTarget {
                        _registration: registration,
                        events,
                    });
                }
            }
            let events = self
                .drag
                .native_target
                .as_ref()
                .map(|target| target.events.try_iter().collect::<Vec<_>>())
                .unwrap_or_default();
            for event in events {
                match event {
                    ::drag::DropEvent::Leave => {
                        if self.pending_drops.is_empty() {
                            self.drag.hovered.clear();
                            self.drag.incoming = None;
                        }
                    }
                    ::drag::DropEvent::Hover { x, y, token, paths } => {
                        self.receive_native(ui, x, y, token, paths, false)
                    }
                    ::drag::DropEvent::Drop { x, y, token, paths } => {
                        self.receive_native(ui, x, y, token, paths, true)
                    }
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::HasWindowHandle;
            if self.drag.linux.is_none() && ui.window().window_handle().window_handle().is_ok() {
                let ctx = self.ctx.clone();
                self.drag.linux =
                    crate::native_drag::Linux::attach(ui.window().window_handle(), move || {
                        ctx.request_repaint()
                    })
                    .ok();
            }
            if let Some(backend) = &mut self.drag.linux {
                for event in backend.poll() {
                    match event {
                        crate::native_drag::Event::Internal(x, y, token, dropped) => {
                            self.receive_native(ui, x, y, Some(token), Vec::new(), dropped)
                        }
                        crate::native_drag::Event::Hover(x, y) => {
                            self.drag.position = Some((x, y));
                            if self.drag.hovered.is_empty() {
                                self.drag
                                    .hovered
                                    .push(std::env::temp_dir().join(".rust-commander-drag-offer"));
                            }
                        }
                        crate::native_drag::Event::Leave => {
                            if self.pending_drops.is_empty() {
                                self.drag.hovered.clear();
                            }
                        }
                        crate::native_drag::Event::Drop(x, y, paths) => {
                            self.drag.position = Some((x, y));
                            self.drag.drop_action = Some(if self.drag.move_modifier {
                                DragAction::Move
                            } else {
                                DragAction::Copy
                            });
                            self.pending_drops = paths;
                            self.pending_drop_side = usize::try_from(ui.invoke_drop_side(x, y))
                                .ok()
                                .filter(|side| *side < 2)
                                .or(Some(self.app.active));
                            self.drop_deadline =
                                Some(std::time::Instant::now() + Duration::from_millis(80));
                        }
                    }
                }
            }
        }

        if let Some(preparation) = &mut self.drag.preparation {
            if let Some(session) = &preparation.external {
                if preparation.cancel.load(Ordering::Relaxed) {
                    session.abort("External drop cancelled.");
                } else if session.idle()
                    && preparation.waiting_since.elapsed() > Duration::from_secs(120)
                {
                    session.abort("The receiving application did not request all promised files. Try preparing files explicitly.");
                } else if !session.idle() {
                    preparation.waiting_since = std::time::Instant::now();
                }
            }
            while let Ok(progress) = preparation.updates.try_recv() {
                preparation.waiting_since = std::time::Instant::now();
                preparation.progress = progress;
            }
            if let Ok(result) = preparation.result.try_recv() {
                let cancelled = preparation.cancel.load(Ordering::Relaxed)
                    && (preparation.external.is_none()
                        || matches!(&result, Err(error) if error.to_ascii_lowercase().contains("cancelled")));
                let external = preparation.external.is_some();
                self.drag.preparation = None;
                if cancelled {
                    if let Ok(ready) = result {
                        discard_export(ready);
                    }
                    self.app.status = if external {
                        "External drag cancelled."
                    } else {
                        "Drag preparation cancelled."
                    }
                    .into();
                    return;
                }
                match result {
                    Ok(ready) => {
                        if !ready.paths.is_empty() {
                            remember(ready.clone());
                        }
                        // Preparation is explicit and completes independently of
                        // any mouse button held elsewhere in the window.
                        self.app.status = if external {
                            "External drop completed."
                        } else {
                            "Files ready to drag. Drag the selection to another application."
                        }
                        .into();
                    }
                    Err(error) => self.app.fail(error),
                }
            }
        }
        if self.drag.finished.as_ref().is_some_and(|receiver| {
            matches!(
                receiver.try_recv(),
                Ok(()) | Err(mpsc::TryRecvError::Disconnected)
            )
        }) {
            self.drag.finished = None;
            self.drag.pressed = false;
            // Destination events may arrive just after native completion.
            ACTIVE.with(|active| {
                if let Some((_, until)) = active.borrow_mut().as_mut() {
                    *until = std::time::Instant::now() + Duration::from_secs(1);
                }
            });
        }
        let active = !self.drag.hovered.is_empty() || !self.pending_drops.is_empty();
        let global = ui.global::<Transfers>();
        if active {
            *self.drag.targets.borrow_mut() = self.native_targets();
            #[cfg(not(target_os = "linux"))]
            use raw_window_handle::HasWindowHandle;
            if self.pending_drops.is_empty() {
                #[cfg(not(target_os = "linux"))]
                if let Ok(handle) = ui.window().window_handle().window_handle() {
                    self.drag.position = crate::native_files::drop_position(
                        handle.as_raw(),
                        f64::from(ui.window().scale_factor()),
                    )
                    .or(self.drag.position);
                }
                #[cfg(target_os = "linux")]
                if let Some(backend) = &self.drag.linux {
                    self.drag.position = backend
                        .pointer(f64::from(ui.window().scale_factor()))
                        .or(self.drag.position);
                }
            }
            if let Some((x, y)) = self.drag.position {
                global.set_native_x(x);
                global.set_native_y(y);
                let paths = if self.pending_drops.is_empty() {
                    self.drag.hovered.clone()
                } else {
                    self.pending_drops.clone()
                };
                global.set_native_data(
                    self.drag
                        .incoming
                        .clone()
                        .unwrap_or_else(|| native_payload(paths)),
                );
                global.set_native_action(if self.drag.buttons().1 {
                    DragAction::Move
                } else {
                    DragAction::Copy
                });
                self.drag.last_hit = self
                    .drag
                    .hits
                    .borrow()
                    .keys()
                    .max_by_key(|(_, _, priority)| *priority)
                    .map(|(side, target, _)| (*side, target.clone()));
                self.drag.hits.borrow_mut().clear();
                global.set_native_revision(global.get_native_revision().wrapping_add(1));
            }
        }
        global.set_native_active(active);
    }
    fn receive_native(
        &mut self,
        ui: &CommanderWindow,
        x: f32,
        y: f32,
        token: Option<String>,
        paths: Vec<PathBuf>,
        dropped: bool,
    ) {
        let scale = if cfg!(target_os = "windows") {
            ui.window().scale_factor()
        } else {
            1.0
        };
        let (x, y) = (x / scale, y / scale);
        let data = if let Some(token) = token {
            private_payload(&token, dropped)
        } else {
            Some(native_payload(paths.clone()))
        };
        let Some(data) = data else {
            self.drag.hovered.clear();
            self.drag.incoming = None;
            return;
        };
        self.drag.incoming = Some(data);
        self.drag.position = Some((x, y));
        self.drag.hovered = if paths.is_empty() {
            vec![std::env::temp_dir().join(".rust-commander-native-offer")]
        } else {
            paths.clone()
        };
        if dropped {
            self.pending_drops = self.drag.hovered.clone();
            self.pending_drop_side = usize::try_from(ui.invoke_drop_side(x, y))
                .ok()
                .filter(|side| *side < 2);
            self.drag.drop_action = Some(if self.drag.buttons().1 {
                DragAction::Move
            } else {
                DragAction::Copy
            });
            self.drop_deadline = Some(std::time::Instant::now() + Duration::from_millis(80));
        }
    }
    pub(super) fn prepare_native_drag(&mut self) {
        let held = self.drag.buttons().0;
        let requested = std::mem::take(&mut self.drag.prepare_requested);
        if !requested && !held {
            self.drag.pending = None;
            self.drag.deferred = None;
            return;
        }
        if !requested && !self.drag.outside {
            if !held {
                self.drag.pending = None;
            }
            return;
        }
        let Some(selection) = self.drag.pending.take() else {
            return;
        };
        if self.blocked() || self.app.job.is_some() {
            return;
        }
        if selection.from.local().is_some() {
            let ready = Ready {
                paths: selection.entries.iter().map(|e| e.path.clone()).collect(),
                selection: (*selection).clone(),
                prepared: std::time::Instant::now(),
            };
            remember(ready.clone());
            if held && !requested {
                self.drag.ready = Some(ready);
            } else {
                self.app.status =
                    "Drag the selection out of the window to use it in another application.".into();
            }
            return;
        }
        let cached = READY.with(|cache| {
            cache
                .borrow()
                .iter()
                .find(|ready| {
                    same(&ready.selection, &selection)
                        && ready.prepared.elapsed() < Duration::from_secs(30)
                        && ready.paths.iter().all(|p| crate::operations::exists(p))
                })
                .cloned()
        });
        if let Some(ready) = cached {
            if held && !requested {
                self.drag.ready = Some(ready);
            } else {
                self.app.status = "Files ready to drag. Drag the selection again.".into();
            }
            return;
        }
        if !requested {
            // A gesture is never authorization to fetch virtual file contents.
            // Other applications require real files: only the explicitly named
            // preparation command may create those exports before a drop.
            self.drag.deferred = Some((*selection).clone());
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, result) = mpsc::channel();
        let (updates_tx, updates) = mpsc::channel();
        self.drag.preparation = Some(Preparation {
            cancel,
            progress: Default::default(),
            updates,
            result,
            external: None,
            waiting_since: std::time::Instant::now(),
        });
        let selection = (*selection).clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let progress_ctx = ctx.clone();
            let mut task = Task::new(worker_cancel, move |progress| {
                let _ = updates_tx.send(progress);
                progress_ctx.request_repaint();
            });
            let result = export_cache()
                .and_then(|cache| crate::drag_drop::export_in(&selection, &mut task, Some(&cache)))
                .and_then(|export| {
                    task.check()?;
                    retain_export(export)
                })
                .map(|paths| Ready {
                    selection,
                    paths,
                    prepared: std::time::Instant::now(),
                })
                .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
}
pub(super) fn dispatch(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        use raw_window_handle::HasWindowHandle;
        let poll = {
            let a = state.borrow();
            a.drag.pressed && crate::native_files::drag_buttons().0
        };
        if poll
            && let Ok(handle) = ui.window().window_handle().window_handle()
            && let Some((x, y)) = crate::native_files::drop_position(
                handle.as_raw(),
                f64::from(ui.window().scale_factor()),
            )
        {
            let size = ui.window().size().to_logical(ui.window().scale_factor());
            let outside = x < 0.0 || y < 0.0 || x >= size.width || y >= size.height;
            if outside {
                prime_exit(state, ui);
            }
            state.borrow_mut().drag.outside = outside;
        }
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if !crate::native_files::drag_buttons().0 && !state.borrow().drag.prepare_requested {
        let mut a = state.borrow_mut();
        a.drag.pending = None;
        a.drag.deferred = None;
        a.drag.ready = None;
        return;
    }
    windows::with_state(state, |a| a.prepare_native_drag());
    let deferred = state.borrow_mut().drag.deferred.take();
    if let Some(selection) = deferred {
        dispatch_deferred(state, ui, selection);
        return;
    }
    let ready = state.borrow_mut().drag.ready.take();
    let Some(ready) = ready else {
        return;
    };
    ACTIVE.with(|active| {
        *active.borrow_mut() = Some((
            ready.clone(),
            std::time::Instant::now() + Duration::from_secs(300),
        ))
    });
    trace("native handoff");
    // End Slint's in-window gesture before the native event loop takes ownership.
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerExited);
    let (sender, receiver) = mpsc::channel();
    state.borrow_mut().drag.finished = Some(receiver);
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        use raw_window_handle::HasWindowHandle;
        let result = ui
            .window()
            .window_handle()
            .window_handle()
            .map_err(anyhow::Error::from)
            .and_then(|handle| {
                crate::native_files::drag_out_with(
                    &crate::native_files::Window(handle.as_raw()),
                    ready.paths,
                    ready.selection.from.local().is_some(),
                    move || {
                        let _ = sender.send(());
                    },
                )
            });
        if let Err(error) = result {
            state.borrow_mut().drag.finished = None;
            state.borrow_mut().app.fail(error);
        }
    }
    #[cfg(target_os = "linux")]
    {
        let result = state
            .borrow_mut()
            .drag
            .linux
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Native drag backend is unavailable."))
            .and_then(|backend| {
                backend.start(
                    ready.paths,
                    ready.selection.from.local().is_some(),
                    move |_| {
                        let _ = sender.send(());
                    },
                )
            });
        if let Err(error) = result {
            state.borrow_mut().drag.finished = None;
            state.borrow_mut().app.fail(error);
        }
    }
}

#[cfg(test)]
mod tests;

// Slint starts its in-window drag on the first movement beyond the threshold.
// A fast OS gesture can release before a second move discovers the drop target.
// Refresh that target before winit forwards the release to Slint.
/// The OS can send CursorLeft before a threshold-crossing CursorMoved. Preserve
/// the real, held row gesture before Slint receives PointerExited and releases it.
pub(super) fn prime_exit(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow) {
    let position = {
        let a = state.borrow();
        (a.drag.pressed && a.drag.pending.is_none() && a.drag.finished.is_none() && !a.blocked())
            .then_some(a.drag.cursor)
            .flatten()
    };
    if let Some(position) = position {
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(position.x + 24.0, position.y),
            });
    }
}

pub(super) fn prime_release(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow) {
    let cursor = {
        let a = state.borrow();
        (!a.drag.outside && a.drag.finished.is_none())
            .then_some(a.drag.cursor)
            .flatten()
    };
    if let Some(position) = cursor {
        trace("release target refresh");
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
    }
}

fn dispatch_deferred(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow, selection: Selection) {
    let (sender, result) = mpsc::channel();
    let (progress_sender, updates) = mpsc::channel();
    let ctx = state.borrow().ctx.clone();
    let progress_ctx = ctx.clone();
    let ready_selection = selection.clone();
    let session = PromiseSession::new(
        selection,
        cfg!(target_os = "macos"),
        Arc::new(move |progress| {
            let _ = progress_sender.send(progress);
            progress_ctx.request_repaint();
        }),
        Box::new(move |result| {
            let _ = sender.send(result.map(|paths| Ready {
                selection: ready_selection.clone(),
                paths,
                prepared: std::time::Instant::now(),
            }));
            ctx.request_repaint();
        }),
    );
    let token = format!("{:032x}", rand::random::<u128>());
    PRIVATE.with(|offers| {
        let mut offers = offers.borrow_mut();
        offers
            .retain(|_, offer| offer.until > std::time::Instant::now() && offer.session.routable());
        offers.insert(
            token.clone(),
            PrivateOffer {
                session: session.clone(),
                until: std::time::Instant::now() + Duration::from_secs(300),
            },
        );
    });
    state.borrow_mut().drag.preparation = Some(Preparation {
        cancel: session.cancel.clone(),
        progress: Default::default(),
        updates,
        result,
        external: Some(session.clone()),
        waiting_since: std::time::Instant::now(),
    });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerExited);
    let (finished_tx, finished) = mpsc::channel();
    state.borrow_mut().drag.finished = Some(finished);
    let done_session = session.clone();
    let callback = move |accepted| {
        done_session.end_drag(accepted);
        let _ = finished_tx.send(());
    };
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let result = {
        use raw_window_handle::HasWindowHandle;
        let entries = session
            .selection
            .entries
            .iter()
            .map(|e| ::drag::PromiseEntry {
                path: PathBuf::from(&e.name),
                directory: e.is_dir,
                size: e.size_known.then_some(e.size),
            })
            .collect::<Vec<_>>();
        let list_session = session.clone();
        let read_session = session.clone();
        let write_session = session.clone();
        let promises = ::drag::PromisedFiles {
            internal_token: Some(token.clone()),
            entries,
            list: Arc::new(move || {
                crate::drag_drop::promises::manifest(&list_session.selection, &list_session.cancel)
                    .map(|entries| {
                        entries
                            .into_iter()
                            .map(|(path, directory, size)| ::drag::PromiseEntry {
                                path,
                                directory,
                                size,
                            })
                            .collect()
                    })
                    .map_err(|e| e.to_string())
            }),
            read: Arc::new(move |path| read_session.local_path(path)),
            write: Arc::new(move |index, destination| write_session.write(index, destination)),
        };
        ui.window()
            .window_handle()
            .window_handle()
            .map_err(anyhow::Error::from)
            .and_then(|handle| {
                crate::native_files::drag_promised(
                    &crate::native_files::Window(handle.as_raw()),
                    promises,
                    callback,
                )
            })
    };
    #[cfg(target_os = "linux")]
    let result = {
        let read_session = session.clone();
        state
            .borrow_mut()
            .drag
            .linux
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Native drag backend is unavailable."))
            .and_then(|backend| {
                backend.start_deferred(
                    token.clone(),
                    windows::x11_windows(),
                    marker(&token),
                    Arc::new(move || read_session.materialize()),
                    callback,
                )
            })
    };
    if let Err(error) = result {
        session.abort(&format!("Could not start native drag: {error:#}"));
        state.borrow_mut().drag.finished = None;
    }
}
