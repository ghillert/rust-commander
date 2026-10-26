//! Bounded PDF text extraction and page-space selection, using the renderer's interpreter.
use crate::operations::Task;
use anyhow::{Context as _, Result, ensure};
use hayro::{
    hayro_interpret as pdf,
    kurbo::{BezPath, Rect},
};
use pdf::{
    Device, DrawMode, DrawProps, ImageDrawProps, TransformExt,
    font::{Glyph, GlyphRun},
    hayro_cmap::BfString,
};
use std::{collections::HashSet, ops::Range, sync::Arc};

const MAX_GLYPHS: usize = 65_536;
const MAX_TEXT: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct TextGlyph {
    pub range: Range<usize>,
    /// Normalized rectangle on the unrotated rendered page (top-left origin).
    pub bounds: [f32; 4],
}
#[derive(Clone, Debug, Default)]
pub struct PageText {
    pub text: String,
    pub glyphs: Vec<TextGlyph>,
    pub unmapped: bool,
    pub error: Option<String>,
}
impl PageText {
    pub fn nearest(&self, x: f32, y: f32, rotation: usize) -> Option<usize> {
        self.glyphs
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let distance = |g: &TextGlyph| {
                    let [x0, y0, x1, y1] = rotate_bounds(g.bounds, rotation);
                    let dx = (x0 - x).max(0.0).max(x - x1);
                    let dy = (y0 - y).max(0.0).max(y - y1);
                    dx * dx + dy * dy
                };
                distance(a).total_cmp(&distance(b))
            })
            .map(|(index, _)| index)
    }
    pub fn selection(&self, first: usize, last: usize) -> Option<Range<usize>> {
        Some(
            self.glyphs.get(first.min(last))?.range.start
                ..self.glyphs.get(first.max(last))?.range.end,
        )
    }
    pub fn rectangles(&self, range: &Range<usize>, rotation: usize) -> Vec<[f32; 4]> {
        self.glyphs
            .iter()
            .filter(|g| g.range.start < range.end && g.range.end > range.start)
            .map(|g| rotate_bounds(g.bounds, rotation))
            .collect()
    }
}
pub fn rotate_bounds([x0, y0, x1, y1]: [f32; 4], rotation: usize) -> [f32; 4] {
    match rotation % 4 {
        1 => [1.0 - y1, x0, 1.0 - y0, x1],
        2 => [1.0 - x1, 1.0 - y1, 1.0 - x0, 1.0 - y0],
        3 => [y0, 1.0 - x1, y1, 1.0 - x0],
        _ => [x0, y0, x1, y1],
    }
}

struct TextDevice<'t> {
    result: PageText,
    size: [f64; 2],
    seen: HashSet<(String, i64, i64)>,
    overflow: bool,
    task: &'t Task,
}
impl Device<'_> for TextDevice<'_> {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'_>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &pdf::ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<pdf::SoftMask<'_>>, _: pdf::BlendMode) {
    }
    fn draw_image(&mut self, _: pdf::Image<'_, '_>, _: ImageDrawProps<'_>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, '_>, props: DrawProps<'_>, _: &DrawMode) {
        for glyph in run.glyphs() {
            if self.task.check().is_err() {
                return;
            }
            if self.result.glyphs.len() >= MAX_GLYPHS || self.result.text.len() >= MAX_TEXT {
                self.overflow = true;
                return;
            }
            let text = match glyph.as_unicode() {
                Some(BfString::Char(c)) => c.to_string(),
                Some(BfString::String(s)) => s,
                None => {
                    self.result.unmapped = true;
                    continue;
                }
            };
            if text.is_empty() || text.contains('\0') {
                continue;
            }
            if self
                .result
                .text
                .len()
                .saturating_add(text.len())
                .saturating_add(1)
                > MAX_TEXT
            {
                self.overflow = true;
                return;
            }
            let advance = match &**glyph {
                Glyph::Outline(g) => g.advance_width().unwrap_or(500.0) as f64,
                Glyph::Type3(_) => 500.0,
            };
            let transform = props.transform * glyph.transform();
            let r = transform.transform_rect_bbox(Rect::new(0.0, -200.0, advance.max(1.0), 800.0));
            let bounds = [
                r.x0 / self.size[0],
                r.y0 / self.size[1],
                r.x1 / self.size[0],
                r.y1 / self.size[1],
            ];
            if bounds.iter().any(|v| !v.is_finite()) {
                continue;
            }
            let key = (text.clone(), (r.x0 * 100.0) as i64, (r.y0 * 100.0) as i64);
            if !self.seen.insert(key) {
                continue;
            }
            if let Some(previous) = self.result.glyphs.last() {
                let [px0, py0, px1, py1] = previous.bounds;
                let height = (py1 - py0).abs().max(0.0001);
                if (bounds[1] as f32 - py0).abs() > height * 0.6 {
                    if !self.result.text.ends_with('\n') {
                        self.result.text.push('\n');
                    }
                } else if bounds[0] as f32 - px1 > (px1 - px0).abs().max(height * 0.15) * 0.4
                    && !self.result.text.ends_with(char::is_whitespace)
                    && !text.starts_with(char::is_whitespace)
                {
                    self.result.text.push(' ');
                }
            }
            let start = self.result.text.len();
            self.result.text.push_str(&text);
            self.result.glyphs.push(TextGlyph {
                range: start..self.result.text.len(),
                bounds: bounds.map(|v| v as f32),
            });
        }
    }
}
pub fn extract_page(page: &pdf::hayro_syntax::page::Page<'_>, task: &Task) -> Result<PageText> {
    task.check()?;
    let (width, height) = page.render_dimensions();
    ensure!(
        width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0,
        "Invalid PDF page dimensions."
    );
    let cache = pdf::InterpreterCache::new();
    let mut context = pdf::Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0.0, 0.0, width as f64, height as f64),
        &cache,
        page.xref(),
        Default::default(),
    );
    let mut device = TextDevice {
        result: Default::default(),
        size: [width as f64, height as f64],
        seen: HashSet::new(),
        overflow: false,
        task,
    };
    pdf::interpret_page(page, &mut context, &mut device);
    task.check()?;
    ensure!(
        !device.overflow,
        "PDF page exceeds the text selection limit (65,536 glyphs / 2 MiB)."
    );
    Ok(device.result)
}
pub fn load_page(bytes: &[u8], password: &str, page: usize, task: &Task) -> Result<PageText> {
    let pdf = pdf::hayro_syntax::Pdf::new_with_password(bytes.to_vec(), password)
        .map_err(|e| anyhow::anyhow!("Cannot read PDF: {e:?}"))?;
    extract_page(
        pdf.pages().get(page).context("PDF page does not exist")?,
        task,
    )
}

pub struct SearchHit {
    pub page: usize,
    pub range: Range<usize>,
    pub text: Arc<PageText>,
    pub wrapped: bool,
}
/// Search visible Unicode text, not compressed PDF bytes. Holds one text page at a time.
pub fn find(
    bytes: &[u8],
    password: &str,
    query: &str,
    start: (usize, usize),
    backwards: bool,
    task: &Task,
) -> Result<Option<SearchHit>> {
    ensure!(
        !query.is_empty() && query.len() <= 4096,
        "Enter a search of 1–4096 bytes"
    );
    let expression = regex::RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()?;
    let pdf = pdf::hayro_syntax::Pdf::new_with_password(bytes.to_vec(), password)
        .map_err(|e| anyhow::anyhow!("Cannot read PDF: {e:?}"))?;
    let count = pdf.pages().len();
    ensure!(
        count > 0 && count <= 10_000,
        "PDF text search supports up to 10,000 pages."
    );
    let first = start.0.min(count - 1);
    for step in 0..=count {
        task.check()?;
        let page = if backwards {
            (first + count - step % count) % count
        } else {
            (first + step) % count
        };
        let text = extract_page(&pdf.pages()[page], task)?;
        let mut matches = expression.find_iter(&text.text).filter(|m| {
            if step == 0 {
                if backwards {
                    m.end() <= start.1
                } else {
                    m.start() >= start.1
                }
            } else if step == count {
                if backwards {
                    m.start() >= start.1
                } else {
                    m.end() <= start.1
                }
            } else {
                true
            }
        });
        let found = if backwards {
            matches.last()
        } else {
            matches.next()
        };
        if let Some(found) = found {
            return Ok(Some(SearchHit {
                page,
                range: found.range(),
                text: Arc::new(text),
                wrapped: step > 0
                    && if backwards {
                        page >= first
                    } else {
                        page <= first
                    },
            }));
        }
    }
    Ok(None)
}
