//! Selection-driven file previews for a pane. [`Preview::select`] tracks the
//! focused entry so the UI can load its preview after the selection settles.

use crate::ui_signal::UiSignal;
use crate::{
    filesystem::{Entry, Location},
    viewer::Content,
};
use std::{
    path::Path,
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
/// State of a pane's preview for the focused entry.
///
/// [`Self::select`] updates the target. The desktop event loop waits for selection
/// changes to settle before loading it.
#[derive(Default)]
pub struct Preview {
    pub(crate) presentation: crate::source_view::State,
    pub(crate) source_text: bool,
    pub(crate) complete: bool,
    pub(crate) target: Option<(Location, Entry)>,
    changed: Option<Instant>,
    pending: Option<Receiver<Result<Content, String>>>,
    pub(crate) text: String,
    pub(crate) image: Option<crate::viewer::RasterImage>,
    pub(crate) loading: bool,
}
impl Preview {
    pub(crate) fn is_video(&self) -> bool {
        self.target.as_ref().is_some_and(|(_, entry)| {
            !entry.is_dir && crate::icons::file_icon(&entry.name, false) == "video"
        })
    }

    /// Launch the displayed file, even if focus or marks have since changed.
    pub(crate) fn external_video_path(&self) -> Option<&Path> {
        self.target.as_ref().and_then(|(location, entry)| {
            (self.is_video() && location.local().is_some()).then_some(entry.path.as_path())
        })
    }

    pub(crate) fn image_target(&self) -> Option<&(Location, Entry)> {
        self.image.as_ref()?;
        self.target
            .as_ref()
            .filter(|(_, entry)| !entry.is_dir && crate::viewer::is_image_extension(&entry.path))
    }

    /// Updates the preview target when the location, path, size, or modification time
    /// changes. Clears the previous preview and queues the new one. Video previews use
    /// listing metadata immediately.
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
            self.presentation = Default::default();
            self.source_text = false;
            self.complete = false;
            self.image = None;
            self.text.clear();
            self.changed = Some(Instant::now());
            self.loading = self.target.is_some();
            if self.is_video() {
                // Listing metadata is enough. Never read, download or extract a
                // video just because the cursor moved over it.
                self.loading = false;
                let (location, entry) = self.target.as_ref().unwrap();
                self.text = video_metadata(location, entry);
            }
        }
    }
    pub(crate) fn poll(&mut self, ctx: &UiSignal) {
        if self.loading
            && let Some(changed) = self.changed
        {
            ctx.request_repaint_after(Duration::from_millis(250).saturating_sub(changed.elapsed()));
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
                        let result = crate::viewer::load_preview(&location, &path)
                            .map_err(|e| format!("{e:#}"));
                        let _ = sender.send(result);
                        ctx.request_repaint();
                    });
                }
            }
        }
        if let Some(result) = crate::background::poll(&mut self.pending) {
            match result {
                Ok(Content::Text(t)) => {
                    self.text = t;
                    self.source_text = true;
                    self.complete = true;
                }
                Ok(Content::Hex { text: t, .. }) => self.text = t,
                Ok(Content::EncodedText(document)) => {
                    self.text = document.text;
                    self.source_text = true;
                    self.complete = true;
                }
                Ok(Content::LargeText(page)) => {
                    self.complete = page.offset == 0 && page.next == page.total;
                    self.text = page.document.text;
                    self.source_text = true;
                }
                Ok(Content::Image(i)) | Ok(Content::Pdf { image: i, .. }) => {
                    self.image = Some(i);
                }
                Ok(Content::LockedPdf { .. }) => {
                    self.text = "Password-protected PDF — open the viewer to unlock it.".into()
                }
                Err(e) => self.text = e,
            }
        }
    }
}

fn video_metadata(location: &Location, entry: &Entry) -> String {
    use crate::{i18n::text as tr, preferences::Column};
    let local = location.local().is_some();
    format!(
        "{}\n\n{}: {}\n{}: {}\n{}: {}\n\n{}",
        tr("Video file"),
        tr("Extension"),
        Column::Extension.value(entry, local).to_uppercase(),
        tr("Size"),
        Column::Size.value(entry, local),
        tr("Modified"),
        Column::Modified.value(entry, local),
        tr(if local {
            "Open this video in your media player."
        } else {
            "Copy or extract this video to a local folder to play it."
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> Entry {
        Entry {
            path: Path::new("/not-read-by-preview").join(name),
            name: name.into(),
            is_dir: false,
            is_symlink: false,
            size: 80 * 1024 * 1024,
            size_known: true,
            modified: Some(std::time::UNIX_EPOCH),
        }
    }

    #[test]
    fn video_previews_use_listing_metadata_without_opening_files() {
        let mut preview = Preview::default();
        for name in [
            "clip.MP4",
            "clip.mov",
            "clip.mkv",
            "clip.webm",
            "clip.avi",
            "clip.m4v",
        ] {
            let entry = entry(name);
            let expected = entry.path.clone();
            preview.select(Location::Local("/not-read-by-preview".into()), Some(entry));
            preview.poll(&UiSignal::default());
            assert!(preview.is_video());
            assert!(preview.text.contains("80.0 MiB"));
            assert_eq!(preview.external_video_path(), Some(expected.as_path()));
            assert!(!preview.source_text);
            assert!(!preview.loading);
            assert!(preview.pending.is_none());
            assert!(preview.image.is_none());
        }
    }

    #[test]
    fn virtual_videos_never_download_or_extract_and_preserve_unknown_metadata() {
        for location in [
            Location::Remote {
                connection: "not-connected".into(),
                path: "videos".into(),
            },
            Location::Archive {
                file: "/missing.zip".into(),
                prefix: "videos".into(),
            },
        ] {
            let mut preview = Preview::default();
            let mut entry = entry("clip.mp4");
            entry.size_known = false;
            entry.modified = None;
            preview.select(location, Some(entry));
            preview.poll(&UiSignal::default());
            assert!(preview.is_video());
            assert_eq!(preview.text.matches('—').count(), 2);
            assert!(!preview.text.contains("80.0 MiB"));
            assert!(preview.external_video_path().is_none());
            assert!(preview.pending.is_none());
            assert!(!preview.loading);
        }
    }

    #[test]
    fn selecting_video_discards_previous_content_and_late_worker_results() {
        let mut preview = Preview::default();
        let location = Location::Local("/not-read-by-preview".into());
        preview.select(location.clone(), Some(entry("previous.txt")));
        let (sender, receiver) = mpsc::channel();
        preview.pending = Some(receiver);
        preview.text = "Old file contents".into();
        preview.source_text = true;
        preview.image = Some(crate::viewer::RasterImage {
            size: [1, 1],
            pixels: vec![[0; 4]],
        });
        sender
            .send(Ok(Content::Text("Late result".into())))
            .unwrap();
        preview.select(location.clone(), Some(entry("clip.mp4")));
        preview.poll(&UiSignal::default());
        assert!(!preview.text.contains("Late result"));
        assert!(!preview.text.contains("Old file contents"));
        assert!(preview.image.is_none());
        assert!(!preview.source_text);
        let mut folder = entry("folder.mp4");
        folder.is_dir = true;
        preview.select(location.clone(), Some(folder));
        assert!(!preview.is_video());
        assert!(preview.external_video_path().is_none());
        preview.select(location, None);
        assert!(preview.text.is_empty());
        assert!(!preview.loading);
    }
}
