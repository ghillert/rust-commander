#[derive(Clone, Debug, PartialEq)]
pub struct RasterImage {
    pub size: [usize; 2],
    pub pixels: Vec<[u8; 4]>,
}
impl RasterImage {
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
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

pub enum Content {
    LockedPdf {
        bytes: std::sync::Arc<Vec<u8>>,
    },
    Pdf {
        bytes: std::sync::Arc<Vec<u8>>,
        image: crate::viewer::RasterImage,
        pages: usize,
        text: std::sync::Arc<crate::pdf_text::PageText>,
    },
    Text(String),
    EncodedText(crate::text_document::Document),
    LargeText(crate::text_document::Page),
    Hex {
        text: String,
        truncated: bool,
    },
    Image(crate::viewer::RasterImage),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Auto,
    Text,
    Hex,
    Image,
    Pdf,
}
pub fn load(location: &Location, path: &Path) -> Result<Content> {
    load_as(location, path, Mode::Auto)
}
pub fn load_as(location: &Location, path: &Path, mode: Mode) -> Result<Content> {
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let image = mode == Mode::Image
        || mode == Mode::Auto
            && [
                "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico",
            ]
            .contains(&extension.as_str());
    let pdf = mode == Mode::Pdf || mode == Mode::Auto && extension == "pdf";
    if mode == Mode::Hex && location.local().is_some() {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(512 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        return Ok(Content::Hex {
            text: hex_dump(&bytes[..bytes.len().min(512 * 1024)]),
            truncated: bytes.len() > 512 * 1024,
        });
    }
    if !image
        && !pdf
        && mode != Mode::Hex
        && location.local().is_some()
        && std::fs::metadata(path)?.len() > 2 * 1024 * 1024
    {
        use std::io::Read;
        let mut prefix = [0; 3];
        let n = std::fs::File::open(path)?.read(&mut prefix)?;
        let encoding = if n >= 2 && prefix[..2] == [0xff, 0xfe] {
            crate::text_document::Encoding::Utf16Le
        } else if n >= 2 && prefix[..2] == [0xfe, 0xff] {
            crate::text_document::Encoding::Utf16Be
        } else {
            crate::text_document::Encoding::Utf8
        };
        if let Ok(page) = crate::text_document::Page::read(path, 0, encoding).or_else(|_| {
            crate::text_document::Page::read(path, 0, crate::text_document::Encoding::Windows1252)
        }) {
            return Ok(Content::LargeText(page));
        }
        if mode == Mode::Text {
            bail!("Cannot decode this file as text; open it as hexadecimal instead.");
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(512 * 1024)
            .read_to_end(&mut bytes)?;
        return Ok(Content::Hex {
            text: hex_dump(&bytes),
            truncated: true,
        });
    }
    let limit = if image || pdf || mode == Mode::Hex {
        32 * 1024 * 1024
    } else {
        2 * 1024 * 1024
    };
    let bytes = read(location, path, limit)?;
    if mode == Mode::Hex {
        return Ok(Content::Hex {
            text: hex_dump(&bytes[..bytes.len().min(512 * 1024)]),
            truncated: bytes.len() > 512 * 1024,
        });
    }
    if pdf {
        if matches!(
            hayro::hayro_syntax::Pdf::new(bytes.clone()),
            Err(hayro::hayro_syntax::LoadPdfError::Decryption(_))
        ) {
            return Ok(Content::LockedPdf {
                bytes: std::sync::Arc::new(bytes),
            });
        }
        let (image, pages, text) = PdfCache::default().render_with_text(&bytes, 0, "")?;
        return Ok(Content::Pdf {
            bytes: std::sync::Arc::new(bytes),
            image,
            pages,
            text,
        });
    }
    if image {
        let mut reader = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode()?.to_rgba8();
        return Ok(Content::Image(
            crate::viewer::RasterImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            ),
        ));
    }
    if mode == Mode::Text {
        return Ok(Content::EncodedText(
            crate::text_document::Document::detect(bytes.clone()).or_else(|_| {
                crate::text_document::Document::decode(
                    bytes,
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
            crate::text_document::Document::detect(bytes)?,
        ));
    }
    if !bytes.contains(&0)
        && let Ok(text) = String::from_utf8(bytes.clone())
    {
        return Ok(Content::Text(text));
    }
    let truncated = bytes.len() > 512 * 1024;
    Ok(Content::Hex {
        text: hex_dump(&bytes[..bytes.len().min(512 * 1024)]),
        truncated,
    })
}

pub fn render_pdf(bytes: &[u8], index: usize) -> Result<(crate::viewer::RasterImage, usize)> {
    render_pdf_password(bytes, index, "")
}
pub fn render_pdf_password(
    bytes: &[u8],
    index: usize,
    password: &str,
) -> Result<(RasterImage, usize)> {
    let pdf = hayro::hayro_syntax::Pdf::new_with_password(bytes.to_vec(), password)
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
pub fn read(location: &Location, path: &Path, limit: u64) -> Result<Vec<u8>> {
    match location {
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
pub fn hex_dump(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::new();
    for (index, row) in bytes.chunks(16).enumerate() {
        let _ = write!(output, "{:08x}  ", index * 16);
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

/// Small LRU of unrotated page images. Passwords and images remain in memory only.
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
pub type PdfPage = (
    RasterImage,
    usize,
    std::sync::Arc<crate::pdf_text::PageText>,
);
impl PdfCache {
    pub fn render(
        &mut self,
        bytes: &[u8],
        page: usize,
        password: &str,
    ) -> Result<(RasterImage, usize)> {
        self.render_with_text(bytes, page, password)
            .map(|(image, count, _)| (image, count))
    }
    pub fn render_with_text(
        &mut self,
        bytes: &[u8],
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
        let (image, count) = render_pdf_password(bytes, page, password)?;
        let text = std::sync::Arc::new(
            crate::pdf_text::load_page(bytes, password, page, &crate::operations::Task::default())
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
