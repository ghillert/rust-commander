//! File-manager preferences, shortcut bindings, and pane-column formatting.
//! Use [`validate`] to check a candidate configuration before applying it.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Presentation of the current pane location as an editable address or navigable
/// breadcrumbs.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocationBar {
    /// Displays an editable location address.
    #[default]
    Address,
    /// Displays clickable path components for navigating to parent directories.
    Breadcrumbs,
}
impl LocationBar {
    /// Location bar choices in the order shown in preferences.
    pub const ALL: [Self; 2] = [Self::Address, Self::Breadcrumbs];
    /// Returns the display label for the address field or breadcrumb location bar.
    pub fn label(self) -> &'static str {
        match self {
            Self::Address => "Editable address",
            Self::Breadcrumbs => "Clickable breadcrumbs",
        }
    }
}

/// Application preferences shared by file panes, dialogs, and background tools.
///
/// Use [`validate`] before applying edits to an existing configuration.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    /// SMTP server, sender, and delivery settings for emailing selected files.
    pub mail: crate::mail::Settings,
    /// Website opened by the Donate command when the feature is enabled.
    pub donation_url: String,
    /// Bandwidth limit and retry allowance given to new transfer controls.
    pub transfers: crate::transfer_control::Settings,
    /// Named comparison and path-filter settings available in the folder tools.
    pub folder_tool_presets: Vec<crate::comparison::Preset>,
    /// Whether to display the expandable directory tree.
    pub directory_tree: bool,
    /// Whether a pane with a parent location displays a `..` row.
    pub parent_entry: bool,
    /// Filename-to-application rules in match order. The first matching rule wins.
    pub associations: Vec<crate::associations::Rule>,
    /// Desktop style used to choose the palette and shape of controls.
    pub desktop_style: crate::appearance::DesktopStyle,
    /// Whether to follow system contrast settings or force a high-contrast palette.
    pub contrast: crate::high_contrast::Contrast,
    /// Light or dark appearance preference, including following the system.
    pub color_scheme: crate::appearance::ColorScheme,
    /// Whether to request native file icons or use the bundled set.
    pub file_icon_source: FileIconSource,
    /// Presentation of the current location above each file pane.
    pub location_bar: LocationBar,
    /// Base interface text size in logical pixels, from 10 through 24.
    pub font_size: f32,
    /// Whether to display the main toolbar.
    pub toolbar: bool,
    /// Whether to display the navigation sidebar.
    pub sidebar: bool,
    /// Width of the navigation sidebar in logical pixels.
    pub sidebar_width: f32,
    /// Width of the directory tree in logical pixels.
    pub tree_width: f32,
    /// Whether to show the command buttons along the bottom of the window.
    pub command_bar: bool,
    /// Version of the command-bar defaults already applied by [`Self::upgrade`].
    #[serde(default)]
    pub command_bar_defaults_version: u8,
    /// Whether to display the window’s status bar.
    pub status_bar: bool,
    /// Whether to display only the active file pane.
    pub single_pane: bool,
    /// Whether file panes refresh their listings in response to filesystem notifications.
    pub auto_refresh: bool,
    /// Whether to ask for confirmation when quitting the application.
    pub confirm_quit: bool,
    /// Whether file listings place directories before files when sorting.
    pub folders_first: bool,
    /// Primary shortcut text by action ID, using the syntax accepted by [`shortcut`].
    pub shortcuts: BTreeMap<String, String>,
    /// Language used for translated interface strings.
    pub language: crate::i18n::Language,
    /// Visible file-list columns in display order. The name column must come first.
    pub columns: Vec<ColumnSpec>,
    /// Commands shown on the toolbar, in button order.
    pub toolbar_actions: Vec<ToolbarAction>,
    /// Size, age, and automatic cleanup settings for retained recovery records.
    pub recovery: crate::history::Retention,
    /// Whether to display a preview of the focused entry.
    pub preview: bool,
    /// Width of the preview pane in logical pixels.
    pub preview_width: f32,
    /// Orientation and split ratio used when displaying both file panes.
    pub layout: crate::navigation::Layout,
    /// Action IDs shown on the command bar in display order, up to 12 entries.
    pub command_actions: Vec<String>,
    /// Optional second shortcut text by action ID, using the same syntax as the primary
    /// binding.
    pub alternate_shortcuts: BTreeMap<String, String>,
    /// Whether to scale configured column widths proportionally to fit the pane.
    pub auto_size_columns: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            mail: Default::default(),
            donation_url: crate::support::DONATION.into(),
            transfers: Default::default(),
            folder_tool_presets: Vec::new(),
            directory_tree: false,
            parent_entry: true,
            associations: Vec::new(),
            desktop_style: Default::default(),
            contrast: Default::default(),
            color_scheme: Default::default(),
            file_icon_source: Default::default(),
            location_bar: Default::default(),
            font_size: 14.0,
            toolbar: true,
            sidebar: true,
            sidebar_width: 186.0,
            tree_width: 220.0,
            recovery: Default::default(),
            preview: false,
            preview_width: 300.0,
            layout: Default::default(),
            auto_size_columns: false,
            alternate_shortcuts: [("MarkSelectedFile".into(), "Insert".into())]
                .into_iter()
                .collect(),
            command_actions: ["Help", "View", "Edit", "Copy", "Move", "Mkdir", "Trash"]
                .map(str::to_owned)
                .to_vec(),
            command_bar: true,
            command_bar_defaults_version: 1,
            status_bar: true,
            single_pane: false,
            auto_refresh: true,
            confirm_quit: true,
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
                ("RecallWindow1", "Command+Alt+1"),
                ("RecallWindow2", "Command+Alt+2"),
                ("RecallWindow3", "Command+Alt+3"),
                ("RecallWindow4", "Command+Alt+4"),
                ("RecallWindow5", "Command+Alt+5"),
                ("RecallWindow6", "Command+Alt+6"),
                ("RecallWindow7", "Command+Alt+7"),
                ("RecallWindow8", "Command+Alt+8"),
                ("RecallWindow9", "Command+Alt+9"),
                ("RecallWindow10", "Command+Alt+0"),
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

/// Policy for choosing between operating-system file icons and the bundled icon set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileIconSource {
    /// Requests native file icons and uses bundled icons when a native icon is unavailable.
    #[default]
    SystemWithFallback,
    /// Uses the bundled filename and folder icons.
    BuiltIn,
}

impl FileIconSource {
    /// File icon source choices in the order shown in preferences.
    pub const ALL: [Self; 2] = [Self::SystemWithFallback, Self::BuiltIn];

    /// Returns the preferences label for the selected source of file icons.
    pub fn label(self) -> &'static str {
        match self {
            Self::SystemWithFallback => "System with built-in fallback",
            Self::BuiltIn => "Built-in icons only",
        }
    }
}

/// Canonical name of the non-modifier key in a shortcut.
///
/// [`Self::from_name`] normalizes common aliases before matching shortcuts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key(String);
impl Key {
    /// Resolves a case-insensitive shortcut key name or supported alias to its canonical
    /// logical key. An unknown name yields `None`.
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
/// Modifier keys required by a shortcut. Command and Control names map to the same
/// command flag when parsed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Requires the command modifier, written as Command or Control in shortcut text.
    pub command: bool,
    /// Requires Shift as part of the shortcut.
    pub shift: bool,
    /// Requires Alt (Option on macOS) as part of the shortcut.
    pub alt: bool,
}
/// Normalized keyboard binding consisting of a logical key and its required modifiers.
///
/// Use [`shortcut()`] to parse the text stored in preferences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// Canonical non-modifier key for the binding.
    pub logical_key: Key,
    /// Modifier keys required along with the logical key.
    pub modifiers: Modifiers,
}

/// Parses a `+`-separated keyboard shortcut with one logical key. Command and Control map
/// to the same command modifier, and Option is an alias for Alt.
///
/// # Errors
///
/// Returns an error for an unknown key, a missing key, or more than one non-modifier key.
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

// Keep saved Donate buttons and bindings available to builds that enable them.
fn command_enabled(id: &str) -> bool {
    cfg!(feature = "donations") || id != "Donate"
}

/// Checks preferences before applying or importing them, including layout limits, toolbar
/// and column choices, transfer and recovery settings, and shortcut assignments.
///
/// # Errors
///
/// Returns an error for an invalid setting, an invalid or duplicate shortcut, or an
/// incomplete configured mail profile.
pub fn validate(preferences: &Preferences) -> Result<()> {
    if !preferences.mail.host.is_empty()
        || !preferences.mail.sender.is_empty()
        || !preferences.mail.username.is_empty()
    {
        preferences.mail.validate()?;
    }
    if preferences.transfers.retries > 8 {
        bail!("Choose 0 to 8 retries.");
    }
    if !(1..=1_000_000).contains(&preferences.recovery.max_mib)
        || !(1..=3650).contains(&preferences.recovery.max_days)
    {
        bail!("Recovery limits must be 1–1,000,000 MiB and 1–3650 days.");
    }
    if !preferences.font_size.is_finite() || !(10.0..=24.0).contains(&preferences.font_size) {
        bail!("Font size must be between 10 and 24.");
    }
    for (width, minimum, maximum) in [
        (preferences.sidebar_width, 120.0, 360.0),
        (preferences.tree_width, 160.0, 480.0),
        (preferences.preview_width, 180.0, 640.0),
    ] {
        if !width.is_finite() || width < minimum || width > maximum {
            bail!("Side panel widths are out of range.");
        }
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
        if (command_enabled(name) && !catalog.contains(name)) || !command_names.insert(name) {
            bail!("Command-bar actions must be supported and unique");
        }
    }
    if !preferences.layout.ratio.is_finite() || !(0.15..=0.85).contains(&preferences.layout.ratio) {
        bail!("Pane ratio must be between 0.15 and 0.85");
    }
    let mut keys = Vec::new();
    for (_, value) in preferences
        .active_shortcuts()
        .filter(|(_, v)| !v.is_empty())
    {
        let key = shortcut(value)?;
        if keys.contains(&key) {
            bail!("Duplicate shortcut: {value}");
        }
        keys.push(key);
    }
    Ok(())
}

/// An entry attribute that can appear in a file pane column and determine its sort
/// order.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Column {
    /// Entry name as displayed in the file pane.
    Name,
    /// File size in human-readable units. Directories and unknown sizes show a dash.
    Size,
    /// Last modification time in the local time zone.
    Modified,
    /// Final filename extension, without the leading dot.
    Extension,
    /// Entry category, such as file, folder, archive, or link.
    Kind,
    /// Owner identity for local files when available.
    Owner,
    /// Group identity for local files when available.
    Group,
    /// Unix mode or Windows read-only status for local files.
    Permissions,
}
impl Column {
    /// Available columns in chooser order.
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
    /// Returns the column heading displayed in a file pane.
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
    /// Returns the listing sort key corresponding to this column.
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
    /// Formats the entry's value for this column. Set `local` when native ownership and
    /// permissions are available. Missing values use a dash.
    pub fn value(self, entry: &crate::filesystem::Entry, local: bool) -> String {
        match self {
            Self::Name => entry.name.clone(),
            Self::Size => {
                if entry.is_dir || !entry.size_known {
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
                    let (owner, group) = crate::metadata::ownership_labels(&entry.path);
                    if self == Self::Owner { owner } else { group }.unwrap_or_else(|| "—".into())
                } else {
                    "—".into()
                }
            }
            Self::Permissions => {
                #[cfg(windows)]
                if local {
                    return std::fs::symlink_metadata(&entry.path)
                        .map(|m| {
                            crate::i18n::text(if m.permissions().readonly() {
                                "Read-only"
                            } else {
                                "Writable"
                            })
                            .into_owned()
                        })
                        .unwrap_or_else(|_| "—".into());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if local {
                        return std::fs::symlink_metadata(&entry.path)
                            .map(|meta| {
                                format!(
                                    "{} ({:04o})",
                                    crate::metadata::symbolic(meta.permissions().mode()),
                                    meta.permissions().mode() & 0o7777
                                )
                            })
                            .unwrap_or_else(|_| "—".into());
                    }
                }
                let _ = local;
                "—".into()
            }
        }
    }
}
/// Configured type and width of a file pane column.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ColumnSpec {
    /// Entry attribute shown in this column.
    pub kind: Column,
    /// Configured width in logical pixels (40 through 800), before optional scaling.
    pub width: f32,
}
/// A file manager command that can appear as a toolbar button.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolbarAction {
    /// Opens the prompt to create a directory in the active pane.
    NewFolder,
    /// Starts copying the active pane’s chosen entries.
    Copy,
    /// Starts moving the active pane’s chosen entries.
    Move,
    /// Opens the rename prompt for the chosen entry.
    Rename,
    /// Opens the file search form for the active location.
    Search,
    /// Opens the embedded terminal or toggles its visibility if already running.
    Terminal,
    /// Opens the archive tools for the current pane selections.
    Archive,
    /// Requests undo of a journaled file operation.
    Undo,
    /// Opens the transfer queue and operation history.
    Queue,
}
impl ToolbarAction {
    /// Available toolbar actions in chooser order.
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
    /// Returns the user-facing name of the toolbar command.
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
    pub(crate) fn active_shortcuts(&self) -> impl Iterator<Item = (&String, &String)> {
        self.shortcuts
            .iter()
            .chain(&self.alternate_shortcuts)
            .filter(|(id, _)| command_enabled(id))
    }

    /// Add new bindings to legacy settings without stealing an existing custom shortcut.
    pub fn upgrade(&mut self) {
        if self.command_bar_defaults_version == 0 {
            // Older macOS defaults hid the Commander function buttons. Restore
            // them once; subsequent explicit choices to hide them are retained.
            if cfg!(target_os = "macos") {
                self.command_bar = true;
            }
            self.command_bar_defaults_version = 1;
        }
        for (name, binding) in Self::default().alternate_shortcuts {
            if self.alternate_shortcuts.get(&name) == Some(&binding)
                && self.shortcuts.iter().any(|(id, value)| {
                    command_enabled(id) && shortcut(value).ok() == shortcut(&binding).ok()
                })
            {
                self.alternate_shortcuts.remove(&name);
            }
        }

        for (name, binding) in Self::default().shortcuts {
            if self.shortcuts.contains_key(&name) {
                continue;
            }
            let reserved = self.shortcuts.iter().any(|(id, value)| {
                command_enabled(id)
                    && !value.is_empty()
                    && shortcut(value).ok() == shortcut(&binding).ok()
            });
            self.shortcuts
                .insert(name, if reserved { String::new() } else { binding });
        }
    }
}
