use super::{ICON_SIZE, IconKind, Key, NativeIcon};
use gtk4::{gdk, gdk_pixbuf::Pixbuf, gio, prelude::*};

pub(super) struct Provider {
    theme: gtk4::IconTheme,
}

impl Provider {
    pub fn new() -> Option<Self> {
        // This process's only GTK user is the icon worker. Native drag-and-drop
        // runs in a separate helper process. All GTK calls stay on this thread.
        gtk4::init().ok()?;
        Some(Self {
            theme: gtk4::IconTheme::for_display(&gdk::Display::default()?),
        })
    }

    pub fn load(&self, key: &Key) -> Option<NativeIcon> {
        let context = gtk4::glib::MainContext::default();
        // Process theme-change notifications without running a second UI loop.
        for _ in 0..16 {
            if !context.pending() {
                break;
            }
            context.iteration(false);
        }
        let file = gio::File::for_path(&key.path);
        let info = file
            .query_info(
                "standard::icon",
                gio::FileQueryInfoFlags::NONE,
                None::<&gio::Cancellable>,
            )
            .ok()?;
        let icon = info.icon()?;
        let paintable = self.theme.lookup_by_gicon(
            &icon,
            ICON_SIZE as i32,
            1,
            gtk4::TextDirection::Ltr,
            gtk4::IconLookupFlags::FORCE_REGULAR,
        );
        // GIO streams also handle icons embedded in GTK resources, whose URI
        // cannot be opened with std::fs or passed as a local image path.
        let resolved = paintable.file()?;
        let kind = if icon.is::<gio::FileIcon>() || key.directory {
            IconKind::Specific
        } else if [
            "text-x-generic",
            "application-octet-stream",
            "application-x-generic",
            "unknown",
        ]
        .iter()
        // Missing reference names resolve to the theme's missing-image artwork;
        // that is not evidence of a generic document icon.
        .filter(|name| self.theme.has_icon(name))
        .any(|name| {
            self.theme
                .lookup_icon(
                    name,
                    &[],
                    ICON_SIZE as i32,
                    1,
                    gtk4::TextDirection::Ltr,
                    gtk4::IconLookupFlags::FORCE_REGULAR,
                )
                .file()
                .is_some_and(|file| file.equal(&resolved))
        }) {
            IconKind::Generic
        } else {
            IconKind::Unknown
        };
        let stream = resolved.read(None::<&gio::Cancellable>).ok()?;
        let pixbuf = Pixbuf::from_stream_at_scale(
            &stream,
            ICON_SIZE as i32,
            ICON_SIZE as i32,
            true,
            None::<&gio::Cancellable>,
        )
        .ok()?;
        let (width, height) = (pixbuf.width() as u32, pixbuf.height() as u32);
        let channels = pixbuf.n_channels() as usize;
        if !(3..=4).contains(&channels) {
            return None;
        }
        let stride = pixbuf.rowstride() as usize;
        let bytes = pixbuf.read_pixel_bytes();
        Some(NativeIcon::new(
            image::RgbaImage::from_fn(width, height, |x, y| {
                let i = y as usize * stride + x as usize * channels;
                image::Rgba([
                    bytes[i],
                    bytes[i + 1],
                    bytes[i + 2],
                    if channels == 4 { bytes[i + 3] } else { 255 },
                ])
            }),
            kind,
        ))
    }
}
