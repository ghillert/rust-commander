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
    Pdf {
        bytes: std::sync::Arc<Vec<u8>>,
        image: crate::viewer::RasterImage,
        pages: usize,
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
pub fn load(location: &Location, path: &Path) -> Result<Content> {
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let image = [
        "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico",
    ]
    .contains(&extension.as_str());
    if !image
        && extension != "pdf"
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
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(512 * 1024)
            .read_to_end(&mut bytes)?;
        return Ok(Content::Hex {
            text: hex_dump(&bytes),
            truncated: true,
        });
    }
    let limit = if image || extension == "pdf" {
        32 * 1024 * 1024
    } else {
        2 * 1024 * 1024
    };
    let bytes = read(location, path, limit)?;
    if extension == "pdf" {
        let (image, pages) = render_pdf(&bytes, 0)?;
        return Ok(Content::Pdf {
            bytes: std::sync::Arc::new(bytes),
            image,
            pages,
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
    let pdf = hayro::hayro_syntax::Pdf::new(bytes.to_vec())
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
