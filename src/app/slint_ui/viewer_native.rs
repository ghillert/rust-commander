//! Native viewer chrome. Callbacks enqueue document-scoped events; the existing
//! controller remains the only owner of search, encoding, source buffers and saves.
#![cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
use super::*;
use std::collections::HashMap;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(target_os = "windows")]
use windows::Platform;

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Button,
    Label,
    Modes,
    Choice,
    Search,
    Field,
}
#[derive(Clone, Debug, PartialEq)]
struct Control {
    id: String,
    kind: Kind,
    label: String,
    value: String,
    choices: Vec<String>,
    enabled: bool,
    frame: [f32; 4],
}
impl Control {
    fn new(id: &str, kind: Kind, label: &str, frame: [f32; 4]) -> Self {
        Self {
            id: id.into(),
            kind,
            label: tr(label).into_owned(),
            value: String::new(),
            choices: vec![],
            enabled: true,
            frame,
        }
    }
}
enum Action {
    Activate(String),
    Change(String, String),
    Key(slint::language::KeyEvent),
}
struct Event {
    document: u64,
    action: Action,
}
type Events = Rc<RefCell<Vec<Event>>>;

#[derive(Default)]
pub(super) struct Host {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    platform: Option<Platform>,
    events: Events,
    document: u64,
    values: HashMap<String, String>,
}
impl Host {
    fn events(&mut self) -> Vec<Event> {
        self.events.take()
    }
    fn value(&self, id: &str) -> String {
        self.values.get(id).cloned().unwrap_or_default()
    }
    #[cfg(test)]
    pub(super) fn test_event(&self, document: u64, id: &str, value: Option<&str>) {
        self.events.borrow_mut().push(Event {
            document,
            action: match value {
                Some(value) => Action::Change(id.into(), value.into()),
                None => Action::Activate(id.into()),
            },
        });
    }
    pub fn sync(
        &mut self,
        ui: &CommanderWindow,
        document: Option<u64>,
        dark: bool,
        preferences: &crate::preferences::Preferences,
    ) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let document = document.filter(|_| {
                ui.get_viewer_visible()
                    && ui.get_dialog_title().is_empty()
                    && !ui.get_native_form_open()
            });
            if document != Some(self.document) {
                ui.set_viewer_native_tools_open(false);
                self.platform = None;
                if document.is_some() {
                    ui.invoke_restore_viewer_focus();
                }
                self.values.clear();
                self.document = document.unwrap_or(0);
            }
            let Some(document) = document else {
                ui.set_viewer_native(false);
                return;
            };
            if self.platform.is_none() {
                self.platform = Platform::new(ui, document, self.events.clone());
            }
            let Some(platform) = self.platform.as_mut() else {
                ui.set_viewer_native(false);
                return;
            };
            ui.set_viewer_native(true);
            let widths = header_widths(ui);
            ui.set_viewer_native_toolbar_width(
                widths.iter().map(|(_, w)| w + 8.0).sum::<f32>() - 8.0,
            );
            let controls = header(ui, &widths);
            let (tools, height) = tools(ui, &self.values);
            let shortcuts = preferences
                .shortcuts
                .iter()
                .chain(preferences.alternate_shortcuts.iter())
                .filter(|(name, _)| matches!(name.as_str(), "SaveText" | "Quit" | "CloseWindow"))
                .filter_map(|(_, binding)| crate::preferences::shortcut(binding).ok())
                .collect::<Vec<_>>();
            platform.sync(ui, &controls, &tools, height, dark, &shortcuts);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (document, dark, preferences);
            ui.set_viewer_native(false);
        }
    }
    #[cfg(feature = "screenshot")]
    pub fn snapshot(&self, ui: &CommanderWindow, path: &Path) -> bool {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(platform) = &self.platform {
            return platform.snapshot(ui, path);
        }
        let _ = (ui, path);
        false
    }
    #[cfg(feature = "screenshot")]
    pub fn smoke(&self, id: &str, value: &str) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(platform) = &self.platform {
            platform.smoke(id, value);
        }
        let _ = (id, value);
    }
}
fn intercept(key: &slint::language::KeyEvent, shortcuts: &[crate::preferences::Shortcut]) -> bool {
    let escape: SharedString = slint::platform::Key::Escape.into();
    key.text == escape
        || (key.modifiers.control && key.text.eq_ignore_ascii_case("f"))
        || shortcuts.iter().any(|s| {
            super::keys::normalized_key(key.text.as_str()).as_ref() == Some(&s.logical_key)
                && s.modifiers.command == key.modifiers.control
                && s.modifiers.shift == key.modifiers.shift
                && s.modifiers.alt == key.modifiers.alt
        })
}
fn header_widths(ui: &CommanderWindow) -> Vec<(&'static str, f32)> {
    let mut v = vec![];
    if ui.get_viewer_source() {
        v.push((
            "mode",
            match ui.get_viewer_source_modes().row_count() {
                3 => 250.0,
                2 => 180.0,
                _ => 80.0,
            },
        ));
    }
    if ui.get_viewer_pdf() {
        v.extend([("previous", 30.0), ("page-label", 120.0), ("next", 30.0)]);
    }
    if ui.get_viewer_pdf_document() && !ui.get_viewer_locked() {
        v.push(("pdf-text", 80.0));
    }
    if ui.get_viewer_text_controls() || (ui.get_viewer_pdf_document() && !ui.get_viewer_locked()) {
        v.push(("find", 64.0));
    }
    if ui.get_viewer_can_save() {
        v.push(("save", 62.0));
    }
    if ui.get_viewer_text_controls() || (ui.get_viewer_pdf_document() && !ui.get_viewer_locked()) {
        v.push(("tools", 32.0));
    }
    v.push(("close", 66.0));
    v
}
fn header(ui: &CommanderWindow, widths: &[(&str, f32)]) -> Vec<Control> {
    let mut x = ui.get_viewer_native_x();
    let y = ui.get_viewer_native_y();
    let mut controls = vec![];
    for &(id, width) in widths {
        let (kind, label) = match id {
            "mode" => (Kind::Modes, "Source view"),
            "page-label" => (Kind::Label, ""),
            "find" => (Kind::Button, "Find"),
            "tools" => (Kind::Button, "…"),
            "previous" => (Kind::Button, "‹"),
            "next" => (Kind::Button, "›"),
            "pdf-text" => (
                Kind::Button,
                if ui.get_viewer_pdf_text() {
                    "Page view"
                } else {
                    "Text view"
                },
            ),
            "save" => (Kind::Button, "Save"),
            _ => (Kind::Button, "Close"),
        };
        let mut control = Control::new(id, kind, label, [x, y, width, 32.0]);
        match id {
            "mode" => {
                control.choices = ui
                    .get_viewer_source_modes()
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                control.value = ui.get_viewer_source_mode().into();
            }
            "page-label" => control.label = ui.get_viewer_page().into(),
            "previous" => control.enabled = ui.get_viewer_has_previous(),
            "next" => control.enabled = ui.get_viewer_has_next(),
            "save" => control.enabled = ui.get_viewer_dirty() && !ui.get_busy(),
            _ => {}
        }
        controls.push(control);
        x += width + 8.0;
    }
    if ui.get_viewer_find_visible() {
        let right = ui.get_viewer_window_width() - 36.0;
        let y = ui.get_viewer_native_find_y() + 8.0;
        let buttons = if ui.get_viewer_pdf_document() {
            vec![
                ("find-previous", "Previous", 72.0),
                ("find-next", "Next", 62.0),
                ("close-find", "Done", 58.0),
            ]
        } else {
            vec![("find-next", "Next", 62.0), ("close-find", "Done", 58.0)]
        };
        let mut x = right - buttons.iter().map(|(_, _, w)| w + 8.0).sum::<f32>();
        let mut field = Control::new(
            "query",
            Kind::Search,
            "Find in file",
            [36.0, y, x - 44.0, 28.0],
        );
        field.value = ui.get_viewer_query().into();
        controls.push(field);
        for (id, label, width) in buttons {
            let mut c = Control::new(id, Kind::Button, label, [x, y, width, 28.0]);
            c.enabled =
                id == "close-find" || (!ui.get_viewer_busy() && !ui.get_viewer_query().is_empty());
            controls.push(c);
            x += width + 8.0;
        }
        if ui.get_viewer_busy() {
            let mut c = Control::new(
                "find-next",
                Kind::Button,
                "Cancel",
                [right - 128.0, y, 62.0, 28.0],
            );
            c.id = "cancel-search".into();
            controls.retain(|c| c.id != "find-next");
            controls.push(c);
        }
    }
    controls
}
fn tools(ui: &CommanderWindow, values: &HashMap<String, String>) -> (Vec<Control>, f32) {
    let mut v = vec![Control::new(
        "tools-title",
        Kind::Label,
        "Viewer tools",
        [16.0, 14.0, 320.0, 20.0],
    )];
    let mut y = 42.0;
    if ui.get_viewer_text_controls() {
        for (id, label, value) in [
            ("encoding", "Text encoding", ui.get_viewer_encoding()),
            (
                "save-encoding",
                "Save encoding",
                ui.get_viewer_save_encoding(),
            ),
        ] {
            if id == "save-encoding" && !ui.get_viewer_can_save() {
                continue;
            }
            v.push(Control::new(
                &format!("{id}-label"),
                Kind::Label,
                label,
                [16.0, y, 320.0, 18.0],
            ));
            y += 22.0;
            let mut c = Control::new(id, Kind::Choice, label, [16.0, y, 320.0, 28.0]);
            c.value = value.into();
            c.choices = [
                "UTF-8",
                "UTF-16 LE",
                "UTF-16 BE",
                "Windows-1252",
                "ISO-8859-1",
            ]
            .map(String::from)
            .to_vec();
            v.push(c);
            y += 40.0;
        }
        v.push(Control::new(
            "bytes-label",
            Kind::Label,
            "File bytes · uses original contents",
            [16.0, y, 320.0, 20.0],
        ));
        y += 28.0;
        for (id, label, button, title) in [
            ("hex", "Hex bytes, e.g. 4A 53", "hex-find", "Find hex"),
            ("offset", "Byte offset, e.g. 0x100", "jump", "Go"),
        ] {
            let mut c = Control::new(id, Kind::Field, label, [16.0, y, 230.0, 28.0]);
            c.value = values.get(id).cloned().unwrap_or_default();
            v.push(c);
            let mut c = Control::new(button, Kind::Button, title, [254.0, y, 82.0, 28.0]);
            c.enabled = !ui.get_viewer_busy();
            v.push(c);
            y += 38.0;
        }
    } else if ui.get_viewer_pdf_document() {
        let mut c = Control::new("page", Kind::Field, "Page number", [16.0, y, 230.0, 28.0]);
        c.value = values.get("page").cloned().unwrap_or_default();
        v.push(c);
        v.push(Control::new(
            "page-go",
            Kind::Button,
            "Go",
            [254.0, y, 82.0, 28.0],
        ));
        y += 38.0;
        for (id, label) in [
            ("rotate", "Rotate clockwise"),
            ("pdf-copy", "Copy selection"),
        ] {
            let mut c = Control::new(id, Kind::Button, label, [16.0, y, 320.0, 28.0]);
            c.enabled = id != "pdf-copy" || ui.get_viewer_pdf_text() || ui.get_viewer_pdf_copy();
            v.push(c);
            y += 38.0;
        }
    }
    (v, y + 8.0)
}
impl Adapter {
    pub(super) fn native_viewer_events(&mut self, ui: &CommanderWindow) {
        let mut closed_tools = false;
        for event in self.native_viewer.events() {
            if !self
                .app
                .editor
                .as_ref()
                .is_some_and(|e| e.navigation.id == event.document)
            {
                continue;
            }
            match event.action {
                Action::Key(key) => {
                    self.global_key(ui, key);
                }
                Action::Change(id, value) => match id.as_str() {
                    "query" => ui.set_viewer_query(value.into()),
                    "mode" => self.viewer_action(&format!("source-mode:{value}")),
                    "encoding" | "save-encoding" => self.viewer_action(&format!("{id}:{value}")),
                    _ => {
                        self.native_viewer.values.insert(id, value);
                    }
                },
                Action::Activate(id) => match id.as_str() {
                    "find" => {
                        if ui.get_viewer_find_visible() {
                            ui.set_viewer_find_visible(false);
                            ui.invoke_focus_viewer();
                        } else {
                            ui.invoke_focus_viewer_search();
                        }
                    }
                    "close-find" => {
                        ui.set_viewer_find_visible(false);
                        ui.invoke_focus_viewer();
                    }
                    "tools" => {
                        if !closed_tools {
                            ui.set_viewer_native_tools_open(!ui.get_viewer_native_tools_open());
                        }
                    }
                    "tools-closed" => {
                        ui.set_viewer_native_tools_open(false);
                        closed_tools = true;
                    }
                    "focus-document" => ui.invoke_focus_viewer(),
                    "find-next" | "find-previous" => {
                        let prefix = if ui.get_viewer_pdf_document() {
                            if id == "find-previous" {
                                "pdf-find-previous:"
                            } else {
                                "pdf-find:"
                            }
                        } else {
                            "find:"
                        };
                        self.viewer_action(&format!("{prefix}{}", ui.get_viewer_query()));
                    }
                    "hex-find" | "jump" | "page-go" => {
                        let field = match id.as_str() {
                            "hex-find" => "hex",
                            "jump" => "offset",
                            _ => "page",
                        };
                        let action = if id == "page-go" { "page" } else { &id };
                        self.viewer_action(&format!(
                            "{action}:{}",
                            self.native_viewer.value(field)
                        ));
                        ui.set_viewer_native_tools_open(false);
                        ui.invoke_focus_viewer();
                    }
                    _ => self.viewer_action(&id),
                },
            }
        }
    }
}
