use crate::{
    operations::{Operation, Task},
    ui_signal::UiSignal,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::AtomicBool,
        mpsc::{self, Receiver},
    },
};
#[derive(Default)]
pub struct Features {
    pub comparison: Option<crate::folder_tools::Form>,
    pub sync: Option<SyncForm>,
    pub recovery: Option<RecoveryForm>,
    pub workspaces: bool,
    pub workspace_name: String,
    pub workspace_selected: Option<usize>,
    pub open_with: Option<(PathBuf, String)>,
    pub open_as: Option<crate::open_as::Form>,
    pub applications: Vec<PathBuf>,
    pub application_filter: String,
    pub remember_application: bool,
    pub application_pattern: String,
    pub error: Option<String>,
}
pub enum Effect {
    Run(Box<Operation>),
    SaveWorkspace(String),
    LoadWorkspace(usize),
    RenameWorkspace(usize, String),
    DeleteWorkspace(usize),
    SaveHistory,
    ReloadHistory,
}
impl Features {
    pub fn active(&self) -> bool {
        self.comparison.is_some()
            || self.sync.is_some()
            || self.recovery.is_some()
            || self.workspaces
            || self.open_with.is_some()
            || self.open_as.is_some()
    }
    pub fn sync(&mut self, left: PathBuf, right: PathBuf) {
        self.sync = Some(SyncForm {
            rules: Default::default(),
            preset_name: String::new(),
            left: left.display().to_string(),
            right: right.display().to_string(),
            direction: 1,
            locations: None,
            plan: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            deletions: false,
            confirmed: false,
            error: None,
        });
    }
    pub fn recovery(&mut self, ctx: &UiSignal) {
        let mut form = RecoveryForm::default();
        form.refresh(ctx);
        self.recovery = Some(form);
    }
}
pub struct SyncForm {
    pub(crate) rules: crate::path_rules::PathRules,
    pub(crate) preset_name: String,
    pub(crate) locations: Option<(crate::filesystem::Location, crate::filesystem::Location)>,
    pub(crate) left: String,
    pub(crate) right: String,
    pub(crate) direction: u8,
    pub(crate) plan: Option<crate::sync::Plan>,
    pub(crate) pending: Option<Receiver<Result<crate::sync::Plan, String>>>,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) deletions: bool,
    pub(crate) confirmed: bool,
    pub(crate) error: Option<String>,
}
impl Drop for SyncForm {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
impl SyncForm {
    pub(crate) fn start_preview(&mut self, ctx: &UiSignal) {
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let left = crate::filesystem::parse_location(
            &self.left,
            &crate::filesystem::Location::Local(crate::filesystem::home()),
        );
        let right = crate::filesystem::parse_location(
            &self.right,
            &crate::filesystem::Location::Local(crate::filesystem::home()),
        );
        let left = self
            .locations
            .as_ref()
            .filter(|(a, _)| a.label() == self.left)
            .map(|(a, _)| Ok(a.clone()))
            .unwrap_or(left);
        let right = self
            .locations
            .as_ref()
            .filter(|(_, b)| b.label() == self.right)
            .map(|(_, b)| Ok(b.clone()))
            .unwrap_or(right);
        let direction = self.direction;
        let rules = self.rules.clone();
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.plan = None;
        self.error = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = left
                .and_then(|left| {
                    right.and_then(|right| {
                        crate::sync::preview_locations_with_rules(
                            &left,
                            &right,
                            direction,
                            &rules,
                            &Task::new(cancel, |_| {}),
                        )
                    })
                })
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
}
#[derive(Default)]
pub struct RecoveryForm {
    pub(crate) stats: Option<crate::history::Storage>,
    pub(crate) pending: Option<Receiver<Result<crate::history::Storage, String>>>,
    pub(crate) cleaning: Option<Receiver<Result<(), String>>>,
    pub(crate) candidates: Option<Vec<String>>,
    pub(crate) error: Option<String>,
}
impl RecoveryForm {
    pub(crate) fn refresh(&mut self, ctx: &UiSignal) {
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::history::root()
                .and_then(|root| crate::history::storage_in(&root))
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
}
