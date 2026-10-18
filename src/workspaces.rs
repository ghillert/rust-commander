use crate::{filesystem::Location, preferences::Preferences};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    #[serde(default)]
    pub tab_states: [Vec<crate::navigation::TabState>; 2],
    pub tabs: [Vec<Location>; 2],
    pub active_tabs: [usize; 2],
    pub active_pane: usize,
    pub preferences: Preferences,
}
impl Workspace {
    pub fn validate_name(name: &str) -> anyhow::Result<String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            anyhow::bail!("Use a workspace name of 1–80 characters");
        }
        Ok(name.into())
    }
}

/// Private archive caches disappear on exit; restore their persistent containing location.
pub fn persistent_location(location: &Location) -> Location {
    let mut location = location.clone();
    for _ in 0..8 {
        if let Location::Archive { file, .. } = &location
            && let Some(origin) = crate::archive_tools::origin(file)
        {
            location = origin;
        } else {
            break;
        }
    }
    location
}
