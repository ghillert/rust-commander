//! Archive dialog state and actions derived from the two pane selections.
//! [`ArchiveForm::new`] fills in the source, member prefix, and suggested destination.

use crate::{
    filesystem::{Entry, Location},
    operations::Operation,
};
use std::path::{Path, PathBuf};
#[derive(Default)]
pub struct ArchiveForm {
    pub file: String,
    pub source_display: Option<String>,
    pub destination: String,
    pub password: String,
    pub compression_level: Option<u8>,
    pub mode: usize,
    pub edit_mode: usize,
    pub additions: String,
    pub source: String,
    pub target: String,
    pub prefix: PathBuf,
    pub selected: Vec<PathBuf>,
    pub sources: Vec<PathBuf>,
    pub error: Option<String>,
}
pub enum ArchiveAction {
    Run(Box<Operation>),
    Unlock(PathBuf, String),
    Browse(PathBuf),
}
impl ArchiveForm {
    /// Initializes archive tools from both pane selections. An open archive supplies its
    /// file and member prefix, and the other local pane is preferred for the suggested
    /// output path.
    pub fn new(
        location: &Location,
        chosen: &[Entry],
        other: &Location,
        other_chosen: &[Entry],
    ) -> Self {
        let file = match location {
            Location::Archive { file, .. } => file.display().to_string(),
            _ => chosen
                .first()
                .map(|entry| entry.path.display().to_string())
                .unwrap_or_default(),
        };
        let prefix = match location {
            Location::Archive { prefix, .. } => prefix.clone(),
            _ => PathBuf::new(),
        };
        let source = chosen
            .first()
            .map(|entry| entry.path.display().to_string())
            .unwrap_or_default();
        let destination = other
            .local()
            .or_else(|| location.local())
            .unwrap_or(Path::new("."))
            .join("Archive.zip")
            .display()
            .to_string();
        Self {
            source_display: match location {
                Location::Archive { file, .. } if crate::archive_tools::origin(file).is_some() => {
                    Some(crate::archive_tools::source_label(file))
                }
                _ => None,
            },
            file,
            destination,
            source: if matches!(location, Location::Archive { .. }) {
                source.clone()
            } else {
                String::new()
            },
            target: if matches!(location, Location::Archive { .. }) {
                source
            } else {
                String::new()
            },
            prefix,
            selected: if matches!(location, Location::Archive { .. }) {
                chosen.iter().map(|entry| entry.path.clone()).collect()
            } else {
                Vec::new()
            },
            sources: if location.local().is_some() {
                chosen.iter().map(|entry| entry.path.clone()).collect()
            } else {
                Vec::new()
            },
            additions: if other.local().is_some() {
                other_chosen
                    .iter()
                    .map(|entry| entry.path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                String::new()
            },
            ..Default::default()
        }
    }
}
