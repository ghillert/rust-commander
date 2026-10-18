use anyhow::{Result, bail};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub desktop_style: crate::appearance::DesktopStyle,
    pub color_scheme: crate::appearance::ColorScheme,
    pub font_size: f32,
    pub toolbar: bool,
    pub command_bar: bool,
    pub status_bar: bool,
    pub single_pane: bool,
    pub auto_refresh: bool,
    pub folders_first: bool,
    pub shortcuts: BTreeMap<String, String>,
    pub language: crate::i18n::Language,
    pub columns: Vec<ColumnSpec>,
    pub toolbar_actions: Vec<ToolbarAction>,
    pub recovery: crate::history::Retention,
    pub preview: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            desktop_style: Default::default(),
            color_scheme: Default::default(),
            font_size: 14.0,
            toolbar: true,
            recovery: Default::default(),
            preview: false,
            command_bar: !cfg!(target_os = "macos"),
            status_bar: true,
            single_pane: false,
            auto_refresh: true,
            folders_first: true,
            language: Default::default(),
            columns: vec![
                ColumnSpec {
                    kind: Column::Name,
                    width: 300.0,
                },
                ColumnSpec {
                    kind: Column::Size,
                    width: 100.0,
                },
                ColumnSpec {
                    kind: Column::Modified,
                    width: 180.0,
                },
            ],
            toolbar_actions: vec![
                ToolbarAction::NewFolder,
                ToolbarAction::Copy,
                ToolbarAction::Move,
            ],
            shortcuts: [
                ("Help", "F1"),
                ("Rename", "F2"),
                ("View", "F3"),
                ("Edit", "F4"),
                ("Copy", "F5"),
                ("Move", "F6"),
                ("Mkdir", "F7"),
                ("Trash", "F8"),
                ("NewTab", "Command+T"),
                ("CloseTab", "Command+W"),
                ("Refresh", "Command+R"),
                ("Bookmark", "Command+D"),
                ("Search", "Command+Shift+F"),
                ("CopyClipboard", "Command+C"),
                ("CutClipboard", "Command+X"),
                ("PasteClipboard", "Command+V"),
                ("Undo", "Command+Z"),
                ("Queue", "Command+J"),
            ]
            .into_iter()
            .map(|(a, key)| (a.into(), key.into()))
            .collect(),
        }
    }
}

pub fn shortcut(text: &str) -> Result<egui::KeyboardShortcut> {
    let mut modifiers = egui::Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_lowercase().as_str() {
            "command" | "cmd" | "ctrl" | "control" => modifiers.command = true,
            "shift" => modifiers.shift = true,
            "alt" | "option" => modifiers.alt = true,
            _ => {
                if key.is_some() {
                    bail!("Shortcut must contain one key.");
                }
                key = egui::Key::from_name(part);
                if key.is_none() {
                    bail!("Unknown key: {part}");
                }
            }
        }
    }
    Ok(egui::KeyboardShortcut::new(
        modifiers,
        key.ok_or_else(|| anyhow::anyhow!("Shortcut must contain a key"))?,
    ))
}

pub fn validate(preferences: &Preferences) -> Result<()> {
    if !preferences.font_size.is_finite() || !(10.0..=24.0).contains(&preferences.font_size) {
        bail!("Font size must be between 10 and 24.");
    }
    if preferences.columns.is_empty() || preferences.columns[0].kind != Column::Name {
        bail!("The Name column must be first.");
    }
    let mut columns = Vec::new();
    for column in &preferences.columns {
        if !column.width.is_finite()
            || !(40.0..=800.0).contains(&column.width)
            || columns.contains(&column.kind)
        {
            bail!("Column widths must be 40–800 and columns must be unique.");
        }
        columns.push(column.kind);
    }
    let mut actions = Vec::new();
    for action in &preferences.toolbar_actions {
        if actions.contains(action) {
            bail!("Toolbar actions must be unique.");
        }
        actions.push(*action);
    }
    let mut keys = Vec::new();
    for value in preferences.shortcuts.values().filter(|v| !v.is_empty()) {
        let key = shortcut(value)?;
        if keys.contains(&key) {
            bail!("Duplicate shortcut: {value}");
        }
        keys.push(key);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Column {
    Name,
    Size,
    Modified,
    Extension,
    Kind,
    Permissions,
}
impl Column {
    pub const ALL: [Self; 6] = [
        Self::Name,
        Self::Size,
        Self::Modified,
        Self::Extension,
        Self::Kind,
        Self::Permissions,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Size => "Size",
            Self::Modified => "Modified",
            Self::Extension => "Extension",
            Self::Kind => "Type",
            Self::Permissions => "Permissions",
        }
    }
    pub fn sort(self) -> Option<crate::filesystem::Sort> {
        Some(match self {
            Self::Size => crate::filesystem::Sort::Size,
            Self::Modified => crate::filesystem::Sort::Modified,
            Self::Extension => crate::filesystem::Sort::Extension,
            Self::Kind => crate::filesystem::Sort::Kind,
            Self::Permissions => return None,
            _ => crate::filesystem::Sort::Name,
        })
    }
    pub fn value(self, entry: &crate::filesystem::Entry, local: bool) -> String {
        match self {
            Self::Name => entry.name.clone(),
            Self::Size => {
                if entry.is_dir {
                    "—".into()
                } else {
                    crate::filesystem::human_size(entry.size)
                }
            }
            Self::Modified => entry
                .modified
                .map(|time| {
                    chrono::DateTime::<chrono::Local>::from(time)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_else(|| "—".into()),
            Self::Extension => entry
                .path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            Self::Kind => crate::i18n::text(entry.kind()).into_owned(),
            Self::Permissions => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if local {
                        return std::fs::symlink_metadata(&entry.path)
                            .map(|meta| format!("{:03o}", meta.permissions().mode() & 0o777))
                            .unwrap_or_else(|_| "—".into());
                    }
                }
                let _ = local;
                "—".into()
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColumnSpec {
    pub kind: Column,
    pub width: f32,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolbarAction {
    NewFolder,
    Copy,
    Move,
    Rename,
    Search,
    Terminal,
    Archive,
    Undo,
    Queue,
}
impl ToolbarAction {
    pub const ALL: [Self; 9] = [
        Self::NewFolder,
        Self::Copy,
        Self::Move,
        Self::Rename,
        Self::Search,
        Self::Terminal,
        Self::Archive,
        Self::Undo,
        Self::Queue,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::NewFolder => "New folder",
            Self::Copy => "Copy",
            Self::Move => "Move",
            Self::Rename => "Rename",
            Self::Search => "Search",
            Self::Terminal => "Open terminal",
            Self::Archive => "Archive tools",
            Self::Undo => "Undo",
            Self::Queue => "Transfer queue and history",
        }
    }
    pub(crate) fn action(self) -> crate::app::Action {
        use crate::app::Action;
        match self {
            Self::NewFolder => Action::Mkdir,
            Self::Copy => Action::Copy,
            Self::Move => Action::Move,
            Self::Rename => Action::Rename,
            Self::Search => Action::Search,
            Self::Terminal => Action::Terminal,
            Self::Archive => Action::ArchiveTools,
            Self::Undo => Action::Undo,
            Self::Queue => Action::Queue,
        }
    }
}

impl Preferences {
    /// Add new bindings to legacy settings without stealing an existing custom shortcut.
    pub fn upgrade(&mut self) {
        for (name, binding) in [
            ("Undo", "Command+Z"),
            ("Queue", "Command+J"),
            ("Redo", "Command+Shift+Z"),
            ("Preview", "Command+P"),
            ("QuickLook", "Command+Shift+P"),
        ] {
            if self.shortcuts.contains_key(name) {
                continue;
            }
            let reserved = self
                .shortcuts
                .values()
                .filter(|value| !value.is_empty())
                .any(|value| shortcut(value).ok() == shortcut(binding).ok());
            self.shortcuts.insert(
                name.into(),
                if reserved {
                    String::new()
                } else {
                    binding.into()
                },
            );
        }
    }
}
