//! Slint presentation adapter. File operations and their guards stay in Commander.
use super::*;
use base64::Engine;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, rc::Rc};

slint::include_modules!();
mod dialogs;
mod enhancements;
mod keys;
mod parity;
pub(in crate::app) mod viewer_tools;
mod windows;

#[derive(Clone, PartialEq)]
struct RowData {
    id: String,
    path: PathBuf,
    name: String,
    cells: Vec<String>,
    directory: bool,
    selected: bool,
    cursor: bool,
    payload: String,
}

pub(super) struct Adapter {
    app: Commander,
    enhancements: enhancements::State,
    extra: parity::State,
    ctx: UiSignal,
    rows: [Rc<VecModel<FileRow>>; 2],
    row_cache: [Vec<RowData>; 2],
    listing_stamp: [String; 2],
    breadcrumbs: [Vec<Location>; 2],
    breadcrumb_stamp: [String; 2],
    selection_stamp: [u64; 2],
    pending_native_drag: Option<Vec<PathBuf>>,
    pending_drops: Vec<PathBuf>,
    pending_drop_side: Option<usize>,
    drop_deadline: Option<std::time::Instant>,
    fields: Rc<VecModel<FormField>>,
    field_cache: Vec<dialogs::Field>,
    form_rows: Rc<VecModel<FormRow>>,
    form_row_cache: Vec<dialogs::Row>,
    dialog_buttons: Rc<VecModel<Command>>,
    dialog_button_cache: Vec<(String, String, bool)>,
    places: Vec<(String, Location)>,
    place_cache: Vec<(String, String, bool)>,
    toolbar_cache: Vec<(String, String, bool)>,
    columns_cache: Vec<(String, f32)>,
    columns: Rc<VecModel<FileColumn>>,
    tabs_cache: [Vec<String>; 2],
    dialog_kind: String,
    field_error: Option<String>,
    invalid_fields: std::collections::BTreeMap<String, (String, String)>,
    last_terminal: String,
    viewer_stamp: Option<(String, usize)>,
    preview_stamp: Option<String>,
    dark: bool,
    window_scheme: Option<crate::appearance::ColorScheme>,
}

fn strings(values: impl IntoIterator<Item = impl Into<SharedString>>) -> ModelRc<SharedString> {
    Rc::new(VecModel::from(
        values.into_iter().map(Into::into).collect::<Vec<_>>(),
    ))
    .into()
}

fn publish<T: Clone + 'static, S: PartialEq>(
    model: &VecModel<T>,
    cache: &mut Vec<S>,
    next: Vec<S>,
    convert: impl Fn(&S) -> T,
) {
    if cache.len() != next.len() {
        model.set_vec(next.iter().map(&convert).collect::<Vec<_>>());
    } else {
        for (index, (old, new)) in cache.iter().zip(&next).enumerate() {
            if old != new {
                model.set_row_data(index, convert(new));
            }
        }
    }
    *cache = next;
}

fn path_id(path: &Path) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(path.as_os_str().as_encoded_bytes())
}

pub(crate) fn supported_commands() -> Vec<String> {
    parity::command_catalog()
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}
fn action_named(name: &str) -> Option<Action> {
    all_actions().into_iter().find(|a| format!("{a:?}") == name)
}

fn image(image: &crate::viewer::RasterImage) -> slint::Image {
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(
        image.size[0] as u32,
        image.size[1] as u32,
    );
    for (output, input) in pixels.make_mut_slice().iter_mut().zip(&image.pixels) {
        *output = slint::Rgba8Pixel::new(input[0], input[1], input[2], input[3]);
    }
    slint::Image::from_rgba8(pixels)
}

impl Adapter {
    fn new(paths: &[PathBuf], loaded: anyhow::Result<Settings>) -> Self {
        let ctx = UiSignal::default();
        let app = Commander::with_settings(&ctx, paths, loaded);
        Self {
            app,
            enhancements: Default::default(),
            extra: Default::default(),
            ctx,
            rows: std::array::from_fn(|_| Rc::new(VecModel::default())),
            row_cache: Default::default(),
            listing_stamp: Default::default(),
            breadcrumbs: Default::default(),
            breadcrumb_stamp: Default::default(),
            selection_stamp: [0, 0],
            pending_native_drag: None,
            pending_drops: Vec::new(),
            pending_drop_side: None,
            drop_deadline: None,
            fields: Rc::new(VecModel::default()),
            field_cache: Vec::new(),
            form_rows: Rc::new(VecModel::default()),
            form_row_cache: Vec::new(),
            dialog_buttons: Rc::new(VecModel::default()),
            dialog_button_cache: Vec::new(),
            places: Vec::new(),
            place_cache: Vec::new(),
            toolbar_cache: Vec::new(),
            columns_cache: Vec::new(),
            columns: Rc::new(VecModel::default()),
            tabs_cache: Default::default(),
            dialog_kind: String::new(),
            field_error: None,
            invalid_fields: Default::default(),
            last_terminal: String::new(),
            viewer_stamp: None,
            preview_stamp: None,
            dark: false,
            window_scheme: None,
        }
    }

    fn blocked(&self) -> bool {
        self.enhancements_modal()
            || self.extra.dialog.is_some()
            || self.app.features.active()
            || self.app.prompt.is_some()
            || self.app.editor.is_some()
            || self.app.help
            || self.app.message.is_some()
            || self.app.confirm_exit
            || self.app.connection_form.is_some()
            || self.app.archive_form.is_some()
            || self.app.show_history
            || self.app.tools.active()
            || self.app.preferences_form.is_some()
    }

    fn entry(&self, side: usize, id: &str) -> Option<Entry> {
        self.app
            .panes
            .get(side)?
            .tab()
            .entries
            .iter()
            .find(|entry| path_id(&entry.path) == id)
            .cloned()
    }

    fn select(&mut self, side: usize, id: &str, toggle: bool, range: bool) {
        if self.blocked() || side > 1 {
            return;
        }
        let Some(entry) = self.entry(side, id) else {
            return;
        };
        self.app.active = side;
        let hidden = self.app.show_hidden;
        let tab = self.app.tab_mut();
        if range {
            let visible = tab.visible(hidden);
            let start = visible
                .iter()
                .position(|&i| Some(&tab.entries[i].path) == tab.cursor.as_ref())
                .unwrap_or(0);
            if let Some(end) = visible
                .iter()
                .position(|&i| tab.entries[i].path == entry.path)
            {
                for &i in &visible[start.min(end)..=start.max(end)] {
                    tab.selected.insert(tab.entries[i].path.clone());
                }
            }
        } else if toggle {
            if !tab.selected.remove(&entry.path) {
                tab.selected.insert(entry.path.clone());
            }
        } else if !tab.selected.contains(&entry.path) {
            tab.selected.clear();
        }
        tab.cursor = Some(entry.path);
    }

    fn open(&mut self, side: usize, id: &str) {
        if self.blocked() || side > 1 {
            return;
        }
        if let Some(entry) = self.entry(side, id) {
            self.app.active = side;
            self.app.open_entry(entry, &self.ctx);
        }
    }

    fn action(&mut self, id: &str) {
        let (name, side) = id.split_once(':').map_or((id, None), |(name, side)| {
            (name, side.parse::<usize>().ok())
        });
        if name == "SaveText" {
            self.viewer_action("save");
            return;
        }
        if name == "CloseWindow" {
            self.request_close();
            return;
        }
        if name == "Quit" {
            self.extra.window_request = Some(windows::Request::Quit);
            return;
        }
        if name == "CancelJob" {
            if let Some(job) = &self.app.job {
                job.cancel.store(true, Ordering::Relaxed);
            }
            return;
        }
        if self.blocked() || self.app.exiting {
            return;
        }
        match name {
            "RemoveBookmark" => {
                if let Some(index) = side
                    && index < self.app.bookmarks.len()
                {
                    self.app.bookmarks.remove(index);
                    self.app.save_settings();
                }
                return;
            }
            "EditConnection" => {
                if let Some(index) = side
                    && let Some(connection) = self.app.connections.get(index).cloned()
                {
                    self.app.connection_form = Some(ConnectionForm {
                        connection,
                        ..Default::default()
                    });
                }
                return;
            }
            "ForgetCredentials" => {
                if let Some(index) = side
                    && let Some(connection) = self.app.connections.get(index)
                    && let Err(error) = crate::remote::forget_credentials(&connection.id)
                {
                    self.app.fail(error);
                }
                return;
            }
            _ => {}
        }
        if let Some(side) = side.filter(|&side| side < 2) {
            self.app.active = side;
        }
        if self.enhancements_action(name) {
            return;
        }
        if self.parity_action(name) {
            return;
        }
        if name == "Back" || name == "Forward" {
            self.app.tab_mut().history(name == "Forward", &self.ctx);
        } else if name == "NativeDrag" {
            if self.app.tab().location.local().is_some() {
                self.pending_native_drag = Some(
                    self.app
                        .tab()
                        .chosen(self.app.show_hidden)
                        .iter()
                        .map(|entry| entry.path.clone())
                        .collect(),
                );
            } else {
                self.app.fail("Native drag-out requires local files.");
            }
        } else if let Some(action) = action_named(name) {
            self.app.act(action, &self.ctx);
        }
    }

    fn navigate(&mut self, side: usize, text: &str) {
        if self.blocked() || side > 1 {
            return;
        }
        self.app.active = side;
        match filesystem::parse_location(text, &self.app.tab().location) {
            Ok(location) => self.app.tab_mut().navigate(location, &self.ctx),
            Err(error) => self.app.fail(error),
        }
    }

    fn drop_files(&mut self, side: usize, data: slint::DataTransfer) -> bool {
        if side > 1 || self.blocked() || self.app.job.is_some() {
            return false;
        }
        let Ok(paths) = data.file_paths() else {
            return false;
        };
        let paths = paths.map(Path::to_owned).collect::<Vec<_>>();
        if paths.is_empty() || paths.iter().any(|path| !path.is_absolute()) {
            return false;
        }
        let destination = self.app.panes[side].tab().location.clone();
        self.app.drop_files(side, paths, destination, &self.ctx);
        // The drop opens a confirmation; no source is removed until the operation succeeds.
        true
    }

    fn request_close(&mut self) {
        if self
            .app
            .job
            .as_ref()
            .is_some_and(|job| job.owner == self.app.window_id)
            || self.app.editor.as_ref().is_some_and(Editor::dirty)
            || self.enhancements.external.is_some()
        {
            self.app.confirm_exit = true;
        } else {
            self.app.exiting = true;
        }
    }

    fn tick(&mut self, ui: &CommanderWindow) {
        #[cfg(target_os = "macos")]
        use slint::winit_030::WinitWindowAccessor;
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if self.app.native_window.is_none() {
            use raw_window_handle::HasWindowHandle;
            if let Ok(handle) = ui.window().window_handle().window_handle() {
                self.app.native_window = Some(crate::native_files::Window(handle.as_raw()));
            }
        }
        #[cfg(target_os = "macos")]
        if self.app.native_window.is_some() && self.app.native_menu.is_none() {
            match crate::native_menu::NativeMenu::new(&self.ctx) {
                Ok(menu) => self.app.native_menu = Some(menu),
                Err(error) => self.app.fail(error),
            }
        }
        #[cfg(target_os = "macos")]
        if ui
            .window()
            .with_winit_window(|window| window.has_focus())
            .unwrap_or(false)
            && let Some(menu) = &self.app.native_menu
        {
            menu.update_bindings(&self.app.preferences);
            let actions = menu.poll(self.blocked() || self.app.exiting);
            for action in actions {
                self.action(&format!("{action:?}"));
            }
        }
        if self
            .drop_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            self.drop_deadline = None;
            let paths = std::mem::take(&mut self.pending_drops);
            if let Some(side) = self.pending_drop_side.take() {
                let mut data = slint::DataTransfer::default();
                data.set_file_paths(paths);
                self.drop_files(side, data);
            }
        }
        self.app.poll(&self.ctx);
        self.enhancements_tick();
        self.viewer_tools_tick();
        self.parity_tick();
        self.app.poll_prompt(&self.ctx);
        self.poll_dialogs();
        if self.app.preferences.preview {
            let entry = self
                .app
                .tab()
                .entries
                .iter()
                .find(|e| Some(&e.path) == self.app.tab().cursor.as_ref())
                .cloned();
            self.app
                .preview
                .select(self.app.tab().location.clone(), entry);
            self.app.preview.poll(&self.ctx);
        }
        if let Some(terminal) = &mut self.app.terminal {
            terminal.poll();
        }
        let (clipboard, close) = self.ctx.take_events();
        for text in clipboard {
            if let Err(error) = self.app.native_files.copy_text(text) {
                self.app.fail(error);
            }
        }
        if close {
            self.request_close();
        }
        self.sync(ui);
    }

    fn sync(&mut self, ui: &CommanderWindow) {
        self.sync_tree(ui);
        let was_modal = !ui.get_dialog_title().is_empty() || ui.get_viewer_visible();
        use slint::winit_030::WinitWindowAccessor;
        if self.window_scheme != Some(self.app.preferences.color_scheme) {
            let theme = match self.app.preferences.color_scheme {
                crate::appearance::ColorScheme::System => None,
                crate::appearance::ColorScheme::Dark => {
                    Some(slint::winit_030::winit::window::Theme::Dark)
                }
                crate::appearance::ColorScheme::Light => {
                    Some(slint::winit_030::winit::window::Theme::Light)
                }
            };
            if ui
                .window()
                .with_winit_window(|window| window.set_theme(theme))
                .is_some()
            {
                self.window_scheme = Some(self.app.preferences.color_scheme);
            }
        }
        let system_dark = ui
            .window()
            .with_winit_window(|window| {
                window
                    .theme()
                    .is_some_and(|theme| theme == slint::winit_030::winit::window::Theme::Dark)
            })
            .unwrap_or(false);
        self.dark = match self.app.preferences.color_scheme {
            crate::appearance::ColorScheme::Dark => true,
            crate::appearance::ColorScheme::Light => false,
            _ => system_dark,
        };
        ui.set_native_menus(cfg!(target_os = "macos"));
        ui.global::<Words>()
            .set_version(self.app.preferences.language as i32);
        ui.set_dark(self.dark);
        ui.global::<Chrome>().set_dark(self.dark);
        ui.global::<Palette>()
            .set_color_scheme(match self.app.preferences.color_scheme {
                crate::appearance::ColorScheme::System => slint::language::ColorScheme::Unknown,
                crate::appearance::ColorScheme::Dark => slint::language::ColorScheme::Dark,
                crate::appearance::ColorScheme::Light => slint::language::ColorScheme::Light,
            });

        let palette =
            crate::appearance::Palette::new(self.app.preferences.desktop_style, self.dark);
        ui.set_accent(palette.accent);
        ui.global::<Chrome>().set_accent(palette.accent);
        ui.global::<Chrome>().set_selection(palette.selection);
        ui.global::<Chrome>()
            .set_selection_text(palette.selection_text);
        ui.set_active_location(self.app.tab().location.label().into());
        ui.set_selection_color(palette.selection);
        ui.set_selection_text(palette.selection_text);
        ui.set_inactive_selection(palette.inactive_selection);
        ui.set_text_size(self.app.preferences.font_size);
        ui.set_toolbar_visible(self.app.preferences.toolbar);
        #[cfg(target_os = "macos")]
        if let Some(window) = &self.app.native_window {
            crate::native_files::align_window_controls(
                window.0,
                f64::from(ui.get_native_header_height()),
            );
        }
        self.parity_sync(ui);
        ui.set_command_bar_visible(self.app.preferences.command_bar);
        ui.set_status_visible(self.app.preferences.status_bar);
        ui.set_single_pane(self.app.preferences.single_pane);
        let active_changed = ui.get_active_pane() != self.app.active as i32;
        ui.set_active_pane(self.app.active as i32);
        ui.set_busy(self.app.job.is_some());
        ui.set_show_hidden(self.app.show_hidden);
        ui.set_progress(0.0);
        let status = if let Some(job) = &self.app.job
            && !self.app.status_error
        {
            format!(
                "{} · {} · {} files · {}",
                tr(&job.label),
                job.progress.current,
                job.progress.files,
                filesystem::human_size(job.progress.bytes)
            )
        } else {
            tr(
                if self.app.status == "Ready · Select files to get started" {
                    "Ready"
                } else {
                    &self.app.status
                },
            )
            .into_owned()
        };
        ui.set_status(status.into());
        ui.set_status_error(self.app.status_error);
        let toolbar = self
            .app
            .preferences
            .toolbar_actions
            .iter()
            .map(|action| {
                (
                    format!("{:?}", action.action()),
                    tr(action.label()).into_owned(),
                    !self.blocked(),
                )
            })
            .collect::<Vec<_>>();
        if self.toolbar_cache != toolbar {
            ui.set_toolbar(
                Rc::new(VecModel::from(
                    toolbar
                        .iter()
                        .map(|(id, title, enabled)| Command {
                            id: id.into(),
                            title: title.into(),
                            enabled: *enabled,
                        })
                        .collect::<Vec<_>>(),
                ))
                .into(),
            );
            self.toolbar_cache = toolbar;
        }
        let columns = self
            .app
            .preferences
            .columns
            .iter()
            .map(|column| (tr(column.kind.label()).into_owned(), column.width))
            .collect::<Vec<_>>();
        publish(
            &self.columns,
            &mut self.columns_cache,
            columns,
            |(title, width)| FileColumn {
                title: title.as_str().into(),
                width: *width,
            },
        );
        ui.set_columns(self.columns.clone().into());
        for side in 0..2 {
            let tab = self.app.panes[side].tab();
            if self.breadcrumb_stamp[side] != tab.location.label() {
                let crumbs = location_breadcrumbs(&tab.location);
                let model = Rc::new(VecModel::from(
                    crumbs
                        .iter()
                        .map(|(title, _)| Place {
                            id: Default::default(),
                            title: title.as_str().into(),
                            detail: Default::default(),
                            heading: false,
                        })
                        .collect::<Vec<_>>(),
                ));
                if side == 0 {
                    ui.set_left_breadcrumbs(model.into());
                } else {
                    ui.set_right_breadcrumbs(model.into());
                }
                self.breadcrumbs[side] = crumbs.into_iter().map(|(_, location)| location).collect();
                self.breadcrumb_stamp[side] = tab.location.label();
            }
            let visible = tab.visible(self.app.show_hidden);
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            tab.selected.hash(&mut hasher);
            tab.cursor.hash(&mut hasher);
            let selection = hasher.finish();
            let listing = format!(
                "{}:{}:{}:{:?}:{}:{}:{}:{:?}:{:?}",
                tab.id,
                tab.revision,
                tab.location.label(),
                tab.sort,
                tab.descending,
                self.app.show_hidden,
                tab.filter,
                self.app
                    .preferences
                    .columns
                    .iter()
                    .map(|c| c.kind)
                    .collect::<Vec<_>>(),
                self.app.preferences.language
            );
            if self.listing_stamp[side] != listing {
                let rows = visible
                    .iter()
                    .map(|&index| {
                        let entry = &tab.entries[index];
                        RowData {
                            id: path_id(&entry.path),
                            path: entry.path.clone(),
                            name: entry.name.clone(),
                            cells: self
                                .app
                                .preferences
                                .columns
                                .iter()
                                .map(|column| {
                                    column.kind.value(entry, tab.location.local().is_some())
                                })
                                .collect(),
                            directory: entry.is_dir,
                            selected: tab.selected.contains(&entry.path),
                            cursor: tab.cursor.as_ref() == Some(&entry.path),
                            payload: if tab.location.local().is_some() {
                                url::Url::from_file_path(&entry.path)
                                    .map(|url| url.to_string())
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            },
                        }
                    })
                    .collect();
                publish(&self.rows[side], &mut self.row_cache[side], rows, |row| {
                    FileRow {
                        id: row.id.as_str().into(),
                        name: row.name.as_str().into(),
                        cells: strings(row.cells.iter().map(String::as_str)),
                        directory: row.directory,
                        selected: row.selected,
                        cursor: row.cursor,
                        payload: row.payload.as_str().into(),
                    }
                });
            } else if self.selection_stamp[side] != selection {
                for (index, row) in self.row_cache[side].iter_mut().enumerate() {
                    let selected = tab.selected.contains(&row.path);
                    let cursor = tab.cursor.as_ref() == Some(&row.path);
                    if selected != row.selected || cursor != row.cursor {
                        row.selected = selected;
                        row.cursor = cursor;
                        if let Some(mut item) = self.rows[side].row_data(index) {
                            item.selected = selected;
                            item.cursor = cursor;
                            self.rows[side].set_row_data(index, item);
                        }
                    }
                }
            }
            if self.selection_stamp[side] != selection || self.listing_stamp[side] != listing {
                let payload = if tab.location.local().is_some() {
                    tab.chosen(self.app.show_hidden)
                        .iter()
                        .filter_map(|entry| url::Url::from_file_path(&entry.path).ok())
                        .map(|url| url.to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    String::new()
                };
                if side == 0 {
                    ui.set_left_selection_payload(payload.into());
                } else {
                    ui.set_right_selection_payload(payload.into());
                }
            }
            self.listing_stamp[side] = listing;
            self.selection_stamp[side] = selection;
            let tabs = self.app.panes[side]
                .tabs
                .iter()
                .map(parity::tab_label)
                .collect::<Vec<_>>();
            if tabs != self.tabs_cache[side] {
                if side == 0 {
                    ui.set_left_tabs(strings(tabs.iter().map(String::as_str)));
                } else {
                    ui.set_right_tabs(strings(tabs.iter().map(String::as_str)));
                }
                self.tabs_cache[side] = tabs;
            }
            let cursor = visible
                .iter()
                .position(|&index| tab.cursor.as_ref() == Some(&tab.entries[index].path))
                .map(|index| index as i32)
                .unwrap_or(-1);
            if side == 0 {
                ui.set_left_cursor(cursor);
            } else {
                ui.set_right_cursor(cursor);
            }
            if tab.scroll_to_cursor {
                ui.invoke_scroll_to_cursor(side as i32);
            }
            let item_label = tr(if visible.len() == 1 { "item" } else { "items" });
            let summary = if tab.selected.is_empty() {
                format!("{} {}", visible.len(), item_label)
            } else {
                format!(
                    "{} {} · {} {}",
                    visible.len(),
                    item_label,
                    tab.selected.len(),
                    tr("selected")
                )
            };
            if side == 0 {
                ui.set_left_rows(self.rows[side].clone().into());
                ui.set_left_tab(self.app.panes[side].active as i32);
                if !tab.focus_address {
                    ui.set_left_address(tab.address.as_str().into());
                }
                ui.set_left_filter(tab.filter.as_str().into());
                ui.set_left_error(tab.error.clone().unwrap_or_default().into());
                ui.set_left_busy(tab.loading);
                ui.set_left_summary(summary.into());
            } else {
                ui.set_right_rows(self.rows[side].clone().into());
                ui.set_right_tab(self.app.panes[side].active as i32);
                if !tab.focus_address {
                    ui.set_right_address(tab.address.as_str().into());
                }
                ui.set_right_filter(tab.filter.as_str().into());
                ui.set_right_error(tab.error.clone().unwrap_or_default().into());
                ui.set_right_busy(tab.loading);
                ui.set_right_summary(summary.into());
            }
            if tab.focus_address {
                ui.invoke_focus_address(side as i32);
            }
            if tab.focus_filter {
                ui.invoke_focus_filter(side as i32);
            }
            self.app.panes[side].tab_mut().focus_address = false;
            self.app.panes[side].tab_mut().focus_filter = false;
            self.app.panes[side].tab_mut().scroll_to_cursor = false;
        }
        if active_changed && !self.blocked() {
            ui.invoke_focus_files(self.app.active as i32);
        }
        self.sync_places(ui);
        self.sync_viewer(ui);
        self.sync_terminal(ui);
        let mut spec = self.dialog();
        if self.dialog_kind != spec.kind {
            self.field_error = None;
            self.invalid_fields.clear();
            self.dialog_kind = spec.kind.clone();
        }
        for field in &mut spec.fields {
            if let Some((value, _)) = self.invalid_fields.get(&field.id) {
                field.value = value.clone();
            }
        }
        ui.set_dialog_preferred_height(match spec.kind.as_str() {
            "exit" => 240.0,
            "prompt" => (spec.fields.len() as f32 * 64.0 + 180.0).clamp(260.0, 700.0),
            "open-with" => 300.0,
            "workspaces" => 480.0,
            "archive" => 520.0,
            _ => 700.0,
        });
        ui.set_dialog_title(spec.title.into());
        ui.set_dialog_description(spec.description.into());
        ui.set_dialog_error(
            self.invalid_fields
                .values()
                .next()
                .map(|(_, error)| error.clone())
                .or_else(|| self.field_error.clone())
                .unwrap_or(spec.error)
                .into(),
        );
        ui.set_dialog_busy(spec.busy);
        publish(
            &self.fields,
            &mut self.field_cache,
            spec.fields,
            dialogs::Field::slint,
        );
        publish(
            &self.form_rows,
            &mut self.form_row_cache,
            spec.rows,
            dialogs::Row::slint,
        );
        ui.set_dialog_fields(self.fields.clone().into());
        ui.set_dialog_rows(self.form_rows.clone().into());
        let buttons = spec
            .buttons
            .into_iter()
            .map(|(id, title, enabled)| (id, tr(&title).into_owned(), enabled))
            .collect::<Vec<_>>();
        publish(
            &self.dialog_buttons,
            &mut self.dialog_button_cache,
            buttons,
            |(id, title, enabled)| Command {
                id: id.into(),
                title: title.into(),
                enabled: *enabled,
            },
        );
        ui.set_dialog_buttons(self.dialog_buttons.clone().into());
        if was_modal && !self.blocked() {
            // Removing a focused dialog otherwise leaves keyboard input without
            // a target (for example Cmd+Z immediately after confirming Copy).
            ui.invoke_focus_files(self.app.active as i32);
        }
    }

    fn sync_places(&mut self, ui: &CommanderWindow) {
        let mut entries = vec![("".into(), tr("Places").into_owned(), true)];
        self.places.clear();
        for (name, path) in [
            ("Home", filesystem::home()),
            ("Desktop", filesystem::home().join("Desktop")),
            ("Documents", filesystem::home().join("Documents")),
            ("Downloads", filesystem::home().join("Downloads")),
            (
                "Computer",
                PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" }),
            ),
        ] {
            let id = format!("place:{}", self.places.len());
            entries.push((id.clone(), tr(name).into_owned(), false));
            self.places.push((id, Location::Local(path)));
        }
        entries.push((String::new(), tr("Favorites").into_owned(), true));
        for (index, bookmark) in self.app.bookmarks.iter().enumerate() {
            let id = format!("bookmark:{index}");
            entries.push((id.clone(), bookmark.name.clone(), false));
            self.places.push((id, bookmark.location.clone()));
        }
        entries.push((String::new(), tr("Devices").into_owned(), true));
        for (name, path) in crate::native_files::mounted_drives() {
            let id = format!("place:{}", self.places.len());
            entries.push((id.clone(), name, false));
            self.places.push((id, Location::Local(path)));
        }
        entries.push((String::new(), tr("Connections").into_owned(), true));
        for (index, connection) in self.app.connections.iter().enumerate() {
            let id = format!("connection:{index}");
            entries.push((id.clone(), connection.name.clone(), false));
            self.places.push((
                id,
                Location::Remote {
                    connection: connection.id.clone(),
                    path: PathBuf::new(),
                },
            ));
        }
        entries.push(("Connections".into(), tr("Connections…").into_owned(), false));
        if entries != self.place_cache {
            ui.set_places(
                Rc::new(VecModel::from(
                    entries
                        .iter()
                        .map(|(id, title, heading)| Place {
                            id: id.into(),
                            title: title.into(),
                            detail: self
                                .places
                                .iter()
                                .find(|(key, _)| key == id)
                                .map(|(_, location)| location.label())
                                .unwrap_or_default()
                                .into(),
                            heading: *heading,
                        })
                        .collect::<Vec<_>>(),
                ))
                .into(),
            );
            self.place_cache = entries;
        }
    }

    fn sync_viewer(&mut self, ui: &CommanderWindow) {
        let opening = self.app.editor.is_some() && !ui.get_viewer_visible();
        ui.set_viewer_visible(self.app.editor.is_some());
        if let Some(editor) = &self.app.editor {
            ui.set_viewer_title(format!("{} · {}", editor.title, editor.kind).into());
            ui.set_viewer_pdf_document(editor.pdf.is_some());
            ui.set_viewer_locked(editor.pdf.as_ref().is_some_and(|p| p.locked));
            ui.set_viewer_busy(
                self.app.pending_pdf.is_some() || editor.navigation.pending.is_some(),
            );
            ui.set_viewer_notice(editor.navigation.notice.as_str().into());
            ui.set_viewer_fit_width(editor.navigation.fit_width);
            ui.set_viewer_text(editor.text.as_str().into());
            ui.set_viewer_editable(editor.editable);
            ui.set_viewer_dirty(editor.dirty());
            ui.set_viewer_save_key(
                self.app
                    .preferences
                    .shortcuts
                    .get("SaveText")
                    .map(|binding| parity::display_binding(binding))
                    .unwrap_or_default()
                    .into(),
            );
            ui.set_viewer_confirm_close(editor.confirm_close);
            ui.set_viewer_error(editor.error.clone().unwrap_or_default().into());
            ui.set_viewer_text_controls(editor.image.is_none() && editor.pdf.is_none());
            ui.set_viewer_encoding(
                editor
                    .document
                    .as_ref()
                    .map(|d| d.encoding)
                    .or_else(|| editor.text_page.as_ref().map(|p| p.document.encoding))
                    .unwrap_or_default()
                    .label()
                    .into(),
            );
            ui.set_viewer_save_encoding(
                editor
                    .document
                    .as_ref()
                    .map(|d| d.encoding)
                    .unwrap_or_default()
                    .label()
                    .into(),
            );
            ui.set_viewer_pdf(editor.pdf.is_some() || editor.text_page.is_some());
            ui.set_viewer_has_previous(
                (editor.pdf.as_ref().is_some_and(|pdf| pdf.page > 0)
                    || editor.text_page.as_ref().is_some_and(|p| p.offset > 0))
                    && self.app.pending_pdf.is_none(),
            );
            ui.set_viewer_has_next(
                editor
                    .pdf
                    .as_ref()
                    .is_some_and(|pdf| pdf.page + 1 < pdf.pages)
                    || editor.text_page.as_ref().is_some_and(|p| p.next < p.total)
                        && self.app.pending_pdf.is_none(),
            );
            ui.set_viewer_page(
                editor
                    .pdf
                    .as_ref()
                    .map(|pdf| format!("{} / {}", pdf.page + 1, pdf.pages))
                    .or_else(|| {
                        editor
                            .text_page
                            .as_ref()
                            .map(|p| format!("Bytes {}–{} / {}", p.offset, p.next, p.total))
                    })
                    .unwrap_or_default()
                    .into(),
            );
            let stamp = (
                format!("{}:{}", editor.title, editor.image_revision),
                editor.pdf.as_ref().map(|pdf| pdf.page).unwrap_or(0),
            );
            if self.viewer_stamp.as_ref() != Some(&stamp) {
                ui.set_viewer_zoom(editor.zoom);
                ui.set_viewer_image(editor.image.as_ref().map(image).unwrap_or_default());
                self.viewer_stamp = Some(stamp);
            }
        } else {
            self.viewer_stamp = None;
        }
        if let Some(selection) = self
            .app
            .editor
            .as_mut()
            .and_then(|e| e.navigation.selection.take())
        {
            ui.invoke_select_viewer(selection.0, selection.1);
        }
        if opening {
            ui.invoke_focus_viewer();
        }
        ui.set_preview_visible(self.app.preferences.preview);
        if self.app.preferences.preview {
            ui.set_preview_title(
                self.app
                    .preview
                    .target
                    .as_ref()
                    .map(|(_, entry)| entry.name.clone())
                    .unwrap_or_else(|| tr("Preview").into_owned())
                    .into(),
            );
            ui.set_preview_text(self.app.preview.text.as_str().into());
            let stamp = self.app.preview.target.as_ref().map(|(location, entry)| {
                format!(
                    "{}:{:?}:{:?}:{}:{}",
                    location.label(),
                    entry.path,
                    entry.modified,
                    entry.size,
                    self.app.preview.image.is_some()
                )
            });
            if self.preview_stamp != stamp {
                ui.set_preview_image(
                    self.app
                        .preview
                        .image
                        .as_ref()
                        .map(image)
                        .unwrap_or_default(),
                );
                self.preview_stamp = stamp;
            }
        }
    }

    fn sync_terminal(&mut self, ui: &CommanderWindow) {
        let was_visible = ui.get_terminal_visible();
        ui.set_terminal_visible(self.app.terminal_visible);
        if !self.app.terminal_visible {
            ui.set_terminal_focused(false);
            if was_visible {
                ui.invoke_focus_files(self.app.active as i32);
            }
        }
        let Some(terminal) = &mut self.app.terminal else {
            return;
        };
        if !self.app.terminal_visible {
            return;
        }
        let cols = (ui.window().size().width as f32 / ui.window().scale_factor() / 8.0)
            .clamp(20.0, 240.0) as u16;
        let _ = terminal.resize(12, cols);
        terminal.focused = ui.get_terminal_focused();
        ui.set_terminal_status(terminal.status.as_str().into());
        let screen = terminal.screen();
        let stamp = format!(
            "{}:{:?}",
            String::from_utf8_lossy(&screen.contents_formatted()),
            screen.cursor_position()
        );
        if self.last_terminal == stamp {
            return;
        }
        self.last_terminal = stamp;
        let mut lines = Vec::new();
        for row in 0..12 {
            let mut cells = Vec::new();
            for col in 0..cols {
                if let Some(cell) = screen.cell(row, col) {
                    cells.push(TerminalCell {
                        text: cell.contents().into(),
                        foreground: terminal_color(cell.fgcolor(), true),
                        background: terminal_color(cell.bgcolor(), false),
                        bold: cell.bold(),
                        cursor: screen.cursor_position() == (row, col),
                    });
                }
            }
            lines.push(TerminalRow {
                text: cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
                    .trim_end()
                    .into(),
                cells: Rc::new(VecModel::from(cells)).into(),
            });
        }
        ui.set_terminal_rows(Rc::new(VecModel::from(lines)).into());
    }
}

fn terminal_color(color: vt100::Color, foreground: bool) -> slint::Color {
    match color {
        vt100::Color::Rgb(r, g, b) => slint::Color::from_rgb_u8(r, g, b),
        vt100::Color::Idx(index) => {
            const BASE: [[u8; 3]; 16] = [
                [0, 0, 0],
                [205, 49, 49],
                [13, 188, 121],
                [229, 229, 16],
                [36, 114, 200],
                [188, 63, 188],
                [17, 168, 205],
                [229, 229, 229],
                [102, 102, 102],
                [241, 76, 76],
                [35, 209, 139],
                [245, 245, 67],
                [59, 142, 234],
                [214, 112, 214],
                [41, 184, 219],
                [255, 255, 255],
            ];
            let [r, g, b] = if index < 16 {
                BASE[index as usize]
            } else if index < 232 {
                let n = index - 16;
                let c = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                [c(n / 36), c((n / 6) % 6), c(n % 6)]
            } else {
                let c = 8 + (index - 232) * 10;
                [c, c, c]
            };
            slint::Color::from_rgb_u8(r, g, b)
        }
        _ => {
            if foreground {
                slint::Color::from_rgb_u8(225, 225, 230)
            } else {
                slint::Color::from_rgb_u8(16, 16, 21)
            }
        }
    }
}

impl Drop for Adapter {
    fn drop(&mut self) {
        if let Some(cancel) = &self.app.oauth_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Some(job) = &self.app.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
        self.app.save_settings();
    }
}

pub fn run(paths: &[PathBuf]) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        // Slint otherwise reinstalls its default menu whenever the window gains
        // focus, replacing our commands and their guarded Quit handler.
        use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
        let mut backend = i_slint_backend_winit::Backend::builder()
            .with_default_menu_bar(false)
            .with_window_attributes_hook(|attributes| {
                attributes
                    .with_titlebar_transparent(true)
                    .with_title_hidden(true)
                    .with_fullsize_content_view(true)
            });
        if let Ok(selection) = std::env::var("SLINT_BACKEND")
            && let Some(renderer) = selection.strip_prefix("winit-")
        {
            backend = backend.with_renderer_name(renderer);
        }
        slint::platform::set_platform(Box::new(backend.build()?))?;
    }
    #[cfg(not(target_os = "macos"))]
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .select()?;
    let smoke_ui = windows::restore(paths, settings::load())?;
    #[cfg(not(feature = "screenshot"))]
    let _ = &smoke_ui;
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        windows::tick,
    );
    #[cfg(feature = "screenshot")]
    let screenshot_timer = slint::Timer::default();
    #[cfg(feature = "screenshot")]
    if let Some(path) = std::env::var_os("RUST_COMMANDER_SCREENSHOT_TO") {
        let weak = smoke_ui.as_weak();
        if let Ok(action) = std::env::var("RUST_COMMANDER_SCREENSHOT_ACTION") {
            let action_ui = smoke_ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(750), move || {
                if let Some(ui) = action_ui.upgrade() {
                    if let Some(title) = action.strip_prefix("Menu:") {
                        ui.invoke_show_menu(title.into());
                    } else if let Some(file) = action.strip_prefix("Open:") {
                        ui.invoke_opened(0, path_id(Path::new(file)).into());
                    } else {
                        ui.invoke_action(action.into());
                    }
                }
            });
        }
        if let Ok(action) = std::env::var("RUST_COMMANDER_SCREENSHOT_VIEWER_ACTION") {
            let viewer_ui = smoke_ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(1250), move || {
                if let Some(ui) = viewer_ui.upgrade() {
                    ui.invoke_viewer_action(action.into());
                }
            });
        }
        screenshot_timer.start(
            slint::TimerMode::SingleShot,
            Duration::from_secs(2),
            move || {
                if let Some(ui) = weak.upgrade() {
                    if ui.get_tree_visible() {
                        let rows = ui.get_tree_rows();
                        let selected = rows
                            .iter()
                            .find(|r| r.selected)
                            .map(|r| (r.index, r.title.to_string()));
                        eprintln!(
                            "Tree snapshot: {} rows, selected {selected:?}, scroll {}",
                            rows.row_count(),
                            ui.get_tree_scroll_position()
                        );
                    }
                    let result = ui
                        .window()
                        .take_snapshot()
                        .map_err(anyhow::Error::from)
                        .and_then(|buffer| {
                            ::image::save_buffer(
                                &path,
                                buffer.as_bytes(),
                                buffer.width(),
                                buffer.height(),
                                ::image::ColorType::Rgba8,
                            )
                            .map_err(anyhow::Error::from)
                        });
                    if let Err(error) = result {
                        eprintln!("Could not save Slint snapshot: {error:#}");
                    }
                }
                if std::env::var_os("RUST_COMMANDER_SMOKE_KEEP_OPEN").is_none() {
                    let _ = slint::quit_event_loop();
                }
            },
        );
    }
    slint::run_event_loop_until_quit()?;
    Ok(())
}

fn wire(ui: &CommanderWindow, state: Rc<RefCell<Adapter>>) {
    let drag_ui = ui.as_weak();
    ui.on_window_drag(move || {
        use slint::winit_030::WinitWindowAccessor;
        if let Some(ui) = drag_ui.upgrade() {
            ui.window().with_winit_window(|window| {
                let _ = window.drag_window();
            });
        }
    });
    ui.global::<Words>()
        .on_translate(|text, _version| tr(&text).into_owned().into());
    let mut selected_payload = String::new();
    let mut selected_data = slint::DataTransfer::default();
    ui.global::<Transfers>().on_payload(move |text| {
        if text.contains('\n') && selected_payload == text.as_str() {
            return selected_data.clone();
        }
        let mut data = slint::DataTransfer::default();
        if let Ok(paths) = crate::native_files::decode_paths(&[text.to_string()]) {
            data.set_file_paths(paths);
        }
        if text.contains('\n') {
            selected_payload = text.to_string();
            selected_data = data.clone();
        }
        data
    });
    ui.global::<Transfers>()
        .on_accepts(|data| data.has_file_paths());
    macro_rules! callback {
        ($method:ident, |$adapter:ident, $window:ident $(, $arg:ident)*| $body:block) => {{
            let state=state.clone(); let weak=ui.as_weak();
            ui.$method(move |$($arg),*| {
                let Some($window)=weak.upgrade() else {return Default::default();};
                windows::with_state(&state,|$adapter| {
                  let result=$body;
                  $adapter.sync(&$window);
                  result
                })
            });
        }};
    }
    callback!(on_action, |a, w, id| {
        if matches!(id.as_str(), "FocusNext" | "FocusPrevious") && w.get_focused_control() >= 0 {
            a.extra.focus_index = w.get_focused_control() as usize;
        }
        a.action(&id);
    });
    callback!(on_tree_key, |a, w, index, event| {
        a.tree_key(index, event)
    });
    callback!(on_tree_action, |a, w, index, toggle| {
        a.tree_action(index, toggle);
    });
    callback!(on_place, |a, w, id| {
        if !a.blocked() {
            if let Some((_, location)) =
                a.places.iter().find(|(key, _)| key == id.as_str()).cloned()
            {
                a.app.tab_mut().navigate(location, &a.ctx);
            } else {
                a.action(&id);
            }
        }
    });
    callback!(on_place_menu, |a, w, id| {
        let commands = if a.blocked() {
            Vec::new()
        } else if let Some(index) = id.strip_prefix("bookmark:") {
            vec![Command {
                id: format!("RemoveBookmark:{index}").into(),
                title: tr("Remove bookmark").into_owned().into(),
                enabled: true,
            }]
        } else if let Some(index) = id.strip_prefix("connection:") {
            vec![
                Command {
                    id: format!("EditConnection:{index}").into(),
                    title: tr("Edit").into_owned().into(),
                    enabled: true,
                },
                Command {
                    id: format!("ForgetCredentials:{index}").into(),
                    title: tr("Forget credentials").into_owned().into(),
                    enabled: true,
                },
            ]
        } else {
            Vec::new()
        };
        let show = !commands.is_empty();
        w.set_menu_items(Rc::new(VecModel::from(commands)).into());
        show
    });
    callback!(on_menu, |a, w, title| {
        let mut entries = action_menus()
            .into_iter()
            .find(|(name, _)| *name == title.as_str())
            .map(|(_, entries)| entries)
            .unwrap_or_default();
        if title == "View" && entries.is_empty() {
            entries = vec![
                ("Show Hidden Files", Action::ToggleHidden),
                ("Appearance…", Action::Preferences),
            ];
        }
        if title == "Help" {
            entries = vec![("Rust Commander Help", Action::Help)];
        }
        w.set_menu_items(
            Rc::new(VecModel::from(
                entries
                    .into_iter()
                    .map(|(title, action)| Command {
                        id: format!("{action:?}").into(),
                        title: parity::caption(&format!("{action:?}"), title, &a.app.preferences)
                            .into(),
                        enabled: !a.blocked(),
                    })
                    .collect::<Vec<_>>(),
            ))
            .into(),
        );
    });
    callback!(on_tab, |a, w, side, index| {
        if !a.blocked()
            && (0..2).contains(&side)
            && index >= 0
            && (index as usize) < a.app.panes[side as usize].tabs.len()
        {
            a.app.active = side as usize;
            a.app.panes[side as usize].active = index as usize;
        }
    });
    callback!(on_address_edited, |a, w, side, text| {
        if !a.blocked() && (0..2).contains(&side) {
            a.app.panes[side as usize].tab_mut().address = text.to_string();
        }
    });
    callback!(on_breadcrumb, |a, w, side, index| {
        if !a.blocked()
            && (0..2).contains(&side)
            && index >= 0
            && let Some(location) = a.breadcrumbs[side as usize].get(index as usize).cloned()
        {
            a.app.active = side as usize;
            a.app.tab_mut().navigate(location, &a.ctx);
        }
    });
    callback!(on_navigate, |a, w, side, text| {
        if side >= 0 {
            a.navigate(side as usize, &text);
        }
    });
    callback!(on_filter, |a, w, side, text| {
        if !a.blocked() && (0..2).contains(&side) {
            a.app.panes[side as usize].tab_mut().filter = text.to_string();
        }
    });
    callback!(on_selected, |a, w, side, id, toggle, range| {
        if side >= 0 {
            a.select(side as usize, &id, toggle, range);
        }
    });
    callback!(on_clicked_file, |a, w, side, id| {
        if !a.blocked() && (0..2).contains(&side) && a.entry(side as usize, &id).is_some() {
            a.app.panes[side as usize].tab_mut().selected.clear();
            a.select(side as usize, &id, false, false);
        }
    });
    callback!(on_opened, |a, w, side, id| {
        if side >= 0 {
            a.open(side as usize, &id);
        }
    });
    callback!(on_resize_column, |a, w, index, width, finished| {
        if !a.blocked()
            && index >= 0
            && width.is_finite()
            && (40.0..=800.0).contains(&width)
            && let Some(column) = a.app.preferences.columns.get_mut(index as usize)
        {
            column.width = width;
            if finished {
                a.app.save_settings();
            }
        }
    });
    callback!(on_sort, |a, w, side, index| {
        if !a.blocked()
            && (0..2).contains(&side)
            && index >= 0
            && let Some(column) = a.app.preferences.columns.get(index as usize)
            && let Some(sort) = column.kind.sort()
        {
            let tab = a.app.panes[side as usize].tab_mut();
            tab.descending = tab.sort == sort && !tab.descending;
            tab.sort = sort;
            filesystem::sort_entries_with_options(
                &mut tab.entries,
                tab.sort,
                tab.descending,
                tab.folders_first,
            );
        }
    });
    callback!(on_resize_panes, |a, w, ratio, finished| {
        a.app.preferences.layout.set_ratio(ratio);
        if finished {
            a.app.save_settings();
        }
    });
    callback!(on_dropped, |a, w, side, data| {
        if side >= 0 {
            a.drop_files(side as usize, data)
        } else {
            false
        }
    });
    callback!(on_global_key, |a, w, event| { a.global_key(&w, event) });
    callback!(on_pane_key, |a, w, side, event| {
        a.pane_key(&w, side, event)
    });
    callback!(on_dialog_field, |a, w, id, value| {
        a.edit_field(&id, &value);
    });
    callback!(on_dialog_action, |a, w, id| {
        a.dialog_action(&id);
    });
    callback!(on_dialog_select, |a, w, id| {
        a.dialog_select(&id);
    });
    callback!(on_viewer_edit, |a, w, text| {
        if let Some(editor) = &mut a.app.editor
            && editor.editable
        {
            editor.text = text.to_string();
        }
    });
    callback!(on_viewer_action, |a, w, id| {
        a.viewer_action(&id);
    });
    callback!(on_terminal_key, |a, w, event| { a.terminal_key(event) });
    callback!(on_terminal_resize, |a, w, rows, cols| {
        if let Some(terminal) = &mut a.app.terminal {
            let _ = terminal.resize(rows.clamp(2, 100) as u16, cols.clamp(20, 240) as u16);
        }
    });
    use slint::winit_030::WinitWindowAccessor;
    let event_state = state.clone();
    let weak = ui.as_weak();
    let modifiers = std::cell::Cell::new([false; 4]);
    ui.window().on_winit_window_event(move |window, event| {
        use slint::winit_030::winit::event::WindowEvent;
        if let Some(ui) = weak.upgrade() {
            match event {
                WindowEvent::ModifiersChanged(next) => {
                    sync_modifiers(&ui, &modifiers, next.state());
                    return slint::winit_030::EventResult::Propagate;
                }
                WindowEvent::Focused(false) => sync_modifiers(&ui, &modifiers, Default::default()),
                _ => {}
            }
            let mut a = event_state.borrow_mut();
            if let WindowEvent::DroppedFile(path) = event
                && !a.blocked()
            {
                if a.pending_drops.is_empty() {
                    use raw_window_handle::HasWindowHandle;
                    let position = window
                        .window_handle()
                        .window_handle()
                        .ok()
                        .and_then(|handle| {
                            crate::native_files::drop_position(
                                handle.as_raw(),
                                f64::from(window.scale_factor()),
                            )
                        });
                    a.pending_drop_side = match position {
                        Some((x, y)) => usize::try_from(ui.invoke_drop_side(x, y))
                            .ok()
                            .filter(|&side| side < 2),
                        // Compositors that prohibit pointer queries use an explicit
                        // confirmation for the active pane, never a stale position.
                        None => Some(a.app.active),
                    };
                }
                if a.pending_drop_side.is_some() {
                    a.pending_drops.push(path.clone());
                    a.drop_deadline = Some(std::time::Instant::now() + Duration::from_millis(80));
                }
            }
        }
        slint::winit_030::EventResult::Propagate
    });
    let weak = ui.as_weak();
    let close_state = state.clone();
    ui.window().on_close_requested(move || {
        windows::with_state(&close_state, |a| {
            a.request_close();
            if let Some(ui) = weak.upgrade() {
                a.sync(&ui);
            }
        });
        slint::CloseRequestResponse::KeepWindowShown
    });
}

// Slint 1.18's winit backend tracks modifiers through key press events.
// macOS synthesized shortcuts can provide only ModifiersChanged plus the
// character key. Reconcile that state before dispatching the character, for
// both standard text widgets and our commands. Do this without borrowing the
// controller: dispatch_event can synchronously call back into it.
fn sync_modifiers(
    ui: &CommanderWindow,
    previous: &std::cell::Cell<[bool; 4]>,
    state: slint::winit_030::winit::keyboard::ModifiersState,
) {
    let (command, meta) = if cfg!(target_os = "macos") {
        (state.super_key(), state.control_key())
    } else {
        (state.control_key(), state.super_key())
    };
    let next = [command, meta, state.shift_key(), state.alt_key()];
    let old = previous.replace(next);
    for (index, key) in [
        slint::platform::Key::Control,
        slint::platform::Key::Meta,
        slint::platform::Key::Shift,
        slint::platform::Key::Alt,
    ]
    .into_iter()
    .enumerate()
    {
        if old[index] != next[index] {
            let text = key.into();
            ui.window().dispatch_event(if next[index] {
                slint::platform::WindowEvent::KeyPressed { text }
            } else {
                slint::platform::WindowEvent::KeyReleased { text }
            });
        }
    }
}

#[cfg(test)]
mod tests;

fn dispatch_native_drag(state: &Rc<RefCell<Adapter>>, ui: &CommanderWindow) {
    let paths = { state.borrow_mut().pending_native_drag.take() };
    let Some(paths) = paths else {
        return;
    };
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let handle = state
            .borrow()
            .app
            .native_window
            .as_ref()
            .map(|window| window.0);
        let result = handle
            .ok_or_else(|| anyhow::anyhow!("Native window handle unavailable."))
            .and_then(|handle| {
                crate::native_files::drag_out(&crate::native_files::Window(handle), paths)
            });
        if let Err(error) = result {
            state.borrow_mut().app.fail(error);
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    state.borrow_mut().app.native_drag(paths);
    ui.invoke_reset_native_drag();
    windows::with_state(state, |a| a.sync(ui));
}

fn location_breadcrumbs(location: &Location) -> Vec<(String, Location)> {
    match location {
        Location::Local(path) => crate::native_files::breadcrumbs(path)
            .into_iter()
            .map(|(name, path)| (name, Location::Local(path)))
            .collect(),
        Location::Archive { file, prefix } => {
            let mut crumbs = vec![(
                file.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                Location::Archive {
                    file: file.clone(),
                    prefix: PathBuf::new(),
                },
            )];
            let mut at = PathBuf::new();
            for component in prefix.components() {
                at.push(component.as_os_str());
                crumbs.push((
                    component.as_os_str().to_string_lossy().into_owned(),
                    Location::Archive {
                        file: file.clone(),
                        prefix: at.clone(),
                    },
                ));
            }
            crumbs
        }
        Location::Remote { connection, path } => {
            let mut crumbs = vec![(
                connection.clone(),
                Location::Remote {
                    connection: connection.clone(),
                    path: PathBuf::new(),
                },
            )];
            let mut at = PathBuf::new();
            for component in path.components() {
                at.push(component.as_os_str());
                crumbs.push((
                    component.as_os_str().to_string_lossy().into_owned(),
                    Location::Remote {
                        connection: connection.clone(),
                        path: at.clone(),
                    },
                ));
            }
            crumbs
        }
    }
}

#[cfg(test)]
mod parity_tests;
