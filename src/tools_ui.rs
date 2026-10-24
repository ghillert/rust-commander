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
    Run(Operation),
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
    pub fn active(&self) -> bool {
        self.rename.is_some() || self.search.is_some()
    }
    pub fn rename(&mut self, sources: Vec<PathBuf>) {
        self.rename = Some(RenameForm {
            sources,
            options: RenameOptions::default(),
            error: None,
        });
    }
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
