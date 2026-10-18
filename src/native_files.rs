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
        let mut contents = vec![ClipboardContent::Files(values)];
        contents.push(ClipboardContent::Other(FILE_MARKER.into(), marker.clone()));
        #[cfg(target_os = "windows")]
        contents.push(ClipboardContent::Other(
            "Preferred DropEffect".into(),
            if moving { 2u32 } else { 1u32 }.to_le_bytes().to_vec(),
        ));
        #[cfg(target_os = "linux")]
        {
            let operation = if moving { "cut" } else { "copy" };
            let uris = paths
                .iter()
                .filter_map(|path| url::Url::from_file_path(path).ok())
                .map(|url| url.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            contents.push(ClipboardContent::Other(
                "x-special/gnome-copied-files".into(),
                format!("{operation}\n{uris}").into_bytes(),
            ));
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
        // The eframe window remains alive for the entire Commander instance.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.0) })
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn drag_out(window: &Window, paths: Vec<PathBuf>) -> Result<()> {
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
        drag::DragItem::Files(paths),
        drag::Image::Raw(preview.into_inner()),
        |_, _| {},
        drag::Options::default(),
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
