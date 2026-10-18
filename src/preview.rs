use crate::{
    filesystem::{Entry, Location},
    viewer::Content,
};
use eframe::egui;
use std::{
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct Preview {
    target: Option<(Location, Entry)>,
    changed: Option<Instant>,
    pending: Option<Receiver<Result<Content, String>>>,
    text: String,
    texture: Option<egui::TextureHandle>,
    loading: bool,
}
impl Preview {
    pub fn select(&mut self, location: Location, entry: Option<Entry>) {
        let changed = match (&self.target, &entry) {
            (Some((l, e)), Some(n)) => {
                *l != location || e.path != n.path || e.modified != n.modified || e.size != n.size
            }
            (None, None) => false,
            _ => true,
        };
        if changed {
            self.target = entry.map(|e| (location, e));
            self.pending = None;
            self.texture = None;
            self.text.clear();
            self.changed = Some(Instant::now());
            self.loading = self.target.is_some();
        }
    }
    pub fn show(&mut self, ctx: &egui::Context, visible: &mut bool) {
        if !*visible {
            return;
        }
        if self.pending.is_none()
            && self.loading
            && self
                .changed
                .is_some_and(|t| t.elapsed() >= Duration::from_millis(250))
        {
            self.loading = false;
            if let Some((location, entry)) = &self.target {
                if entry.is_dir {
                    self.text = format!("{}\nFolder\n{}", entry.name, location.label());
                } else {
                    let (sender, receiver) = mpsc::channel();
                    self.pending = Some(receiver);
                    let location = location.clone();
                    let path = entry.path.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let result =
                            crate::viewer::load(&location, &path).map_err(|e| format!("{e:#}"));
                        let _ = sender.send(result);
                        ctx.request_repaint();
                    });
                }
            }
        }
        if let Some(result) = crate::background::poll(&mut self.pending) {
            match result {
                Ok(Content::Text(t)) | Ok(Content::Hex { text: t, .. }) => self.text = t,
                Ok(Content::Image(i)) | Ok(Content::Pdf { image: i, .. }) => {
                    self.texture = Some(ctx.load_texture(
                        "persistent_preview",
                        i,
                        egui::TextureOptions::LINEAR,
                    ))
                }
                Err(e) => self.text = e,
            }
        }
        egui::SidePanel::right("preview_pane")
            .resizable(true)
            .default_width(300.0)
            .width_range(220.0..=700.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(crate::i18n::text("Preview"));
                    if ui.small_button(crate::i18n::text("Close")).clicked() {
                        *visible = false;
                    }
                });
                ui.separator();
                if let Some((location, entry)) = &self.target {
                    ui.strong(&entry.name);
                    ui.label(crate::filesystem::human_size(entry.size));
                    #[cfg(target_os = "macos")]
                    if location.local().is_some()
                        && ui.button(crate::i18n::text("Quick Look")).clicked()
                    {
                        let _ =
                            crate::native_actions::quick_look(std::slice::from_ref(&entry.path));
                    }
                    #[cfg(not(target_os = "macos"))]
                    let _ = location;
                } else {
                    ui.label(crate::i18n::text("Select a file to preview"));
                }
                if self.loading || self.pending.is_some() {
                    ui.spinner();
                }
                egui::ScrollArea::both().show(ui, |ui| {
                    if let Some(texture) = &self.texture {
                        let size = texture.size_vec2();
                        ui.image((
                            texture.id(),
                            size * (ui.available_width() / size.x).min(1.0),
                        ));
                    } else {
                        ui.add(
                            egui::Label::new(egui::RichText::new(&self.text).monospace()).wrap(),
                        );
                    }
                });
            });
        if self.loading || self.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
