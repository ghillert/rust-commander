//! Native preview action, separate from the document viewer's native chrome.
//! Native callbacks only record a click; the adapter validates and opens the file.
use super::*;
use std::cell::Cell;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows::Platform;

#[derive(Default)]
pub(super) struct Host {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    platform: Option<Platform>,
    target: Option<PathBuf>,
    clicked: Rc<Cell<bool>>,
}
impl Host {
    pub fn take_clicked(&self) -> Option<PathBuf> {
        self.clicked
            .replace(false)
            .then(|| self.target.clone())
            .flatten()
    }

    pub fn sync(&mut self, ui: &CommanderWindow, target: Option<PathBuf>, dark: bool) {
        let target = target.filter(|_| {
            ui.get_preview_visible()
                && ui.get_preview_can_open()
                && !ui.get_viewer_visible()
                && ui.get_dialog_title().is_empty()
                && !ui.get_native_form_open()
        });
        if self.target != target {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            {
                self.platform = None;
            }
            self.clicked = Rc::new(Cell::new(false));
            self.target = target;
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            if self.target.is_some() && self.platform.is_none() {
                self.platform = Platform::new(ui, self.clicked.clone());
            }
            ui.set_preview_native(self.platform.is_some());
            if let Some(platform) = &mut self.platform {
                platform.sync(ui, dark);
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = dark;
            ui.set_preview_native(false);
        }
    }

    pub fn focus(&self) {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(platform) = &self.platform {
            platform.focus();
        }
    }

    #[cfg(all(target_os = "macos", feature = "screenshot"))]
    pub fn snapshot(&self, ui: &CommanderWindow, path: &Path) -> bool {
        self.platform
            .as_ref()
            .is_some_and(|platform| platform.snapshot(ui, path))
    }
}

impl Adapter {
    pub(super) fn preview_native_events(&mut self) {
        if let Some(path) = self.native_preview.take_clicked()
            && !self.blocked()
            && !self.app.exiting
            && self.app.preferences.preview
        {
            if self.app.preview.external_video_path() == Some(path.as_path()) {
                self.app.remember_opened_file(path.clone());
                if let Err(error) = open::that_detached(path) {
                    self.app.fail(error);
                }
            } else if self.app.preview.image.is_some()
                && let Some((location, entry)) = self.app.preview.target.clone()
                && entry.path == path
                && crate::icons::file_icon(&entry.name, entry.is_dir) == "image"
            {
                self.app
                    .view_as(entry, false, location, crate::viewer::Mode::Auto, &self.ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_click_is_consumed_once_and_old_selection_callbacks_are_retired() {
        i_slint_backend_testing::init_no_event_loop();
        let ui = CommanderWindow::new().unwrap();
        ui.set_preview_visible(true);
        ui.set_preview_can_open(true);
        let mut host = Host::default();
        host.sync(&ui, Some("first.mp4".into()), false);
        let old_callback = host.clicked.clone();
        old_callback.set(true);
        assert_eq!(host.take_clicked(), Some("first.mp4".into()));
        assert!(host.take_clicked().is_none());
        host.sync(&ui, Some("second.mp4".into()), false);
        old_callback.set(true);
        assert!(host.take_clicked().is_none());
        host.clicked.set(true);
        ui.set_dialog_title("Modal dialog".into());
        host.sync(&ui, Some("second.mp4".into()), false);
        assert!(host.take_clicked().is_none());
        assert!(!ui.get_preview_native());
    }

    #[test]
    fn native_image_click_opens_the_preview_target() {
        i_slint_backend_testing::init_no_event_loop();
        let (_root, mut adapter, left, _) = crate::app::slint_ui::tests::fixture();
        let path = left.join("preview.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        let entry = filesystem::entry_from_path(&path).unwrap();
        adapter.app.preferences.preview = true;
        adapter
            .app
            .preview
            .select(Location::Local(left), Some(entry));
        adapter.app.preview.image = Some(crate::viewer::RasterImage {
            size: [2, 2],
            pixels: vec![[255, 0, 0, 255]; 4],
        });
        let ui = CommanderWindow::new().unwrap();
        adapter.sync(&ui);
        assert_eq!(adapter.native_preview.target, Some(path));
        adapter.native_preview.clicked.set(true);
        adapter.preview_native_events();
        assert!(adapter.app.pending_view.is_some());
    }
}
