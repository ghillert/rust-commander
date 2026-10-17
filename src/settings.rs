use crate::filesystem::{Location, home};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub location: Location,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
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
    if let Some(path) = std::env::var_os("MUCOMMANDER_CONFIG_DIR") {
        return Ok(PathBuf::from(path).join("settings.json"));
    }
    let dirs = directories::ProjectDirs::from("com", "mucommander", "mucommander-rust")
        .context("Could not determine the configuration directory")?;
    Ok(dirs.config_dir().join("settings.json"))
}

pub fn load() -> Result<Settings> {
    let path = path()?;
    match fs::read(&path) {
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
