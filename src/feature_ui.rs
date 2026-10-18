use crate::{
    archive_tools::ZipEdit,
    filesystem::{Entry, Location},
    operations::Operation,
    preferences::{Column, ColumnSpec, Preferences, ToolbarAction},
};
use eframe::egui;
use std::path::{Path, PathBuf};
fn tr(text: &str) -> std::borrow::Cow<'_, str> {
    crate::i18n::text(text)
}

#[derive(Default)]
pub struct ArchiveForm {
    pub file: String,
    pub destination: String,
    pub password: String,
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
    Run(Operation),
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
    pub fn show(&mut self, ctx: &egui::Context) -> (bool, Option<ArchiveAction>) {
        let mut close = false;
        let mut action = None;
        egui::Modal::new(egui::Id::new("archive_tools")).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.heading(tr("Archive tools"));
            ui.horizontal(|ui| {
                for (mode, label) in [
                    (0, "Edit archive"),
                    (1, "Create encrypted archive"),
                    (2, "Unlock archive"),
                ] {
                    ui.selectable_value(&mut self.mode, mode, tr(label));
                }
            });
            ui.separator();
            if self.mode != 1 {
                ui.label(tr("Archive file"));
                ui.text_edit_singleline(&mut self.file);
                if ui.button(tr("Browse")).clicked() {
                    action = Some(ArchiveAction::Browse(self.file.clone().into()));
                    close = true;
                }
            }
            match self.mode {
                0 => {
                    ui.horizontal(|ui| {
                        for (mode, label) in
                            [(0, "Add files"), (1, "Remove entries"), (2, "Rename entry")]
                        {
                            ui.selectable_value(&mut self.edit_mode, mode, tr(label));
                        }
                    });
                    match self.edit_mode {
                        0 => {
                            ui.label(tr("One local path per line"));
                            ui.add(
                                egui::TextEdit::multiline(&mut self.additions)
                                    .desired_rows(4)
                                    .desired_width(f32::INFINITY),
                            );
                        }
                        1 => {
                            ui.label(tr("Selected entries"));
                            if self.selected.is_empty() {
                                ui.label(tr("Browse the archive and select entries first."));
                            }
                            egui::ScrollArea::vertical()
                                .max_height(140.0)
                                .show(ui, |ui| {
                                    for path in &self.selected {
                                        ui.monospace(path.display().to_string());
                                    }
                                });
                        }
                        _ => {
                            ui.label(tr("Source entry"));
                            ui.text_edit_singleline(&mut self.source);
                            ui.label(tr("New entry path"));
                            ui.text_edit_singleline(&mut self.target);
                        }
                    }
                    if ui
                        .add_enabled(
                            self.edit_mode != 1 || !self.selected.is_empty(),
                            egui::Button::new(tr("Apply archive edit")),
                        )
                        .clicked()
                    {
                        let edit = match self.edit_mode {
                            0 => ZipEdit::Add {
                                sources: self
                                    .additions
                                    .lines()
                                    .filter(|line| !line.trim().is_empty())
                                    .map(|line| PathBuf::from(line.trim()))
                                    .collect(),
                                prefix: self.prefix.clone(),
                            },
                            1 => ZipEdit::Remove(self.selected.clone()),
                            _ => ZipEdit::Rename {
                                source: self.source.clone().into(),
                                target: self.target.clone().into(),
                            },
                        };
                        action = Some(ArchiveAction::Run(Operation::ArchiveEdit {
                            archive: self.file.clone().into(),
                            edit,
                        }));
                        close = true;
                    }
                }
                1 => {
                    ui.label(tr("Selected entries"));
                    egui::ScrollArea::vertical()
                        .max_height(140.0)
                        .show(ui, |ui| {
                            for path in &self.sources {
                                ui.monospace(path.display().to_string());
                            }
                        });
                    ui.label(tr("Destination"));
                    ui.text_edit_singleline(&mut self.destination);
                    ui.label(tr("Password"));
                    ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
                    if ui.button(tr("Create encrypted archive")).clicked() {
                        if self.sources.is_empty() || self.password.is_empty() {
                            self.error = Some("Select local files and enter a password.".into());
                        } else {
                            action = Some(ArchiveAction::Run(Operation::PackProtected {
                                sources: self.sources.clone(),
                                destination: self.destination.clone().into(),
                                password: self.password.clone(),
                            }));
                            close = true;
                        }
                    }
                }
                _ => {
                    ui.label(tr("Password"));
                    ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
                    if ui.button(tr("Unlock archive")).clicked() {
                        action = Some(ArchiveAction::Unlock(
                            self.file.clone().into(),
                            self.password.clone(),
                        ));
                        close = true;
                    }
                }
            }
            if let Some(error) = &self.error {
                ui.colored_label(egui::Color32::RED, tr(error));
            }
            ui.separator();
            if ui.button(tr("Cancel")).clicked()
                || ui.input(|input| input.key_pressed(egui::Key::Escape))
            {
                close = true;
            }
        });
        (close, action)
    }
}

pub fn preferences(ui: &mut egui::Ui, preferences: &mut Preferences) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(tr("Language"));
        egui::ComboBox::from_id_salt("language")
            .selected_text(preferences.language.label())
            .show_ui(ui, |ui| {
                for language in crate::i18n::Language::ALL {
                    ui.selectable_value(&mut preferences.language, language, language.label());
                }
            });
    });
    ui.collapsing(tr("Columns"), |ui| {
        let mut remove = None;
        let mut swap = None;
        for index in 0..preferences.columns.len() {
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    let column = &mut preferences.columns[index];
                    ui.label(tr(column.kind.label()));
                    ui.add(egui::Slider::new(&mut column.width, 40.0..=800.0).show_value(true));
                    if index > 0 {
                        if ui.add_enabled(index > 1, egui::Button::new("↑")).clicked() {
                            swap = Some((index, index - 1));
                        }
                        if ui
                            .add_enabled(
                                index + 1 < preferences.columns.len(),
                                egui::Button::new("↓"),
                            )
                            .clicked()
                        {
                            swap = Some((index, index + 1));
                        }
                        if ui.small_button(tr("Remove")).clicked() {
                            remove = Some(index);
                        }
                    }
                });
            });
        }
        if let Some(index) = remove {
            preferences.columns.remove(index);
        }
        if let Some((a, b)) = swap {
            preferences.columns.swap(a, b);
        }
        ui.menu_button(tr("Add column"), |ui| {
            for kind in Column::ALL {
                if !preferences.columns.iter().any(|column| column.kind == kind)
                    && ui.button(tr(kind.label())).clicked()
                {
                    preferences.columns.push(ColumnSpec { kind, width: 140.0 });
                    ui.close();
                }
            }
        });
    });
    ui.collapsing(tr("Toolbar actions"), |ui| {
        let mut remove = None;
        let mut swap = None;
        for (index, action) in preferences.toolbar_actions.iter().enumerate() {
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr(action.label()));
                    if ui.add_enabled(index > 0, egui::Button::new("↑")).clicked() {
                        swap = Some((index, index - 1));
                    }
                    if ui
                        .add_enabled(
                            index + 1 < preferences.toolbar_actions.len(),
                            egui::Button::new("↓"),
                        )
                        .clicked()
                    {
                        swap = Some((index, index + 1));
                    }
                    if ui.small_button(tr("Remove")).clicked() {
                        remove = Some(index);
                    }
                });
            });
        }
        if let Some(index) = remove {
            preferences.toolbar_actions.remove(index);
        }
        if let Some((a, b)) = swap {
            preferences.toolbar_actions.swap(a, b);
        }
        ui.menu_button(tr("Add action"), |ui| {
            for action in ToolbarAction::ALL {
                if !preferences.toolbar_actions.contains(&action)
                    && ui.button(tr(action.label())).clicked()
                {
                    preferences.toolbar_actions.push(action);
                    ui.close();
                }
            }
        });
    });
}
