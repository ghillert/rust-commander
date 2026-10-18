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
    pub(crate) revision: u64,
    pub location: Location,
    pub address: String,
    pub filter: String,
    pub entries: Vec<Entry>,
    pub selected: BTreeSet<PathBuf>,
    pub cursor: Option<PathBuf>,
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
            revision: 0,
            address: location.label(),
            history: vec![location.clone()],
            history_index: 0,
            location,
            filter: String::new(),
            entries: Vec::new(),
            selected: BTreeSet::new(),
            cursor: None,
            sort: Sort::Name,
            descending: false,
            loading: false,
            error: None,
            pending: None,
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
        std::thread::spawn(move || {
            let result = filesystem::list(&location).map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
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
            self.revision = self.revision.wrapping_add(1);
            self.loading = false;
            match result {
                Ok(mut entries) => {
                    filesystem::sort_entries_with_options(
                        &mut entries,
                        self.sort,
                        self.descending,
                        self.folders_first,
                    );
                    self.entries = entries;
                    self.selected
                        .retain(|path| self.entries.iter().any(|e| &e.path == path));
                    if !self
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
                Err(error) => {
                    self.entries.clear();
                    self.selected.clear();
                    self.cursor = None;
                    self.error = Some(error);
                }
            }
        }
    }

    pub fn navigate(&mut self, location: Location, ctx: &crate::ui_signal::UiSignal) {
        if self.location != location {
            self.history.truncate(self.history_index + 1);
            self.history.push(location.clone());
            self.history_index = self.history.len() - 1;
        }
        self.set_location(location, ctx);
    }

    fn set_location(&mut self, location: Location, ctx: &crate::ui_signal::UiSignal) {
        self.address = location.label();
        self.location = location;
        self.entries.clear();
        self.selected.clear();
        self.cursor = None;
        self.filter.clear();
        self.refresh(ctx);
    }

    pub fn history(&mut self, forward: bool, ctx: &crate::ui_signal::UiSignal) {
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

    pub fn chosen(&self, hidden: bool) -> Vec<Entry> {
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

    pub fn move_cursor(&mut self, delta: isize, extend: bool, hidden: bool) {
        let visible = self.visible(hidden);
        if visible.is_empty() {
            return;
        }
        let position = visible
            .iter()
            .position(|&i| Some(&self.entries[i].path) == self.cursor.as_ref())
            .unwrap_or(0);
        if extend {
            self.selected
                .insert(self.entries[visible[position]].path.clone());
        }
        let next = position.saturating_add_signed(delta).min(visible.len() - 1);
        self.cursor = Some(self.entries[visible[next]].path.clone());
        if extend {
            self.selected
                .insert(self.entries[visible[next]].path.clone());
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
