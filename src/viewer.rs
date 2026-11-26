//! File viewers for text, hexadecimal data, images, and PDFs.
//!
//! Open a selected file with [`load`], or choose a [`Mode`] with [`load_as`].
//! Match the returned [`Content`] to get decoded text, a raster, or a page
//! that can fetch more data. Large text files and hexadecimal views use
//! [`source::Source`] for byte-offset navigation.

pub mod source;

/// Maximum size in bytes for loading a whole file as editable text (16 MiB).
pub const EDIT_LIMIT: u64 = 16 * 1024 * 1024;
/// Maximum source size in bytes for image and PDF snapshots (512 MiB).
pub const MEDIA_LIMIT: u64 = 512 * 1024 * 1024;
/// Maximum uncompressed size in bytes of an archive member opened for paging (2 GiB).
pub const PAGE_CACHE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
/// Shared backing data for a PDF, retained across rendering and password prompts.
pub type PdfBytes = hayro::hayro_syntax::PdfData;

/// A row-major image with four straight-alpha RGBA channels per pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct RasterImage {
    /// Image dimensions as `[width, height]` in pixels.
    pub size: [usize; 2],
    /// Rows of red, green, blue, and alpha values, starting at the top left.
    pub pixels: Vec<[u8; 4]>,
}
impl RasterImage {
    /// Copies straight-alpha RGBA bytes into pixels.
    ///
    /// Supply four bytes per pixel in `size`. The constructor copies each
    /// complete group of four and leaves dimension validation to the caller.
    pub fn from_rgba_unmultiplied(size: [usize; 2], bytes: &[u8]) -> Self {
        Self {
            size,
            pixels: bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| [p[0], p[1], p[2], p[3]])
                .collect(),
        }
    }
    /// Converts premultiplied RGBA bytes into straight-alpha pixels.
    ///
    /// Supply four bytes per pixel as in [`Self::from_rgba_unmultiplied`].
    /// Partially transparent pixels have their color channels rescaled by alpha.
    pub fn from_rgba_premultiplied(size: [usize; 2], bytes: &[u8]) -> Self {
        let mut image = Self::from_rgba_unmultiplied(size, bytes);
        for pixel in &mut image.pixels {
            let a = u32::from(pixel[3]);
            if a > 0 && a < 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((u32::from(*channel) * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        image
    }
}
use crate::filesystem::Location;
use anyhow::{Context, Result, bail};
use resvg::{tiny_skia, usvg};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
    sync::{Arc, LazyLock},
};

/// The first view of a file, together with the data needed for further navigation.
pub enum Content {
    /// A PDF that requires a password before its first page can be displayed.
    LockedPdf {
        /// Original PDF data to retry with [`render_pdf_data`] or [`PdfCache`].
        bytes: PdfBytes,
    },
    /// An open PDF with its first page rendered.
    Pdf {
        /// Original PDF data shared with subsequent page renders.
        bytes: PdfBytes,
        /// Unrotated raster of the first page.
        image: crate::viewer::RasterImage,
        /// Total number of pages in the document.
        pages: usize,
        /// Searchable first-page text and its positions, or a text-extraction diagnostic.
        text: std::sync::Arc<crate::pdf_text::PageText>,
    },
    /// Plain UTF-8 text detected in automatic mode.
    Text(String),
    /// A complete text document with encoding and line-ending information.
    EncodedText(crate::text_document::Document),
    /// The first text page, with a source for reading subsequent pages.
    LargeText(crate::text_document::Page),
    /// A hexadecimal view of the first 64 KiB or less.
    Hex {
        /// Formatted byte offsets, hexadecimal bytes, and printable ASCII.
        text: String,
        /// Whether additional bytes follow the displayed portion.
        truncated: bool,
        /// Source for paging through the file when available.
        source: Option<std::sync::Arc<source::Source>>,
    },
    /// A decoded image or thumbnail.
    Image(crate::viewer::RasterImage),
}
/// How [`load_as`] should interpret the selected file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Recognizes image and PDF extensions, then tries text or hexadecimal for other files.
    #[default]
    Auto,
    /// Decodes text with encoding information, paging files larger than [`EDIT_LIMIT`].
    Text,
    /// Opens a paged hexadecimal view regardless of the file extension.
    Hex,
    /// Passes the file to the image decoder regardless of the file extension.
    Image,
    /// Opens the file as a PDF regardless of the file extension.
    Pdf,
}
/// Opens a file using [`Mode::Auto`]. See [`load_as`] for path conventions.
///
/// # Errors
///
/// Returns an error if the source cannot be read or the detected format cannot be opened.
pub fn load(location: &Location, path: &Path) -> Result<Content> {
    load_as(location, path, Mode::Auto)
}
/// Produces a thumbnail fitting within 800 × 800 pixels for recognized image extensions.
///
/// Other files use [`load`], including its paging behavior for large text.
///
/// # Errors
///
/// Returns an error if the file cannot be opened, exceeds the applicable
/// size limit, or fails to decode.
pub fn load_preview(location: &Location, path: &Path) -> Result<Content> {
    if !is_image_extension(path) {
        return load(location, path);
    }
    #[cfg(target_os = "macos")]
    if matches!(location, Location::Local(_))
        && !is_svg_extension(path)
        && let Some(image) = macos_thumbnail(path)
    {
        return Ok(Content::Image(image));
    }
    let snapshot = snapshot(
        location,
        path,
        MEDIA_LIMIT,
        &mut crate::operations::Task::default(),
    )?;
    let bytes: &[u8] = snapshot.as_ref().as_ref();
    Ok(Content::Image(decode_image(bytes, Some(800))?))
}
#[cfg(target_os = "macos")]
fn macos_thumbnail(path: &Path) -> Option<RasterImage> {
    // ImageIO (through sips) downsamples during decoding, avoiding a full-size
    // raster and the associated copies for large local images.
    if std::fs::metadata(path).ok()?.len() > MEDIA_LIMIT {
        return None;
    }
    let output = tempfile::Builder::new().suffix(".png").tempfile().ok()?;
    let status = std::process::Command::new("/usr/bin/sips")
        .args(["-Z", "800", "-s", "format", "png", "--out"])
        .arg(output.path())
        .arg(path)
        .output()
        .ok()?;
    if !status.status.success() {
        return None;
    }
    let bytes = std::fs::read(output.path()).ok()?;
    decode_image(&bytes, None).ok()
}
pub(crate) fn is_svg_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
}
pub(crate) fn is_image_extension(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        [
            "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico", "svg",
        ]
        .iter()
        .any(|format| extension.eq_ignore_ascii_case(format))
    })
}
fn decode_image(bytes: &[u8], preview_edge: Option<u32>) -> Result<RasterImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    if reader.format().is_none() {
        let (encoding, bytes) = encoding_rs::Encoding::for_bom(bytes)
            .map_or((encoding_rs::UTF_8, bytes), |(encoding, bom)| {
                (encoding, &bytes[bom..])
            });
        if let Some(text) = encoding.decode_without_bom_handling_and_without_replacement(bytes)
            && text.trim_start().starts_with('<')
        {
            return render_svg(&text, preview_edge);
        }
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode()?;
    let decoded = if let Some(edge) = preview_edge {
        decoded.thumbnail(edge, edge)
    } else {
        decoded
    };
    let image = decoded.to_rgba8();
    Ok(RasterImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}

fn render_svg(text: &str, preview_edge: Option<u32>) -> Result<RasterImage> {
    static FONTS: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut fonts = usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        Arc::new(fonts)
    });

    let select_font = usvg::FontResolver::default_font_selector();
    let options = usvg::Options {
        font_resolver: usvg::FontResolver {
            select_font: Box::new(move |font, database| {
                if database.is_empty() {
                    *database = FONTS.clone();
                }
                select_font(font, database)
            }),
            ..Default::default()
        },
        image_href_resolver: usvg::ImageHrefResolver {
            // Preview the snapshot without resolving references to other local files.
            resolve_string: Box::new(|_, _| None),
            ..Default::default()
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_str(text, &options).context("Cannot read SVG")?;
    let size = tree.size();
    let edge = size.width().max(size.height());
    // Rasterize even small icons at preview resolution for a sharp fit in the pane.
    // The full viewer uses the SVG's own dimensions, up to a bounded raster size.
    let target = preview_edge.map_or(edge.min(4096.0), |edge| edge as f32);
    let scale = target / edge;
    anyhow::ensure!(scale.is_finite(), "Invalid SVG scale");
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height).context("Invalid SVG raster size")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(RasterImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        pixmap.data(),
    ))
}

/// Opens the entry at `path` using the requested viewer mode.
///
/// Pass the path from [`Entry::path`](crate::filesystem::Entry::path). For a
/// local location this is the filesystem path. For a remote or archive
/// location it is relative to the connection or archive root. Images and
/// PDFs use snapshots up to [`MEDIA_LIMIT`]. A PDF that needs a password
/// yields [`Content::LockedPdf`].
///
/// # Errors
///
/// Returns an error for virtual discovery lists, unreadable sources, size
/// limit violations, or content the selected decoder cannot open.
///
/// # Examples
///
/// View the same file as text and as hexadecimal bytes.
///
/// ```
/// use rust_commander::filesystem::Location;
/// use rust_commander::viewer::{self, Content, Mode};
///
/// let dir = tempfile::tempdir()?;
/// let path = dir.path().join("notes.txt");
/// std::fs::write(&path, b"ABC")?;
/// let location = Location::Local(dir.path().to_owned());
///
/// assert!(matches!(viewer::load(&location, &path)?, Content::Text(s) if s == "ABC"));
/// let Content::Hex { text, truncated, .. } = viewer::load_as(&location, &path, Mode::Hex)? else {
///     panic!("expected a hexadecimal view");
/// };
/// assert!(text.starts_with("00000000  41 42 43"));
/// assert!(!truncated);
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn load_as(location: &Location, path: &Path, mode: Mode) -> Result<Content> {
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let image = mode == Mode::Image || mode == Mode::Auto && is_image_extension(path);
    let pdf = mode == Mode::Pdf || mode == Mode::Auto && extension == "pdf";
    let size = match location {
        Location::Network { .. } | Location::Containers => {
            bail!("Open a share or export to view files.")
        }
        Location::Local(_) => Some(std::fs::metadata(path)?.len()),
        Location::Remote { connection, .. } => crate::remote::viewer_stamp(connection, path)
            .ok()
            .map(|v| v.0),
        Location::Archive { file, .. } => {
            crate::archive::list(file, path.parent().unwrap_or(Path::new("")))
                .ok()
                .and_then(|entries| entries.into_iter().find(|entry| entry.path == path))
                .map(|entry| entry.size)
        }
    };
    if mode == Mode::Hex || (!image && !pdf && size.is_some_and(|size| size > EDIT_LIMIT)) {
        let source = source::Source::open(location.clone(), path.to_owned())?;
        if mode == Mode::Hex {
            return Ok(Content::Hex {
                text: source.hex(0)?,
                truncated: source.total > 65536,
                source: Some(source),
            });
        }
        let prefix = source.bytes(0, 3)?;
        let encoding = if prefix.starts_with(&[0xff, 0xfe]) {
            crate::text_document::Encoding::Utf16Le
        } else if prefix.starts_with(&[0xfe, 0xff]) {
            crate::text_document::Encoding::Utf16Be
        } else {
            crate::text_document::Encoding::Utf8
        };
        if let Ok(page) = source
            .page(0, encoding)
            .or_else(|_| source.page(0, crate::text_document::Encoding::Windows1252))
        {
            return Ok(Content::LargeText(page));
        }
        if mode == Mode::Text {
            bail!("Cannot decode this source as text; open it as hexadecimal instead.");
        }
        return Ok(Content::Hex {
            text: source.hex(0)?,
            truncated: source.total > 65536,
            source: Some(source),
        });
    }
    let snapshot = if image || pdf {
        Some(snapshot(
            location,
            path,
            MEDIA_LIMIT,
            &mut crate::operations::Task::default(),
        )?)
    } else {
        None
    };
    let bytes = if let Some(snapshot) = &snapshot {
        snapshot.as_ref().as_ref()
    } else {
        &[]
    };
    let text_bytes = if snapshot.is_none() {
        read(location, path, EDIT_LIMIT)?
    } else {
        Vec::new()
    };
    let bytes = if snapshot.is_none() {
        &text_bytes[..]
    } else {
        bytes
    };
    if pdf {
        let data: PdfBytes = snapshot.unwrap().into();
        if matches!(
            hayro::hayro_syntax::Pdf::new(data.clone()),
            Err(hayro::hayro_syntax::LoadPdfError::Decryption(_))
        ) {
            return Ok(Content::LockedPdf {
                bytes: data.clone(),
            });
        }
        let (image, pages, text) = PdfCache::default().render_data_with_text(&data, 0, "")?;
        return Ok(Content::Pdf {
            bytes: data.clone(),
            image,
            pages,
            text,
        });
    }
    if image {
        return Ok(Content::Image(decode_image(bytes, None)?));
    }
    if mode == Mode::Text {
        return Ok(Content::EncodedText(
            crate::text_document::Document::detect(bytes.to_vec()).or_else(|_| {
                crate::text_document::Document::decode(
                    bytes.to_vec(),
                    crate::text_document::Encoding::Windows1252,
                )
            })?,
        ));
    }
    if bytes.starts_with(&[0xff, 0xfe])
        || bytes.starts_with(&[0xfe, 0xff])
        || bytes.starts_with(&[0xef, 0xbb, 0xbf])
    {
        return Ok(Content::EncodedText(
            crate::text_document::Document::detect(bytes.to_vec())?,
        ));
    }
    if !bytes.contains(&0)
        && let Ok(text) = String::from_utf8(bytes.to_vec())
    {
        return Ok(Content::Text(text));
    }
    let source = source::Source::open(location.clone(), path.to_owned())?;
    Ok(Content::Hex {
        text: source.hex(0)?,
        truncated: source.total > 65536,
        source: Some(source),
    })
}

/// Renders the zero-based page at `index` using an empty password.
///
/// Returns the unrotated image and total page count.
///
/// # Errors
///
/// Returns an error if the PDF cannot be opened with an empty password or
/// the requested page is missing or has invalid dimensions.
pub fn render_pdf(bytes: &[u8], index: usize) -> Result<(crate::viewer::RasterImage, usize)> {
    render_pdf_password(bytes, index, "")
}
/// Renders one zero-based page from PDF bytes with the supplied password.
///
/// Returns the unrotated image and total page count. Use [`render_pdf_data`]
/// when the PDF is already available as shared [`PdfBytes`].
///
/// # Errors
///
/// Returns an error if the password or PDF is invalid, the page is missing,
/// or its dimensions are invalid.
pub fn render_pdf_password(
    bytes: &[u8],
    index: usize,
    password: &str,
) -> Result<(RasterImage, usize)> {
    render_pdf_data(&bytes.to_vec().into(), index, password)
}
/// Renders a zero-based page from shared PDF data and returns the image and total page count.
///
/// The raster uses a white background and targets 1,400 pixels on its longest
/// edge, with upscaling limited to twice the page's original dimensions.
///
/// # Errors
///
/// Returns an error if the PDF cannot be opened with `password` or the page
/// is missing or has invalid dimensions.
pub fn render_pdf_data(
    data: &PdfBytes,
    index: usize,
    password: &str,
) -> Result<(RasterImage, usize)> {
    let pdf = hayro::hayro_syntax::Pdf::new_with_password(data.clone(), password)
        .map_err(|e| anyhow::anyhow!("Cannot read PDF: {e:?}"))?;
    let page = pdf.pages().get(index).context("PDF page does not exist")?;
    let (width, height) = page.render_dimensions();
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        bail!("Invalid PDF page dimensions.");
    }
    let scale = (1400.0 / width.max(height)).min(2.0);
    let pixmap = hayro::render(
        page,
        &hayro::RenderCache::new(),
        &Default::default(),
        &Default::default(),
        &hayro::PixmapSettings {
            x_scale: scale,
            y_scale: scale,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    Ok((
        crate::viewer::RasterImage::from_rgba_premultiplied(
            [pixmap.width() as usize, pixmap.height() as usize],
            pixmap.data_as_u8_slice(),
        ),
        pdf.pages().len(),
    ))
}
/// Reads an entire local file, remote object, or archive member into memory.
///
/// Use the entry's path as for [`load_as`] and a byte limit below `u64::MAX`.
/// For a bounded portion of a larger file, use [`source::Source::bytes`].
///
/// # Errors
///
/// Returns an error if the file exceeds `limit`, the backend cannot read it,
/// or the location is a virtual discovery list.
pub fn read(location: &Location, path: &Path, limit: u64) -> Result<Vec<u8>> {
    match location {
        Location::Network { .. } | Location::Containers => {
            bail!("Open a share or export to view files.")
        }
        Location::Local(_) => {
            let mut bytes = Vec::new();
            File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > limit {
                bail!("This file exceeds the built-in viewer size limit.");
            }
            Ok(bytes)
        }
        Location::Remote { connection, .. } => crate::remote::read_bytes(connection, path, limit),
        Location::Archive { file, .. } => crate::archive::read_bytes(file, path, limit),
    }
    .context("Cannot load the selected file")
}
/// Formats all supplied bytes as a hexadecimal dump starting at offset zero.
pub fn hex_dump(bytes: &[u8]) -> String {
    hex_dump_at(bytes, 0)
}
/// Formats up to 16 bytes per row, labeling the first row with the byte `offset`.
///
/// Each row ends with printable ASCII, using a dot for nonprintable bytes.
pub fn hex_dump_at(bytes: &[u8], offset: u64) -> String {
    use std::fmt::Write;
    let mut output = String::new();
    for (index, row) in bytes.chunks(16).enumerate() {
        let _ = write!(output, "{:08x}  ", offset + (index * 16) as u64);
        for i in 0..16 {
            if let Some(byte) = row.get(i) {
                let _ = write!(output, "{byte:02x} ");
            } else {
                output.push_str("   ");
            }
        }
        output.push_str(" | ");
        for &byte in row {
            output.push(if byte.is_ascii_graphic() || byte == b' ' {
                byte as char
            } else {
                '.'
            });
        }
        output.push('\n');
    }
    output
}

/// In-memory cache of recently rendered pages and searchable text for one PDF.
///
/// Create a fresh cache for each document. Lookups use the page index and
/// changing the password clears cached pages. The cache holds up to three
/// pages with a combined raster budget of 24 MiB.
#[derive(Default)]
pub struct PdfCache {
    password: String,
    pages: std::collections::VecDeque<CachedPdfPage>,
}
struct CachedPdfPage {
    page: usize,
    image: RasterImage,
    count: usize,
    text: std::sync::Arc<crate::pdf_text::PageText>,
}
/// An unrotated page image, the document's page count, and the page's searchable text.
pub type PdfPage = (
    RasterImage,
    usize,
    std::sync::Arc<crate::pdf_text::PageText>,
);
impl PdfCache {
    /// Returns the unrotated raster and total page count for a zero-based page.
    ///
    /// # Errors
    ///
    /// Returns an error if [`render_pdf_data`] cannot render an uncached page.
    pub fn render(
        &mut self,
        bytes: &[u8],
        page: usize,
        password: &str,
    ) -> Result<(RasterImage, usize)> {
        self.render_with_text(bytes, page, password)
            .map(|(image, count, _)| (image, count))
    }
    /// Returns the raster, page count, and searchable text for a zero-based page.
    ///
    /// A text-extraction failure is recorded in [`PageText::error`](crate::pdf_text::PageText::error)
    /// so the raster can still be displayed.
    ///
    /// # Errors
    ///
    /// Returns an error if [`render_pdf_data`] cannot render an uncached page.
    pub fn render_with_text(
        &mut self,
        bytes: &[u8],
        page: usize,
        password: &str,
    ) -> Result<PdfPage> {
        self.render_data_with_text(&bytes.to_vec().into(), page, password)
    }
    /// Renders or retrieves a zero-based [`PdfPage`] using shared document data.
    ///
    /// Prefer this method when paging through a [`Content::Pdf`]. Text-extraction
    /// failures are reported alongside the rendered image as in [`Self::render_with_text`].
    ///
    /// # Errors
    ///
    /// Returns an error if [`render_pdf_data`] cannot render an uncached page.
    pub fn render_data_with_text(
        &mut self,
        data: &PdfBytes,
        page: usize,
        password: &str,
    ) -> Result<PdfPage> {
        if self.password != password {
            self.pages.clear();
            self.password = password.into();
        }
        if let Some(index) = self.pages.iter().position(|p| p.page == page) {
            let hit = self.pages.remove(index).unwrap();
            let result = (hit.image.clone(), hit.count, hit.text.clone());
            self.pages.push_back(hit);
            return Ok(result);
        }
        let (image, count) = render_pdf_data(data, page, password)?;
        let text = std::sync::Arc::new(
            crate::pdf_text::load_page_data(
                data,
                password,
                page,
                &crate::operations::Task::default(),
            )
            .unwrap_or_else(|e| crate::pdf_text::PageText {
                error: Some(e.to_string()),
                ..Default::default()
            }),
        );
        while self.pages.len() >= 3
            || self
                .pages
                .iter()
                .map(|p| p.image.pixels.len() * 4)
                .sum::<usize>()
                + image.pixels.len() * 4
                > 24 * 1024 * 1024
        {
            if self.pages.pop_front().is_none() {
                break;
            }
        }
        if image.pixels.len() * 4 <= 24 * 1024 * 1024 {
            self.pages.push_back(CachedPdfPage {
                page,
                image: image.clone(),
                count,
                text: text.clone(),
            });
        }
        Ok((image, count, text))
    }
}
/// Returns a copy of `image` rotated clockwise by `turns` quarter turns.
///
/// # Panics
///
/// A nontrivial rotation panics if the pixel buffer is shorter than the
/// declared width times height.
pub fn rotate(image: &RasterImage, turns: usize) -> RasterImage {
    let turns = turns % 4;
    let [w, h] = image.size;
    if turns == 0 {
        return image.clone();
    }
    let size = if turns % 2 == 1 { [h, w] } else { [w, h] };
    let mut pixels = vec![[0; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match turns {
                1 => (h - 1 - y, x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            pixels[ny * size[0] + nx] = image.pixels[y * w + x];
        }
    }
    RasterImage { size, pixels }
}

/// Streams a local file, remote object, or archive member into a private, immutable snapshot.
///
/// `limit` bounds the number of source bytes. The [`Task`](crate::operations::Task)
/// reports transfer progress and supports cancellation while copying.
///
/// # Errors
///
/// Returns an error if the source cannot be read, the copy exceeds `limit`,
/// the task is cancelled, or the temporary backing file cannot be created or mapped.
pub fn snapshot(
    location: &Location,
    path: &Path,
    limit: u64,
    task: &mut crate::operations::Task,
) -> Result<std::sync::Arc<crate::private_bytes::Bytes>> {
    if let Location::Archive { file, .. } = location {
        return crate::archive::snapshot_member(file, path, limit, task);
    }
    crate::private_bytes::Bytes::capture(
        &mut *crate::storage::reader(location, path)?,
        limit,
        path,
        task,
    )
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn preview_images_are_bounded_without_reducing_full_view_resolution() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.png");
        image::RgbaImage::from_pixel(1600, 1000, image::Rgba([20, 40, 60, 255]))
            .save(&path)
            .unwrap();
        let location = Location::Local(directory.path().to_path_buf());
        let Content::Image(preview) = load_preview(&location, &path).unwrap() else {
            panic!("Expected an image preview");
        };
        assert_eq!(preview.size, [800, 500]);
        let Content::Image(full) = load(&location, &path).unwrap() else {
            panic!("Expected an image viewer");
        };
        assert_eq!(full.size, [1600, 1000]);
    }
}
