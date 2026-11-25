//! Dialog state for batch rename and recursive search. [`Tools`] owns the forms
//! and tracks which one is active.

use crate::{
    desktop::{RenameOptions, SearchOptions, SearchResult},
    filesystem::Location,
    operations::Operation,
};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool, mpsc::Receiver},
};
pub enum Effect {
    /// Submit the operation prepared by the tool as a file job.
    Run(Box<Operation>),
    /// Request pane navigation to the supplied location.
    Navigate(Location),
}
pub struct RenameForm {
    pub(crate) sources: Vec<PathBuf>,
    pub(crate) options: RenameOptions,
    pub(crate) error: Option<String>,
}
pub struct SearchForm {
    pub(crate) connection: Option<String>,
    pub(crate) options: SearchOptions,
    pub(crate) result: Option<SearchResult>,
    pub(crate) pending: Option<Receiver<Result<SearchResult, String>>>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) error: Option<String>,
}
impl Drop for SearchForm {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
#[derive(Default)]
pub struct Tools {
    pub(crate) rename: Option<RenameForm>,
    pub(crate) search: Option<SearchForm>,
}
impl Tools {
    /// Returns whether the batch rename or search form is open.
    pub fn active(&self) -> bool {
        self.rename.is_some() || self.search.is_some()
    }
    /// Opens a batch rename form for the supplied paths with default rename options.
    pub fn rename(&mut self, sources: Vec<PathBuf>) {
        self.rename = Some(RenameForm {
            sources,
            options: RenameOptions::default(),
            error: None,
        });
    }
    /// Opens a recursive search rooted at a local directory or remote connection path. Uses
    /// the supplied hidden-file preference and a 10,000-result limit. Other location types
    /// leave the form as it was.
    pub fn search(&mut self, location: Location, hidden: bool) {
        let (root, connection) = match location {
            Location::Local(root) => (root, None),
            Location::Remote { connection, path } => (path, Some(connection)),
            _ => return,
        };
        self.search = Some(SearchForm {
            connection,
            options: SearchOptions {
                root,
                pattern: "*".into(),
                content: String::new(),
                hidden,
                max_results: 10_000,
                filters: Default::default(),
            },
            result: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            error: None,
        });
    }
}
