use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub directory_tree: bool,
    pub associations: Vec<crate::associations::Rule>,
    pub desktop_style: crate::appearance::DesktopStyle,
    pub color_scheme: crate::appearance::ColorScheme,
    pub font_size: f32,
    pub toolbar: bool,
    pub sidebar: bool,
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
    pub layout: crate::navigation::Layout,
    pub command_actions: Vec<String>,
    pub alternate_shortcuts: BTreeMap<String, String>,
    pub auto_size_columns: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            directory_tree: false,
            associations: Vec::new(),
            desktop_style: Default::default(),
            color_scheme: Default::default(),
            font_size: 14.0,
            toolbar: true,
            sidebar: true,
            recovery: Default::default(),
            preview: false,
            layout: Default::default(),
            auto_size_columns: false,
            alternate_shortcuts: [("MarkSelectedFile".into(), "Insert".into())]
                .into_iter()
                .collect(),
            command_actions: ["Help", "View", "Edit", "Copy", "Move", "Mkdir", "Trash"]
                .map(str::to_owned)
                .to_vec(),
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
                ("MarkAll", "Command+A"),
                ("ClearSelection", "Escape"),
                ("Redo", "Command+Shift+Z"),
                ("SaveText", "Command+S"),
                ("Preview", "Command+P"),
                ("QuickLook", "Command+Shift+P"),
                ("Quit", "Command+Q"),
                ("Preferences", "Command+Comma"),
                ("FocusAddress", "Command+L"),
                ("FocusFilter", "Command+F"),
                ("Back", "Alt+ArrowLeft"),
                ("Forward", "Alt+ArrowRight"),
                ("ToggleHidden", "Command+Shift+Period"),
                ("NewWindow", "Command+N"),
                ("CloseWindow", "Command+Shift+W"),
                ("QuickLists", "Command+Shift+L"),
                ("QuickFindNext", "Command+G"),
                ("QuickFindPrevious", "Command+Shift+G"),
                ("FileContextMenu", "Shift+F10"),
                ("FocusNext", "Command+F6"),
                ("FocusPrevious", "Command+Shift+F6"),
                ("Open", "Enter"),
                ("Parent", "Backspace"),
                ("SwitchPane", "Tab"),
                ("SelectNextRow", "ArrowDown"),
                ("SelectPreviousRow", "ArrowUp"),
                ("SelectNextPage", "PageDown"),
                ("SelectPreviousPage", "PageUp"),
                ("SelectFirstRow", "Home"),
                ("SelectLastRow", "End"),
                ("MarkNextRow", "Shift+ArrowDown"),
                ("MarkPreviousRow", "Shift+ArrowUp"),
                ("MarkNextPage", "Shift+PageDown"),
                ("MarkPreviousPage", "Shift+PageUp"),
                ("MarkToFirstRow", "Shift+Home"),
                ("MarkToLastRow", "Shift+End"),
                ("MarkSelectedFile", "Space"),
                ("SelectNextBlock", "Command+ArrowDown"),
                ("SelectPreviousBlock", "Command+ArrowUp"),
                ("MarkNextBlock", "Command+Shift+ArrowDown"),
                ("MarkPreviousBlock", "Command+Shift+ArrowUp"),
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key(String);
impl Key {
    pub fn from_name(name: &str) -> Option<Self> {
        let upper = name.to_ascii_uppercase();
        let canonical = match upper.as_str() {
            "UP" | "ARROWUP" => "ARROWUP",
            "DOWN" | "ARROWDOWN" => "ARROWDOWN",
            "LEFT" | "ARROWLEFT" => "ARROWLEFT",
            "RIGHT" | "ARROWRIGHT" => "ARROWRIGHT",
            "ESC" | "ESCAPE" => "ESCAPE",
            "RETURN" | "ENTER" => "ENTER",
            " " | "SPACE" => "SPACE",
            "," | "COMMA" => "COMMA",
            "." | "PERIOD" => "PERIOD",
            "/" | "SLASH" => "SLASH",
            "\\" | "BACKSLASH" => "BACKSLASH",
            ";" | "SEMICOLON" => "SEMICOLON",
            ":" | "COLON" => "COLON",
            "-" | "MINUS" => "MINUS",
            "+" | "PLUS" => "PLUS",
            "=" | "EQUALS" => "EQUALS",
            "[" | "OPENBRACKET" => "OPENBRACKET",
            "]" | "CLOSEBRACKET" => "CLOSEBRACKET",
            "`" | "BACKTICK" => "BACKTICK",
            "'" | "QUOTE" => "QUOTE",
            "?" | "QUESTIONMARK" => "QUESTIONMARK",
            "!" | "EXCLAMATIONMARK" => "EXCLAMATIONMARK",
            "|" | "PIPE" => "PIPE",
            "TAB" | "BACKSPACE" | "DELETE" | "INSERT" | "HOME" | "END" | "PAGEUP" | "PAGEDOWN" => {
                upper.as_str()
            }
            _ => {
                if upper.len() == 1 && upper.as_bytes()[0].is_ascii_alphanumeric() {
                    return Some(Self(upper));
                }
                if upper
                    .strip_prefix('F')
                    .and_then(|n| n.parse::<u8>().ok())
                    .is_some_and(|n| (1..=35).contains(&n))
                {
                    return Some(Self(upper));
                }
                return None;
            }
        };
        Some(Self(canonical.into()))
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub logical_key: Key,
    pub modifiers: Modifiers,
}

pub fn shortcut(text: &str) -> Result<Shortcut> {
    let mut modifiers = Modifiers::default();
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "command" | "cmd" | "ctrl" | "control" => modifiers.command = true,
            "shift" => modifiers.shift = true,
            "alt" | "option" => modifiers.alt = true,
            _ => {
                if key.is_some() {
                    bail!("Shortcut must contain one key.");
                }
                key = Key::from_name(part);
                if key.is_none() {
                    bail!("Unknown key: {part}");
                }
            }
        }
    }
    Ok(Shortcut {
        modifiers,
        logical_key: key.ok_or_else(|| anyhow::anyhow!("Shortcut must contain a key"))?,
    })
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
    let catalog = crate::app::slint_ui::supported_commands();
    if preferences.command_actions.len() > 12 {
        bail!("Choose at most 12 command-bar actions");
    }
    let mut command_names = std::collections::BTreeSet::new();
    for name in &preferences.command_actions {
        if !catalog.contains(name) || !command_names.insert(name) {
            bail!("Command-bar actions must be supported and unique");
        }
    }
    if !preferences.layout.ratio.is_finite() || !(0.15..=0.85).contains(&preferences.layout.ratio) {
        bail!("Pane ratio must be between 0.15 and 0.85");
    }
    let mut keys = Vec::new();
    for value in preferences
        .shortcuts
        .values()
        .chain(preferences.alternate_shortcuts.values())
        .filter(|v| !v.is_empty())
    {
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
    Owner,
    Group,
    Permissions,
}
impl Column {
    pub const ALL: [Self; 8] = [
        Self::Name,
        Self::Size,
        Self::Modified,
        Self::Extension,
        Self::Kind,
        Self::Owner,
        Self::Group,
        Self::Permissions,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Size => "Size",
            Self::Modified => "Modified",
            Self::Extension => "Extension",
            Self::Kind => "Type",
            Self::Owner => "Owner",
            Self::Group => "Group",
            Self::Permissions => "Permissions",
        }
    }
    pub fn sort(self) -> Option<crate::filesystem::Sort> {
        Some(match self {
            Self::Size => crate::filesystem::Sort::Size,
            Self::Modified => crate::filesystem::Sort::Modified,
            Self::Extension => crate::filesystem::Sort::Extension,
            Self::Kind => crate::filesystem::Sort::Kind,
            Self::Permissions => crate::filesystem::Sort::Permissions,
            Self::Owner => crate::filesystem::Sort::Owner,
            Self::Group => crate::filesystem::Sort::Group,
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
            Self::Owner | Self::Group => {
                if local {
                    let (owner, group, _) = crate::filesystem::ownership(&entry.path);
                    if self == Self::Owner {
                        owner.map(|n| n.to_string())
                    } else {
                        group.map(|n| n.to_string())
                    }
                    .unwrap_or_else(|| "—".into())
                } else {
                    "—".into()
                }
            }
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
        for (name, binding) in Self::default().alternate_shortcuts {
            if self.alternate_shortcuts.get(&name) == Some(&binding)
                && self
                    .shortcuts
                    .values()
                    .any(|value| shortcut(value).ok() == shortcut(&binding).ok())
            {
                self.alternate_shortcuts.remove(&name);
            }
        }

        for (name, binding) in Self::default().shortcuts {
            if self.shortcuts.contains_key(&name) {
                continue;
            }
            let reserved = self
                .shortcuts
                .values()
                .filter(|value| !value.is_empty())
                .any(|value| shortcut(value).ok() == shortcut(&binding).ok());
            self.shortcuts
                .insert(name, if reserved { String::new() } else { binding });
        }
    }
}
