//! Persistence of application preferences, connections, and window sessions.
//! [`load`] restores the settings at startup and [`save`] replaces the settings file.

use crate::filesystem::{Location, home};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Named pane location saved for quick navigation.
#[derive(Clone, Serialize, Deserialize)]
pub struct Bookmark {
    /// Display name for this saved location.
    pub name: String,
    /// Location to navigate to when the bookmark is opened.
    pub location: Location,
}

/// Persisted application configuration, including window sessions, connection profiles,
/// and shared preferences.
///
/// Use [`load`] at startup and [`save`] to replace the settings file.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Snapshots used to restore the individual windows of the last session.
    pub window_sessions: Vec<WindowSession>,
    /// Tab titles and location locks, indexed by pane and then by tab in the same order as
    /// [`Self::tabs`].
    pub tab_states: [Vec<crate::navigation::TabState>; 2],
    /// Recently visited locations and their display titles, most recent first.
    pub recent_locations: Vec<crate::navigation::RecentLocation>,
    /// Paths of recently opened files, most recent first.
    pub recent_files: Vec<PathBuf>,
    /// Command aliases and argument templates imported from muCommander.
    pub imported_commands: Vec<crate::config_import::ImportedCommand>,
    /// Named pane and preference snapshots available to restore.
    pub workspaces: Vec<crate::workspaces::Workspace>,
    /// Ordered tab locations for each of the two panes.
    pub tabs: [Vec<Location>; 2],
    /// Zero-based active tab index for each pane.
    pub active_tabs: [usize; 2],
    /// Zero-based index of the active pane (0 or 1).
    pub active_pane: usize,
    /// Named locations offered by the bookmarks menu.
    pub bookmarks: Vec<Bookmark>,
    /// Whether the panes show entries with dot-prefixed names.
    pub show_hidden: bool,
    /// Legacy flag that selects dark mode on restore when the color scheme is set to System.
    pub dark_mode: bool,
    /// Saved remote connection profiles used to reopen remote locations.
    pub connections: Vec<crate::remote::Connection>,
    /// User preferences shared by the file manager and its tools.
    pub preferences: crate::preferences::Preferences,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            window_sessions: Vec::new(),
            tab_states: Default::default(),
            recent_locations: Vec::new(),
            recent_files: Vec::new(),
            imported_commands: Vec::new(),
            workspaces: Vec::new(),
            tabs: [vec![Location::Local(home())], vec![Location::Local(home())]],
            active_tabs: [0, 0],
            active_pane: 0,
            bookmarks: Vec::new(),
            show_hidden: false,
            dark_mode: false,
            connections: Vec::new(),
            preferences: crate::preferences::Preferences::default(),
        }
    }
}

/// Returns the settings JSON path, honoring `RUST_COMMANDER_CONFIG_DIR` and the legacy
/// `MUCOMMANDER_CONFIG_DIR` override before the platform configuration directory.
///
/// # Errors
///
/// Returns an error if the platform configuration directory cannot be determined.
pub fn path() -> Result<PathBuf> {
    Ok(configuration_paths()?.0)
}

#[cfg(test)]
fn configuration_paths() -> Result<(PathBuf, Option<PathBuf>)> {
    // Unit tests construct and drop real adapters, which can autosave. Never
    // resolve the user's configuration, even if an app override is exported.
    // Workers must share the same private directory so recovery data survives
    // their completion and remains available to the UI's Undo/history logic.
    static CONFIG: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let directory = CONFIG.get_or_init(|| tempfile::tempdir().expect("private test configuration"));
    Ok((directory.path().join("settings.json"), None))
}

#[cfg(not(test))]
fn configuration_paths() -> Result<(PathBuf, Option<PathBuf>)> {
    // Existing launch scripts may still use the preview's environment variable.
    if let Some(path) = std::env::var_os("RUST_COMMANDER_CONFIG_DIR")
        .or_else(|| std::env::var_os("MUCOMMANDER_CONFIG_DIR"))
    {
        return Ok((PathBuf::from(path).join("settings.json"), None));
    }
    let dirs = directories::ProjectDirs::from("io.github", "ghillert", "rust-commander")
        .context("Could not determine the configuration directory")?;
    let legacy = directories::ProjectDirs::from("com", "mucommander", "mucommander-rust")
        .map(|dirs| dirs.config_dir().join("settings.json"));
    Ok((dirs.config_dir().join("settings.json"), legacy))
}

/// Loads saved settings, falling back to the legacy location when applicable. Returns
/// defaults when no settings file exists.
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed. Invalid settings are preserved
/// for inspection.
pub fn load() -> Result<Settings> {
    let (current, legacy) = configuration_paths()?;
    load_from_paths(&current, legacy.as_deref())
}

fn load_from_paths(current: &Path, legacy: Option<&Path>) -> Result<Settings> {
    let contents = match fs::read(current) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Read the old file only when the new one does not exist. Saving
            // subsequently uses the new path and leaves the original intact.
            match legacy {
                Some(path) => fs::read(path),
                None => return Ok(Settings::default()),
            }
        }
        result => result,
    };
    match contents {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .context("Settings are invalid; the existing file will be preserved"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.into()),
    }
}

/// Serializes settings to a temporary file beside the configured settings file and replaces
/// it once the contents have been synced.
///
/// # Errors
///
/// Returns an error if the path cannot be resolved or the configuration directory or file
/// cannot be written.
pub fn save(settings: &Settings) -> Result<()> {
    let path = path()?;
    save_to(&path, settings)
}

pub(crate) fn save_to(path: &Path, settings: &Settings) -> Result<()> {
    let directory = path.parent().context("Missing configuration folder")?;
    fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(&serde_json::to_vec_pretty(settings)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_test_autosaves_and_workers_share_private_configuration() {
        let current = path().unwrap();
        let other = std::thread::spawn(|| {
            let other = path().unwrap();
            save(&Settings::default()).unwrap();
            assert!(other.is_file());
            other
        })
        .join()
        .unwrap();
        assert_eq!(
            current, other,
            "Workers must retain the UI's recovery directory"
        );
        assert!(current.starts_with(std::env::temp_dir()));
        assert_eq!(current.file_name().unwrap(), "settings.json");
        save(&Settings::default()).unwrap();
        assert!(load().unwrap().preferences.command_bar);
    }

    #[test]
    fn command_bar_defaults_migrate_once_and_preserve_later_explicit_choices() {
        let mut preferences: crate::preferences::Preferences =
            serde_json::from_str(r#"{"command_bar":false}"#).unwrap();
        preferences.upgrade();
        assert_eq!(preferences.command_bar, cfg!(target_os = "macos"));
        assert_eq!(preferences.command_bar_defaults_version, 1);
        preferences.command_bar = false;
        let saved = serde_json::to_string(&preferences).unwrap();
        let mut loaded: crate::preferences::Preferences = serde_json::from_str(&saved).unwrap();
        loaded.upgrade();
        assert!(
            !loaded.command_bar,
            "An explicit hide choice after migration must survive"
        );
        assert!(crate::preferences::Preferences::default().command_bar);
    }

    #[test]
    fn legacy_preferences_are_loaded_without_moving_or_modifying_the_file() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("current.json");
        let legacy = root.path().join("legacy.json");
        let contents = r#"{"active_pane":1,"show_hidden":true,"preferences":{"font_size":19},"bookmarks":[{"name":"Saved","location":{"Local":"/saved"}}]}"#;
        fs::write(&legacy, contents).unwrap();
        let settings = load_from_paths(&current, Some(&legacy)).unwrap();
        assert_eq!(settings.active_pane, 1);
        assert!(settings.show_hidden);
        assert_eq!(settings.preferences.font_size, 19.0);
        assert_eq!(settings.bookmarks[0].name, "Saved");
        assert_eq!(fs::read_to_string(legacy).unwrap(), contents);
        assert!(!current.exists());
    }

    #[test]
    fn current_settings_take_precedence_over_legacy_settings() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("current.json");
        let legacy = root.path().join("legacy.json");
        fs::write(&current, r#"{"show_hidden":false}"#).unwrap();
        fs::write(&legacy, r#"{"show_hidden":true}"#).unwrap();
        assert!(
            !load_from_paths(&current, Some(&legacy))
                .unwrap()
                .show_hidden
        );
    }

    #[test]
    fn invalid_current_settings_are_preserved_and_do_not_fall_back() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("current.json");
        let legacy = root.path().join("legacy.json");
        fs::write(&current, "invalid settings").unwrap();
        fs::write(&legacy, "{}").unwrap();
        assert!(load_from_paths(&current, Some(&legacy)).is_err());
        assert_eq!(fs::read_to_string(current).unwrap(), "invalid settings");
    }

    #[test]
    fn invalid_legacy_settings_are_preserved_and_reported() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("current.json");
        let legacy = root.path().join("legacy.json");
        fs::write(&legacy, "invalid legacy settings").unwrap();
        assert!(load_from_paths(&current, Some(&legacy)).is_err());
        assert_eq!(
            fs::read_to_string(legacy).unwrap(),
            "invalid legacy settings"
        );
        assert!(!current.exists());
    }

    #[test]
    fn missing_current_and_legacy_settings_use_defaults() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("current.json");
        let legacy = root.path().join("legacy.json");
        let settings = load_from_paths(&current, Some(&legacy)).unwrap();
        assert_eq!(settings.active_pane, 0);
        assert!(!settings.show_hidden);
    }
}

/// Saved pane state and geometry for one desktop window.
///
/// [`Self::settings`] combines this window's layout and navigation state with the
/// current global settings.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowSession {
    /// Snapshot of this window’s panes, tabs, and layout.
    pub workspace: crate::workspaces::Workspace,
    /// Window width in logical pixels at the time of saving.
    pub width: f32,
    /// Window height in logical pixels at the time of saving.
    pub height: f32,
    /// Window position on the desktop as `[x, y]` in physical pixels, when recorded.
    #[serde(default)]
    pub position: Option<[i32; 2]>,
}
impl WindowSession {
    /// Combines this window's saved tabs, active pane, and layout with the supplied global
    /// settings so a restored window keeps its own navigation state.
    pub fn settings(&self, globals: &Settings) -> Settings {
        let mut settings = globals.clone();
        settings.tabs = self.workspace.tabs.clone();
        settings.tab_states = self.workspace.tab_states.clone();
        settings.active_tabs = self.workspace.active_tabs;
        settings.active_pane = self.workspace.active_pane;
        settings.preferences.layout = self.workspace.preferences.layout.clone();
        settings.preferences.single_pane = self.workspace.preferences.single_pane;
        settings
    }
}
