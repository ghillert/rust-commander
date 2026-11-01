//! Publish bounded, visible source runs to the two shared Slint viewers.
use super::*;
use crate::source_view::{Mode, PREVIEW_LIMIT, State, VIEWER_LIMIT};

#[derive(Default)]
pub(super) struct Cache {
    stamp: Option<(u64, u64, usize, usize)>,
    runs: ModelRc<SourceRun>,
    choices: Vec<&'static str>,
    choice_model: ModelRc<SharedString>,
    layout_stamp: Option<(u64, u64)>,
    to_layout: ModelRc<i32>,
    to_source: ModelRc<i32>,
}
impl Cache {
    fn offsets(&mut self, state: &State) -> (ModelRc<i32>, ModelRc<i32>) {
        if self.layout_stamp != Some((state.id, state.revision)) {
            self.to_layout = Rc::new(VecModel::from(
                state
                    .prepared
                    .as_ref()
                    .map_or_else(Vec::new, |p| p.to_layout.clone()),
            ))
            .into();
            self.to_source = Rc::new(VecModel::from(
                state
                    .prepared
                    .as_ref()
                    .map_or_else(Vec::new, |p| p.to_source.clone()),
            ))
            .into();
            self.layout_stamp = Some((state.id, state.revision));
        }
        (self.to_layout.clone(), self.to_source.clone())
    }
    fn modes(&mut self, state: &State) -> ModelRc<SharedString> {
        let mut choices = vec![Mode::Plain.label()];
        if state.supported || state.busy() {
            choices.push(Mode::Highlighted.label());
        }
        if state.pretty {
            choices.push(Mode::Pretty.label());
        }
        if choices != self.choices {
            self.choice_model = strings(choices.iter().copied());
            self.choices = choices;
        }
        self.choice_model.clone()
    }
    fn runs(&mut self, state: &State, first: i32, count: i32) -> ModelRc<SourceRun> {
        let first = first.max(0) as usize;
        let count = count.clamp(24, 256) as usize;
        let stamp = (state.id, state.revision, first, count);
        if self.stamp != Some(stamp) {
            let mut runs = Vec::new();
            let mut bytes = 0usize;
            if let Some(prepared) = &state.prepared {
                for (index, line) in prepared.lines.iter().enumerate().skip(first).take(count) {
                    let spans = &prepared.spans[line.spans.clone()];
                    let mapped = |n: usize| {
                        if prepared.to_layout.is_empty() {
                            n
                        } else {
                            prepared.to_layout[n] as usize
                        }
                    };
                    let line_start = mapped(line.range.start);
                    let text = &prepared.layout[line_start..mapped(line.range.end)];
                    let needed = spans
                        .iter()
                        .map(|s| {
                            text.len() + mapped(s.range.start) + mapped(s.range.end)
                                - 2 * line_start
                        })
                        .sum::<usize>();
                    // Bound prefix/full-line storage and keep bidirectional lines
                    // in one shaping run rather than slicing visual glyph order.
                    if spans.len() > 512
                        || bytes + needed > 4 * 1024 * 1024
                        || text.chars().any(|c| matches!(c, '\u{0590}'..='\u{08ff}'))
                    {
                        continue; // The base text remains readable without colors.
                    } else {
                        for span in spans.iter().filter(|s| s.color != prepared.foreground) {
                            bytes += text.len() + mapped(span.range.start) + mapped(span.range.end)
                                - 2 * line_start;
                            runs.push(SourceRun {
                                line: index as i32,
                                prefix: prepared.layout[line_start..mapped(span.range.start)]
                                    .into(),
                                through: prepared.layout[line_start..mapped(span.range.end)].into(),
                                text: text.into(),
                                color: color(span.color),
                            });
                        }
                    }
                }
            }
            self.runs = Rc::new(VecModel::from(runs)).into();
            self.stamp = Some(stamp);
        }
        self.runs.clone()
    }
}
fn color([r, g, b, a]: [u8; 4]) -> slint::Color {
    slint::Color::from_argb_u8(a, r, g, b)
}
impl Adapter {
    pub(super) fn sync_source_views(&mut self, ui: &CommanderWindow) {
        ui.set_source_font_family(
            if cfg!(target_os = "macos") {
                "Menlo"
            } else if cfg!(target_os = "windows") {
                "Consolas"
            } else {
                "DejaVu Sans Mono"
            }
            .into(),
        );
        let source = self
            .app
            .editor
            .as_mut()
            .filter(|e| e.image.is_none() && e.pdf.is_none() && !e.kind.starts_with("Hex"));
        ui.set_viewer_source(source.is_some());
        if let Some(editor) = source {
            let state = &mut editor.navigation.presentation;
            state.update(
                &editor.navigation.path,
                &editor.text,
                editor.text_page.is_none(),
                VIEWER_LIMIT,
                self.dark,
                &self.ctx,
            );
            ui.set_viewer_source_mode(state.mode.label().into());
            ui.set_viewer_source_modes(self.viewer_source_cache.modes(state));
            ui.set_viewer_source_language(state.language.as_str().into());
            ui.set_viewer_source_notice(state.notice.as_str().into());
            let active = state.prepared.is_some() && state.mode != Mode::Plain;
            let restore_search_selection = active && !ui.get_viewer_source_active();
            ui.set_viewer_source_active(active);
            ui.set_viewer_source_revision(state.revision as i32);
            ui.set_viewer_source_lines(state.prepared.as_ref().map_or(1, |p| p.lines.len() as i32));
            ui.set_viewer_source_background(color(
                state.prepared.as_ref().map_or([255; 4], |p| p.background),
            ));
            ui.set_viewer_source_foreground(color(
                state
                    .prepared
                    .as_ref()
                    .map_or([0, 0, 0, 255], |p| p.foreground),
            ));
            ui.set_viewer_source_layout(
                state
                    .prepared
                    .as_ref()
                    .map_or("", |p| p.layout.as_ref())
                    .into(),
            );
            let (to_layout, to_source) = self.viewer_source_cache.offsets(state);
            ui.set_viewer_source_to_layout(to_layout);
            ui.set_viewer_source_to_source(to_source);
            ui.set_viewer_source_runs(self.viewer_source_cache.runs(
                state,
                ui.get_viewer_source_first(),
                ui.get_viewer_source_count(),
            ));
            // A search can finish while colors are still being prepared. Move
            // its selection to the new presentation after publishing the layout.
            if restore_search_selection {
                editor.navigation.selection = editor.navigation.selection_in_display(&editor.text);
            }
        } else {
            ui.set_viewer_source_active(false);
            ui.set_viewer_source_notice("".into());
        }

        let preview = &mut self.app.preview;
        let source = self.app.preferences.preview && preview.source_text;
        ui.set_preview_source(source);
        if source {
            let state = &mut preview.presentation;
            let path = &preview.target.as_ref().unwrap().1.path;
            state.update(
                path,
                &preview.text,
                preview.complete,
                PREVIEW_LIMIT,
                self.dark,
                &self.ctx,
            );
            ui.set_preview_source_mode(state.mode.label().into());
            ui.set_preview_source_modes(self.preview_source_cache.modes(state));
            ui.set_preview_source_notice(state.notice.as_str().into());
            ui.set_preview_source_active(state.prepared.is_some() && state.mode != Mode::Plain);
            ui.set_preview_source_revision(state.revision as i32);
            ui.set_preview_source_lines(
                state.prepared.as_ref().map_or(1, |p| p.lines.len() as i32),
            );
            ui.set_preview_source_background(color(
                state.prepared.as_ref().map_or([255; 4], |p| p.background),
            ));
            ui.set_preview_source_foreground(color(
                state
                    .prepared
                    .as_ref()
                    .map_or([0, 0, 0, 255], |p| p.foreground),
            ));
            ui.set_preview_source_layout(
                state
                    .prepared
                    .as_ref()
                    .map_or("", |p| p.layout.as_ref())
                    .into(),
            );
            let (to_layout, to_source) = self.preview_source_cache.offsets(state);
            ui.set_preview_source_to_layout(to_layout);
            ui.set_preview_source_to_source(to_source);
            ui.set_preview_source_runs(self.preview_source_cache.runs(
                state,
                ui.get_preview_source_first(),
                ui.get_preview_source_count(),
            ));
        } else {
            ui.set_preview_source_active(false);
            ui.set_preview_source_notice("".into());
        }
    }
}
