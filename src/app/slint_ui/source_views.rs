//! Publish bounded, visible syntax highlighting to the two shared Slint viewers.
use super::*;
use crate::source_view::{Mode, PREVIEW_LIMIT, Prepared, State, VIEWER_LIMIT};
use std::fmt::Write;

const GROUP_LINES: usize = 16;
const GROUP_BYTE_LIMIT: usize = 256 * 1024;

#[derive(Default)]
pub(super) struct Cache {
    stamp: Option<(u64, u64, usize, usize)>,
    groups: Rc<VecModel<SourceLineGroup>>,
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
    fn groups(&mut self, state: &State, first: i32, count: i32) -> ModelRc<SourceLineGroup> {
        let Some(prepared) = &state.prepared else {
            if self.stamp.take().is_some() {
                self.groups = Default::default();
            }
            return self.groups.clone().into();
        };
        let first = first.max(0) as usize / GROUP_LINES;
        // Round the viewport up with room for a partial group at either edge.
        let count = (count.clamp(24, 256) as usize + GROUP_LINES - 1).div_ceil(GROUP_LINES);
        let stamp = (state.id, state.revision, first, count);
        if self.stamp != Some(stamp) {
            let previous = self.stamp.filter(|&(id, revision, _, old_count)| {
                (id, revision, old_count) == (state.id, state.revision, count)
            });
            if let Some((_, _, old_first, _)) = previous {
                // Reuse the slots and the renderer's text layouts for groups
                // that are still in the viewport.
                for group in first..first + count {
                    if !(old_first..old_first + count).contains(&group) {
                        self.groups
                            .set_row_data(group % count, highlighted_group(prepared, group));
                    }
                }
            } else {
                let mut groups = vec![SourceLineGroup::default(); count];
                for group in first..first + count {
                    groups[group % count] = highlighted_group(prepared, group);
                }
                self.groups = Rc::new(VecModel::from(groups));
            }
            self.stamp = Some(stamp);
        }
        self.groups.clone().into()
    }
}

fn highlighted_group(prepared: &Prepared, group: usize) -> SourceLineGroup {
    let first = group * GROUP_LINES;
    let mut lines = Vec::new();
    let mut bytes = 0;
    let mapped = |offset: usize| {
        prepared
            .to_layout
            .get(offset)
            .map_or(offset, |&n| n as usize)
    };
    for index in first..(first + GROUP_LINES).min(prepared.lines.len()) {
        let line = &prepared.lines[index];
        let spans = &prepared.spans[line.spans.clone()];
        let text = &prepared.layout[mapped(line.range.start)..mapped(line.range.end)];
        if spans.len() > 512 || text.contains(['\r', '\0']) {
            continue;
        }
        if spans.iter().all(|span| span.color == prepared.foreground) {
            continue;
        }
        let mut markup = String::new();
        for span in spans {
            let text = &prepared.layout[mapped(span.range.start)..mapped(span.range.end)];
            if span.color == prepared.foreground {
                push_literal(&mut markup, text);
            } else {
                let [r, g, b, a] = span.color;
                let _ = write!(markup, "<font color=\"#{r:02x}{g:02x}{b:02x}{a:02x}\">");
                push_literal(&mut markup, text);
                markup.push_str("</font>");
            }
        }
        if bytes + markup.len() > GROUP_BYTE_LIMIT {
            continue;
        }
        if let Ok(text) = slint::StyledText::from_markdown(&markup) {
            bytes += markup.len();
            lines.push(SourceLine {
                line: index as i32,
                text,
            });
        }
    }
    SourceLineGroup {
        lines: Rc::new(VecModel::from(lines)).into(),
    }
}

fn push_literal(markup: &mut String, text: &str) {
    // Only the tags added above have Markdown meaning. Encode spaces as well
    // so indentation and trailing whitespace survive CommonMark's trimming.
    for ch in text.chars() {
        if ch.is_ascii_punctuation() || ch == ' ' {
            let _ = write!(markup, "&#{};", ch as u32);
        } else {
            markup.push(ch);
        }
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
            ui.set_viewer_source_groups(self.viewer_source_cache.groups(
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
            ui.set_preview_source_groups(self.preview_source_cache.groups(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_view::{Line, Span};

    #[test]
    fn source_characters_are_literal_in_the_highlighting_markup() {
        let source = r#"  [link](url) <tag attr="value"/> &lt; \# 雪  "#;
        let mut markup = String::new();
        push_literal(&mut markup, source);
        assert_eq!(
            slint::StyledText::from_markdown(&markup).unwrap(),
            slint::StyledText::from_plain_text(source)
        );
    }

    #[test]
    fn scrolling_only_replaces_highlights_leaving_the_viewport() {
        let mut state = State::default();
        let text: Arc<str> = "x\n".repeat(64).into();
        state.prepared = Some(Prepared {
            text: text.clone(),
            layout: text,
            to_layout: vec![],
            to_source: vec![],
            lines: (0..64)
                .map(|i| Line {
                    range: 2 * i..2 * i + 1,
                    spans: i..i + 1,
                })
                .collect(),
            spans: (0..64)
                .map(|i| Span {
                    range: 2 * i..2 * i + 1,
                    color: [0, 128, 0, 255],
                })
                .collect(),
            background: [255; 4],
            foreground: [0, 0, 0, 255],
        });
        let mut cache = Cache::default();
        let groups = cache.groups(&state, 0, 32);
        let overlapping = groups.row_data(1).unwrap().lines;
        assert_eq!(overlapping.row_data(0).unwrap().line, 16);
        assert_eq!(cache.groups(&state, 16, 32), groups);
        assert_eq!(groups.row_data(1).unwrap().lines, overlapping);
        assert_eq!(
            groups.row_data(0).unwrap().lines.row_data(0).unwrap().line,
            48
        );

        assert_eq!(cache.groups(&state, 0, 32), groups);
        assert_eq!(groups.row_data(1).unwrap().lines, overlapping);
        assert_eq!(
            groups.row_data(0).unwrap().lines.row_data(0).unwrap().line,
            0
        );

        state.revision += 1;
        assert_ne!(cache.groups(&state, 0, 32), groups);
    }
}
