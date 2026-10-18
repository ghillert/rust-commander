//! Persistent pane state and reusable navigation commands.
use crate::filesystem::Location;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabState {
    pub title: String,
    pub locked: Option<Location>,
}

impl TabState {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Orientation {
    #[default]
    SideBySide,
    Stacked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub orientation: Orientation,
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
    pub fn set_ratio(&mut self, ratio: f32) {
        if ratio.is_finite() {
            self.ratio = ratio.clamp(0.15, 0.85);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecentLocation {
    pub title: String,
    pub location: Location,
}

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
