//! Dialogs for recovery, synchronization, workspaces and native application selection.
use crate::{
    history::{Record, Retention},
    operations::{Operation, Task},
    workspaces::Workspace,
};
use eframe::egui;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};
fn tr(text: &str) -> std::borrow::Cow<'_, str> {
    crate::i18n::text(text)
}

#[derive(Default)]
pub struct Features {
    pub sync: Option<SyncForm>,
    pub recovery: Option<RecoveryForm>,
    pub workspaces: bool,
    pub workspace_name: String,
    pub workspace_selected: Option<usize>,
    pub open_with: Option<(PathBuf, String)>,
    pub error: Option<String>,
}
pub enum Effect {
    Run(Operation),
    SaveWorkspace(String),
    LoadWorkspace(usize),
    RenameWorkspace(usize, String),
    DeleteWorkspace(usize),
    SaveHistory,
    ReloadHistory,
}
impl Features {
    pub fn active(&self) -> bool {
        self.sync.is_some()
            || self.recovery.is_some()
            || self.workspaces
            || self.open_with.is_some()
    }
    pub fn sync(&mut self, left: PathBuf, right: PathBuf) {
        self.sync = Some(SyncForm {
            left: left.display().to_string(),
            right: right.display().to_string(),
            direction: 1,
            plan: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            deletions: false,
            confirmed: false,
            error: None,
        });
    }
    pub fn recovery(&mut self, ctx: &egui::Context) {
        let mut form = RecoveryForm::default();
        form.refresh(ctx);
        self.recovery = Some(form);
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        records: &mut [Record],
        policy: &mut Retention,
        workspaces: &[Workspace],
        idle: bool,
    ) -> Option<Effect> {
        let mut effect = None;
        if let Some(mut form) = self.sync.take() {
            let (close, operation) = form.show(ctx, idle);
            if let Some(operation) = operation {
                effect = Some(Effect::Run(operation));
            }
            if !close {
                self.sync = Some(form);
            } else {
                form.cancel.store(true, Ordering::Relaxed);
            }
        }
        if let Some(mut form) = self.recovery.take() {
            let (close, result) = form.show(ctx, records, policy, idle);
            if result.is_some() {
                effect = result;
            }
            if !close {
                self.recovery = Some(form);
            }
        }
        if self.workspaces {
            effect = self.workspace_dialog(ctx, workspaces).or(effect);
        }
        self.application_dialog(ctx);
        effect
    }
    fn workspace_dialog(
        &mut self,
        ctx: &egui::Context,
        workspaces: &[Workspace],
    ) -> Option<Effect> {
        let mut effect = None;
        egui::Modal::new(egui::Id::new("workspaces")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.heading(tr("Named workspaces"));
            ui.label(tr("Save both panes, tabs, active locations and layout."));
            ui.label(tr(
                "Connection credentials stay in the OS credential store.",
            ));
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.workspace_name);
                if ui.button(tr("Save current workspace")).clicked() {
                    effect = Some(Effect::SaveWorkspace(self.workspace_name.clone()));
                }
            });
            egui::ScrollArea::vertical()
                .max_height(250.0)
                .show(ui, |ui| {
                    for (index, workspace) in workspaces.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(
                                    self.workspace_selected == Some(index),
                                    &workspace.name,
                                )
                                .clicked()
                            {
                                self.workspace_selected = Some(index);
                                self.workspace_name = workspace.name.clone();
                            }
                            if ui.button(tr("Open")).clicked() {
                                effect = Some(Effect::LoadWorkspace(index));
                                self.workspaces = false;
                            }
                        });
                    }
                });
            if let Some(index) = self.workspace_selected {
                ui.horizontal(|ui| {
                    if ui.button(tr("Rename selected")).clicked() {
                        effect = Some(Effect::RenameWorkspace(index, self.workspace_name.clone()));
                    }
                    if ui.button(tr("Delete saved workspace")).clicked() {
                        effect = Some(Effect::DeleteWorkspace(index));
                        self.workspace_selected = None;
                    }
                });
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, tr(error));
            }
            if ui.button(tr("Close")).clicked() {
                self.workspaces = false;
            }
        });
        effect
    }
    fn application_dialog(&mut self, ctx: &egui::Context) {
        let Some((path, application)) = &mut self.open_with else {
            return;
        };
        let mut close = false;
        egui::Modal::new(egui::Id::new("open_with")).show(ctx, |ui| {
            ui.set_width(600.0);
            ui.heading(tr("Open With"));
            ui.label(path.display().to_string());
            ui.label(tr("Application path"));
            ui.small(tr(
                "Use a .app on macOS, executable on Windows, or executable/.desktop on Linux.",
            ));
            ui.text_edit_singleline(application);
            #[cfg(target_os = "macos")]
            ui.collapsing(tr("Applications"), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        if let Ok(entries) = std::fs::read_dir("/Applications") {
                            let mut paths = entries
                                .flatten()
                                .map(|e| e.path())
                                .filter(|p| crate::native_actions::is_application(p))
                                .collect::<Vec<_>>();
                            paths.sort();
                            for path in paths {
                                if ui
                                    .button(path.file_name().unwrap_or_default().to_string_lossy())
                                    .clicked()
                                {
                                    *application = path.display().to_string();
                                }
                            }
                        }
                    });
            });
            if ui.button(tr("Open")).clicked() {
                match crate::native_actions::open_with(path, &PathBuf::from(application.as_str())) {
                    Ok(()) => close = true,
                    Err(error) => self.error = Some(format!("{error:#}")),
                }
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, tr(error));
            }
            if ui.button(tr("Cancel")).clicked() {
                close = true;
            }
        });
        if close {
            self.open_with = None;
        }
    }
}

pub struct SyncForm {
    left: String,
    right: String,
    direction: u8,
    plan: Option<crate::sync::Plan>,
    pending: Option<Receiver<Result<crate::sync::Plan, String>>>,
    cancel: Arc<AtomicBool>,
    deletions: bool,
    confirmed: bool,
    error: Option<String>,
}
impl SyncForm {
    fn start_preview(&mut self, ctx: &egui::Context) {
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let left = PathBuf::from(&self.left);
        let right = PathBuf::from(&self.right);
        let direction = self.direction;
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        self.plan = None;
        self.error = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::sync::preview(&left, &right, direction, &Task::new(cancel, |_| {}))
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    fn show(&mut self, ctx: &egui::Context, idle: bool) -> (bool, Option<Operation>) {
        if let Some(result) = crate::background::poll(&mut self.pending) {
            match result {
                Ok(plan) => self.plan = Some(plan),
                Err(error) => self.error = Some(error),
            }
        }
        let mut close = false;
        let mut operation = None;
        let mut preview = false;
        egui::Modal::new(egui::Id::new("synchronize")).show(ctx, |ui| {
            ui.set_width(820.0);
            ui.heading(tr("Synchronize folders"));
            ui.label(tr("Preview changes, resolve conflicts, then apply."));
            ui.label(tr("Two-way synchronization does not infer deletions."));
            let mut changed = false;
            egui::Grid::new("sync_roots").num_columns(2).show(ui, |ui| {
                ui.label(tr("Left"));
                changed |= ui.text_edit_singleline(&mut self.left).changed();
                ui.end_row();
                ui.label(tr("Right"));
                changed |= ui.text_edit_singleline(&mut self.right).changed();
                ui.end_row();
            });
            ui.horizontal(|ui| {
                for (value, label) in [(1, "Left → right"), (2, "Right → left"), (0, "Two-way")]
                {
                    changed |= ui
                        .selectable_value(&mut self.direction, value, tr(label))
                        .changed();
                }
            });
            if changed {
                self.plan = None;
                self.cancel.store(true, Ordering::Relaxed);
                self.pending = None;
            }
            if ui
                .add_enabled(
                    self.pending.is_none(),
                    egui::Button::new(tr("Preview changes")),
                )
                .clicked()
            {
                preview = true;
            }
            if self.pending.is_some() {
                ui.spinner();
                if ui.button(tr("Cancel preview")).clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
            }
            if let Some(plan) = &mut self.plan {
                ui.label(format!(
                    "{} {}",
                    plan.items.len(),
                    tr("differing entries. Conflicts start with Skip.")
                ));
                ui.checkbox(&mut self.deletions, tr("Allow explicit deletions"));
                if !self.deletions {
                    for item in &mut plan.items {
                        if matches!(
                            item.choice,
                            crate::sync::Choice::DeleteLeft | crate::sync::Choice::DeleteRight
                        ) {
                            item.choice = crate::sync::Choice::Skip;
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button(tr("Copy all left → right")).clicked() {
                        for item in &mut plan.items {
                            if item.left {
                                item.choice = crate::sync::Choice::LeftToRight;
                            }
                        }
                    }
                    if ui.button(tr("Copy all right → left")).clicked() {
                        for item in &mut plan.items {
                            if item.right {
                                item.choice = crate::sync::Choice::RightToLeft;
                            }
                        }
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 430.0).clamp(60.0, 300.0))
                    .show(ui, |ui| {
                        for item in &mut plan.items {
                            ui.horizontal(|ui| {
                                ui.label(tr(if item.conflict {
                                    "Conflict"
                                } else {
                                    "Only on one side"
                                }));
                                ui.monospace(item.path.display().to_string());
                                egui::ComboBox::from_id_salt(&item.path)
                                    .selected_text(tr(item.choice.label()))
                                    .show_ui(ui, |ui| {
                                        use crate::sync::Choice::*;
                                        for (choice, allowed) in [
                                            (Skip, true),
                                            (LeftToRight, item.left),
                                            (RightToLeft, item.right),
                                            (DeleteLeft, self.deletions && item.left),
                                            (DeleteRight, self.deletions && item.right),
                                        ] {
                                            if allowed {
                                                ui.selectable_value(
                                                    &mut item.choice,
                                                    choice,
                                                    tr(choice.label()),
                                                );
                                            }
                                        }
                                    });
                            });
                        }
                    });
                let deleting = plan
                    .items
                    .iter()
                    .filter(|item| {
                        matches!(
                            item.choice,
                            crate::sync::Choice::DeleteLeft | crate::sync::Choice::DeleteRight
                        )
                    })
                    .count();
                if deleting > 0 {
                    ui.checkbox(
                        &mut self.confirmed,
                        format!("{} {deleting} {}", tr("I confirm"), tr("deletions")),
                    );
                }
                if ui
                    .add_enabled(
                        idle && (deleting == 0 || self.confirmed),
                        egui::Button::new(tr("Apply reviewed plan")),
                    )
                    .clicked()
                {
                    operation = Some(Operation::Sync(plan.clone()));
                    close = true;
                }
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, tr(error));
            }
            if ui.button(tr("Close")).clicked() {
                close = true;
            }
        });
        if preview {
            self.start_preview(ctx);
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        (close, operation)
    }
}

#[derive(Default)]
pub struct RecoveryForm {
    stats: Option<crate::history::Storage>,
    pending: Option<Receiver<Result<crate::history::Storage, String>>>,
    cleaning: Option<Receiver<Result<(), String>>>,
    candidates: Option<Vec<String>>,
    error: Option<String>,
}
impl RecoveryForm {
    fn refresh(&mut self, ctx: &egui::Context) {
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
    fn show(
        &mut self,
        ctx: &egui::Context,
        records: &mut [Record],
        policy: &mut Retention,
        idle: bool,
    ) -> (bool, Option<Effect>) {
        let mut close = false;
        let mut effect = None;
        if let Some(result) = crate::background::poll(&mut self.pending) {
            match result {
                Ok(stats) => self.stats = Some(stats),
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(result) = crate::background::poll(&mut self.cleaning) {
            match result {
                Ok(()) => {
                    effect = Some(Effect::ReloadHistory);
                    self.candidates = None;
                    self.refresh(ctx);
                }
                Err(error) => self.error = Some(error),
            }
        }
        egui::Modal::new(egui::Id::new("recovery_storage")).show(ctx,|ui| {
            ui.set_width(650.0); ui.heading(tr("Recovery storage"));
            if let Some(stats)=&self.stats {
                ui.label(format!("{} {} · {} {} · {} {}",crate::filesystem::human_size(stats.bytes),tr("used"),
                    stats.backups,tr("backups"),crate::filesystem::human_size(stats.orphan_bytes),tr("in incomplete journals")));
            } else { ui.spinner(); }
            ui.label(tr("Recovery copies contain file contents. Cleanup expires Undo/Redo for the selected chains."));
            ui.label(tr("Pinned chains and the latest reversible chain are retained."));
            ui.horizontal(|ui| {
                ui.label(tr("Limit (MiB)")); ui.add(egui::DragValue::new(&mut policy.max_mib).range(1..=1_000_000));
                ui.label(tr("Days")); ui.add(egui::DragValue::new(&mut policy.max_days).range(1..=3650));
            });
            if ui.button(tr("Open recovery folder")).clicked() && let Ok(root)=crate::history::root() && let Err(error)=open::that(root){self.error=Some(error.to_string());}
            ui.small(tr("Incomplete journals are kept for manual review."));
            ui.checkbox(&mut policy.automatic,tr("Automatically clean eligible recovery copies after operations"));
            if ui.add_enabled(idle&&self.cleaning.is_none(),egui::Button::new(tr("Preview cleanup"))).clicked() {
                match crate::history::root().and_then(|root|crate::history::cleanup_candidates(records,&root,policy,crate::oauth::now())) {
                    Ok(candidates)=>self.candidates=Some(candidates), Err(error)=>self.error=Some(format!("{error:#}")),
                }
            }
            if let Some(ids)=&self.candidates {
                ui.label(format!("{} {}",ids.len(),tr("recovery records will expire")));
                egui::ScrollArea::vertical().max_height(120.0).show(ui,|ui| {
                    for record in records.iter().filter(|r|ids.contains(&r.id)) { ui.label(format!("{} · {}",tr(&record.label),record.id)); }
                });
                if ui.add_enabled(idle&&!ids.is_empty()&&self.cleaning.is_none(),
                    egui::Button::new(tr("Expire these records and delete their backups"))).clicked() {
                    let ids=ids.clone(); let (sender,receiver)=mpsc::channel();
                    self.cleaning=Some(receiver); let ctx=ctx.clone();
                    std::thread::spawn(move|| {
                        let result=crate::history::root().and_then(|root|crate::history::cleanup_in(&root,&ids))
                            .map(|_|()).map_err(|error|format!("{error:#}"));
                        let _=sender.send(result); ctx.request_repaint();
                    });
                }
            }
            ui.collapsing(tr("Pin recovery records"),|ui| {
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-390.0).clamp(60.0,180.0)).show(ui,|ui| {
                    for record in records.iter_mut().rev().filter(|r|!r.expired&&!r.changes.is_empty()) {
                        if ui.add_enabled(idle&&self.cleaning.is_none(),egui::Checkbox::new(&mut record.pinned,
                            format!("{} · {}",tr(&record.label),record.id))).changed() {
                            effect=Some(Effect::SaveHistory); self.candidates=None;
                        }
                    }
                });
            });
            if let Some(error)=&self.error { ui.colored_label(egui::Color32::RED,tr(error)); }
            if self.cleaning.is_some() { ui.spinner(); }
            if ui.add_enabled(self.cleaning.is_none(),egui::Button::new(tr("Close"))).clicked() { close=true; }
        });
        if self.pending.is_some() || self.cleaning.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        (close, effect)
    }
}
