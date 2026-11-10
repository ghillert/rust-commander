use crate::filesystem::{self, Entry, Location, Sort};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct Tab {
    pub folders_first: bool,
    watcher: Option<notify::RecommendedWatcher>,
    watch_path: Option<PathBuf>,
    changed: Arc<AtomicBool>,
    last_refresh: Instant,
    pub id: u64,
    pub state: crate::navigation::TabState,
    pub(crate) requested_location: Option<Location>,
    pub(crate) revision: u64,
    pub(crate) navigation_epoch: u64,
    pub location: Location,
    pub address: String,
    pub filter: String,
    pub entries: Vec<Entry>,
    pub selected: BTreeSet<PathBuf>,
    pub cursor: Option<PathBuf>,
    // The synthetic parent entry must never become a file-operation source.
    parent_cursor: bool,
    pub sort: Sort,
    pub descending: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub focus_address: bool,
    pub focus_filter: bool,
    pub scroll_to_cursor: bool,
    history: Vec<Location>,
    history_index: usize,
    pending: Option<Receiver<Result<Vec<Entry>, String>>>,
    network_updates: Option<Receiver<Vec<Entry>>>,
}

impl Tab {
    pub fn new(location: Location, ctx: &crate::ui_signal::UiSignal) -> Self {
        let mut tab = Self {
            folders_first: true,
            watcher: None,
            watch_path: None,
            changed: Arc::new(AtomicBool::new(false)),
            last_refresh: Instant::now(),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            state: Default::default(),
            requested_location: None,
            revision: 0,
            navigation_epoch: 0,
            address: location.label(),
            history: vec![location.clone()],
            history_index: 0,
            location,
            filter: String::new(),
            entries: Vec::new(),
            selected: BTreeSet::new(),
            cursor: None,
            parent_cursor: false,
            sort: Sort::Name,
            descending: false,
            loading: false,
            error: None,
            pending: None,
            network_updates: None,
            focus_address: false,
            focus_filter: false,
            scroll_to_cursor: false,
        };
        tab.refresh(ctx);
        tab
    }

    pub fn refresh(&mut self, ctx: &crate::ui_signal::UiSignal) {
        use notify::Watcher;
        let watch_path = match &self.location {
            Location::Local(path) => Some(path.clone()),
            Location::Archive { file, .. } => Some(file.clone()),
            _ => None,
        };
        if watch_path != self.watch_path {
            self.watcher = None;
            self.watch_path = watch_path.clone();
            if let Some(path) = watch_path {
                let changed = self.changed.clone();
                let ctx = ctx.clone();
                if let Ok(mut watcher) =
                    notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                        if event.is_ok_and(|e| !matches!(e.kind, notify::EventKind::Access(_))) {
                            changed.store(true, Ordering::Relaxed);
                            ctx.request_repaint();
                        }
                    })
                    && watcher
                        .watch(&path, notify::RecursiveMode::NonRecursive)
                        .is_ok()
                {
                    self.watcher = Some(watcher);
                }
            }
        }
        self.last_refresh = Instant::now();
        let (sender, receiver) = mpsc::channel();
        let location = self.location.clone();
        let ctx = ctx.clone();
        self.pending = Some(receiver);
        self.loading = true;
        self.error = None;
        let updates = if location == (Location::Network { server: None }) {
            let (sender, receiver) = mpsc::sync_channel(1);
            self.network_updates = Some(receiver);
            Some(sender)
        } else {
            self.network_updates = None;
            None
        };
        std::thread::spawn(move || {
            let result = if let Some(updates) = updates {
                crate::discovery::network_entries(|entries| {
                    if updates.try_send(entries).is_ok() {
                        ctx.request_repaint();
                    }
                })
            } else {
                filesystem::list(&location)
            };
            let _ = sender.send(result.map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
    }

    pub(crate) fn pending_network_scan(&self) -> bool {
        self.network_updates.is_some()
    }

    pub(crate) fn stop_network_scan(&mut self) {
        if self.network_updates.take().is_some() {
            self.pending = None;
            self.loading = false;
        }
    }

    pub fn poll_changes(&mut self, ctx: &crate::ui_signal::UiSignal) {
        if self.changed.load(Ordering::Relaxed) {
            if !self.loading && self.last_refresh.elapsed() > Duration::from_millis(250) {
                self.changed.store(false, Ordering::Relaxed);
                self.refresh(ctx);
            } else {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        }
    }

    pub fn poll(&mut self, hidden: bool) {
        if let Some(result) = crate::background::poll(&mut self.pending) {
            self.network_updates = None;
            self.parent_cursor = self.parent_focused();
            self.revision = self.revision.wrapping_add(1);
            self.loading = false;
            match result {
                Ok(entries) => self.apply_listing(entries, hidden),
                Err(error) => {
                    self.entries.clear();
                    self.selected.clear();
                    self.cursor = None;
                    self.error = Some(error);
                }
            }
        } else if let Some(entries) = self
            .network_updates
            .as_ref()
            .and_then(|updates| updates.try_recv().ok())
        {
            self.revision = self.revision.wrapping_add(1);
            self.apply_listing(entries, hidden);
        }
    }

    pub(crate) fn listing_sort(&self) -> Sort {
        if matches!(self.location, Location::Network { .. }) {
            Sort::Name
        } else {
            self.sort
        }
    }

    fn apply_listing(&mut self, mut entries: Vec<Entry>, hidden: bool) {
        filesystem::sort_entries_with_options(
            &mut entries,
            self.listing_sort(),
            self.descending,
            self.folders_first,
        );
        self.entries = entries;
        self.selected
            .retain(|path| self.entries.iter().any(|e| &e.path == path));
        if !self.parent_focused()
            && !self
                .entries
                .iter()
                .any(|e| Some(&e.path) == self.cursor.as_ref())
        {
            self.cursor = self
                .visible(hidden)
                .first()
                .map(|&i| self.entries[i].path.clone());
        }
    }

    pub fn requested_navigation(&self) -> Option<&Location> {
        self.requested_location.as_ref()
    }

    pub(crate) fn navigate_loaded(
        &mut self,
        location: Location,
        mut entries: Vec<Entry>,
        cursor: Option<PathBuf>,
        ctx: &crate::ui_signal::UiSignal,
    ) {
        if !self.prepare_navigation(&location) {
            ctx.request_repaint();
            return;
        }
        if self.location != location {
            self.assign_location(location);
        }
        self.watcher = None;
        self.watch_path = None;
        self.changed.store(false, Ordering::Relaxed);
        self.pending = None;
        self.network_updates = None;
        self.loading = false;
        self.error = None;
        filesystem::sort_entries_with_options(
            &mut entries,
            self.listing_sort(),
            self.descending,
            self.folders_first,
        );
        self.cursor = cursor
            .or_else(|| self.cursor.clone())
            .filter(|path| entries.iter().any(|entry| &entry.path == path))
            .or_else(|| {
                if self.parent_focused() {
                    None
                } else {
                    entries.first().map(|e| e.path.clone())
                }
            });
        self.entries = entries;
        self.selected
            .retain(|path| self.entries.iter().any(|entry| &entry.path == path));
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn navigate(&mut self, location: Location, ctx: &crate::ui_signal::UiSignal) {
        if self.prepare_navigation(&location) {
            if self.location == location {
                self.address = location.label();
                self.refresh(ctx);
            } else {
                self.set_location(location, ctx);
            }
        }
    }

    fn prepare_navigation(&mut self, location: &Location) -> bool {
        self.navigation_epoch = self.navigation_epoch.wrapping_add(1);
        if self
            .state
            .locked
            .as_ref()
            .is_some_and(|locked| locked != location)
        {
            self.requested_location = Some(location.clone());
            return false;
        }
        if &self.location != location {
            self.history.truncate(self.history_index + 1);
            self.history.push(location.clone());
            self.history_index = self.history.len() - 1;
        }
        true
    }

    fn set_location(&mut self, location: Location, ctx: &crate::ui_signal::UiSignal) {
        self.assign_location(location);
        self.refresh(ctx);
    }
    fn assign_location(&mut self, location: Location) {
        let returning = self.location.parent().as_ref() == Some(&location);
        let return_cursor = returning.then(|| match &self.location {
            Location::Network { server } => server
                .as_deref()
                .and_then(crate::remote::profile)
                .map_or_else(PathBuf::new, |profile| PathBuf::from(profile.endpoint)),
            Location::Remote { connection, path } if path.as_os_str().is_empty() => {
                crate::remote::profile(connection).map_or_else(PathBuf::new, |profile| {
                    PathBuf::from(if profile.service == "smb" {
                        profile.bucket
                    } else {
                        profile.root
                    })
                })
            }
            Location::Local(path) | Location::Remote { path, .. } => path.clone(),
            Location::Archive { file, prefix } if prefix.as_os_str().is_empty() => {
                crate::archive_tools::cached_origin(file)
                    .map_or_else(|| file.clone(), |origin| origin.source)
            }
            Location::Archive { prefix, .. } => prefix.clone(),
        });
        self.navigation_epoch = self.navigation_epoch.wrapping_add(1);
        self.address = location.label();
        self.location = location;
        self.pending = None;
        self.network_updates = None;
        self.loading = false;
        self.entries.clear();
        self.selected.clear();
        self.cursor = return_cursor;
        self.parent_cursor = false;
        self.filter.clear();
    }

    pub fn history(&mut self, forward: bool, ctx: &crate::ui_signal::UiSignal) {
        if self.state.locked.is_some() {
            let index = if forward {
                self.history_index.checked_add(1)
            } else {
                self.history_index.checked_sub(1)
            };
            if let Some(location) = index.and_then(|i| self.history.get(i)).cloned() {
                self.navigate(location, ctx);
            }
            return;
        }
        if forward && self.history_index + 1 < self.history.len() {
            self.history_index += 1;
        } else if !forward && self.history_index > 0 {
            self.history_index -= 1;
        } else {
            return;
        }
        self.set_location(self.history[self.history_index].clone(), ctx);
    }

    pub fn visible(&self, hidden: bool) -> Vec<usize> {
        let query = self.filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (hidden || !e.name.starts_with('.')) && e.name.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn parent_focused(&self) -> bool {
        self.parent_cursor && self.cursor.is_none()
    }

    pub fn focused(&self, hidden: bool) -> Option<Entry> {
        self.visible(hidden).into_iter().find_map(|index| {
            let entry = &self.entries[index];
            (self.cursor.as_ref() == Some(&entry.path)).then(|| entry.clone())
        })
    }

    pub fn clear_cursor(&mut self) {
        self.cursor = None;
        self.parent_cursor = false;
    }

    pub fn focus_parent(&mut self) {
        if self.location.parent().is_some() {
            self.cursor = None;
            self.parent_cursor = true;
        }
    }

    pub fn chosen(&self, hidden: bool) -> Vec<Entry> {
        if matches!(self.location, Location::Network { .. }) {
            return Vec::new();
        }
        let visible = self.visible(hidden);
        let selected: Vec<_> = visible
            .iter()
            .map(|&i| &self.entries[i])
            .filter(|e| self.selected.contains(&e.path))
            .cloned()
            .collect();
        if !selected.is_empty() {
            selected
        } else {
            visible
                .iter()
                .map(|&i| &self.entries[i])
                .filter(|e| Some(&e.path) == self.cursor.as_ref())
                .cloned()
                .collect()
        }
    }

    /// Toggle the current visible row's mark, then advance without marking the next row.
    pub fn toggle_mark_and_advance(&mut self, hidden: bool) {
        if self.parent_focused() {
            self.move_cursor(1, false, hidden);
            return;
        }
        let visible = self.visible(hidden);
        let Some(position) = visible
            .iter()
            .position(|&i| Some(&self.entries[i].path) == self.cursor.as_ref())
        else {
            return;
        };
        let path = self.entries[visible[position]].path.clone();
        if !self.selected.remove(&path) {
            self.selected.insert(path);
        }
        self.move_cursor(1, false, hidden);
    }

    pub fn move_cursor(&mut self, delta: isize, extend: bool, hidden: bool) {
        self.move_cursor_with_parent(delta, extend, hidden, false);
    }

    pub fn move_cursor_with_parent(
        &mut self,
        delta: isize,
        extend: bool,
        hidden: bool,
        include_parent: bool,
    ) {
        let visible = self.visible(hidden);
        let offset = usize::from(
            self.parent_focused() || (include_parent && self.location.parent().is_some()),
        );
        if visible.len() + offset == 0 {
            return;
        }
        let focused_position = visible
            .iter()
            .position(|&i| Some(&self.entries[i].path) == self.cursor.as_ref());
        let position = if self.parent_focused() {
            0
        } else {
            focused_position.map_or(offset, |index| index + offset)
        };
        let next = if focused_position.is_none() && !self.parent_focused() {
            if delta > 0 {
                (delta as usize - 1)
                    .saturating_add(offset)
                    .min(visible.len() + offset - 1)
            } else {
                0
            }
        } else {
            position
                .saturating_add_signed(delta)
                .min(visible.len() + offset - 1)
        };
        if extend {
            for index in position.min(next)..=position.max(next) {
                if let Some(&entry) = index
                    .checked_sub(offset)
                    .and_then(|index| visible.get(index))
                {
                    self.selected.insert(self.entries[entry].path.clone());
                }
            }
        }
        if offset == 1 && next == 0 {
            self.focus_parent();
        } else {
            self.parent_cursor = false;
            self.cursor = Some(self.entries[visible[next - offset]].path.clone());
        }
        self.scroll_to_cursor = true;
    }
}

pub struct Pane {
    pub tabs: Vec<Tab>,
    pub active: usize,
}
impl Pane {
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
    pub fn add_tab(&mut self, ctx: &crate::ui_signal::UiSignal) {
        self.tabs.push(Tab::new(self.tab().location.clone(), ctx));
        self.active = self.tabs.len() - 1;
    }
    pub fn close_tab(&mut self, index: usize) {
        if self.tabs.len() > 1 {
            self.tabs.remove(index);
            if index < self.active {
                self.active -= 1;
            }
            self.active = self.active.min(self.tabs.len() - 1);
        }
    }
}
