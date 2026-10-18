use super::*;
use crate::preferences::Key;

fn key_is(text: &str, key: slint::platform::Key) -> bool {
    let value: SharedString = key.into();
    text == value.as_str()
}

fn normalized_key(text: &str) -> Option<Key> {
    if text.chars().count() == 1
        && let Some(code) = text.chars().next().map(u32::from)
        && (0xF704..=0xF726).contains(&code)
    {
        return Key::from_name(&format!("F{}", code - 0xF704 + 1));
    }
    use slint::platform::Key as S;
    for (slint, name) in [
        (S::F1, "F1"),
        (S::F2, "F2"),
        (S::F3, "F3"),
        (S::F4, "F4"),
        (S::F5, "F5"),
        (S::F6, "F6"),
        (S::F7, "F7"),
        (S::F8, "F8"),
        (S::F9, "F9"),
        (S::F10, "F10"),
        (S::F11, "F11"),
        (S::F12, "F12"),
        (S::Return, "Enter"),
        (S::Escape, "Escape"),
        (S::Backspace, "Backspace"),
        (S::Delete, "Delete"),
        (S::Insert, "Insert"),
        (S::Tab, "Tab"),
        (S::UpArrow, "ArrowUp"),
        (S::DownArrow, "ArrowDown"),
        (S::LeftArrow, "ArrowLeft"),
        (S::RightArrow, "ArrowRight"),
        (S::Home, "Home"),
        (S::End, "End"),
        (S::PageUp, "PageUp"),
        (S::PageDown, "PageDown"),
    ] {
        if key_is(text, slint) {
            return Key::from_name(name);
        }
    }
    Key::from_name(&text.to_uppercase())
}

fn configured_action(
    preferences: &crate::preferences::Preferences,
    event: &slint::language::KeyEvent,
) -> Option<String> {
    let key = normalized_key(event.text.as_str())?;
    preferences
        .shortcuts
        .iter()
        .chain(preferences.alternate_shortcuts.iter())
        .find_map(|(name, binding)| {
            let binding = crate::preferences::shortcut(binding).ok()?;
            (binding.logical_key == key
                && binding.modifiers.command == event.modifiers.control
                && binding.modifiers.shift == event.modifiers.shift
                && binding.modifiers.alt == event.modifiers.alt)
                .then(|| name.clone())
        })
}

impl Adapter {
    pub(super) fn global_key(
        &mut self,
        ui: &CommanderWindow,
        event: slint::language::KeyEvent,
    ) -> bool {
        // Slint maps macOS Command to `control` and physical Control to `meta`.
        let text = event.text.as_str();
        if self.parity_list_key(&event) {
            return true;
        }
        if self.extra.dialog.is_some() && key_is(text, slint::platform::Key::Escape) {
            self.extra.dialog = None;
            return true;
        }
        if self.extra.dialog.is_some() && key_is(text, slint::platform::Key::Return) {
            self.parity_dialog_action("accept");
            return true;
        }
        if self.blocked() {
            if key_is(text, slint::platform::Key::Escape) {
                if self.app.editor.is_some() && !self.app.confirm_exit {
                    self.viewer_action(
                        if self
                            .app
                            .editor
                            .as_ref()
                            .is_some_and(|editor| editor.confirm_close)
                        {
                            "keep"
                        } else {
                            "close"
                        },
                    );
                } else {
                    self.dialog_action("cancel");
                }
                return true;
            }
            if let Some(action) = configured_action(&self.app.preferences, &event)
                && matches!(action.as_str(), "SaveText" | "Quit" | "CloseWindow")
            {
                self.action(&action);
                return true;
            }
            if key_is(text, slint::platform::Key::Return) && self.app.prompt.is_some() {
                self.dialog_action("accept");
                return true;
            }
            return false;
        }
        if self.app.terminal_visible && ui.get_terminal_focused() && !event.modifiers.control {
            return false;
        }
        if let Some(action) = configured_action(&self.app.preferences, &event) {
            self.action(&action);
            return true;
        }
        false
    }

    pub(super) fn pane_key(
        &mut self,
        ui: &CommanderWindow,
        side: i32,
        event: slint::language::KeyEvent,
    ) -> bool {
        if !(0..2).contains(&side) || self.blocked() {
            return self.global_key(ui, event);
        }
        self.app.active = side as usize;
        if self.quick_find_consumes(side as usize, &event) && self.parity_key(side as usize, &event)
        {
            return true;
        }
        if self.global_key(ui, event.clone()) {
            return true;
        }
        self.parity_key(side as usize, &event)
    }

    pub(super) fn viewer_action(&mut self, action: &str) {
        let Some(mut editor) = self.app.editor.take() else {
            return;
        };
        if action.starts_with("encoding:") || action.starts_with("save-encoding:") {
            let (kind, value) = action.split_once(':').unwrap();
            let result = (|| -> anyhow::Result<()> {
                let encoding = crate::text_document::Encoding::named(value)
                    .ok_or_else(|| anyhow::anyhow!("Unknown encoding"))?;
                if kind == "save-encoding" {
                    if let Some(document) = &mut editor.document {
                        let mut candidate = document.clone();
                        candidate.encoding = encoding;
                        candidate.encode(&editor.text)?;
                        *document = candidate;
                    }
                } else {
                    anyhow::ensure!(
                        !editor.dirty(),
                        "Save or discard edits before changing the decoding"
                    );
                    if editor.text_page.is_some()
                        || editor
                            .path
                            .as_ref()
                            .and_then(|p| std::fs::metadata(p).ok())
                            .is_some_and(|m| m.len() > 2 * 1024 * 1024)
                    {
                        let path = editor
                            .path
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("A local file is required"))?;
                        let offset = editor.text_page.as_ref().map(|p| p.offset).unwrap_or(0);
                        let next = crate::text_document::Page::read(path, offset, encoding)?;
                        editor.text = next.document.text.clone();
                        editor.original = editor.text.clone();
                        editor.text_page = Some(next);
                        editor.document = None;
                        editor.editable = false;
                        editor.kind = "Paged text viewer";
                    } else {
                        let bytes = if let Some(document) = &editor.document {
                            document.original.clone()
                        } else if let Some(path) = &editor.path {
                            crate::viewer::read(
                                &Location::Local(path.clone()),
                                path,
                                2 * 1024 * 1024,
                            )?
                        } else if let Some((connection, path)) = &editor.remote {
                            crate::remote::read_bytes(connection, path, 2 * 1024 * 1024)?
                        } else {
                            anyhow::bail!("Text encoding is unavailable")
                        };
                        let document = crate::text_document::Document::decode(bytes, encoding)?;
                        editor.text = document.text.clone();
                        editor.original = editor.text.clone();
                        editor.document = Some(document);
                        editor.kind = "Text viewer";
                        editor.editable = editor.can_edit;
                    }
                }
                Ok(())
            })();
            editor.error = result.err().map(|e| format!("{e:#}"));
            self.app.editor = Some(editor);
            return;
        }
        if matches!(action, "next" | "previous")
            && let Some(page) = &editor.text_page
        {
            let offset = if action == "next" {
                page.next
            } else {
                page.offset.saturating_sub(crate::text_document::PAGE_SIZE)
            };
            if let Some(path) = &editor.path {
                match crate::text_document::Page::read(path, offset, page.document.encoding) {
                    Ok(next) => {
                        editor.text = next.document.text.clone();
                        editor.text_page = Some(next);
                    }
                    Err(error) => editor.error = Some(error.to_string()),
                }
            }
            self.app.editor = Some(editor);
            return;
        }
        match action {
            "save" => self.app.save_editor(&editor, &self.ctx),
            "close" => {
                if editor.dirty() {
                    editor.confirm_close = true;
                } else {
                    self.app.refresh(&self.ctx);
                    return;
                }
            }
            "keep" => editor.confirm_close = false,
            "discard" => {
                self.app.refresh(&self.ctx);
                return;
            }
            "next" | "previous" => {
                if let Some(pdf) = &editor.pdf {
                    let page = if action == "next" {
                        pdf.page.saturating_add(1).min(pdf.pages.saturating_sub(1))
                    } else {
                        pdf.page.saturating_sub(1)
                    };
                    if page != pdf.page && self.app.pending_pdf.is_none() {
                        let bytes = pdf.bytes.clone();
                        let title = editor.title.clone();
                        let ctx = self.ctx.clone();
                        let (sender, receiver) = mpsc::channel();
                        self.app.pending_pdf = Some(receiver);
                        std::thread::spawn(move || {
                            let result = crate::viewer::render_pdf(&bytes, page)
                                .map(|(image, _)| (title, page, image))
                                .map_err(|error| error.to_string());
                            let _ = sender.send(result);
                            ctx.request_repaint();
                        });
                    }
                }
            }
            _ => {}
        }
        self.app.editor = Some(editor);
    }

    pub(super) fn terminal_key(&mut self, event: slint::language::KeyEvent) -> bool {
        if self.blocked() {
            return false;
        }
        let text = event.text.as_str();
        // Slint maps macOS Command to `control` and physical Control to `meta`.
        let command = event.modifiers.control;
        let control = if cfg!(target_os = "macos") {
            event.modifiers.meta
        } else {
            event.modifiers.control
        };
        if ((cfg!(target_os = "macos") && command)
            || (!cfg!(target_os = "macos") && event.modifiers.meta))
            && !text.eq_ignore_ascii_case("v")
        {
            return true;
        }
        let bytes = if command && text.eq_ignore_ascii_case("v") {
            match self.app.native_files.text() {
                Ok(text) => text.into_bytes(),
                Err(error) => {
                    self.app.fail(error);
                    return true;
                }
            }
        } else if key_is(text, slint::platform::Key::Return) {
            b"\r".to_vec()
        } else if key_is(text, slint::platform::Key::Backspace) {
            vec![127]
        } else if key_is(text, slint::platform::Key::Delete) {
            b"\x1b[3~".to_vec()
        } else if key_is(text, slint::platform::Key::Tab) {
            vec![9]
        } else if key_is(text, slint::platform::Key::Escape) {
            vec![27]
        } else if key_is(text, slint::platform::Key::UpArrow) {
            b"\x1b[A".to_vec()
        } else if key_is(text, slint::platform::Key::DownArrow) {
            b"\x1b[B".to_vec()
        } else if key_is(text, slint::platform::Key::RightArrow) {
            b"\x1b[C".to_vec()
        } else if key_is(text, slint::platform::Key::LeftArrow) {
            b"\x1b[D".to_vec()
        } else if key_is(text, slint::platform::Key::Home) {
            b"\x1b[H".to_vec()
        } else if key_is(text, slint::platform::Key::End) {
            b"\x1b[F".to_vec()
        } else if control && text.len() == 1 && text.as_bytes()[0].is_ascii_alphabetic() {
            vec![text.as_bytes()[0].to_ascii_uppercase() - b'@']
        } else if text.chars().any(char::is_control) || text.is_empty() {
            return false;
        } else {
            let mut bytes = Vec::new();
            if event.modifiers.alt {
                bytes.push(27);
            }
            bytes.extend_from_slice(text.as_bytes());
            bytes
        };
        if let Some(terminal) = &mut self.app.terminal
            && let Err(error) = terminal.send(&bytes)
        {
            self.app.fail(error);
        }
        true
    }
}
