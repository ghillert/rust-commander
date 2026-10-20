//! Native file icons, queried on one dedicated thread. No file contents are
//! previewed, and no platform objects cross into the Slint event loop.
use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender, SyncSender},
    },
    time::SystemTime,
};

#[cfg(target_os = "linux")]
#[path = "native_icons/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "native_icons/macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "native_icons/windows.rs"]
mod platform;

// Enough detail for an 18 logical pixel icon even on a 3x display.
pub(crate) const ICON_SIZE: u32 = 64;
const QUEUE_LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub path: PathBuf,
    pub directory: bool,
    pub modified: Option<SystemTime>,
}

pub(crate) struct Request {
    pub key: Key,
    pub wanted: Arc<AtomicBool>,
    pub reply: Sender<Response>,
}

pub(crate) enum Response {
    Loaded(Key, Option<image::RgbaImage>),
    Cancelled(Key),
}

pub(crate) fn worker() -> Option<SyncSender<Request>> {
    static WORKER: OnceLock<Option<SyncSender<Request>>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Request>(QUEUE_LIMIT);
            std::thread::Builder::new()
                .name("native-file-icons".into())
                .spawn(move || {
                    let provider = platform::Provider::new();
                    for request in receiver {
                        let response = if request.wanted.load(Ordering::Relaxed) {
                            let icon = provider.as_ref().and_then(|p| p.load(&request.key));
                            Response::Loaded(request.key, icon.map(normalize))
                        } else {
                            Response::Cancelled(request.key)
                        };
                        let _ = request.reply.send(response);
                    }
                })
                .ok()?;
            Some(sender)
        })
        .clone()
}

fn normalize(icon: image::RgbaImage) -> image::RgbaImage {
    let mut canvas = image::RgbaImage::new(ICON_SIZE, ICON_SIZE);
    if icon.width() == 0 || icon.height() == 0 {
        return canvas;
    }
    let longest = u64::from(icon.width().max(icon.height()));
    let fit = |size| (u64::from(size) * u64::from(ICON_SIZE) / longest).max(1) as u32;
    let icon = image::imageops::thumbnail(&icon, fit(icon.width()), fit(icon.height()));
    image::imageops::replace(
        &mut canvas,
        &icon,
        i64::from((ICON_SIZE - icon.width()) / 2),
        i64::from((ICON_SIZE - icon.height()) / 2),
    );
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_normalization_preserves_transparency_and_aspect_ratio() {
        let source = image::RgbaImage::from_pixel(20, 10, image::Rgba([25, 120, 220, 180]));
        let icon = normalize(source);
        assert_eq!(icon.dimensions(), (ICON_SIZE, ICON_SIZE));
        assert_eq!(icon.get_pixel(0, 0)[3], 0);
        assert_eq!(*icon.get_pixel(32, 32), image::Rgba([25, 120, 220, 180]));
    }

    #[test]
    #[cfg_attr(target_os = "linux", ignore = "requires GTK display; run under Xvfb")]
    fn native_file_icons_load_on_the_worker() {
        let root = tempfile::tempdir().unwrap();
        let text = root.path().join("café notes.txt");
        std::fs::write(&text, "text").unwrap();
        let folder = root.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        let sender = worker().expect("native worker started");
        let (reply, receiver) = mpsc::channel();
        let mut icons = Vec::new();
        for (path, directory) in [(text, false), (folder, true)] {
            let key = Key {
                path,
                directory,
                modified: None,
            };
            sender
                .send(Request {
                    key: key.clone(),
                    wanted: Arc::new(AtomicBool::new(true)),
                    reply: reply.clone(),
                })
                .unwrap();
            let response = receiver
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap();
            let Response::Loaded(loaded, icon) = response else {
                panic!("request was unexpectedly cancelled");
            };
            assert_eq!(loaded, key);
            let icon = icon.expect("system returned a file icon");
            assert_eq!(icon.dimensions(), (ICON_SIZE, ICON_SIZE));
            assert!(icon.pixels().any(|p| p[3] > 0));
            icons.push(icon);
        }
        assert_ne!(
            icons[0], icons[1],
            "folder and document should have distinct icons"
        );
    }
}
