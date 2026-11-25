//! Browsing state for the two file panes.
//!
//! Each [`Pane`] owns its tabs. A [`Tab`] tracks one location, its navigation
//! history, and the last directory listing received from a background worker.
//! Call [`Tab::poll`] to apply listing updates. Use [`Tab::chosen`] to turn
//! the current marks and cursor into file-operation sources.

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

struct DockerListing(Arc<AtomicBool>);

impl Drop for DockerListing {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A directory view with its own history, filter, cursor, and marked paths.
///
/// The cursor identifies one row. Marks can span several rows and survive
/// filtering. The parent row (`..`) can hold the cursor but cannot be marked.
pub struct Tab {
    /// Whether to group directories ahead of files when sorting the listing.
    pub folders_first: bool,
    watcher: Option<notify::RecommendedWatcher>,
    watch_path: Option<PathBuf>,
    changed: Arc<AtomicBool>,
    last_refresh: Instant,
    /// Process-local identifier that stays with this tab as it moves between locations.
    pub id: u64,
    /// Per-tab settings, including the custom title and optional location lock.
    pub state: crate::navigation::TabState,
    pub(crate) requested_location: Option<Location>,
    pub(crate) revision: u64,
    pub(crate) navigation_epoch: u64,
    /// Location whose entries this tab displays.
    pub location: Location,
    /// Editable address text. Navigation resets it to the location's label.
    pub address: String,
    /// Case-insensitive substring used to filter entries by name.
    pub filter: String,
    /// Sorted listing before the name and hidden-file filters are applied.
    pub entries: Vec<Entry>,
    /// Marked paths, including entries temporarily hidden by a filter.
    pub selected: BTreeSet<PathBuf>,
    /// Path of the focused entry. See [`Self::parent_focused`] when this is `None`.
    pub cursor: Option<PathBuf>,
    // The synthetic parent entry must never become a file-operation source.
    parent_cursor: bool,
    /// Sort key for directory entries. Virtual lists use name order.
    pub sort: Sort,
    /// Whether to reverse the order within each sort group.
    pub descending: bool,
    /// Whether a listing has started and [`Self::poll`] is still awaiting its final result.
    pub loading: bool,
    /// Error message from the latest failed listing or cancellation.
    pub error: Option<String>,
    /// Requests keyboard focus for the address field.
    pub focus_address: bool,
    /// Requests keyboard focus for the name filter.
    pub focus_filter: bool,
    /// Requests that the pane scroll the focused row into view.
    pub scroll_to_cursor: bool,
    history: Vec<Location>,
    history_index: usize,
    pending: Option<Receiver<Result<Vec<Entry>, String>>>,
    network_updates: Option<Receiver<Vec<Entry>>>,
    docker: Option<Arc<crate::uri_backend::docker::Container>>,
    docker_listing: Option<DockerListing>,
}

impl Tab {
    /// Opens a tab at `location` and starts its first background listing.
    ///
    /// New tabs sort by name with directories first. Call [`Self::poll`] after
    /// a repaint signal to make newly listed entries available.
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
            docker: None,
            docker_listing: None,
            focus_address: false,
            focus_filter: false,
            scroll_to_cursor: false,
        };
        tab.refresh(ctx);
        tab
    }

    /// Requests a fresh listing and keeps the current rows available until it arrives.
    ///
    /// A refresh preserves an in-progress Docker listing. Local directories
    /// and archive files are watched for changes when the platform permits it.
    pub fn refresh(&mut self, ctx: &crate::ui_signal::UiSignal) {
        // Routine refreshes from unrelated file jobs must not cancel an export.
        if self.docker_listing.is_none() {
            self.refresh_impl(ctx, false);
        }
    }

    pub(crate) fn refresh_docker(&mut self, ctx: &crate::ui_signal::UiSignal) {
        self.refresh_impl(ctx, true);
    }

    pub(crate) fn cancel_docker_listing(&mut self) -> bool {
        if self.docker_listing.take().is_none() {
            return false;
        }
        self.pending = None;
        self.loading = false;
        self.error = Some(
            crate::i18n::text("Docker browsing cancelled. Refresh to try again.").into_owned(),
        );
        true
    }

    pub(crate) fn docker_status(&self) -> Option<String> {
        let status = self.docker.as_ref().and_then(|docker| docker.status());
        if self.docker_listing.is_some() {
            let cancel = crate::i18n::text("Esc to cancel");
            Some(status.map_or_else(
                || cancel.to_string(),
                |status| format!("{status} · {cancel}"),
            ))
        } else {
            status
        }
    }

    fn refresh_impl(&mut self, ctx: &crate::ui_signal::UiSignal, refresh_docker: bool) {
        use notify::Watcher;
        self.docker_listing = None;
        self.docker = self
            .location
            .docker_target()
            .as_ref()
            .map(crate::uri_backend::docker::attach);
        let docker = self.docker.clone();
        let cancel = if docker.is_some() || self.location == Location::Containers {
            let cancel = Arc::new(AtomicBool::new(false));
            self.docker_listing = Some(DockerListing(cancel.clone()));
            Some(cancel)
        } else {
            None
        };
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
            } else if let Some(cancel) = cancel {
                match (&location, docker) {
                    (Location::Containers, _) => {
                        crate::uri_backend::docker::list_containers(&cancel)
                    }
                    (Location::Remote { path, .. }, Some(container)) => {
                        container.entries(path, refresh_docker, &cancel)
                    }
                    _ => filesystem::browse(&location),
                }
            } else {
                filesystem::browse(&location)
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

    /// Refreshes after a watched filesystem change once the tab is idle.
    ///
    /// Closely spaced changes are coalesced. While a refresh is pending, the
    /// tab requests another repaint so the event loop can call this again.
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

    /// Applies any available background listing without blocking.
    ///
    /// A successful listing retains marks for paths still present. If the
    /// cursor needs a new row, `hidden` controls whether dot-prefixed names
    /// participate. A failed listing clears the entries and marks and records
    /// the reason in [`error`](Self::error).
    pub fn poll(&mut self, hidden: bool) {
        if let Some(result) = crate::background::poll(&mut self.pending) {
            self.docker_listing = None;
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
        if self.location.is_virtual_list() {
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
        if !self.selected.is_empty() {
            let present: std::collections::HashSet<_> =
                self.entries.iter().map(|e| &e.path).collect();
            self.selected.retain(|path| present.contains(path));
        }
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

    /// Returns the destination deferred by a location lock, so the caller can open it elsewhere.
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
        self.docker_listing = None;
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

    /// Navigates to `location` and requests its listing.
    ///
    /// Changing location clears the filter and marks and replaces any forward
    /// history. Revisiting the current location refreshes it. A location lock
    /// leaves the tab in place and exposes the target via [`Self::requested_navigation`].
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
            Location::Containers => PathBuf::new(),
            Location::Network { server } => server
                .as_deref()
                .and_then(crate::remote::profile)
                .map_or_else(PathBuf::new, |profile| PathBuf::from(profile.endpoint)),
            Location::Remote { connection, path } if path.as_os_str().is_empty() => {
                crate::remote::profile(connection).map_or_else(PathBuf::new, |profile| {
                    if let Some(target) = profile.docker {
                        return crate::uri_backend::docker::row_key(&target);
                    }
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
        if self.location.docker_target().is_none() {
            self.docker = None;
        }
        self.pending = None;
        self.network_updates = None;
        self.docker_listing = None;
        self.loading = false;
        self.entries.clear();
        self.selected.clear();
        self.cursor = return_cursor;
        self.parent_cursor = false;
        self.filter.clear();
    }

    /// Moves one history entry forward when `forward` is true, or back when false.
    ///
    /// At either end of the history, the tab stays put. A location lock can
    /// defer the destination just as it does for [`Self::navigate`].
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

    /// Returns indices into [`Self::entries`] in display order after name filtering.
    ///
    /// Set `hidden` to include names beginning with a dot. The synthetic parent
    /// row has no index in the listing.
    pub fn visible(&self, hidden: bool) -> Vec<usize> {
        self.visible_indices(hidden).collect()
    }

    pub(crate) fn visible_indices(&self, hidden: bool) -> impl Iterator<Item = usize> + '_ {
        let query = self.filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(move |(_, e)| {
                (hidden || !e.name.starts_with('.'))
                    && (query.is_empty() || e.name.to_lowercase().contains(&query))
            })
            .map(|(i, _)| i)
    }

    /// Returns whether the cursor is on the synthetic parent row (`..`).
    pub fn parent_focused(&self) -> bool {
        self.parent_cursor && self.cursor.is_none()
    }

    /// Returns a copy of the focused entry if it passes the current visibility filters.
    ///
    /// An empty cursor or a focused parent row yields `None`.
    pub fn focused(&self, hidden: bool) -> Option<Entry> {
        let cursor = self.cursor.as_ref()?;
        self.visible_indices(hidden).find_map(|index| {
            let entry = &self.entries[index];
            (&entry.path == cursor).then(|| entry.clone())
        })
    }

    /// Clears row focus while preserving marked paths.
    pub fn clear_cursor(&mut self) {
        self.cursor = None;
        self.parent_cursor = false;
    }

    /// Moves focus to the parent row when the location has a parent.
    pub fn focus_parent(&mut self) {
        if self.location.parent().is_some() {
            self.cursor = None;
            self.parent_cursor = true;
        }
    }

    /// Returns the entries that a file operation should act on, in display order.
    ///
    /// Visible marks take precedence. If all marks are filtered out, the
    /// focused visible entry is used. Network and container discovery lists
    /// yield an empty selection, and the parent row is never a source.
    ///
    /// # Examples
    ///
    /// Mark one file, then filter the listing down to the focused file.
    ///
    /// ```
    /// use rust_commander::{filesystem::Location, model::Tab, ui_signal::UiSignal};
    /// use std::{fs, time::Duration};
    ///
    /// let dir = tempfile::tempdir()?;
    /// fs::write(dir.path().join("alpha.txt"), "first")?;
    /// fs::write(dir.path().join("beta.txt"), "second")?;
    /// let mut tab = Tab::new(Location::Local(dir.path().to_owned()), &UiSignal::default());
    /// # let start = std::time::Instant::now();
    /// while tab.loading {
    ///     tab.poll(false);
    ///     if tab.loading {
    ///         # assert!(start.elapsed() < Duration::from_secs(30), "listing timed out");
    ///         std::thread::sleep(Duration::from_millis(1));
    ///     }
    /// }
    ///
    /// tab.toggle_mark_and_advance(false);
    /// assert_eq!(tab.focused(false).unwrap().name, "beta.txt");
    /// assert_eq!(tab.chosen(false)[0].name, "alpha.txt");
    ///
    /// tab.filter = "beta".into();
    /// assert_eq!(tab.chosen(false)[0].name, "beta.txt");
    /// tab.focus_parent();
    /// assert!(tab.chosen(false).is_empty());
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn chosen(&self, hidden: bool) -> Vec<Entry> {
        if self.location.is_virtual_list() {
            return Vec::new();
        }
        if !self.selected.is_empty() {
            let selected: Vec<_> = self
                .visible_indices(hidden)
                .map(|i| &self.entries[i])
                .filter(|e| self.selected.contains(&e.path))
                .cloned()
                .collect();
            if !selected.is_empty() {
                return selected;
            }
        }
        self.focused(hidden).into_iter().collect()
    }

    /// Toggles the focused visible entry's mark and advances one row.
    ///
    /// On the parent row this only advances the cursor. On the last visible
    /// entry the cursor stays in place.
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

    /// Moves the cursor by a signed number of visible rows, stopping at either end.
    ///
    /// With `extend`, marks the entries in the traversed range, including both
    /// ends. Use [`Self::move_cursor_with_parent`] to move up onto the parent row.
    pub fn move_cursor(&mut self, delta: isize, extend: bool, hidden: bool) {
        self.move_cursor_with_parent(delta, extend, hidden, false);
    }

    /// Moves as in [`Self::move_cursor`], optionally including the parent row.
    ///
    /// The parent participates when it is already focused or `include_parent`
    /// is true and the location has a parent. Extending a selection across it
    /// marks only real entries.
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

/// One side of the file manager. A pane has at least one tab and a valid active index.
pub struct Pane {
    /// Open tabs in display order.
    pub tabs: Vec<Tab>,
    /// Index of the active tab in [`Self::tabs`].
    pub active: usize,
}
impl Pane {
    /// Borrows the active tab.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::active`] is outside [`Self::tabs`].
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }
    /// Mutably borrows the active tab.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::active`] is outside [`Self::tabs`].
    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
    /// Opens and activates a fresh tab at the active tab's location.
    ///
    /// The new tab starts with its own history and an empty set of marks.
    ///
    /// # Panics
    ///
    /// Panics if the pane has no valid active tab.
    pub fn add_tab(&mut self, ctx: &crate::ui_signal::UiSignal) {
        self.tabs.push(Tab::new(self.tab().location.clone(), ctx));
        self.active = self.tabs.len() - 1;
    }
    /// Closes the tab at `index` if another tab can remain open.
    ///
    /// Closing the active tab activates its right neighbor, or its left neighbor
    /// if it was last. Closing a different tab preserves the active tab.
    ///
    /// # Panics
    ///
    /// Panics if the pane has multiple tabs and `index` is out of bounds.
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
