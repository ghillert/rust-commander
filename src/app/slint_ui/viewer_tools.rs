use super::*;
use std::io::Read;
pub(in crate::app) struct Navigation {
    pub id: u64,
    pub location: Location,
    pub path: PathBuf,
    pub notice: String,
    pub pending: Option<Receiver<Result<Hit, String>>>,
    query: String,
    last: Option<u64>,
    cancel: Arc<AtomicBool>,
    pub fit_width: bool,
    pub selection: Option<(i32, i32)>,
}
pub(in crate::app) struct Hit {
    offset: u64,
    text: String,
    page: Option<crate::text_document::Page>,
    hex: bool,
    selection: Option<(i32, i32)>,
}
impl Navigation {
    pub fn new(location: Location, path: PathBuf) -> Self {
        Self {
            id: rand::random(),
            location,
            path,
            notice: String::new(),
            pending: None,
            query: String::new(),
            last: None,
            cancel: Arc::new(AtomicBool::new(false)),
            fit_width: false,
            selection: None,
        }
    }
}
impl Drop for Navigation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Adapter {
    pub(super) fn render_pdf_page(
        &mut self,
        editor: &mut Editor,
        page: usize,
        password: Option<String>,
        rotation: Option<usize>,
    ) {
        let Some(pdf) = &mut editor.pdf else {
            return;
        };
        if self.app.pending_pdf.is_some() || pdf.search_pending.is_some() {
            return;
        }
        if let Some(password) = password {
            pdf.password = password;
        }
        let rotation = rotation.unwrap_or(pdf.rotation);
        let bytes = pdf.bytes.clone();
        let password = pdf.password.clone();
        let cache = pdf.cache.clone();
        let title = format!("{}#{}", editor.title, editor.navigation.id);
        let ctx = self.ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.app.pending_pdf = Some(receiver);
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<_> {
                let (image, count, text) = cache
                    .lock()
                    .map_err(|_| anyhow::anyhow!("PDF cache unavailable"))?
                    .render_with_text(&bytes, page, &password)?;
                Ok((
                    title,
                    page,
                    rotation,
                    count,
                    crate::viewer::rotate(&image, rotation),
                    text,
                ))
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    pub(super) fn viewer_tools_tick(&mut self) {
        let Some(editor) = &mut self.app.editor else {
            return;
        };
        if let Some(result) = crate::background::poll(&mut editor.navigation.pending) {
            match result {
                Ok(hit) => {
                    editor.navigation.notice =
                        format!("Match at byte {} (0x{:x})", hit.offset, hit.offset);
                    editor.navigation.last = Some(hit.offset);
                    editor.navigation.selection = hit.selection;
                    editor.text = hit.text;
                    editor.text_page = hit.page;
                    if hit.hex {
                        editor.kind = "Hex viewer";
                        editor.editable = false;
                        editor.document = None;
                    }
                }
                Err(e) => editor.navigation.notice = e,
            }
        }
        let result = editor
            .pdf
            .as_mut()
            .and_then(|pdf| crate::background::poll(&mut pdf.search_pending));
        if let Some(result) = result {
            let mut editor = self.app.editor.take().unwrap();
            match result {
                Ok(hit) => {
                    let pdf = editor.pdf.as_mut().unwrap();
                    pdf.search_last = Some((hit.page, hit.range.clone()));
                    pdf.pending_selection = Some((hit.page, hit.range));
                    editor.navigation.notice = format!(
                        "Match on page {}{}",
                        hit.page + 1,
                        if hit.wrapped {
                            " · Wrapped search"
                        } else {
                            ""
                        }
                    );
                    self.render_pdf_page(&mut editor, hit.page, None, None);
                }
                Err(error) => editor.navigation.notice = error,
            }
            self.app.editor = Some(editor);
        }
    }
    pub(super) fn viewer_tool_action(&mut self, editor: &mut Editor, action: &str) -> bool {
        if editor.pdf.is_some() && self.pdf_tool_action(editor, action) {
            return true;
        }
        if let Some(password) = action.strip_prefix("unlock:") {
            self.render_pdf_page(editor, 0, Some(password.into()), None);
            return true;
        }
        if let Some(page) = action.strip_prefix("page:") {
            match page.parse::<usize>() {
                Ok(page) if page > 0 && editor.pdf.as_ref().is_some_and(|p| page <= p.pages) => {
                    self.render_pdf_page(editor, page - 1, None, None)
                }
                _ => editor.error = Some("Enter a page number within this document".into()),
            }
            return true;
        }
        if action == "rotate" {
            if let Some(pdf) = &editor.pdf {
                self.render_pdf_page(editor, pdf.page, None, Some((pdf.rotation + 1) % 4));
            }
            return true;
        }
        if action == "fit-width" {
            editor.navigation.fit_width = !editor.navigation.fit_width;
            return true;
        }
        if action == "cancel-search" {
            editor.navigation.cancel.store(true, Ordering::Relaxed);
            return true;
        }
        if let Some(value) = action.strip_prefix("jump:") {
            let result = (|| -> anyhow::Result<()> {
                anyhow::ensure!(
                    !editor.dirty(),
                    "Save edits before jumping to a file offset"
                );
                anyhow::ensure!(
                    editor.path.is_some(),
                    "Byte-offset navigation requires a local file"
                );
                let offset = crate::viewer_search::offset(value)?;
                if editor.kind.starts_with("Hex") {
                    editor.text =
                        crate::viewer_search::hex_page(editor.path.as_ref().unwrap(), offset)?;
                    editor.text_page = None;
                } else {
                    let encoding = editor
                        .document
                        .as_ref()
                        .map(|d| d.encoding)
                        .or_else(|| editor.text_page.as_ref().map(|p| p.document.encoding))
                        .unwrap_or_default();
                    let page = crate::text_document::Page::read(
                        editor.path.as_ref().unwrap(),
                        offset,
                        encoding,
                    )?;
                    editor.text = page.document.text.clone();
                    editor.text_page = Some(page);
                    editor.editable = false;
                    editor.kind = "Paged text viewer";
                }
                editor.navigation.last = Some(offset);
                editor.navigation.notice = format!("Byte {offset}");
                Ok(())
            })();
            editor.error = result.err().map(|e| format!("{e:#}"));
            return true;
        }
        let (hex, query) = if let Some(q) = action.strip_prefix("find:") {
            (false, q)
        } else if let Some(q) = action.strip_prefix("hex-find:") {
            (true, q)
        } else {
            return false;
        };
        if editor.navigation.pending.is_some() {
            return true;
        }
        if editor.navigation.query != format!("{hex}:{query}") {
            editor.navigation.last = None;
        }
        editor.navigation.query = format!("{hex}:{query}");
        if editor.editable && !hex {
            let start = editor
                .navigation
                .last
                .map(|n| n as usize + 1)
                .unwrap_or(0)
                .min(editor.text.len());
            let start = editor.text.ceil_char_boundary(start);
            if let Some(at) = editor.text[start..]
                .find(query)
                .map(|n| n + start)
                .filter(|_| !query.is_empty())
            {
                editor.navigation.last = Some(at as u64);
                editor.navigation.selection = Some((at as i32, (at + query.len()) as i32));
                editor.navigation.notice = format!(
                    "Match at character {}",
                    editor.text[..at].chars().count() + 1
                );
            } else {
                editor.navigation.notice =
                    "No further match; edit the search to start again".into();
            }
            return true;
        }
        if editor.dirty() {
            editor.error = Some("Save edits before searching file bytes".into());
            return true;
        }
        let encoding = editor
            .document
            .as_ref()
            .map(|d| d.encoding)
            .or_else(|| editor.text_page.as_ref().map(|p| p.document.encoding))
            .unwrap_or_default();
        let needle = match crate::viewer_search::needle(query, hex, encoding) {
            Ok(n) => n,
            Err(e) => {
                editor.error = Some(e.to_string());
                return true;
            }
        };
        editor.navigation.cancel = Arc::new(AtomicBool::new(false));
        let cancel = editor.navigation.cancel.clone();
        let ctx = self.ctx.clone();
        let location = editor.navigation.location.clone();
        let path = editor.navigation.path.clone();
        let start = editor.navigation.last.map(|n| n + 1).unwrap_or(0);
        let (sender, receiver) = mpsc::channel();
        editor.navigation.pending = Some(receiver);
        editor.navigation.notice = "Searching…".into();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<Hit> {
                let mut input: Box<dyn Read> = match &location {
                    Location::Local(_) => Box::new(std::fs::File::open(&path)?),
                    _ => Box::new(std::io::Cursor::new(crate::viewer::read(
                        &location,
                        &path,
                        32 * 1024 * 1024,
                    )?)),
                };
                let alignment = if !hex
                    && matches!(
                        encoding,
                        crate::text_document::Encoding::Utf16Le
                            | crate::text_document::Encoding::Utf16Be
                    ) {
                    2
                } else {
                    1
                };
                let at = crate::viewer_search::find(
                    &mut input,
                    &needle,
                    start,
                    alignment,
                    &Task::new(cancel, |_| {}),
                )?
                .ok_or_else(|| {
                    anyhow::anyhow!("No further match; edit the search to start again")
                })?;
                if hex {
                    let text = if matches!(location, Location::Local(_)) {
                        crate::viewer_search::hex_page(&path, at)?
                    } else {
                        let bytes = crate::viewer::read(&location, &path, 32 * 1024 * 1024)?;
                        crate::viewer::hex_dump(
                            &bytes[at as usize..bytes.len().min(at as usize + 65536)],
                        )
                    };
                    return Ok(Hit {
                        offset: at,
                        text,
                        page: None,
                        hex: true,
                        selection: None,
                    });
                }
                let (text, page, base) = if matches!(location, Location::Local(_)) {
                    let p =
                        crate::text_document::Page::read(&path, at.saturating_sub(1024), encoding)?;
                    (p.document.text.clone(), Some(p.clone()), p.offset)
                } else {
                    let doc = crate::text_document::Document::decode(
                        crate::viewer::read(&location, &path, 32 * 1024 * 1024)?,
                        encoding,
                    )?;
                    (doc.text, None, 0)
                };
                let raw = match &page {
                    Some(p) => p.document.original.clone(),
                    None => crate::viewer::read(&location, &path, 32 * 1024 * 1024)?,
                };
                let relative = (at - base) as usize;
                let prefix =
                    crate::text_document::Document::decode(raw[..relative].to_vec(), encoding)
                        .map(|d| d.text.len())
                        .unwrap_or(0);
                let found = crate::text_document::Document::decode(needle, encoding)?
                    .text
                    .len();
                Ok(Hit {
                    offset: at,
                    text,
                    page,
                    hex: false,
                    selection: Some((prefix as i32, (prefix + found) as i32)),
                })
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        true
    }
}

impl Adapter {
    pub(super) fn pdf_select(&mut self, x: f32, y: f32, begin: bool) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        if self.app.pending_pdf.is_some() {
            return;
        }
        let Some(editor) = &mut self.app.editor else {
            return;
        };
        let Some(pdf) = &mut editor.pdf else {
            return;
        };
        if pdf.locked || pdf.search_pending.is_some() {
            return;
        }
        if let Some(index) = pdf.text.nearest(x, y, pdf.rotation) {
            if begin {
                pdf.anchor = Some(index);
            }
            pdf.selection = pdf.text.selection(pdf.anchor.unwrap_or(index), index);
        }
    }
    fn pdf_tool_action(&mut self, editor: &mut Editor, action: &str) -> bool {
        let pdf = editor.pdf.as_mut().unwrap();
        match action {
            "pdf-text" => {
                pdf.text_view = !pdf.text_view;
                editor.image_revision += 1;
                return true;
            }
            "pdf-select-all" => {
                pdf.selection = Some(0..pdf.text.text.len());
                return true;
            }
            "pdf-copy" => {
                if let Some(text) = pdf
                    .selection
                    .as_ref()
                    .and_then(|range| pdf.text.text.get(range.clone()))
                {
                    editor.error = self
                        .app
                        .native_files
                        .copy_text(text.to_owned())
                        .err()
                        .map(|e| e.to_string());
                }
                return true;
            }
            "cancel-search" => {
                pdf.search_cancel.store(true, Ordering::Relaxed);
                return true;
            }
            _ => {}
        }
        let (query, backwards) = if let Some(query) = action
            .strip_prefix("pdf-find:")
            .or_else(|| action.strip_prefix("find:"))
        {
            (query, false)
        } else if let Some(query) = action.strip_prefix("pdf-find-previous:") {
            (query, true)
        } else {
            return false;
        };
        if pdf.locked || self.app.pending_pdf.is_some() || pdf.search_pending.is_some() {
            return true;
        }
        if query.is_empty() || query.len() > 4096 {
            editor.error = Some("Enter a search of 1–4096 bytes".into());
            return true;
        }
        if pdf.search_query != query {
            pdf.search_last = None;
            pdf.search_query = query.to_owned();
        }
        let start = pdf
            .search_last
            .as_ref()
            .map(|(page, range)| (*page, if backwards { range.start } else { range.end }))
            .unwrap_or((pdf.page, if backwards { usize::MAX } else { 0 }));
        pdf.search_cancel = Arc::new(AtomicBool::new(false));
        let task = Task::new(pdf.search_cancel.clone(), |_| {});
        let bytes = pdf.bytes.clone();
        let password = pdf.password.clone();
        let query = query.to_owned();
        let (sender, receiver) = mpsc::channel();
        pdf.search_pending = Some(receiver);
        editor.error = None;
        editor.navigation.notice = "Searching PDF text…".into();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = crate::pdf_text::find(&bytes, &password, &query, start, backwards, &task)
                .and_then(|hit| {
                    hit.ok_or_else(|| {
                        anyhow::anyhow!("No matching PDF text. Scanned pages need OCR.")
                    })
                })
                .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        true
    }
}
