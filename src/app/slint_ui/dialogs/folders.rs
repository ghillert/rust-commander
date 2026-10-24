use super::*;
use crate::comparison::{Mode, Options, Preset};
use anyhow::Context;

pub(super) fn rules_fields(d: &mut Dialog, rules: &crate::path_rules::PathRules) {
    d.fields.extend([
        Field::text(
            "folder-include",
            "Include files (glob patterns separated by ;)",
            &rules.include,
        ),
        Field::text(
            "folder-exclude",
            "Exclude files/folders (glob patterns separated by ;)",
            &rules.exclude,
        ),
        Field::check("folder-case", "Match pattern case", rules.case_sensitive),
    ]);
}
pub(super) fn presets(d: &mut Dialog, presets: &[Preset], name: &str) {
    d.fields.push(Field::text(
        "folder-preset-name",
        "Preset name (filters only; folders are not saved)",
        name,
    ));
    d.button("folder-preset-save", "Save preset", !d.busy);
    for (i, preset) in presets.iter().enumerate() {
        d.rows.push(
            Row::new(
                format!("preset-{i}"),
                format!("Preset: {}", preset.name),
                "",
            )
            .action(format!("folder-preset-load-{i}"), "Load", !d.busy)
            .action(format!("folder-preset-delete-{i}"), "Delete", !d.busy),
        );
    }
}
impl Adapter {
    pub(super) fn folder_dialog(&self) -> Option<Dialog> {
        let form = self.app.features.comparison.as_ref()?;
        if let Some(diff) = &form.diff {
            let mut d = Dialog::new("text-diff", "Text comparison");
            d.description = format!(
                "{}\nLeft: {}\nRight: {}\n! marks changed lines. This view is read-only.",
                diff.name,
                form.left.label(),
                form.right.label()
            );
            for row in &diff.rows {
                d.fields.push(Field {
                    id: if row.changed { "changed" } else { "equal" }.into(),
                    title: String::new(),
                    value: row.left.clone(),
                    kind: 7,
                    choices: vec![row.right.clone()],
                });
            }
            if diff.rows.is_empty() {
                d.description.push_str("\nBoth files are empty.");
            }
            return Some(d);
        }
        let mut d = Dialog::new("comparison", "Compare folders");
        d.error = form.error.clone().unwrap_or_default();
        d.busy = form.pending.is_some() || form.diff_pending.is_some();
        d.description = format!(
            "Left: {}\nRight: {}\nQuick metadata results are unverified. Synchronization always checks contents.",
            form.left.label(),
            form.right.label()
        );
        rules_fields(&mut d, &form.options.rules);
        d.fields.push(Field::choice(
            "comparison-mode",
            "Compare using",
            form.options.mode.label(),
            [Mode::Content.label(), Mode::Metadata.label()],
        ));
        presets(
            &mut d,
            &self.app.preferences.folder_tool_presets,
            &form.preset_name,
        );
        if let Some(result) = &form.result {
            d.description
                .push_str(&format!("\n{} entries; showing up to 5,000.", result.len()));
            for (i, item) in result.iter().take(5000).enumerate() {
                d.rows.push(
                    Row::new(format!("comparison-{i}"), &item.name, &item.status).action(
                        format!("comparison-diff-{i}"),
                        "Text diff",
                        item.can_diff && !d.busy,
                    ),
                );
            }
        }
        d.button("stop", "Stop", d.busy);
        d.button("accept", "Compare", !d.busy);
        Some(d)
    }
    pub(super) fn comparison_edit(&mut self, id: &str, value: &str) {
        if let Some(form) = &mut self.app.features.comparison {
            if id == "folder-preset-name" {
                form.preset_name = value.into();
                return;
            }
            form.invalidate();
            match id {
                "folder-include" => form.options.rules.include = value.into(),
                "folder-exclude" => form.options.rules.exclude = value.into(),
                "folder-case" => form.options.rules.case_sensitive = value == "true",
                "comparison-mode" => {
                    form.options.mode = if value == Mode::Metadata.label() {
                        Mode::Metadata
                    } else {
                        Mode::Content
                    }
                }
                _ => {}
            }
        }
    }
    pub(super) fn folder_preset_action(&mut self, id: &str) -> anyhow::Result<()> {
        if id == "folder-preset-save" {
            let (name, options) = if self.dialog_kind == "comparison" {
                let form = self
                    .app
                    .features
                    .comparison
                    .as_ref()
                    .context("Open Compare folders first")?;
                (form.preset_name.trim().to_owned(), form.options.clone())
            } else {
                let form = self
                    .app
                    .features
                    .sync
                    .as_ref()
                    .context("Open Synchronize first")?;
                (
                    form.preset_name.trim().to_owned(),
                    Options {
                        rules: form.rules.clone(),
                        mode: Mode::Content,
                    },
                )
            };
            anyhow::ensure!(
                !name.is_empty() && name.len() <= 80,
                "Enter a preset name of 1–80 bytes"
            );
            options.rules.compile()?;
            let presets = &mut self.app.preferences.folder_tool_presets;
            if let Some(preset) = presets.iter_mut().find(|p| p.name == name) {
                preset.options = options;
            } else {
                anyhow::ensure!(presets.len() < 30, "At most 30 presets can be saved");
                presets.push(Preset { name, options });
            }
        } else if let Some(index) = id
            .strip_prefix("folder-preset-load-")
            .and_then(|s| s.parse::<usize>().ok())
        {
            let preset = self
                .app
                .preferences
                .folder_tool_presets
                .get(index)
                .context("Preset no longer exists")?
                .clone();
            if self.dialog_kind == "comparison" {
                if let Some(form) = &mut self.app.features.comparison {
                    form.invalidate();
                    form.options = preset.options;
                    form.preset_name = preset.name;
                }
            } else if let Some(form) = &mut self.app.features.sync {
                form.cancel.store(true, Ordering::Relaxed);
                form.pending = None;
                form.plan = None;
                form.confirmed = false;
                form.rules = preset.options.rules;
                form.preset_name = preset.name;
            }
        } else if let Some(index) = id
            .strip_prefix("folder-preset-delete-")
            .and_then(|s| s.parse::<usize>().ok())
        {
            anyhow::ensure!(
                index < self.app.preferences.folder_tool_presets.len(),
                "Preset no longer exists"
            );
            self.app.preferences.folder_tool_presets.remove(index);
        }
        self.app.save_settings();
        Ok(())
    }
}
