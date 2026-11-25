//! Persistent pane state and reusable navigation commands.
use crate::filesystem::Location;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// User-customized title and optional location lock for one tab.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabState {
    /// Custom tab title. An empty value lets the UI derive a title from the location.
    pub title: String,
    /// Location this tab is pinned to. Navigation elsewhere can be opened in another tab.
    pub locked: Option<Location>,
}

impl TabState {
    /// Copies the tab title and converts a locked location to the form used in saved
    /// workspaces.
    pub fn persistent(&self) -> Self {
        Self {
            title: self.title.clone(),
            locked: self
                .locked
                .as_ref()
                .map(crate::workspaces::persistent_location),
        }
    }
}

/// Direction in which the two file panes are arranged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    /// Arrange the two panes horizontally, left and right.
    #[default]
    SideBySide,
    /// Arrange the two panes vertically, top and bottom.
    Stacked,
}

/// Orientation and space allocation of the split between the file panes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Layout {
    /// Direction along which the file panes share the available space.
    pub orientation: Orientation,
    /// Fraction of the split assigned to the first pane, from 0.15 through 0.85.
    pub ratio: f32,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            orientation: Orientation::SideBySide,
            ratio: 0.5,
        }
    }
}
impl Layout {
    /// Sets the pane split ratio, clamping finite values to `0.15..=0.85`. Nonfinite input
    /// leaves the current ratio in place.
    pub fn set_ratio(&mut self, ratio: f32) {
        if ratio.is_finite() {
            self.ratio = ratio.clamp(0.15, 0.85);
        }
    }
}

/// A previously visited pane location with the title shown in the recent-locations list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentLocation {
    /// Display label saved with the recent location.
    pub title: String,
    /// Persistent location to reopen when this history entry is selected.
    pub location: Location,
}

/// Moves this location to the front of the recent list under the given title. Stores the
/// persistent form, removes any previous occurrence, and keeps up to 50 locations.
pub fn remember(list: &mut Vec<RecentLocation>, title: String, location: Location) {
    let location = crate::workspaces::persistent_location(&location);
    list.retain(|item| item.location != location);
    list.insert(0, RecentLocation { title, location });
    list.truncate(50);
}

/// Select visible matching entries only. Validation happens before selection changes.
pub fn select_pattern(
    tab: &mut crate::model::Tab,
    patterns: &str,
    selected: bool,
    case_sensitive: bool,
    hidden: bool,
) -> anyhow::Result<usize> {
    let mut builder = globset::GlobSetBuilder::new();
    let patterns = patterns
        .split(';')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>();
    anyhow::ensure!(!patterns.is_empty(), "Enter at least one wildcard pattern.");
    for pattern in patterns {
        builder.add(
            globset::GlobBuilder::new(pattern)
                .case_insensitive(!case_sensitive)
                .literal_separator(false)
                .build()?,
        );
    }
    let matcher = builder.build()?;
    let matches = tab
        .visible(hidden)
        .into_iter()
        .filter(|&i| matcher.is_match(&tab.entries[i].name))
        .map(|i| tab.entries[i].path.clone())
        .collect::<Vec<_>>();
    for path in &matches {
        if selected {
            tab.selected.insert(path.clone());
        } else {
            tab.selected.remove(path);
        }
    }
    Ok(matches.len())
}

/// Moves the cursor to a visible filename starting with `query`, using Unicode
/// normalization and case-insensitive matching. The search wraps in the requested
/// direction. Set `cycle` to start beyond the current row. Returns whether it found a
/// match.
pub fn quick_find(
    tab: &mut crate::model::Tab,
    query: &str,
    backwards: bool,
    cycle: bool,
    hidden: bool,
) -> bool {
    let query = query.nfc().flat_map(char::to_lowercase).collect::<String>();
    if query.is_empty() {
        return false;
    }
    let visible = tab.visible(hidden);
    if visible.is_empty() {
        return false;
    }
    let current = visible
        .iter()
        .position(|&i| Some(&tab.entries[i].path) == tab.cursor.as_ref())
        .unwrap_or(0);
    for offset in 0..visible.len() {
        let step = offset + usize::from(cycle);
        let index = if backwards {
            (current + visible.len() - step % visible.len()) % visible.len()
        } else {
            (current + step) % visible.len()
        };
        let entry = &tab.entries[visible[index]];
        if entry
            .name
            .nfc()
            .flat_map(char::to_lowercase)
            .collect::<String>()
            .starts_with(&query)
        {
            tab.cursor = Some(entry.path.clone());
            tab.scroll_to_cursor = true;
            return true;
        }
    }
    false
}

/// Directories keep their complete names; files lose only the final extension.
pub fn base_name(entry: &crate::filesystem::Entry) -> String {
    if entry.is_dir {
        entry.name.clone()
    } else {
        entry
            .path
            .file_stem()
            .unwrap_or(entry.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
}
