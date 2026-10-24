use crate::{
    filesystem::{Entry, Location},
    operations::Operation,
};
use std::path::{Path, PathBuf};
#[derive(Default)]
pub struct ArchiveForm {
    pub file: String,
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
