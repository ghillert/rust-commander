//! AppKit menus. Constructed and updated only on eframe's main UI thread.
use crate::app::{Action, action_menus};
use eframe::egui;
use muda::{
    Menu, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use std::sync::mpsc::{self, Receiver};

pub struct NativeMenu {
    _menu: Menu,
    items: Vec<(MenuItem, Action)>,
    receiver: Receiver<muda::MenuId>,
}
impl NativeMenu {
    pub fn new(ctx: &egui::Context) -> muda::Result<Self> {
        let menu = Menu::new();
        let mut items = Vec::new();
        let app = Submenu::new("muCommander Rust", true);
        app.append(&PredefinedMenuItem::about(
            None,
            Some(muda::AboutMetadata {
                name: Some("muCommander Rust".into()),
                version: Some(env!("CARGO_PKG_VERSION").into()),
                ..Default::default()
            }),
        ))?;
        app.append(&PredefinedMenuItem::separator())?;
        for (label, action, key) in [
            ("Settings…", Action::Preferences, Code::Comma),
            ("Quit muCommander Rust", Action::Quit, Code::KeyQ),
        ] {
            let item = MenuItem::with_id(
                format!("{action:?}"),
                label,
                true,
                Some(Accelerator::new(Modifiers::META, key)),
            );
            app.append(&item)?;
            items.push((item, action));
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
            "Window",
            true,
            &[
                &PredefinedMenuItem::minimize(None),
                &PredefinedMenuItem::maximize(None),
                &PredefinedMenuItem::fullscreen(None),
            ],
        )?;
        for (title, entries) in action_menus().into_iter().chain([
            (
                "View",
                vec![
                    ("Show Hidden Files", Action::ToggleHidden),
                    ("Appearance…", Action::Preferences),
                ],
            ),
            ("Help", vec![("muCommander Help", Action::Help)]),
        ]) {
            let submenu = Submenu::new(title, true);
            for (label, action) in entries {
                let item = MenuItem::with_id(
                    format!("{title}-{action:?}"),
                    label.split("  F").next().unwrap_or(label),
                    true,
                    None,
                );
                submenu.append(&item)?;
                items.push((item, action));
            }
            if title == "Help" {
                menu.append(&window)?;
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
            receiver,
        })
    }

    pub fn poll(&self, blocked: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Ok(id) = self.receiver.try_recv() {
            if let Some((_, action)) = self.items.iter().find(|(item, _)| item.id() == &id)
                && (!blocked || matches!(action, Action::Quit))
            {
                actions.push(*action);
            }
        }
        for (item, action) in &self.items {
            item.set_enabled(!blocked || matches!(action, Action::Quit));
        }
        actions
    }
}
