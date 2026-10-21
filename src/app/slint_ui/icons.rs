//! Bounded icon cache and viewport updates. Slint images stay on the UI thread.
use super::*;
use crate::native_icons::{IconKind, Key, Request, Response};
use crate::preferences::FileIconSource;
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::{
        Arc,
        atomic::AtomicBool,
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    time::Instant,
};

const CACHE_LIMIT: usize = 256;
const CACHE_LIFETIME: Duration = Duration::from_secs(60);

struct Cached {
    image: slint::Image,
    kind: IconKind,
    loaded: Instant,
    used: u64,
}

pub(super) struct Cache {
    sender: Option<SyncSender<Request>>,
    started: bool,
    reply: Sender<Response>,
    results: Receiver<Response>,
    pending: HashMap<Key, Arc<AtomicBool>>,
    entries: HashMap<Key, Cached>,
    clock: u64,
}

impl Default for Cache {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Cache {
    fn new(sender: Option<SyncSender<Request>>) -> Self {
        let (reply, results) = mpsc::channel();
        Self {
            started: sender.is_some(),
            sender,
            reply,
            results,
            pending: HashMap::new(),
            entries: HashMap::new(),
            clock: 0,
        }
    }
    fn prepare(&mut self, wanted: &HashSet<Key>) {
        for (key, active) in &self.pending {
            active.store(wanted.contains(key), Ordering::Relaxed);
        }
        while let Ok(response) = self.results.try_recv() {
            let (key, raster) = match response {
                Response::Loaded(key, raster) => (key, raster),
                Response::Cancelled(key) => {
                    self.pending.remove(&key);
                    continue;
                }
            };
            let Some(active) = self.pending.remove(&key) else {
                continue;
            };
            if !active.load(Ordering::Relaxed) {
                continue;
            }
            let kind = raster.as_ref().map(|icon| icon.kind).unwrap_or_default();
            let image = raster
                .map(|icon| {
                    let raster = icon.pixels;
                    slint::Image::from_rgba8(
                        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                            raster.as_raw(),
                            raster.width(),
                            raster.height(),
                        ),
                    )
                })
                .unwrap_or_default();
            if self.entries.len() >= CACHE_LIMIT
                && let Some(oldest) = self
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.used)
                    .map(|(key, _)| key.clone())
            {
                self.entries.remove(&oldest);
            }
            self.clock += 1;
            self.entries.insert(
                key,
                Cached {
                    image,
                    kind,
                    loaded: Instant::now(),
                    used: self.clock,
                },
            );
        }
    }

    fn get(&mut self, key: &Key, specific_fallback: bool) -> slint::Image {
        if let Some(entry) = self.entries.get_mut(key)
            && entry.loaded.elapsed() < CACHE_LIFETIME
        {
            self.clock += 1;
            entry.used = self.clock;
            return if entry.kind == IconKind::Generic && specific_fallback {
                Default::default()
            } else {
                entry.image.clone()
            };
        }
        self.entries.remove(key);
        if let Some(active) = self.pending.get(key) {
            active.store(true, Ordering::Relaxed);
            return Default::default();
        }
        if !self.started {
            self.sender = crate::native_icons::worker();
            self.started = true;
        }
        if let Some(sender) = &self.sender {
            let active = Arc::new(AtomicBool::new(true));
            if sender
                .try_send(Request {
                    key: key.clone(),
                    wanted: active.clone(),
                    reply: self.reply.clone(),
                })
                .is_ok()
            {
                self.pending.insert(key.clone(), active);
            }
        }
        Default::default()
    }
}

impl Drop for Cache {
    fn drop(&mut self) {
        for active in self.pending.values() {
            active.store(false, Ordering::Relaxed);
        }
    }
}

fn viewport(first: f32, page: f32, count: usize) -> Range<usize> {
    let first = (first as usize).min(count);
    let page = if page > 0.0 { page.ceil() as usize } else { 40 };
    first.saturating_sub(6)..first.saturating_add(page.clamp(1, 160) + 6).min(count)
}

impl Adapter {
    pub(super) fn sync_icons(&mut self, ui: &CommanderWindow) {
        let ranges = std::array::from_fn::<_, 2, _>(|side| {
            if self.app.preferences.single_pane && self.app.active != side {
                0..0
            } else {
                viewport(
                    [ui.get_left_first_row(), ui.get_right_first_row()][side],
                    [ui.get_left_page_rows(), ui.get_right_page_rows()][side],
                    self.row_cache[side].len(),
                )
            }
        });
        let system = self.app.preferences.file_icon_source == FileIconSource::SystemWithFallback;
        let wanted = (0..2)
            .filter(|_| system)
            .flat_map(|side| {
                self.row_cache[side][ranges[side].clone()]
                    .iter()
                    .filter_map(|row| row.icon_key.clone())
            })
            .collect();
        self.icons.prepare(&wanted);
        for (side, range) in ranges.iter().enumerate() {
            // File models must not retain every icon ever scrolled past.
            for index in self.icon_rows[side].clone() {
                if !range.contains(&index)
                    && let Some(mut row) = self.rows[side].row_data(index)
                    && row.native_icon
                {
                    row.icon = Default::default();
                    row.native_icon = false;
                    self.rows[side].set_row_data(index, row);
                }
            }
            for index in range.clone() {
                let icon = self.row_cache[side][index]
                    .icon_key
                    .as_ref()
                    .filter(|_| system)
                    .map(|key| {
                        self.icons.get(
                            key,
                            crate::icons::specific(self.row_cache[side][index].fallback_icon),
                        )
                    })
                    .unwrap_or_default();
                if let Some(mut row) = self.rows[side].row_data(index)
                    && row.icon != icon
                {
                    row.native_icon = icon.size().width > 0;
                    row.icon = icon;
                    self.rows[side].set_row_data(index, row);
                }
            }
        }
        self.icon_rows = ranges;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(path: &str) -> Key {
        Key {
            path: path.into(),
            directory: false,
            modified: None,
        }
    }
    fn fixture() -> (Cache, Receiver<Request>) {
        let (sender, requests) = mpsc::sync_channel(1);
        (Cache::new(Some(sender)), requests)
    }

    #[test]
    fn requests_are_deduplicated_and_queue_pressure_never_blocks() {
        let (mut cache, requests) = fixture();
        let a = key("/one/file.txt");
        let b = key("/two/file.txt");
        assert_eq!(cache.get(&a, false).size().width, 0);
        cache.get(&a, false);
        cache.get(&b, false); // Full queue; retry on the next UI tick.
        assert_eq!(cache.pending.len(), 1);
        let request = requests.try_recv().unwrap();
        assert_eq!(request.key, a);
        assert!(requests.try_recv().is_err());
        request
            .reply
            .send(Response::Loaded(
                a.clone(),
                Some(crate::native_icons::NativeIcon::new(
                    image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255])),
                    IconKind::Specific,
                )),
            ))
            .unwrap();
        cache.prepare(&HashSet::from([a.clone(), b.clone()]));
        assert_eq!(cache.get(&a, false).size().width, 64);
        cache.get(&b, false);
        assert_eq!(
            requests.try_recv().unwrap().key,
            b,
            "same extension must retain per-file icons"
        );
    }

    #[test]
    fn cancelled_requests_retry_and_missing_icons_are_cached() {
        let (mut cache, requests) = fixture();
        let a = key("/file.txt");
        cache.get(&a, false);
        let request = requests.try_recv().unwrap();
        cache.prepare(&HashSet::new());
        assert!(!request.wanted.load(Ordering::Relaxed));
        request.reply.send(Response::Cancelled(a.clone())).unwrap();
        // Scrolling back before a cancelled result is drained must retry,
        // rather than caching cancellation as a missing icon for a minute.
        cache.prepare(&HashSet::from([a.clone()]));
        assert!(!cache.entries.contains_key(&a));
        cache.get(&a, false);
        requests
            .try_recv()
            .unwrap()
            .reply
            .send(Response::Loaded(a.clone(), None))
            .unwrap();
        cache.prepare(&HashSet::from([a.clone()]));
        cache.get(&a, false);
        assert!(requests.try_recv().is_err());
        cache.entries.get_mut(&a).unwrap().loaded = Instant::now() - CACHE_LIFETIME;
        cache.get(&a, false);
        assert_eq!(requests.try_recv().unwrap().key, a);
    }

    #[test]
    fn cache_evicts_old_icons_and_viewports_stay_bounded() {
        let (mut cache, requests) = fixture();
        for i in 0..=CACHE_LIMIT {
            let key = key(&format!("/{i}.txt"));
            cache.get(&key, false);
            requests
                .try_recv()
                .unwrap()
                .reply
                .send(Response::Loaded(key.clone(), None))
                .unwrap();
            cache.prepare(&HashSet::from([key]));
        }
        assert_eq!(cache.entries.len(), CACHE_LIMIT);
        assert!(!cache.entries.contains_key(&key("/0.txt")));
        assert_eq!(viewport(0.0, 20.0, 100), 0..26);
        assert_eq!(viewport(90.0, 20.0, 100), 84..100);
        assert_eq!(viewport(0.0, 0.0, 0), 0..0);
        assert!(viewport(500.0, 10_000.0, 100_000).len() <= 172);
    }

    #[test]
    fn file_rows_keep_native_icons_on_selection_and_virtual_entries_use_fallbacks() {
        i_slint_backend_testing::init_no_event_loop();
        let (root, mut adapter, left, _right) = super::super::tests::fixture();
        let (sender, requests) = mpsc::sync_channel::<Request>(32);
        adapter.icons = Cache::new(Some(sender));
        let ui = CommanderWindow::new().unwrap();
        adapter.sync(&ui);
        for request in requests.try_iter() {
            request
                .reply
                .send(Response::Loaded(
                    request.key,
                    Some(crate::native_icons::NativeIcon::new(
                        image::RgbaImage::from_pixel(64, 64, image::Rgba([20, 120, 220, 255])),
                        IconKind::Specific,
                    )),
                ))
                .unwrap();
        }
        adapter.sync(&ui);
        let id = path_id(&left.join("alpha.txt"));
        let row = || ui.get_left_rows().iter().find(|row| row.id == id).unwrap();
        let icon = row().icon;
        assert!(row().native_icon);
        adapter.select(0, &id, false, false);
        adapter.sync(&ui);
        assert!(row().cursor);
        assert_eq!(row().icon, icon);
        for location in [
            Location::Archive {
                file: root.path().join("archive.zip"),
                prefix: PathBuf::new(),
            },
            Location::Remote {
                connection: "test".into(),
                path: "/".into(),
            },
        ] {
            adapter.app.panes[0].tab_mut().location = location;
            adapter.sync(&ui);
            assert!(
                ui.get_left_rows()
                    .iter()
                    .all(|row| !row.native_icon && row.icon.size().width == 0)
            );
            assert!(
                requests.try_recv().is_err(),
                "virtual paths must never reach native lookup"
            );
        }
    }

    #[test]
    fn single_pane_requests_icons_for_the_active_side() {
        i_slint_backend_testing::init_no_event_loop();
        let (_root, mut adapter, _left, right) = super::super::tests::fixture();
        std::fs::write(right.join("right.txt"), "right").unwrap();
        adapter.app.panes[1].tab_mut().entries =
            filesystem::list(&Location::Local(right.clone())).unwrap();
        let (sender, requests) = mpsc::sync_channel::<Request>(32);
        adapter.icons = Cache::new(Some(sender));
        adapter.app.preferences.single_pane = true;
        adapter.app.active = 1;
        let ui = CommanderWindow::new().unwrap();
        adapter.sync(&ui);
        assert_eq!(
            requests.try_recv().unwrap().key.path,
            right.join("right.txt")
        );
        assert!(requests.try_recv().is_err());
        assert!(adapter.icon_rows[0].is_empty());
    }

    #[test]
    fn generic_native_icons_only_yield_to_specific_builtins() {
        for kind in [IconKind::Specific, IconKind::Generic, IconKind::Unknown] {
            let (mut cache, requests) = fixture();
            let key = key("/file.xml");
            cache.get(&key, true);
            let request = requests.try_recv().unwrap();
            request
                .reply
                .send(Response::Loaded(
                    key.clone(),
                    Some(crate::native_icons::NativeIcon::new(
                        image::RgbaImage::from_pixel(64, 64, image::Rgba([200, 60, 20, 255])),
                        kind,
                    )),
                ))
                .unwrap();
            cache.prepare(&HashSet::from([key.clone()]));
            assert_eq!(
                cache.get(&key, true).size().width > 0,
                kind != IconKind::Generic
            );
            assert!(
                cache.get(&key, false).size().width > 0,
                "retain generic OS icons when no specific built-in matches"
            );
            assert!(requests.try_recv().is_err());
        }
    }

    #[test]
    fn builtin_only_never_queries_native_and_late_results_cannot_override_it() {
        i_slint_backend_testing::init_no_event_loop();
        let (_root, mut adapter, _left, _right) = super::super::tests::fixture();
        let (sender, requests) = mpsc::sync_channel::<Request>(32);
        adapter.icons = Cache::new(Some(sender));
        adapter.app.preferences.file_icon_source = FileIconSource::BuiltIn;
        let ui = CommanderWindow::new().unwrap();
        adapter.sync(&ui);
        assert!(requests.try_recv().is_err());
        assert!(
            ui.get_left_rows()
                .iter()
                .all(|r| !r.native_icon && !r.fallback_icon.is_empty())
        );
        adapter.app.preferences.file_icon_source = FileIconSource::SystemWithFallback;
        adapter.sync(&ui);
        let pending: Vec<_> = requests.try_iter().collect();
        assert!(!pending.is_empty());
        adapter.app.preferences.file_icon_source = FileIconSource::BuiltIn;
        adapter.sync(&ui);
        for request in pending {
            assert!(!request.wanted.load(Ordering::Relaxed));
            request
                .reply
                .send(Response::Loaded(
                    request.key,
                    Some(crate::native_icons::NativeIcon::new(
                        image::RgbaImage::from_pixel(64, 64, image::Rgba([200, 60, 20, 255])),
                        IconKind::Specific,
                    )),
                ))
                .unwrap();
        }
        adapter.sync(&ui);
        assert!(
            ui.get_left_rows()
                .iter()
                .all(|r| !r.native_icon && r.icon.size().width == 0)
        );
        assert!(requests.try_recv().is_err());
        adapter.app.preferences.file_icon_source = FileIconSource::SystemWithFallback;
        adapter.sync(&ui);
        assert!(
            requests.try_recv().is_ok(),
            "re-enabling system icons must retry discarded work"
        );
    }

    #[test]
    fn file_icon_settings_apply_and_cancel_preserve_the_existing_contract() {
        i_slint_backend_testing::init_no_event_loop();
        let (_root, mut adapter, _left, _right) = super::super::tests::fixture();
        adapter.action("Preferences");
        adapter.dialog_kind = "preferences".into();
        adapter.edit_field("file-icons", "Built-in icons only");
        assert_eq!(
            adapter.app.preferences.file_icon_source,
            FileIconSource::SystemWithFallback
        );
        adapter.dialog_action("cancel");
        assert_eq!(
            adapter.app.preferences.file_icon_source,
            FileIconSource::SystemWithFallback
        );
        adapter.action("Preferences");
        adapter.edit_field("file-icons", "Built-in icons only");
        adapter.dialog_action("accept");
        assert!(adapter.app.preferences_form.is_none());
        assert_eq!(
            adapter.app.preferences.file_icon_source,
            FileIconSource::BuiltIn
        );
    }
}
