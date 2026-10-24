use super::dialogs::{Dialog, Field, Row};
use super::*;
use std::collections::BTreeMap;

#[derive(Clone)]
pub(super) enum Target {
    Location(Location),
    Tab(usize, usize),
    File(PathBuf),
    Window(usize),
    Command(usize),
}
pub(super) struct Form {
    kind: &'static str,
    values: BTreeMap<String, String>,
    targets: Vec<(String, String, Target)>,
    selected: usize,
    error: String,
    import: Option<crate::config_import::Preview>,
    expected_settings: Option<Option<Vec<u8>>>,
}
impl Form {
    fn new(kind: &'static str) -> Self {
        Self {
            kind,
            values: Default::default(),
            targets: Vec::new(),
            selected: 0,
            error: String::new(),
            import: None,
            expected_settings: None,
        }
    }
    fn value(&self, id: &str) -> &str {
        self.values.get(id).map(String::as_str).unwrap_or("")
    }
}
#[derive(Default)]
pub(super) struct State {
    pub dialog: Option<Form>,
    pub shortcut_filter: String,
    command_buttons: Rc<VecModel<CommandButton>>,
    command_cache: Vec<(String, String, String, bool)>,
    query: [String; 2],
    deadline: [Option<std::time::Instant>; 2],
    last_location: [String; 2],
    pub(super) focus_index: usize,
    page_rows: [usize; 2],
    focus_request: Option<usize>,
    context_request: bool,
    pub quit_all: bool,
    pub window_request: Option<super::windows::Request>,
    native_pending: Option<Receiver<Result<(), String>>>,
}

pub(super) fn command_catalog() -> Vec<(String, String)> {
    let labels = action_menus()
        .into_iter()
        .flat_map(|(_, items)| {
            items.into_iter().map(|(title, action)| {
                (
                    format!("{action:?}"),
                    title.split("  F").next().unwrap_or(title).to_owned(),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    all_actions()
        .into_iter()
        .map(|action| {
            let id = format!("{action:?}");
            let label = labels.get(&id).cloned().unwrap_or_else(|| {
                let mut label = String::new();
                let mut previous_lower = false;
                for c in id.chars() {
                    if c.is_uppercase() && previous_lower {
                        label.push(' ');
                    }
                    label.push(c);
                    previous_lower = c.is_lowercase();
                }
                label
            });
            (id, label)
        })
        .chain([
            ("Back".into(), "Back".into()),
            ("Forward".into(), "Forward".into()),
            ("SaveText".into(), "Save text".into()),
        ])
        .collect()
}
impl Adapter {
    pub(super) fn parity_action(&mut self, name: &str) -> bool {
        let side = self.app.active;
        let page = self.extra.page_rows[side].max(1) as isize;
        match name {
            "FocusAddress" => self.extra.focus_request = Some(side * 3 + 1),
            "FocusFilter" => self.extra.focus_request = Some(side * 3 + 2),
            "SelectNextRow" | "SelectPreviousRow" | "SelectNextPage" | "SelectPreviousPage"
            | "SelectFirstRow" | "SelectLastRow" | "MarkNextRow" | "MarkPreviousRow"
            | "MarkNextPage" | "MarkPreviousPage" | "MarkToFirstRow" | "MarkToLastRow" => {
                let delta = match name {
                    "SelectNextRow" | "MarkNextRow" => 1,
                    "SelectPreviousRow" | "MarkPreviousRow" => -1,
                    "SelectNextPage" | "MarkNextPage" => page,
                    "SelectPreviousPage" | "MarkPreviousPage" => -page,
                    "SelectFirstRow" | "MarkToFirstRow" => -isize::MAX,
                    _ => isize::MAX,
                };
                let hidden = self.app.show_hidden;
                self.app
                    .tab_mut()
                    .move_cursor(delta, name.starts_with("Mark"), hidden);
            }
            "MarkSelectedFile" => {
                let hidden = self.app.show_hidden;
                self.app.tab_mut().toggle_mark_and_advance(hidden);
            }
            "Swap" => {
                self.app.act(Action::Swap, &self.ctx);
                self.extra.query.swap(0, 1);
                self.extra.deadline.swap(0, 1);
                self.extra.last_location.swap(0, 1);
                self.extra.focus_index = self.app.active * 3;
                self.extra.focus_request = Some(self.extra.focus_index);
            }
            "ClearSelection" => {
                self.app.tab_mut().selected.clear();
                self.app.tab_mut().filter.clear();
            }
            "QuickFind" => {
                self.extra.deadline[side] =
                    Some(std::time::Instant::now() + Duration::from_secs(3600));
                self.extra.query[side].clear();
                self.extra.focus_request = Some(side * 3);
            }
            "QuickFindNext" | "QuickFindPrevious" => {
                let query = self.extra.query[side].clone();
                let hidden = self.app.show_hidden;
                crate::navigation::quick_find(
                    self.app.tab_mut(),
                    &query,
                    name == "QuickFindPrevious",
                    true,
                    hidden,
                );
            }
            "SelectPattern" | "UnselectPattern" => {
                let mut form = Form::new("selection");
                form.values.insert("pattern".into(), "*".into());
                form.values
                    .insert("select".into(), (name == "SelectPattern").to_string());
                self.extra.dialog = Some(form);
            }
            "SelectExtension" => {
                let extension = self
                    .app
                    .tab()
                    .cursor
                    .as_ref()
                    .and_then(|p| p.extension())
                    .map(|s| s.to_string_lossy().to_lowercase());
                let hidden = self.app.show_hidden;
                let tab = self.app.tab_mut();
                for i in tab.visible(hidden) {
                    if tab.entries[i]
                        .path
                        .extension()
                        .map(|s| s.to_string_lossy().to_lowercase())
                        == extension
                    {
                        tab.selected.insert(tab.entries[i].path.clone());
                    }
                }
            }
            "SelectNextBlock" | "SelectPreviousBlock" | "MarkNextBlock" | "MarkPreviousBlock" => {
                let hidden = self.app.show_hidden;
                let tab = self.app.tab_mut();
                let visible = tab.visible(hidden);
                let current = visible
                    .iter()
                    .position(|&i| Some(&tab.entries[i].path) == tab.cursor.as_ref())
                    .unwrap_or(0);
                let backwards = name.contains("Previous");
                let delta = if backwards { -5 } else { 5 };
                if name.starts_with("Mark") {
                    let selected = visible
                        .get(current)
                        .is_none_or(|&i| !tab.selected.contains(&tab.entries[i].path));
                    let indices = if backwards {
                        (current.saturating_sub(4)..=current).collect::<Vec<_>>()
                    } else {
                        (current..current.saturating_add(5).min(visible.len())).collect::<Vec<_>>()
                    };
                    for index in indices {
                        if let Some(&i) = visible.get(index) {
                            let path = tab.entries[i].path.clone();
                            if selected {
                                tab.selected.insert(path);
                            } else {
                                tab.selected.remove(&path);
                            }
                        }
                    }
                }
                tab.move_cursor(delta, false, hidden);
            }
            "LockTab" => {
                let tab = self.app.tab_mut();
                tab.state.locked = if tab.state.locked.is_some() {
                    None
                } else {
                    Some(tab.location.clone())
                };
                self.app.save_settings();
            }
            "TabTitle" => {
                let mut form = Form::new("tab-title");
                form.values
                    .insert("title".into(), self.app.tab().state.title.clone());
                self.extra.dialog = Some(form);
            }
            "MoveTabOther" => {
                if self.app.panes[1 - side].tabs.len() >= 32 {
                    self.app.fail("The other pane already has 32 tabs.");
                    return true;
                }
                let pane = &mut self.app.panes[side];
                let tab = pane.tabs.remove(pane.active);
                if pane.tabs.is_empty() {
                    pane.tabs
                        .push(Tab::new(Location::Local(filesystem::home()), &self.ctx));
                }
                pane.active = pane.active.min(pane.tabs.len() - 1);
                let other = &mut self.app.panes[1 - side];
                other.tabs.push(tab);
                other.active = other.tabs.len() - 1;
                self.app.active = 1 - side;
                self.app.save_settings();
            }
            "MoveTabLeft" | "MoveTabRight" => {
                let pane = &mut self.app.panes[side];
                let target = if name == "MoveTabLeft" {
                    pane.active.checked_sub(1)
                } else {
                    pane.active.checked_add(1).filter(|&i| i < pane.tabs.len())
                };
                if let Some(target) = target {
                    pane.tabs.swap(pane.active, target);
                    pane.active = target;
                    self.app.save_settings();
                }
            }
            "SideBySide" | "Stacked" => {
                self.app.preferences.layout.orientation = if name == "Stacked" {
                    crate::navigation::Orientation::Stacked
                } else {
                    crate::navigation::Orientation::SideBySide
                };
                self.app.save_settings();
            }
            "EqualPanes" => {
                self.app.preferences.layout.ratio = 0.5;
                self.app.save_settings();
            }
            "AutoSizeColumns" => {
                self.app.preferences.auto_size_columns = !self.app.preferences.auto_size_columns;
                for column in &mut self.app.preferences.columns {
                    let width = self
                        .app
                        .panes
                        .iter()
                        .flat_map(|p| {
                            p.tab().entries.iter().map(|e| {
                                column
                                    .kind
                                    .value(e, p.tab().location.local().is_some())
                                    .chars()
                                    .count()
                            })
                        })
                        .take(10_000)
                        .max()
                        .unwrap_or(8);
                    column.width = ((width as f32 + 3.0) * self.app.preferences.font_size * 0.58)
                        .clamp(60.0, 800.0);
                }
                self.app.save_settings();
            }
            "OpenBoth" => {
                if let Some(entry) = self.app.tab().chosen(self.app.show_hidden).first()
                    && let Some(location) = self.app.tab().location.enter(entry)
                {
                    for pane in &mut self.app.panes {
                        pane.tab_mut().navigate(location.clone(), &self.ctx);
                    }
                } else {
                    self.app
                        .fail("Select a folder or archive to open in both panes.");
                }
            }
            "ParentBoth" | "ParentOther" => {
                let sides = if name == "ParentBoth" {
                    vec![0, 1]
                } else {
                    vec![1 - side]
                };
                for i in sides {
                    if let Some(parent) = self.app.panes[i].tab().location.parent() {
                        self.app.panes[i].tab_mut().navigate(parent, &self.ctx);
                    }
                }
            }
            "SwitchPane" => {
                self.app.active = 1 - side;
                self.extra.focus_index = self.app.active * 3;
                self.extra.focus_request = Some(self.extra.focus_index);
            }
            "FocusNext" | "FocusPrevious" => {
                self.extra.focus_index =
                    (self.extra.focus_index + if name == "FocusNext" { 1 } else { 5 }) % 6;
                self.app.active = self.extra.focus_index / 3;
                self.extra.focus_request = Some(self.extra.focus_index);
            }
            "FileContextMenu" => self.extra.context_request = true,
            "QuickLists" | "RecentLocations" | "RecentFiles" | "BookmarksList" | "TabsList"
            | "RootsList" | "WindowsList" | "ImportedCommands" => self.open_quick_list(name),
            "OpenTrash" => self.native_trash(false),
            "EmptyTrash" => {
                self.extra.dialog = Some(Form::new("empty-trash"));
            }
            "NewWindow" => self.extra.window_request = Some(super::windows::Request::New),
            "CloseWindow" => self.request_close(),
            "NextWindow" => self.extra.window_request = Some(super::windows::Request::Cycle(false)),
            "PreviousWindow" => {
                self.extra.window_request = Some(super::windows::Request::Cycle(true))
            }
            "ImportSettings" => {
                let mut form = Form::new("import");
                for key in ["bookmarks", "shortcuts", "commands"] {
                    form.values.insert(key.into(), "true".into());
                }
                self.extra.dialog = Some(form);
            }
            "Open" => {
                if let Some(entry) = self.app.tab().chosen(self.app.show_hidden).first().cloned() {
                    self.app.open_entry(entry, &self.ctx);
                }
            }
            _ => return false,
        }
        true
    }
    fn open_quick_list(&mut self, kind: &str) {
        let mut form = Form::new("quick-list");
        form.values.insert("kind".into(), kind.into());
        if matches!(kind, "QuickLists" | "RecentLocations") {
            for recent in &self.app.recent_locations {
                form.targets.push((
                    recent.title.clone(),
                    recent.location.label(),
                    Target::Location(recent.location.clone()),
                ));
            }
        }
        if matches!(kind, "QuickLists" | "BookmarksList") {
            for bookmark in &self.app.bookmarks {
                form.targets.push((
                    bookmark.name.clone(),
                    bookmark.location.label(),
                    Target::Location(bookmark.location.clone()),
                ));
            }
        }
        if matches!(kind, "QuickLists" | "TabsList") {
            for (side, pane) in self.app.panes.iter().enumerate() {
                for (i, tab) in pane.tabs.iter().enumerate() {
                    form.targets.push((
                        format!(
                            "{} · {}",
                            if side == 0 { "Left" } else { "Right" },
                            tab_label(tab)
                        ),
                        tab.location.label(),
                        Target::Tab(side, i),
                    ));
                }
            }
        }
        if matches!(kind, "QuickLists" | "RootsList") {
            for (name, path) in crate::native_files::mounted_drives() {
                form.targets.push((
                    name,
                    path.display().to_string(),
                    Target::Location(Location::Local(path)),
                ));
            }
        }
        if matches!(kind, "QuickLists" | "RecentFiles") {
            for path in &self.app.recent_files {
                form.targets.push((
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    path.display().to_string(),
                    Target::File(path.clone()),
                ));
            }
        }
        if kind == "WindowsList" {
            for (id, title) in super::windows::entries() {
                form.targets
                    .push((title, String::new(), Target::Window(id)));
            }
        }
        if kind == "ImportedCommands" {
            for (i, command) in self.app.imported_commands.iter().enumerate() {
                form.targets.push((
                    command.alias.clone(),
                    command.arguments.join(" "),
                    Target::Command(i),
                ));
            }
        }
        self.extra.dialog = Some(form);
    }
    pub(super) fn parity_tick(&mut self) {
        for side in 0..2 {
            let location = self.app.panes[side].tab().location.clone();
            let label = location.label();
            if self.extra.last_location[side] != label {
                self.extra.last_location[side] = label;
                self.extra.query[side].clear();
                self.extra.deadline[side] = None;
                crate::navigation::remember(
                    &mut self.app.recent_locations,
                    tab_label(self.app.panes[side].tab()),
                    location,
                );
            }
        }
        if let Some(result) = crate::background::poll(&mut self.extra.native_pending) {
            match result {
                Ok(()) => {
                    self.app.status = "Native Trash action completed.".into();
                    self.app.status_error = false;
                }
                Err(error) => self.app.fail(error),
            }
        }
    }
    pub(super) fn parity_sync(&mut self, ui: &CommanderWindow) {
        self.extra.page_rows = [
            ui.get_left_page_rows().max(1.0) as usize,
            ui.get_right_page_rows().max(1.0) as usize,
        ];
        ui.set_auto_size_columns(self.app.preferences.auto_size_columns);
        ui.set_column_total(
            self.app
                .preferences
                .columns
                .iter()
                .map(|c| c.width)
                .sum::<f32>(),
        );
        ui.set_stacked(
            self.app.preferences.layout.orientation == crate::navigation::Orientation::Stacked,
        );
        ui.set_pane_ratio(self.app.preferences.layout.ratio.clamp(0.15, 0.85));
        for side in 0..2 {
            if self.extra.deadline[side].is_some_and(|d| std::time::Instant::now() > d) {
                self.extra.deadline[side] = None;
                self.extra.query[side].clear();
            }
        }
        ui.set_left_quick_find(self.extra.query[0].clone().into());
        ui.set_right_quick_find(self.extra.query[1].clone().into());
        if let Some(focus) = self.extra.focus_request.take() {
            match focus % 3 {
                0 => ui.invoke_focus_files((focus / 3) as i32),
                1 => ui.invoke_focus_address((focus / 3) as i32),
                _ => ui.invoke_focus_filter((focus / 3) as i32),
            }
        }
        if std::mem::take(&mut self.extra.context_request) {
            ui.invoke_show_file_menu();
        }
        let catalog = command_catalog();
        let buttons = self
            .app
            .preferences
            .command_actions
            .iter()
            .filter_map(|id| {
                catalog.iter().find(|(key, _)| key == id).map(|(_, title)| {
                    (
                        id.clone(),
                        tr(title).into_owned(),
                        display_binding(
                            self.app
                                .preferences
                                .shortcuts
                                .get(id)
                                .map(String::as_str)
                                .unwrap_or(""),
                        ),
                        !self.blocked() && self.action_available(id),
                    )
                })
            })
            .collect::<Vec<_>>();
        publish(
            &self.extra.command_buttons,
            &mut self.extra.command_cache,
            buttons,
            |row| CommandButton {
                id: row.0.as_str().into(),
                title: row.1.as_str().into(),
                key: row.2.as_str().into(),
                enabled: row.3,
            },
        );
        ui.set_command_buttons(self.extra.command_buttons.clone().into());
    }

    pub(super) fn parity_list_key(&mut self, event: &slint::language::KeyEvent) -> bool {
        let Some(form) = &mut self.extra.dialog else {
            return false;
        };
        if form.kind != "quick-list" {
            return false;
        }
        let text = event.text.as_str();
        let down = text == SharedString::from(slint::platform::Key::DownArrow).as_str();
        let up = text == SharedString::from(slint::platform::Key::UpArrow).as_str();
        if !down && !up {
            return false;
        }
        let query = form.value("filter").to_lowercase();
        let matches = form
            .targets
            .iter()
            .enumerate()
            .filter(|(_, (a, b, _))| format!("{a} {b}").to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            return true;
        }
        let current = matches
            .iter()
            .position(|&i| i == form.selected)
            .unwrap_or(0);
        let next = (current + if down { 1 } else { matches.len() - 1 }) % matches.len();
        form.selected = matches[next];
        true
    }
    pub(super) fn quick_find_consumes(
        &self,
        side: usize,
        event: &slint::language::KeyEvent,
    ) -> bool {
        self.extra.deadline[side].is_some()
            && !event.modifiers.control
            && !event.modifiers.alt
            && !event.modifiers.meta
            && [
                slint::platform::Key::UpArrow,
                slint::platform::Key::DownArrow,
                slint::platform::Key::Backspace,
                slint::platform::Key::Escape,
            ]
            .into_iter()
            .any(|key| event.text == SharedString::from(key))
    }
    pub(super) fn parity_key(&mut self, side: usize, event: &slint::language::KeyEvent) -> bool {
        let text = event.text.as_str();
        let modifiers = event.modifiers;
        if modifiers.control || modifiers.alt || modifiers.meta {
            return false;
        }
        let active = self.extra.deadline[side].is_some();
        if active && text == slint::SharedString::from(slint::platform::Key::Escape).as_str() {
            self.extra.query[side].clear();
            self.extra.deadline[side] = None;
            return true;
        }
        if active && text == slint::SharedString::from(slint::platform::Key::Backspace).as_str() {
            self.extra.query[side].pop();
        } else if active
            && [
                slint::platform::Key::UpArrow,
                slint::platform::Key::DownArrow,
            ]
            .into_iter()
            .any(|k| text == slint::SharedString::from(k).as_str())
        {
            let query = self.extra.query[side].clone();
            let hidden = self.app.show_hidden;
            crate::navigation::quick_find(
                self.app.tab_mut(),
                &query,
                text == slint::SharedString::from(slint::platform::Key::UpArrow).as_str(),
                true,
                hidden,
            );
            return true;
        } else if !text.is_empty()
            && text
                .chars()
                .all(|c| !c.is_control() && !(0xe000..=0xf8ff).contains(&(c as u32)))
            && text != " "
        {
            if !active {
                self.extra.query[side].clear();
            }
            if self.extra.query[side].chars().count() < 256 {
                self.extra.query[side].push_str(text);
            }
        } else {
            return false;
        }
        self.extra.deadline[side] = Some(std::time::Instant::now() + Duration::from_secs(3));
        let query = self.extra.query[side].clone();
        let hidden = self.app.show_hidden;
        crate::navigation::quick_find(self.app.tab_mut(), &query, false, false, hidden);
        true
    }
    pub(super) fn parity_dialog(&self) -> Option<Dialog> {
        let form = self.extra.dialog.as_ref()?;
        let mut d = Dialog::new(
            form.kind,
            match form.kind {
                "selection" => "Select by pattern",
                "tab-title" => "Name tab",
                "quick-list" => "Quick list",
                "empty-trash" => "Empty system Trash?",
                "import" => "Import muCommander settings",
                "run-command" => "Run imported command?",
                _ => "Tools",
            },
        );
        d.error = form.error.clone();
        match form.kind {
            "selection" => {
                d.fields.extend([
                    Field::text(
                        "pattern",
                        "Patterns (separate with ;)",
                        form.value("pattern"),
                    ),
                    Field::check("select", "Select matches", form.value("select") == "true"),
                    Field::check("case", "Case sensitive", form.value("case") == "true"),
                ]);
                d.button("accept", "Apply to visible files", true);
            }
            "tab-title" => {
                d.fields.push(Field::text(
                    "title",
                    "Title (blank restores folder name)",
                    form.value("title"),
                ));
                d.button("accept", "Apply", true);
            }
            "quick-list" => {
                d.fields
                    .push(Field::text("filter", "Search", form.value("filter")));
                let query = form.value("filter").to_lowercase();
                for (i, (title, detail, _)) in form.targets.iter().enumerate() {
                    if format!("{title} {detail}").to_lowercase().contains(&query) {
                        let mut row = Row::new(format!("quick-select-{i}"), title, detail).action(
                            format!("quick-open-{i}"),
                            "Open",
                            true,
                        );
                        row.selected = form.selected == i;
                        d.rows.push(row);
                    }
                }
                d.button("accept", "Open selected", !d.rows.is_empty());
            }
            "empty-trash" => {
                d.description="Permanently empty the system Trash/Recycle Bin, including files deleted by other applications. This cannot be undone.".into();
                d.fields.push(Field::text(
                    "confirm",
                    "Type EMPTY to confirm",
                    form.value("confirm"),
                ));
                d.button(
                    "accept",
                    "Empty permanently",
                    form.value("confirm") == "EMPTY"
                        && self.app.job.is_none()
                        && self.app.queue.is_empty()
                        && self.extra.native_pending.is_none(),
                );
            }
            "import" => {
                d.fields.push(Field::text(
                    "directory",
                    "muCommander configuration directory",
                    form.value("directory"),
                ));
                for (key, title) in [
                    ("bookmarks", "Import bookmarks"),
                    (
                        "shortcuts",
                        "Import shortcuts (keep existing custom bindings)",
                    ),
                    ("commands", "Import commands"),
                    ("preferences", "Import supported preferences"),
                ] {
                    d.fields
                        .push(Field::check(key, title, form.value(key) == "true"));
                }
                d.button("preview", "Preview import", true);
                if let Some(preview) = &form.import {
                    d.description = format!(
                        "{} bookmarks, {} shortcuts, {} commands, {} supported preferences. Existing bookmarks, commands and customized shortcut bindings take precedence. A settings backup is written before applying.",
                        preview.bookmarks.len(),
                        preview.shortcuts.len() + preview.alternate_shortcuts.len(),
                        preview.commands.len(),
                        preview.preferences.len()
                    );
                    for bookmark in &preview.bookmarks {
                        d.rows
                            .push(Row::new("", &bookmark.name, bookmark.location.label()));
                    }
                    let catalog = command_catalog().into_iter().collect::<BTreeMap<_, _>>();
                    for (id, binding) in &preview.shortcuts {
                        let title = catalog.get(id).map(String::as_str).unwrap_or(id);
                        d.rows
                            .push(Row::new("", format!("Shortcut: {title}"), binding));
                    }
                    for (id, binding) in &preview.alternate_shortcuts {
                        let title = catalog.get(id).map(String::as_str).unwrap_or(id);
                        d.rows.push(Row::new(
                            "",
                            format!("Alternate shortcut: {title}"),
                            binding,
                        ));
                    }
                    for (name, value) in &preview.preferences {
                        d.rows
                            .push(Row::new("", format!("Preference: {name}"), value));
                    }
                    for warning in &preview.unsupported {
                        d.rows.push(Row::new("", "Unsupported", warning));
                    }
                    for command in &preview.commands {
                        d.rows
                            .push(Row::new("", &command.alias, command.arguments.join(" ")));
                    }
                    d.button("accept", "Apply import", true);
                }
            }
            "run-command" => {
                d.description = form.value("command").into();
                d.fields.push(Field::check(
                    "confirm",
                    "Run this program with the displayed arguments",
                    form.value("confirm") == "true",
                ));
                d.button("accept", "Run", form.value("confirm") == "true");
            }
            _ => {}
        }
        Some(d)
    }
    pub(super) fn parity_edit(&mut self, id: &str, value: &str) -> bool {
        if self.dialog_kind == "preferences" && id == "shortcut-filter" {
            self.extra.shortcut_filter = value.into();
            return true;
        }
        if self.dialog_kind == "preferences"
            && let Some(index) = id
                .strip_prefix("command-slot-")
                .and_then(|v| v.parse::<usize>().ok())
            && index < 12
        {
            let command = command_catalog()
                .into_iter()
                .find(|(_, title)| title == value)
                .map(|(id, _)| id);
            if let Some(p) = &mut self.app.preferences_form {
                if let Some(command) = command {
                    if index < p.command_actions.len() {
                        p.command_actions[index] = command;
                    } else {
                        p.command_actions.push(command);
                    }
                } else if index < p.command_actions.len() {
                    p.command_actions.remove(index);
                }
            }
            return true;
        }
        if self.dialog_kind == "preferences" && id.starts_with("alternate-") {
            if let Some(p) = &mut self.app.preferences_form {
                p.alternate_shortcuts
                    .insert(id.trim_start_matches("alternate-").into(), value.into());
            }
            return true;
        }
        if self.dialog_kind == "preferences" && id == "command-actions" {
            if let Some(p) = &mut self.app.preferences_form {
                p.command_actions = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            return true;
        }
        if let Some(form) = &mut self.extra.dialog {
            form.values.insert(id.into(), value.into());
            if id == "directory" {
                form.import = None;
            }
            form.error.clear();
            if id == "filter" {
                let query = value.to_lowercase();
                form.selected = form
                    .targets
                    .iter()
                    .position(|(a, b, _)| format!("{a} {b}").to_lowercase().contains(&query))
                    .unwrap_or(0);
            }
            return true;
        }
        false
    }
    pub(super) fn parity_select(&mut self, id: &str) -> bool {
        if let Some(index) = id
            .strip_prefix("quick-select-")
            .and_then(|s| s.parse().ok())
            && let Some(form) = &mut self.extra.dialog
        {
            form.selected = index;
            return true;
        }
        false
    }
    pub(super) fn parity_dialog_action(&mut self, id: &str) -> bool {
        if self.extra.dialog.is_none() {
            return false;
        }
        if id == "cancel" {
            self.extra.dialog = None;
            return true;
        }
        let mut form = self.extra.dialog.take().unwrap();
        let result = (|| -> anyhow::Result<bool> {
            match form.kind {
                "selection" if id == "accept" => {
                    let hidden = self.app.show_hidden;
                    let n = crate::navigation::select_pattern(
                        self.app.tab_mut(),
                        form.value("pattern"),
                        form.value("select") == "true",
                        form.value("case") == "true",
                        hidden,
                    )?;
                    self.app.status = format!("Updated {n} visible selections.");
                    return Ok(true);
                }
                "tab-title" if id == "accept" => {
                    let title = form.value("title").trim();
                    anyhow::ensure!(
                        title.chars().count() <= 80 && !title.chars().any(char::is_control),
                        "Use at most 80 characters without control characters."
                    );
                    self.app.tab_mut().state.title = title.into();
                    self.app.save_settings();
                    return Ok(true);
                }
                "quick-list" => {
                    let index = id
                        .strip_prefix("quick-open-")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(form.selected);
                    if id == "accept" || id.starts_with("quick-open-") {
                        let target = form
                            .targets
                            .get(index)
                            .map(|(_, _, t)| t.clone())
                            .ok_or_else(|| anyhow::anyhow!("Select a result"))?;
                        match target {
                            Target::Location(location) => {
                                self.app.tab_mut().navigate(location, &self.ctx)
                            }
                            Target::Tab(side, index) => {
                                if index < self.app.panes[side].tabs.len() {
                                    self.app.active = side;
                                    self.app.panes[side].active = index;
                                }
                            }
                            Target::File(path) => {
                                anyhow::ensure!(path.is_file(), "The file no longer exists");
                                open::that_detached(path)?;
                            }
                            Target::Window(index) => {
                                self.extra.window_request =
                                    Some(super::windows::Request::Raise(index))
                            }
                            Target::Command(index) => {
                                let command =
                                    self.app.imported_commands.get(index).ok_or_else(|| {
                                        anyhow::anyhow!("Command no longer exists")
                                    })?;
                                let folder = self.app.tab().location.local().ok_or_else(|| {
                                    anyhow::anyhow!("Imported commands require a local folder")
                                })?;
                                let files = self
                                    .app
                                    .tab()
                                    .chosen(self.app.show_hidden)
                                    .into_iter()
                                    .map(|e| e.path)
                                    .collect::<Vec<_>>();
                                let args = command.expand(&files, folder)?;
                                let mut confirm = Form::new("run-command");
                                confirm.values.insert(
                                    "command".into(),
                                    format!(
                                        "Program: {}\nArguments: {:?}\nWorking folder: {}",
                                        args[0],
                                        &args[1..],
                                        folder.display()
                                    ),
                                );
                                confirm.values.insert("index".into(), index.to_string());
                                confirm
                                    .values
                                    .insert("files".into(), serde_json::to_string(&files)?);
                                confirm
                                    .values
                                    .insert("folder".into(), folder.display().to_string());
                                self.extra.dialog = Some(confirm);
                                return Ok(true);
                            }
                        }
                        self.extra.focus_request = Some(self.app.active * 3);
                        return Ok(true);
                    }
                }
                "empty-trash" if id == "accept" => {
                    anyhow::ensure!(
                        form.value("confirm") == "EMPTY" && self.app.job.is_none(),
                        "Confirm emptying Trash while no transfer is running"
                    );
                    self.native_trash(true);
                    return Ok(true);
                }
                "import" if id == "preview" => {
                    form.import = Some(crate::config_import::preview(Path::new(
                        form.value("directory"),
                    ))?);
                    form.expected_settings = Some(match std::fs::read(settings::path()?) {
                        Ok(bytes) => Some(bytes),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                        Err(e) => return Err(e.into()),
                    });
                }
                "import" if id == "accept" => {
                    anyhow::ensure!(
                        self.app.settings_valid,
                        "Repair the current invalid settings before importing."
                    );
                    let preview = form
                        .import
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Preview the import first"))?;
                    let mut preview = preview.clone();
                    if form.value("bookmarks") != "true" {
                        preview.bookmarks.clear();
                    }
                    if form.value("shortcuts") != "true" {
                        preview.shortcuts.clear();
                        preview.alternate_shortcuts.clear();
                    }
                    if form.value("commands") != "true" {
                        preview.commands.clear();
                    }
                    let expected = form
                        .expected_settings
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("Preview the import first"))?;
                    let (settings, backup) = crate::config_import::commit(
                        &settings::path()?,
                        &self.app.snapshot_settings(),
                        &preview,
                        form.value("preferences") == "true",
                        expected.as_deref(),
                    )?;
                    self.app.status = format!(
                        "Imported supported settings. Backup: {}",
                        backup
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "no previous settings file".into())
                    );
                    self.app.bookmarks = settings.bookmarks;
                    self.app.preferences = settings.preferences;
                    self.app.imported_commands = settings.imported_commands;
                    self.app.show_hidden = settings.show_hidden;

                    return Ok(true);
                }
                "run-command" if id == "accept" => {
                    anyhow::ensure!(form.value("confirm") == "true", "Confirm the command first");
                    let index: usize = form.value("index").parse()?;
                    let files: Vec<PathBuf> = serde_json::from_str(form.value("files"))?;
                    let folder = Path::new(form.value("folder"));
                    let command = self
                        .app
                        .imported_commands
                        .get(index)
                        .ok_or_else(|| anyhow::anyhow!("Command no longer exists"))?;
                    let args = command.expand(&files, folder)?;
                    std::process::Command::new(&args[0])
                        .args(&args[1..])
                        .current_dir(folder)
                        .spawn()?;
                    return Ok(true);
                }
                _ => {}
            }
            Ok(false)
        })();
        match result {
            Ok(true) => {}
            Ok(false) => self.extra.dialog = Some(form),
            Err(error) => {
                form.error = format!("{error:#}");
                self.extra.dialog = Some(form);
            }
        }
        true
    }
    fn native_trash(&mut self, empty: bool) {
        if empty {
            if self.app.job.is_some() || !self.app.queue.is_empty() {
                self.app
                    .fail("Wait for queued transfers before emptying Trash.");
                return;
            }
            self.app.start_job(Operation::EmptyTrash, &self.ctx);
            return;
        }
        if self.extra.native_pending.is_some() {
            return;
        }
        let (send, receive) = mpsc::channel();
        self.extra.native_pending = Some(receive);
        std::thread::spawn(move || {
            let result = crate::native_actions::trash_action(empty).map_err(|e| format!("{e:#}"));
            let _ = send.send(result);
        });
    }
}
pub(super) fn tab_label(tab: &Tab) -> String {
    let title = if tab.state.title.is_empty() {
        tab.location.title()
    } else {
        tab.state.title.clone()
    };
    if tab.state.locked.is_some() {
        format!("🔒 {title}")
    } else {
        title
    }
}

pub(super) fn caption(
    id: &str,
    label: &str,
    preferences: &crate::preferences::Preferences,
) -> String {
    let title = tr(label.split("  F").next().unwrap_or(label));
    match preferences.shortcuts.get(id).filter(|s| !s.is_empty()) {
        Some(binding) => format!("{title}   {}", display_binding(binding)),
        None => title.into_owned(),
    }
}
pub(super) fn display_binding(binding: &str) -> String {
    if cfg!(target_os = "macos") {
        binding
            .replace("Command+", "⌘")
            .replace("Shift+", "⇧")
            .replace("Alt+", "⌥")
    } else {
        binding.replace("Command+", "Ctrl+")
    }
}
