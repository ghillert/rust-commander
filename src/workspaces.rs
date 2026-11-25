//! Named snapshots of pane tabs, their active locations, and preferences.
//! [`persistent_location`] resolves temporary archive paths before saving a workspace.

use crate::{filesystem::Location, preferences::Preferences};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Workspace {
    /// Name displayed when choosing a saved workspace.
    pub name: String,
    /// Saved titles and location locks in the same pane and tab order as [`Self::tabs`].
    #[serde(default)]
    pub tab_states: [Vec<crate::navigation::TabState>; 2],
    /// Tab locations in display order for each of the two panes.
    pub tabs: [Vec<Location>; 2],
    /// Zero-based index of the active tab in each pane.
    pub active_tabs: [usize; 2],
    /// Zero-based index of the pane to activate on restore.
    pub active_pane: usize,
    /// File manager preferences captured when the workspace was saved.
    pub preferences: Preferences,
}
impl Workspace {
    /// Trims a workspace name and returns the value to store.
    ///
    /// # Errors
    ///
    /// Returns an error if the trimmed name is empty, exceeds 80 characters, or contains a
    /// control character.
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
