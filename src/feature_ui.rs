//! Archive dialog state and actions derived from the two pane selections.
//! [`ArchiveForm::new`] fills in the source, member prefix, and suggested destination.

use crate::{
    filesystem::{Entry, Location},
    operations::Operation,
};
use std::path::{Path, PathBuf};
#[derive(Default)]
pub struct ArchiveForm {
    /// Local path of the archive to edit, unlock, or browse, including a materialized copy.
    pub file: String,
    /// Display label for the original source when `file` points to a cached archive.
    pub source_display: Option<String>,
    /// Proposed output path when creating an encrypted archive.
    pub destination: String,
    /// Password entered for encrypted archive creation or unlocking.
    pub password: String,
    /// Optional compression level for encrypted archive creation. `None` uses the format
    /// default.
    pub compression_level: Option<u8>,
    /// Selected action index, with 0 for editing, 1 for encrypted creation, and 2 for
    /// unlocking.
    pub mode: usize,
    /// Selected edit index, with 0 for adding files, 1 for removing members, and 2 for
    /// renaming.
    pub edit_mode: usize,
    /// Local paths to add to the archive, one per line.
    pub additions: String,
    /// Existing archive-root-relative member path for a rename.
    pub source: String,
    /// New archive-root-relative member path for a rename.
    pub target: String,
    /// Directory within the archive where new members will be added.
    pub prefix: PathBuf,
    /// Archive-root-relative paths of the members selected for removal.
    pub selected: Vec<PathBuf>,
    /// Local entries selected for inclusion in a new encrypted archive.
    pub sources: Vec<PathBuf>,
    /// Error message shown in the archive dialog, when present.
    pub error: Option<String>,
}
pub enum ArchiveAction {
    /// Submit the enclosed archive operation as a file job.
    Run(Box<Operation>),
    /// Validate the supplied password for the archive at the given path and browse its root
    /// on success.
    Unlock(PathBuf, String),
    /// Open the archive at the supplied path in a file pane.
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
