//! Pane and popup interaction rules, shared by pointer and keyboard entry points.
use super::*;
use crate::preferences::{Column, ColumnSpec};

#[derive(Default)]
pub(super) struct State {
    pub last_mark: [Option<(u64, PathBuf)>; 2],
}

impl Adapter {
    pub(super) fn context_target(&mut self, side: usize, kind: &str) {
        if self.blocked() || side > 1 {
            return;
        }
        self.app.active = side;
        if kind == "ParentContext" {
            self.app.tab_mut().focus_parent();
        } else if kind == "BackgroundContext" {
            self.app.tab_mut().clear_cursor();
        } else if let Some(index) = kind
            .strip_prefix("TabContext:")
            .and_then(|v| v.parse::<usize>().ok())
            && index < self.app.panes[side].tabs.len()
        {
            self.app.panes[side].active = index;
        }
    }

    pub(super) fn context_commands(&self, kind: &str) -> Option<Vec<MenuEntry>> {
        let tab = self.app.tab();
        let focused = tab.focused(self.app.show_hidden);
        let chosen = tab.chosen(self.app.show_hidden);
        let local = tab.location.local().is_some();
        let row = |id: &str, label: &str, enabled: bool, checked: Option<bool>| MenuEntry {
            id: id.into(),
            title: parity::caption(id, label, &self.app.preferences).into(),
            enabled: !self.blocked() && enabled && self.action_available(id),
            checkable: checked.is_some(),
            checked: checked.unwrap_or(false),
        };
        if kind == "ColumnContext" {
            let mut items: Vec<_> = Column::ALL
                .into_iter()
                .filter(|c| *c != Column::Name)
                .map(|column| {
                    row(
                        &format!("Column:{column:?}"),
                        column.label(),
                        true,
                        Some(
                            self.app
                                .preferences
                                .columns
                                .iter()
                                .any(|c| c.kind == column),
                        ),
                    )
                })
                .collect();
            items.push(row(
                "AutoSizeColumns",
                "Fit columns to content",
                true,
                Some(self.app.preferences.auto_size_columns),
            ));
            return Some(items);
        }
        if kind.starts_with("TabContext:") {
            let pane = &self.app.panes[self.app.active];
            return Some(
                [
                    row("NewTab", "New tab", pane.tabs.len() < 32, None),
                    row("DuplicateTab", "Duplicate tab", pane.tabs.len() < 32, None),
                    row(
                        "CloneTab",
                        "Clone tab to other pane",
                        self.app.panes[1 - self.app.active].tabs.len() < 32,
                        None,
                    ),
                    row("TabTitle", "Name tab…", true, None),
                    row(
                        "LockTab",
                        "Lock tab",
                        true,
                        Some(tab.state.locked.is_some()),
                    ),
                    row("MoveTabLeft", "Move tab left", pane.active > 0, None),
                    row(
                        "MoveTabRight",
                        "Move tab right",
                        pane.active + 1 < pane.tabs.len(),
                        None,
                    ),
                    row(
                        "MoveTabOther",
                        "Move tab to other pane",
                        self.app.panes[1 - self.app.active].tabs.len() < 32,
                        None,
                    ),
                    row(
                        "CloseOtherTabs",
                        "Close other tabs",
                        pane.tabs.len() > 1,
                        None,
                    ),
                    row(
                        "CloseDuplicateTabs",
                        "Close duplicate tabs",
                        pane.tabs.len() > 1,
                        None,
                    ),
                    row("CloseTab", "Close tab", pane.tabs.len() > 1, None),
                ]
                .into(),
            );
        }
        if !matches!(kind, "FileContext" | "ParentContext" | "BackgroundContext") {
            return None;
        }
        let mut items = Vec::new();
        let mut add = |id, label, enabled| items.push(row(id, label, enabled, None));
        if kind != "BackgroundContext" {
            add("Open", "Open", focused.is_some() || tab.parent_focused());
        }
        if kind == "ParentContext" {
            add("OpenOther", "Open in other pane", true);
            add(
                "OpenNewTab",
                "Open in new tab",
                self.app.panes[self.app.active].tabs.len() < 32,
            );
        }
        if let Some(entry) = &focused {
            add("External", "Open externally", local);
            add("OpenWith", "Open With…", local);
            add("OpenAs", "Open As…", !entry.is_dir);
            let browsable = tab.location.enter(entry).is_some();
            add("OpenOther", "Open in other pane", browsable);
            add(
                "OpenNewTab",
                "Open in new tab",
                browsable && self.app.panes[self.app.active].tabs.len() < 32,
            );
            add("View", "View", !entry.is_dir);
            add(
                "Edit",
                "Edit",
                !entry.is_dir && !matches!(tab.location, Location::Archive { .. }),
            );
        }
        if !chosen.is_empty() {
            for (id, label) in [
                ("Copy", "Copy"),
                ("Move", "Move"),
                ("CopyClipboard", "Copy files"),
                ("CutClipboard", "Cut files"),
                ("CopyNames", "Copy names"),
                ("CopyBaseNames", "Copy names without extensions"),
                ("CopyPaths", "Copy paths"),
                ("Pack", "Pack files…"),
                ("Trash", "Move to Trash"),
                ("Properties", "Properties"),
            ] {
                add(id, label, true);
            }
            add("Permissions", "Permissions…", local);
            add("OwnershipAcl", "Ownership and ACL…", local);
            add("Modified", "Modification time…", local);
        }
        add("PasteClipboard", "Paste files", true);
        if focused.is_some() {
            add("Rename", "Rename", true);
            add("MarkSelectedFile", "Mark / unmark", true);
            add("Reveal", "Reveal in file manager", local);
        }
        add("MarkAll", "Mark all", !tab.entries.is_empty());
        add("UnmarkAll", "Unmark all", !tab.selected.is_empty());
        add("Refresh", "Refresh", true);
        add("Mkdir", "New folder", true);
        add("Mkfile", "New file", true);
        Some(items)
    }

    pub(super) fn interaction_action(&mut self, name: &str) -> bool {
        let Some(kind) = name.strip_prefix("Column:") else {
            return false;
        };
        if let Some(column) = Column::ALL
            .into_iter()
            .find(|c| format!("{c:?}") == kind && *c != Column::Name)
        {
            if self
                .app
                .preferences
                .columns
                .iter()
                .any(|c| c.kind == column)
            {
                self.app.preferences.columns.retain(|c| c.kind != column);
            } else {
                self.app.preferences.columns.push(ColumnSpec {
                    kind: column,
                    width: 120.0,
                });
            }
            self.app.save_settings();
        }
        true
    }

    pub(super) fn menu_key(&self, ui: &CommanderWindow, event: &slint::language::KeyEvent) -> i32 {
        let text = event.text.as_str();
        let is = |key| event.text == SharedString::from(key);
        if ui.get_menu_top()
            && (is(slint::platform::Key::LeftArrow) || is(slint::platform::Key::RightArrow))
        {
            let step = if is(slint::platform::Key::LeftArrow) {
                6
            } else {
                1
            };
            ui.set_menu_heading_index((ui.get_menu_heading_index().max(0) + step) % 7);
            return 5;
        }
        if is(slint::platform::Key::Escape) || is(slint::platform::Key::Tab) {
            return if ui.get_menu_top() && is(slint::platform::Key::Escape) {
                4
            } else {
                3
            };
        }
        if is(slint::platform::Key::Return) || text == " " {
            return 2;
        }
        let items: Vec<_> = ui
            .get_menu_items()
            .iter()
            .enumerate()
            .filter(|(_, item)| item.enabled)
            .collect();
        if items.is_empty() {
            return 1;
        }
        let current = items
            .iter()
            .position(|(i, _)| *i as i32 == ui.get_menu_index());
        let next = if is(slint::platform::Key::Home) {
            Some(0)
        } else if is(slint::platform::Key::End) {
            Some(items.len() - 1)
        } else if is(slint::platform::Key::DownArrow) {
            Some(current.map_or(0, |i| (i + 1) % items.len()))
        } else if is(slint::platform::Key::UpArrow) {
            Some(current.map_or(items.len() - 1, |i| (i + items.len() - 1) % items.len()))
        } else if !event.modifiers.control
            && !event.modifiers.alt
            && !event.modifiers.meta
            && text.chars().count() == 1
        {
            let start = current.map_or(0, |i| (i + 1) % items.len());
            (0..items.len())
                .map(|i| (start + i) % items.len())
                .find(|&i| {
                    items[i]
                        .1
                        .title
                        .to_lowercase()
                        .starts_with(&text.to_lowercase())
                })
        } else {
            None
        };
        if let Some(next) = next {
            ui.set_menu_index(items[next].0 as i32);
        }
        1
    }
}

impl Adapter {
    pub(super) fn menubar_key(
        &self,
        ui: &CommanderWindow,
        event: &slint::language::KeyEvent,
    ) -> bool {
        if ui.get_system_menubar() || self.blocked() {
            return false;
        }
        let text = event.text.as_str();
        let is = |key| event.text == SharedString::from(key);
        let titles = [
            "File",
            "Operations",
            "Navigate",
            "Selection",
            "View",
            "Window",
            "Help",
        ];
        let mut used = std::collections::BTreeSet::new();
        let mnemonics = titles.map(|title| {
            tr(title)
                .chars()
                .find(|c| c.is_alphabetic() && used.insert(c.to_lowercase().collect::<String>()))
                .map(|c| c.to_lowercase().collect::<String>())
        });
        let index = event
            .modifiers
            .alt
            .then(|| {
                mnemonics
                    .iter()
                    .position(|m| m.as_deref() == Some(text.to_lowercase().as_str()))
            })
            .flatten();
        if let Some(index) = index {
            ui.set_menu_heading_index(index as i32);
            ui.set_menu_heading_active(true);
            let weak = ui.as_weak();
            let title = titles[index];
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() {
                    ui.invoke_show_menu(title.into());
                }
            });
            return true;
        }
        if (is(slint::platform::Key::F10) && !event.modifiers.alt || is(slint::platform::Key::Alt))
            && !event.modifiers.shift
            && !event.modifiers.control
            && !event.modifiers.meta
        {
            ui.set_menu_heading_active(!ui.get_menu_heading_active());
            if ui.get_menu_heading_active() {
                ui.set_menu_heading_index(0);
                ui.invoke_focus_menubar();
            } else {
                ui.invoke_restore_menu_focus();
            }
            return true;
        }
        if !ui.get_menu_heading_active() {
            return false;
        }
        if is(slint::platform::Key::LeftArrow) || is(slint::platform::Key::RightArrow) {
            let step = if is(slint::platform::Key::LeftArrow) {
                6
            } else {
                1
            };
            ui.set_menu_heading_index((ui.get_menu_heading_index().max(0) + step) % 7);
            return true;
        }
        if is(slint::platform::Key::Escape) || is(slint::platform::Key::Tab) {
            ui.set_menu_heading_active(false);
            ui.invoke_restore_menu_focus();
            return true;
        }
        if is(slint::platform::Key::DownArrow) || is(slint::platform::Key::Return) || text == " " {
            let weak = ui.as_weak();
            let title = titles[ui.get_menu_heading_index().clamp(0, 6) as usize];
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() {
                    ui.invoke_show_menu(title.into());
                }
            });
            return true;
        }
        false
    }
}
