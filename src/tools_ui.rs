use crate::{
    desktop::{self, RenameOptions, SearchOptions, SearchResult},
    filesystem::Location,
    operations::{Operation, Task},
};
use eframe::egui::{self, RichText};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

pub enum Effect {
    Run(Operation),
    Navigate(Location),
}
pub struct RenameForm {
    sources: Vec<PathBuf>,
    options: RenameOptions,
    error: Option<String>,
}
pub struct SearchForm {
    options: SearchOptions,
    result: Option<SearchResult>,
    pending: Option<Receiver<Result<SearchResult, String>>>,
    cancel: Arc<AtomicBool>,
    error: Option<String>,
}
#[derive(Default)]
pub struct Tools {
    rename: Option<RenameForm>,
    search: Option<SearchForm>,
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
    pub fn search(&mut self, root: PathBuf, hidden: bool) {
        self.search = Some(SearchForm {
            options: SearchOptions {
                root,
                pattern: "*".into(),
                content: String::new(),
                hidden,
                max_results: 10_000,
            },
            result: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            error: None,
        });
    }
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Effect> {
        let mut effect = None;
        if let Some(form) = &mut self.rename {
            let mut close = false;
            egui::Modal::new(egui::Id::new("batch_rename")).show(ctx, |ui| {
                ui.set_width(700.0); ui.heading("Batch rename");
                egui::Grid::new("rename_fields").num_columns(2).show(ui, |ui| {
                    ui.label("Find"); ui.text_edit_singleline(&mut form.options.find); ui.end_row();
                    ui.label("Replace"); ui.text_edit_singleline(&mut form.options.replace); ui.end_row();
                    ui.label("Use regular expression"); ui.checkbox(&mut form.options.regex, ""); ui.end_row();
                    ui.label("Template"); ui.text_edit_singleline(&mut form.options.template); ui.end_row();
                    ui.label("First number / padding"); ui.horizontal(|ui| { ui.add(egui::DragValue::new(&mut form.options.start)); ui.add(egui::DragValue::new(&mut form.options.width).range(0..=12)); }); ui.end_row();
                });
                ui.label("{name} = filename without extension · {ext} = extension including dot · {n} = number");
                let plan = desktop::rename_plan(&form.sources, &form.options);
                match &plan {
                    Ok(plan) => { egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| { egui::Grid::new("rename_preview").striped(true).show(ui, |ui| {
                        for (source, target) in plan { ui.label(source.file_name().unwrap_or_default().to_string_lossy()); ui.label("→"); ui.label(target.file_name().unwrap_or_default().to_string_lossy()); ui.end_row(); }
                    }); }); }
                    Err(error) => { ui.colored_label(egui::Color32::RED, error.to_string()); }
                }
                if let Some(error) = &form.error { ui.label(error); }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { close = true; }
                    if ui.add_enabled(plan.is_ok(), egui::Button::new("Rename all")).clicked()
                        && let Ok(plan) = plan { effect = Some(Effect::Run(Operation::BatchRename(plan))); close = true; }
                });
            });
            if close {
                self.rename = None;
            }
        }
        if let Some(form) = &mut self.search {
            if let Some(result) = crate::background::poll(&mut form.pending) {
                match result {
                    Ok(result) => form.result = Some(result),
                    Err(error) => form.error = Some(error),
                }
            }
            if form.pending.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            let mut close = false;
            let mut search = false;
            egui::Modal::new(egui::Id::new("search")).show(ctx, |ui| {
                ui.set_width(800.0);
                ui.heading("Find files");
                ui.label(form.options.root.display().to_string());
                ui.horizontal(|ui| {
                    ui.label("Filename pattern");
                    ui.text_edit_singleline(&mut form.options.pattern);
                });
                ui.horizontal(|ui| {
                    ui.label("Containing text");
                    ui.text_edit_singleline(&mut form.options.content);
                });
                ui.checkbox(&mut form.options.hidden, "Include hidden files and folders");
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(form.pending.is_none(), egui::Button::new("Search"))
                        .clicked()
                    {
                        search = true;
                    }
                    if form.pending.is_some() {
                        ui.spinner();
                        if ui.button("Stop").clicked() {
                            form.cancel.store(true, Ordering::Relaxed);
                        }
                    }
                    if ui.button("Close").clicked() {
                        form.cancel.store(true, Ordering::Relaxed);
                        close = true;
                    }
                });
                if let Some(error) = &form.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                if let Some(result) = &form.result {
                    ui.label(format!(
                        "{} results{} · {} unreadable entries",
                        result.paths.len(),
                        if result.truncated {
                            " (limit reached)"
                        } else {
                            ""
                        },
                        result.errors.len()
                    ));
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for path in &result.paths {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(path.display().to_string()).monospace(),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    effect =
                                        Some(Effect::Navigate(Location::Local(if path.is_dir() {
                                            path.clone()
                                        } else {
                                            path.parent().unwrap_or(&form.options.root).to_owned()
                                        })));
                                    close = true;
                                }
                            }
                        });
                    ui.collapsing("Unreadable entries", |ui| {
                        for error in &result.errors {
                            ui.label(error);
                        }
                    });
                }
            });
            if search {
                let options = form.options.clone();
                form.cancel = Arc::new(AtomicBool::new(false));
                let cancel = form.cancel.clone();
                let (sender, receiver) = mpsc::channel();
                form.pending = Some(receiver);
                form.result = None;
                form.error = None;
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let mut task = Task::new(cancel, |_| {});
                    let result = desktop::search(&options, &mut task).map_err(|e| format!("{e:#}"));
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
            }
            if close {
                self.search = None;
            }
        }
        effect
    }
}
