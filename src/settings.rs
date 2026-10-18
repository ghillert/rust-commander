use crate::filesystem::{Location, home};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub location: Location,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub workspaces: Vec<crate::workspaces::Workspace>,
    pub tabs: [Vec<Location>; 2],
    pub active_tabs: [usize; 2],
    pub active_pane: usize,
    pub bookmarks: Vec<Bookmark>,
    pub show_hidden: bool,
    pub dark_mode: bool,
    pub connections: Vec<crate::remote::Connection>,
    pub preferences: crate::preferences::Preferences,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
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

pub fn path() -> Result<PathBuf> {
    Ok(configuration_paths()?.0)
}

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

pub fn save(settings: &Settings) -> Result<()> {
    let path = path()?;
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
