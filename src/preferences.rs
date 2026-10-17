use anyhow::{Result, bail};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub desktop_style: crate::appearance::DesktopStyle,
    pub color_scheme: crate::appearance::ColorScheme,
    pub font_size: f32,
    pub toolbar: bool,
    pub command_bar: bool,
    pub status_bar: bool,
    pub single_pane: bool,
    pub auto_refresh: bool,
    pub folders_first: bool,
    pub shortcuts: BTreeMap<String, String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            desktop_style: Default::default(),
            color_scheme: Default::default(),
            font_size: 14.0,
            toolbar: true,
            command_bar: !cfg!(target_os = "macos"),
            status_bar: true,
            single_pane: false,
            auto_refresh: true,
            folders_first: true,
            shortcuts: [
                ("Help", "F1"),
                ("Rename", "F2"),
                ("View", "F3"),
                ("Edit", "F4"),
                ("Copy", "F5"),
                ("Move", "F6"),
                ("Mkdir", "F7"),
                ("Trash", "F8"),
                ("NewTab", "Command+T"),
                ("CloseTab", "Command+W"),
                ("Refresh", "Command+R"),
                ("Bookmark", "Command+D"),
                ("Search", "Command+Shift+F"),
                ("CopyClipboard", "Command+C"),
                ("CutClipboard", "Command+X"),
                ("PasteClipboard", "Command+V"),
            ]
            .into_iter()
            .map(|(a, key)| (a.into(), key.into()))
            .collect(),
        }
    }
}

pub fn shortcut(text: &str) -> Result<egui::KeyboardShortcut> {
    let mut modifiers = egui::Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_lowercase().as_str() {
            "command" | "cmd" | "ctrl" | "control" => modifiers.command = true,
            "shift" => modifiers.shift = true,
            "alt" | "option" => modifiers.alt = true,
            _ => {
                if key.is_some() {
                    bail!("Shortcut must contain one key.");
                }
                key = egui::Key::from_name(part);
                if key.is_none() {
                    bail!("Unknown key: {part}");
                }
            }
        }
    }
    Ok(egui::KeyboardShortcut::new(
        modifiers,
        key.ok_or_else(|| anyhow::anyhow!("Shortcut must contain a key"))?,
    ))
}

pub fn validate(preferences: &Preferences) -> Result<()> {
    if !preferences.font_size.is_finite() || !(10.0..=24.0).contains(&preferences.font_size) {
        bail!("Font size must be between 10 and 24.");
    }
    let mut keys = Vec::new();
    for value in preferences.shortcuts.values().filter(|v| !v.is_empty()) {
        let key = shortcut(value)?;
        if keys.contains(&key) {
            bail!("Duplicate shortcut: {value}");
        }
        keys.push(key);
    }
    Ok(())
}
