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

impl Adapter {
    pub(super) fn global_key(
        &mut self,
        ui: &CommanderWindow,
        event: slint::language::KeyEvent,
    ) -> bool {
        // Slint maps macOS Command to `control` and physical Control to `meta`.
        let command = event.modifiers.control;
        let text = event.text.as_str();
        if command && text.eq_ignore_ascii_case("q") {
            self.request_close();
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
            if command && text.eq_ignore_ascii_case("s") && self.app.editor.is_some() {
                self.viewer_action("save");
                return true;
            }
            if key_is(text, slint::platform::Key::Return) && self.app.prompt.is_some() {
                self.dialog_action("accept");
                return true;
            }
            return false;
        }
        if self.app.terminal_visible && ui.get_terminal_focused() {
            return false;
        }
        if command
            && !event.modifiers.shift
            && !event.modifiers.alt
            && text.eq_ignore_ascii_case("l")
        {
            ui.invoke_focus_address(self.app.active as i32);
            return true;
        }
        if command
            && !event.modifiers.shift
            && !event.modifiers.alt
            && text.eq_ignore_ascii_case("f")
        {
            ui.invoke_focus_filter(self.app.active as i32);
            return true;
        }
        if command && event.modifiers.shift && text.eq_ignore_ascii_case("z") {
            self.action("Redo");
            return true;
        }
        if command && text.eq_ignore_ascii_case("p") {
            self.action("Preview");
            return true;
        }
        if command && text == "," {
            self.action("Preferences");
            return true;
        }
        if (cfg!(target_os = "macos") && command && event.modifiers.shift && text == ".")
            || (!cfg!(target_os = "macos") && command && text.eq_ignore_ascii_case("h"))
        {
            self.action("ToggleHidden");
            return true;
        }
        if event.modifiers.alt && key_is(text, slint::platform::Key::LeftArrow) {
            self.action("Back");
            return true;
        }
        if event.modifiers.alt && key_is(text, slint::platform::Key::RightArrow) {
            self.action("Forward");
            return true;
        }
        if let Some(key) = normalized_key(text) {
            let action = self
                .app
                .preferences
                .shortcuts
                .iter()
                .find_map(|(name, binding)| {
                    let binding = crate::preferences::shortcut(binding).ok()?;
                    (binding.logical_key == key
                        && binding.modifiers.command == command
                        && binding.modifiers.shift == event.modifiers.shift
                        && binding.modifiers.alt == event.modifiers.alt)
                        .then(|| name.clone())
                });
            if let Some(action) = action {
                self.action(&action);
                return true;
            }
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
        let text = event.text.as_str();
        let shift = event.modifiers.shift;
        let hidden = self.app.show_hidden;
        if key_is(text, slint::platform::Key::Tab) {
            self.app.active = 1 - self.app.active;
            ui.invoke_focus_files(self.app.active as i32);
            return true;
        }
        if key_is(text, slint::platform::Key::UpArrow) {
            self.app.tab_mut().move_cursor(-1, shift, hidden);
            return true;
        }
        if key_is(text, slint::platform::Key::DownArrow) {
            self.app.tab_mut().move_cursor(1, shift, hidden);
            return true;
        }
        if key_is(text, slint::platform::Key::PageUp) {
            self.app.tab_mut().move_cursor(-15, shift, hidden);
            return true;
        }
        if key_is(text, slint::platform::Key::PageDown) {
            self.app.tab_mut().move_cursor(15, shift, hidden);
            return true;
        }
        if key_is(text, slint::platform::Key::Home) {
            self.app.tab_mut().move_cursor(-isize::MAX, shift, hidden);
            return true;
        }
        if key_is(text, slint::platform::Key::End) {
            self.app.tab_mut().move_cursor(isize::MAX, shift, hidden);
            return true;
        }
        if key_is(text, slint::platform::Key::Return) {
            if let Some(entry) = self.app.tab().chosen(hidden).first().cloned() {
                self.app.open_entry(entry, &self.ctx);
            }
            return true;
        }
        if key_is(text, slint::platform::Key::Backspace) {
            self.action("Parent");
            return true;
        }
        if text == " " || key_is(text, slint::platform::Key::Insert) {
            if let Some(path) = self.app.tab().cursor.clone() {
                let tab = self.app.tab_mut();
                if !tab.selected.remove(&path) {
                    tab.selected.insert(path);
                }
            }
            return true;
        }
        if key_is(text, slint::platform::Key::Escape) {
            self.app.tab_mut().selected.clear();
            self.app.tab_mut().filter.clear();
            return true;
        }
        // Slint maps macOS Command to `control` and physical Control to `meta`.
        let command = event.modifiers.control;
        if command && text.eq_ignore_ascii_case("a") {
            self.action("MarkAll");
            return true;
        }
        self.global_key(ui, event)
    }

    pub(super) fn viewer_action(&mut self, action: &str) {
        let Some(mut editor) = self.app.editor.take() else {
            return;
        };
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
