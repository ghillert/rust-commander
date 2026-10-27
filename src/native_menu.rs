//! AppKit menus. Constructed and updated only on Slint's main UI thread.
use crate::ui_signal::UiSignal;
use crate::{
    APP_NAME,
    app::{Action, action_menus},
};
use muda::{
    Menu, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use std::sync::mpsc::{self, Receiver};

pub struct NativeMenu {
    _menu: Menu,
    items: Vec<(MenuItem, Action, &'static str)>,
    submenu_labels: Vec<(Submenu, &'static str)>,
    receiver: Receiver<muda::MenuId>,
    binding_cache: std::cell::RefCell<std::collections::BTreeMap<String, String>>,
}
impl NativeMenu {
    pub fn new(ctx: &UiSignal) -> muda::Result<Self> {
        let menu = Menu::new();
        let mut items = Vec::new();
        let mut submenu_labels = Vec::new();
        let app = Submenu::new(APP_NAME, true);
        app.append(&PredefinedMenuItem::about(
            None,
            Some(muda::AboutMetadata {
                name: Some(APP_NAME.into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                ..Default::default()
            }),
        ))?;
        app.append(&PredefinedMenuItem::separator())?;
        for (label, action, key) in [
            ("Settings…", Action::Preferences, Code::Comma),
            ("Quit Rust Commander", Action::Quit, Code::KeyQ),
        ] {
            let item = MenuItem::with_id(
                format!("{action:?}"),
                crate::i18n::text(label).as_ref(),
                true,
                Some(Accelerator::new(Modifiers::META, key)),
            );
            app.append(&item)?;
            items.push((item, action, label));
            if matches!(action, Action::Preferences) {
                app.append(&PredefinedMenuItem::separator())?;
                app.append(&PredefinedMenuItem::hide(None))?;
                app.append(&PredefinedMenuItem::hide_others(None))?;
                app.append(&PredefinedMenuItem::show_all(None))?;
                app.append(&PredefinedMenuItem::separator())?;
            }
        }
        menu.append(&app)?;
        let window = Submenu::with_items(
            crate::i18n::text("Window").as_ref(),
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::maximize(None),
                &PredefinedMenuItem::fullscreen(None),
            ],
        )?;

        for (title, entries) in action_menus() {
            let submenu = if title == "Window" {
                window.clone()
            } else {
                Submenu::new(crate::i18n::text(title).as_ref(), true)
            };
            for (label, action) in entries {
                let item = MenuItem::with_id(
                    format!("{title}-{action:?}"),
                    crate::i18n::text(label.split("  F").next().unwrap_or(label)).as_ref(),
                    true,
                    None,
                );
                submenu.append(&item)?;
                items.push((item, action, label));
            }
            submenu_labels.push((submenu.clone(), title));
            if title == "Help" {
                submenu.set_as_help_menu_for_nsapp();
            }
            menu.append(&submenu)?;
        }
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
            let _ = sender.send(event.id);
            ctx.request_repaint();
        }));
        menu.init_for_nsapp();
        window.set_as_windows_menu_for_nsapp();
        Ok(Self {
            _menu: menu,
            items,
            submenu_labels,
            receiver,
            binding_cache: Default::default(),
        })
    }

    /// Keep the event receiver alive: muda installs its global handler only once.
    pub fn localize(&self) {
        self.binding_cache.borrow_mut().clear();
        for (item, _, label) in &self.items {
            item.set_text(crate::i18n::text(label.split("  F").next().unwrap_or(label)).as_ref());
        }
        for (submenu, label) in &self.submenu_labels {
            submenu.set_text(crate::i18n::text(label).as_ref());
        }
    }

    pub fn update_bindings(&self, preferences: &crate::preferences::Preferences) {
        if *self.binding_cache.borrow() == preferences.shortcuts {
            return;
        }
        for (item, action, label) in &self.items {
            let id = format!("{action:?}");
            let binding = preferences
                .shortcuts
                .get(&id)
                .map(String::as_str)
                .unwrap_or("");
            // Other accelerators stay in Slint so native text editing retains
            // ownership of Copy/Cut/Paste/Undo while a text field has focus.
            if matches!(action, Action::Quit | Action::Preferences) {
                let accelerator = if binding.is_empty() {
                    None
                } else {
                    binding.parse().ok()
                };
                let _ = item.set_accelerator(accelerator);
                item.set_text(crate::i18n::text(label).as_ref());
            } else {
                let title = crate::i18n::text(label.split("  F").next().unwrap_or(label));
                let text = if binding.is_empty() {
                    title.into_owned()
                } else {
                    format!("{title}   {binding}")
                };
                item.set_text(text);
            }
        }
        *self.binding_cache.borrow_mut() = preferences.shortcuts.clone();
    }

    pub fn poll(&self, blocked: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Ok(id) = self.receiver.try_recv() {
            if let Some((_, action, _)) = self.items.iter().find(|(item, _, _)| item.id() == &id)
                && (!blocked || matches!(action, Action::Quit))
            {
                actions.push(*action);
            }
        }
        for (item, action, _) in &self.items {
            item.set_enabled(!blocked || matches!(action, Action::Quit));
        }
        actions
    }
}
