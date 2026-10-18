use crate::ui_signal::UiSignal;
use crate::{
    filesystem::{Entry, Location},
    viewer::Content,
};
use std::{
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
#[derive(Default)]
pub struct Preview {
    pub(crate) target: Option<(Location, Entry)>,
    changed: Option<Instant>,
    pending: Option<Receiver<Result<Content, String>>>,
    pub(crate) text: String,
    pub(crate) image: Option<crate::viewer::RasterImage>,
    pub(crate) loading: bool,
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
            self.image = None;
            self.text.clear();
            self.changed = Some(Instant::now());
            self.loading = self.target.is_some();
        }
    }
    pub(crate) fn poll(&mut self, ctx: &UiSignal) {
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
                Ok(Content::EncodedText(document)) => self.text = document.text,
                Ok(Content::LargeText(page)) => self.text = page.document.text,
                Ok(Content::Image(i)) | Ok(Content::Pdf { image: i, .. }) => {
                    self.image = Some(i.clone());
                }
                Err(e) => self.text = e,
            }
        }
    }
}
