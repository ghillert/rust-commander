#[cfg(not(target_os = "linux"))]
use anyhow::Context;
use anyhow::{Result, bail};
use clipboard_rs::{Clipboard, ClipboardContent, ClipboardContext};
use rand::RngCore;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const FILE_MARKER: &str = "io.github.ghillert.rust-commander.files";
#[cfg(not(target_os = "macos"))]
const FILE_MARKER: &str = "application/x-rust-commander-files";

#[derive(Default)]
pub struct NativeFiles {
    context: Option<ClipboardContext>,
    copied: Vec<PathBuf>,
    moving: bool,
    marker: Vec<u8>,
}
impl NativeFiles {
    fn context(&mut self) -> Result<&ClipboardContext> {
        if self.context.is_none() {
            self.context = Some(
                ClipboardContext::new()
                    .map_err(|e| anyhow::anyhow!("System clipboard unavailable: {e}"))?,
            );
        }
        Ok(self.context.as_ref().unwrap())
    }
    pub(crate) fn copy_text(&mut self, text: String) -> Result<()> {
        self.context()?
            .set_text(text)
            .map_err(|e| anyhow::anyhow!("Clipboard unavailable: {e}"))
    }
    pub(crate) fn text(&mut self) -> Result<String> {
        self.context()?
            .get_text()
            .map_err(|e| anyhow::anyhow!("Clipboard unavailable: {e}"))
    }
    pub fn copy(&mut self, paths: &[PathBuf], moving: bool) -> Result<()> {
        if paths.is_empty() {
            bail!("Select local files to copy.");
        }
        for path in paths {
            if !crate::operations::exists(path) {
                bail!("File no longer exists: {}", path.display());
            }
        }
        #[cfg(target_os = "linux")]
        let values = paths
            .iter()
            .map(|path| {
                url::Url::from_file_path(path)
                    .map(|url| url.to_string())
                    .map_err(|_| anyhow::anyhow!("Invalid local file path"))
            })
            .collect::<Result<Vec<_>>>()?;
        #[cfg(not(target_os = "linux"))]
        let values = paths
            .iter()
            .map(|path| {
                path.to_str()
                    .map(str::to_owned)
                    .context("Clipboard paths must be UTF-8")
            })
            .collect::<Result<Vec<_>>>()?;
        let mut marker = vec![0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut marker);
        #[cfg(not(target_os = "linux"))]
        let mut contents = vec![ClipboardContent::Files(values)];
        // On X11, Files also inserts GNOME/Nautilus formats with a hard-coded
        // "copy" operation. Publishing those alongside "cut" creates duplicate
        // targets whose first value wins. Supply each Linux format exactly once.
        #[cfg(target_os = "linux")]
        let mut contents = vec![
            ClipboardContent::Other("text/uri-list".into(), values.join("\r\n").into_bytes()),
            ClipboardContent::Text(
                paths
                    .iter()
                    .map(|path| path.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        ];
        contents.push(ClipboardContent::Other(FILE_MARKER.into(), marker.clone()));
        #[cfg(target_os = "windows")]
        contents.push(ClipboardContent::Other(
            "Preferred DropEffect".into(),
            if moving { 2u32 } else { 1u32 }.to_le_bytes().to_vec(),
        ));
        #[cfg(target_os = "linux")]
        {
            let operation = if moving { "cut" } else { "copy" };
            let data = format!("{operation}\n{}", values.join("\n")).into_bytes();
            for format in [
                "x-special/gnome-copied-files",
                "x-special/nautilus-clipboard",
            ] {
                contents.push(ClipboardContent::Other(format.into(), data.clone()));
            }
        }
        self.context()?
            .set(contents)
            .map_err(|error| anyhow::anyhow!("Could not copy files: {error}"))?;
        self.marker = marker;
        self.copied = paths.into();
        self.moving = moving;
        Ok(())
    }
    pub fn paste(&mut self) -> Result<(Vec<PathBuf>, bool)> {
        let values = self
            .context()?
            .get_files()
            .map_err(|e| anyhow::anyhow!("No files available on the system clipboard: {e}"))?;
        let mut paths = decode_paths(&values)?;
        let marker = self.context()?.get_buffer(FILE_MARKER).ok();
        let owned = !self.marker.is_empty() && marker.as_deref() == Some(self.marker.as_slice());
        let same_selection = paths.len() == self.copied.len()
            && paths
                .iter()
                .zip(&self.copied)
                .all(|(a, b)| a == b || same_file::is_same_file(a, b).unwrap_or(false));
        if owned && same_selection {
            paths.clone_from(&self.copied);
        }
        let moving = self.moving && owned && same_selection;
        #[cfg(target_os = "windows")]
        let moving = self
            .context()?
            .get_buffer("Preferred DropEffect")
            .map(|effect| effect.get(..4) == Some(2u32.to_le_bytes().as_slice()))
            .unwrap_or(moving);
        #[cfg(target_os = "linux")]
        let moving = self
            .context()?
            .get_buffer("x-special/gnome-copied-files")
            .map(|value| value.starts_with(b"cut\n"))
            .unwrap_or(moving);
        Ok((paths, moving))
    }
    pub fn owns_selection(&mut self) -> bool {
        let marker = self
            .context()
            .ok()
            .and_then(|context| context.get_buffer(FILE_MARKER).ok());
        !self.marker.is_empty() && marker.as_deref() == Some(self.marker.as_slice())
    }
    pub fn mark_remote(&mut self) -> Result<()> {
        let mut marker = vec![0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut marker);
        self.context()?
            .set(vec![
                ClipboardContent::Text("Remote files selected in Rust Commander".into()),
                ClipboardContent::Other(FILE_MARKER.into(), marker.clone()),
            ])
            .map_err(|error| anyhow::anyhow!("Could not copy remote selection: {error}"))?;
        self.marker = marker;
        self.copied.clear();
        self.moving = false;
        Ok(())
    }
    pub fn consume_cut(&mut self) {
        self.moving = false;
    }
}

pub fn decode_paths(values: &[String]) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    for value in values {
        for line in value
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let path = if line.starts_with("file:") {
                let url = url::Url::parse(line)?;
                if url
                    .host_str()
                    .is_some_and(|host| !host.is_empty() && host != "localhost")
                {
                    bail!("Clipboard URL is not a local file");
                }
                url.to_file_path()
                    .map_err(|_| anyhow::anyhow!("Clipboard URL is not a local file"))?
            } else {
                PathBuf::from(line)
            };
            if !path.is_absolute() {
                bail!("Clipboard contains a relative path.");
            }
            if !result.contains(&path) {
                result.push(path);
            }
        }
    }
    if result.is_empty() {
        bail!("No local files on the clipboard.");
    }
    Ok(result)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub struct Window(pub raw_window_handle::RawWindowHandle);
#[cfg(any(target_os = "macos", target_os = "windows"))]
impl raw_window_handle::HasWindowHandle for Window {
    fn window_handle(
        &self,
    ) -> std::result::Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError>
    {
        // The Slint window remains alive for the entire Commander instance.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.0) })
    }
}

/// Keep the real AppKit controls centered in the Slint header. AppKit can reset
/// its title-bar layout after resizing or leaving fullscreen, so the UI tick
/// checks the frames and only changes them when they differ.
#[cfg(target_os = "macos")]
pub(crate) fn align_window_controls(
    handle: raw_window_handle::RawWindowHandle,
    header_height: f64,
) {
    use objc2_app_kit::{NSView, NSWindowButton, NSWindowStyleMask};

    let raw_window_handle::RawWindowHandle::AppKit(handle) = handle else {
        return;
    };
    // Called on the UI thread while winit owns and keeps this NSView alive.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else {
        return;
    };
    if window.styleMask().contains(NSWindowStyleMask::FullScreen)
        || window.isMiniaturized()
        || !window.isVisible()
    {
        return;
    }
    let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) else {
        return;
    };
    // These AppKit-owned views remain retained for this main-thread callback.
    let Some(title_bar) = (unsafe { close.superview() }) else {
        return;
    };
    let Some(container) = (unsafe { title_bar.superview() }) else {
        return;
    };
    let Some(container_parent) = (unsafe { container.superview() }) else {
        return;
    };
    let bounds = view.bounds();
    let mut area = view.convertRect_fromView(container.bounds(), Some(&container));
    area.size.height = header_height;
    area.origin.y = bounds.origin.y
        + if view.isFlipped() {
            0.0
        } else {
            bounds.size.height - header_height
        };
    let frame = container_parent.convertRect_fromView(area, Some(view));
    // Expand the native hit-test area too; moving just the buttons can clip
    // them or leave the lower part of a button unclickable.
    if container.frame() != frame {
        container.setFrame(frame);
    }
    let center_y = bounds.origin.y
        + if view.isFlipped() {
            header_height / 2.0
        } else {
            bounds.size.height - header_height / 2.0
        };
    for kind in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        let Some(button) = window.standardWindowButton(kind) else {
            continue;
        };
        let Some(parent) = (unsafe { button.superview() }) else {
            continue;
        };
        let frame = button.frame();
        let mut center = frame.origin;
        center.x += frame.size.width / 2.0;
        center.y += frame.size.height / 2.0;
        let mut target = view.convertPoint_fromView(center, Some(&parent));
        target.y = center_y;
        let target = parent.convertPoint_fromView(target, Some(view));
        let mut origin = frame.origin;
        origin.y = target.y - frame.size.height / 2.0;
        if (origin.y - frame.origin.y).abs() > 0.25 {
            button.setFrameOrigin(origin);
        }
    }
}

/// Read the pointer in client coordinates at a native file drop. winit 0.30
/// does not emit CursorMoved while an external native drag is over the window.
#[cfg(target_os = "macos")]
pub(crate) fn drop_position(
    handle: raw_window_handle::RawWindowHandle,
    _scale: f64,
) -> Option<(f32, f32)> {
    let raw_window_handle::RawWindowHandle::AppKit(handle) = handle else {
        return None;
    };
    // Called on the UI thread while winit owns and keeps this NSView alive.
    let view = unsafe { handle.ns_view.cast::<objc2_app_kit::NSView>().as_ref() };
    let window = view.window()?;
    let point = view.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None);
    let y = if view.isFlipped() {
        point.y
    } else {
        view.bounds().size.height - point.y
    };
    Some((point.x as f32, y as f32))
}

#[cfg(target_os = "windows")]
pub(crate) fn drop_position(
    handle: raw_window_handle::RawWindowHandle,
    scale: f64,
) -> Option<(f32, f32)> {
    use windows_sys::Win32::{
        Foundation::POINT, Graphics::Gdi::ScreenToClient, UI::WindowsAndMessaging::GetCursorPos,
    };
    let raw_window_handle::RawWindowHandle::Win32(handle) = handle else {
        return None;
    };
    let mut point = POINT { x: 0, y: 0 };
    // Both APIs fill a stack-owned POINT; HWND is valid for this UI callback.
    if unsafe { GetCursorPos(&mut point) } == 0
        || unsafe { ScreenToClient(handle.hwnd.get() as _, &mut point) } == 0
    {
        return None;
    }
    Some((
        (f64::from(point.x) / scale) as f32,
        (f64::from(point.y) / scale) as f32,
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn drop_position(
    handle: raw_window_handle::RawWindowHandle,
    scale: f64,
) -> Option<(f32, f32)> {
    use x11rb::protocol::xproto::ConnectionExt;
    let window = match handle {
        raw_window_handle::RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok()?,
        raw_window_handle::RawWindowHandle::Xcb(handle) => handle.window.get(),
        _ => return None,
    };
    let (connection, _) = x11rb::connect(None).ok()?;
    let point = connection.query_pointer(window).ok()?.reply().ok()?;
    if !point.same_screen {
        return None;
    }
    Some((
        (f64::from(point.win_x) / scale) as f32,
        (f64::from(point.win_y) / scale) as f32,
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(crate) fn drop_position(
    _handle: raw_window_handle::RawWindowHandle,
    _scale: f64,
) -> Option<(f32, f32)> {
    None
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn drag_out(window: &Window, paths: Vec<PathBuf>) -> Result<()> {
    drag_out_with(window, paths, false, || {})
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn drag_out_with(
    window: &Window,
    paths: Vec<PathBuf>,
    moving: bool,
    finished: impl Fn() + Send + 'static,
) -> Result<()> {
    drag_item(window, drag::DragItem::Files(paths), moving, move |_, _| {
        finished()
    })
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn drag_promised(
    window: &Window,
    promises: drag::PromisedFiles,
    finished: impl Fn(bool) + Send + 'static,
) -> Result<()> {
    drag_item(
        window,
        drag::DragItem::Promised(promises),
        false,
        move |result, _| finished(matches!(result, drag::DragResult::Dropped)),
    )
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn drag_item(
    window: &Window,
    item: drag::DragItem,
    moving: bool,
    finished: impl Fn(drag::DragResult, drag::CursorPosition) + Send + 'static,
) -> Result<()> {
    let mut preview = std::io::Cursor::new(Vec::new());
    let image = image::RgbaImage::from_fn(32, 32, |x, y| {
        if x > 2 && x < 29 && y > 7 && y < 27 {
            image::Rgba([61, 142, 231, 255])
        } else {
            image::Rgba([0, 0, 0, 0])
        }
    });
    image::DynamicImage::ImageRgba8(image).write_to(&mut preview, image::ImageFormat::Png)?;
    drag::start_drag(
        window,
        item,
        drag::Image::Raw(preview.into_inner()),
        finished,
        drag::Options {
            mode: if moving {
                drag::DragMode::CopyMove
            } else {
                drag::DragMode::Copy
            },
            ..Default::default()
        },
    )
    .map_err(|e| anyhow::anyhow!("Native drag failed: {e}"))?;
    Ok(())
}

pub fn mounted_drives() -> Vec<(String, PathBuf)> {
    let mut result = Vec::new();
    #[cfg(target_os = "macos")]
    if let Ok(entries) = std::fs::read_dir("/Volumes") {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                result.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                ));
            }
        }
    }
    #[cfg(target_os = "windows")]
    for letter in b'A'..=b'Z' {
        let path = PathBuf::from(format!("{}:\\", letter as char));
        if path.is_dir() {
            result.push((format!("Drive {}", letter as char), path));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/proc/self/mountinfo") {
            for line in contents.lines() {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                if let Some(mount) = fields.get(4) {
                    let mount = mount
                        .replace("\\040", " ")
                        .replace("\\011", "\t")
                        .replace("\\134", "\\");
                    let path = PathBuf::from(mount);
                    if ["/media", "/mnt", "/run/media"]
                        .iter()
                        .any(|root| path.starts_with(root))
                        && path.is_dir()
                    {
                        let label = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        if !result.iter().any(|(_, existing)| existing == &path) {
                            result.push((label, path));
                        }
                    }
                }
            }
        }
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    result
}

pub fn breadcrumbs(path: &Path) -> Vec<(String, PathBuf)> {
    let mut current = PathBuf::new();
    let mut result = Vec::new();
    for component in path.components() {
        current.push(component.as_os_str());
        let label = component.as_os_str().to_string_lossy().into_owned();
        result.push((label, current.clone()));
    }
    result
}

/// Current physical button/modifier state, including while the OS owns a drag.
#[cfg(target_os = "macos")]
pub(crate) fn drag_buttons() -> (bool, bool) {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags as Flags};
    let flags = NSEvent::modifierFlags_class();
    let moving = (flags.contains(Flags::Shift) || flags.contains(Flags::Command))
        && !flags.intersects(Flags::Option | Flags::Control);
    (NSEvent::pressedMouseButtons() & 1 != 0, moving)
}
#[cfg(target_os = "windows")]
pub(crate) fn drag_buttons() -> (bool, bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LBUTTON, VK_MENU, VK_SHIFT,
    };
    let down = |key| unsafe { GetAsyncKeyState(i32::from(key)) } < 0;
    (
        down(VK_LBUTTON),
        down(VK_SHIFT) && !down(VK_CONTROL) && !down(VK_MENU),
    )
}
