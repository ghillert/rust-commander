//! Display-only source presentations. No formatted text enters the editor/save path.
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};
use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};

pub const VIEWER_LIMIT: usize = 2 * 1024 * 1024;
pub const PREVIEW_LIMIT: usize = 256 * 1024;
const LINE_LIMIT: usize = 16 * 1024;
const SPAN_LIMIT: usize = 100_000;
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);
static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    Plain,
    #[default]
    Highlighted,
    Pretty,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain original",
            Self::Highlighted => "Highlighted original",
            Self::Pretty => "Pretty view",
        }
    }
    pub fn named(label: &str) -> Option<Self> {
        [Self::Plain, Self::Highlighted, Self::Pretty]
            .into_iter()
            .find(|mode| mode.label() == label)
    }
}

#[derive(Clone, Debug)]
pub struct Span {
    pub range: Range<usize>,
    pub color: [u8; 4],
}
#[derive(Debug)]
pub struct Line {
    pub range: Range<usize>,
    pub spans: Range<usize>,
}
#[derive(Debug)]
pub struct Prepared {
    pub text: Arc<str>,
    pub layout: Arc<str>,
    pub to_layout: Vec<i32>,
    pub to_source: Vec<i32>,
    pub lines: Vec<Line>,
    pub spans: Vec<Span>,
    pub background: [u8; 4],
    pub foreground: [u8; 4],
}
struct Output {
    mode: Mode,
    language: String,
    pretty: bool,
    notice: String,
    prepared: Option<Prepared>,
}
struct Input {
    path: PathBuf,
    text: Arc<str>,
    len: usize,
    complete: bool,
    dark: bool,
    mode: Mode,
    limit: usize,
}

pub struct State {
    pub id: u64,
    pub revision: u64,
    pub mode: Mode,
    pub language: String,
    pub pretty: bool,
    pub supported: bool,
    pub notice: String,
    pub prepared: Option<Prepared>,
    input: Option<Input>,
    pending: Option<Receiver<std::result::Result<Output, String>>>,
    cancel: Arc<AtomicBool>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            id: rand::random(),
            revision: 0,
            mode: Mode::Highlighted,
            language: String::new(),
            pretty: false,
            supported: true,
            notice: String::new(),
            prepared: None,
            input: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl State {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn display<'a>(&'a self, source: &'a str) -> &'a str {
        self.prepared.as_ref().map_or(source, |p| &p.text)
    }
    pub fn select(&mut self, label: &str) {
        if let Some(mode) = Mode::named(label)
            && (mode != Mode::Pretty || self.pretty)
        {
            self.mode = mode;
        }
    }
    pub fn update(
        &mut self,
        path: &Path,
        text: &str,
        complete: bool,
        limit: usize,
        dark: bool,
        ctx: &crate::ui_signal::UiSignal,
    ) {
        let same = self.input.as_ref().is_some_and(|input| {
            input.path == path
                && input.len == text.len()
                && (text.len() > limit || input.text.as_ref() == text)
                && input.complete == complete
                && input.dark == dark
                && input.mode == self.mode
                && input.limit == limit
        });
        if !same {
            let discover = self.input.as_ref().is_none_or(|input| {
                input.path != path
                    || !input.complete
                    || input.len > limit
                    || input.limit != limit
                    || self.language.is_empty() && input.text.as_ref() != text
            });
            self.cancel.store(true, Ordering::Relaxed);
            self.pending = None; // A retired receiver cannot publish stale results.
            self.cancel = Arc::new(AtomicBool::new(false));
            self.prepared = None;
            self.notice.clear();
            self.revision = self.revision.wrapping_add(1);
            let source: Arc<str> = if text.len() <= limit {
                text.into()
            } else {
                "".into()
            };
            self.input = Some(Input {
                path: path.into(),
                text: source.clone(),
                len: text.len(),
                complete,
                dark,
                mode: self.mode,
                limit,
            });
            if !complete || text.len() > limit {
                self.mode = Mode::Plain;
                self.pretty = false;
                self.supported = false;
                self.notice = if !complete {
                    "Original view: enhanced views require a complete document."
                } else {
                    "Original view: this document exceeds the enhanced-view size limit."
                }
                .into();
            } else if self.mode != Mode::Plain
                || discover
                || self.language.is_empty() && self.supported
            {
                let path = path.to_owned();
                let mode = self.mode;
                let cancel = self.cancel.clone();
                let ctx = ctx.clone();
                let (send, receive) = mpsc::channel();
                self.pending = Some(receive);
                self.notice = "Preparing source view…".into();
                std::thread::spawn(move || {
                    let output = prepare(&path, source, mode, dark, limit, &cancel)
                        .map_err(|e| format!("{e:#}"));
                    let _ = send.send(output);
                    ctx.request_repaint();
                });
            }
            self.input.as_mut().unwrap().mode = self.mode;
        }
        if let Some(result) = crate::background::poll(&mut self.pending) {
            match result {
                Ok(output) => {
                    self.mode = output.mode;
                    self.language = output.language;
                    self.pretty = output.pretty;
                    self.supported = !self.language.is_empty();
                    self.notice = output.notice;
                    self.prepared = output.prepared;
                }
                Err(error) => {
                    self.mode = Mode::Plain;
                    self.notice = format!("Original view: {error:#}");
                }
            }
            self.input.as_mut().unwrap().mode = self.mode;
            self.revision = self.revision.wrapping_add(1);
        }
    }
}

fn prepare(
    path: &Path,
    source: Arc<str>,
    requested: Mode,
    dark: bool,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<Output> {
    ensure!(!cancel.load(Ordering::Relaxed), "Source view cancelled");
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let first_line = source.lines().next().unwrap_or("");
    let first_line =
        &first_line[..first_line.floor_char_boundary(first_line.len().min(LINE_LIMIT))];
    let syntax = SYNTAXES
        .find_syntax_by_extension(&file_name)
        .or_else(|| SYNTAXES.find_syntax_by_extension(&extension))
        .or_else(|| SYNTAXES.find_syntax_by_first_line(first_line))
        .filter(|s| s.name != "Plain Text");
    let Some(syntax) = syntax else {
        return Ok(Output {
            mode: Mode::Plain,
            language: String::new(),
            pretty: false,
            notice: String::new(),
            prepared: None,
        });
    };
    let json = syntax.name == "JSON";
    let xml = syntax.name == "XML";
    let pretty = json || xml;
    if requested == Mode::Plain {
        return Ok(Output {
            mode: requested,
            language: syntax.name.clone(),
            pretty,
            notice: String::new(),
            prepared: None,
        });
    }
    let mut mode = requested;
    let mut notice = String::new();
    let text = if requested == Mode::Pretty && pretty {
        match if json {
            pretty_json(&source, limit)
        } else {
            pretty_xml(&source, limit)
        } {
            Ok(text) => Arc::<str>::from(text),
            Err(error) => {
                mode = Mode::Highlighted;
                notice = format!("Cannot format this document: {error:#}");
                source
            }
        }
    } else {
        source
    };
    let theme = &THEMES.themes[if dark {
        "base16-ocean.dark"
    } else {
        "InspiredGitHub"
    }];
    let foreground = theme
        .settings
        .foreground
        .unwrap_or(syntect::highlighting::Color::BLACK);
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut spans: Vec<Span> = Vec::new();
    let mut lines = Vec::new();
    let mut offset = 0;
    let mut stopped = false;
    for line in text.split_inclusive('\n') {
        ensure!(!cancel.load(Ordering::Relaxed), "Source view cancelled");
        ensure!(
            lines.len() < SPAN_LIMIT,
            "Document has too many lines for enhanced viewing"
        );
        let body = line.strip_suffix('\n').unwrap_or(line);
        let body = body.strip_suffix('\r').unwrap_or(body);
        let start = spans.len();
        if line.len() > LINE_LIMIT || spans.len() >= SPAN_LIMIT {
            stopped = true;
            if notice.is_empty() {
                notice =
                    "Some text is shown without colors because a highlighting limit was reached."
                        .into();
            }
        }
        if stopped {
            spans.push(Span {
                range: offset..offset + body.len(),
                color: [foreground.r, foreground.g, foreground.b, foreground.a],
            });
        } else {
            let mut at = offset;
            for (style, run) in highlighter.highlight_line(line, &SYNTAXES)? {
                let end = (at + run.len()).min(offset + body.len());
                let color = [
                    style.foreground.r,
                    style.foreground.g,
                    style.foreground.b,
                    style.foreground.a,
                ];
                if end > at {
                    if spans.len() > start && spans.last().is_some_and(|s| s.color == color) {
                        spans.last_mut().unwrap().range.end = end;
                    } else {
                        spans.push(Span {
                            range: at..end,
                            color,
                        });
                    }
                }
                at += run.len();
            }
        }
        lines.push(Line {
            range: offset..offset + body.len(),
            spans: start..spans.len(),
        });
        offset += line.len();
    }
    if text.is_empty() || text.ends_with('\n') {
        lines.push(Line {
            range: text.len()..text.len(),
            spans: spans.len()..spans.len(),
        });
    }
    let background = theme
        .settings
        .background
        .unwrap_or(syntect::highlighting::Color::WHITE);
    let (layout, to_layout, to_source) = tab_layout(&text, limit)?;
    Ok(Output {
        mode,
        language: syntax.name.clone(),
        pretty,
        notice,
        prepared: Some(Prepared {
            text,
            layout,
            to_layout,
            to_source,
            lines,
            spans,
            background: [background.r, background.g, background.b, background.a],
            foreground: [foreground.r, foreground.g, foreground.b, foreground.a],
        }),
    })
}

/// Slint's text shaper gives tabs zero advance. Expand them only for layout,
/// retaining byte maps so search, selection, and copying use the original text.
fn tab_layout(text: &Arc<str>, limit: usize) -> Result<(Arc<str>, Vec<i32>, Vec<i32>)> {
    if !text.contains('\t') {
        return Ok((text.clone(), Vec::new(), Vec::new()));
    }
    let mut layout = Vec::new();
    let mut to_layout = Vec::with_capacity(text.len() + 1);
    let mut to_source = Vec::new();
    let mut column = 0;
    for (offset, byte) in text.bytes().enumerate() {
        to_layout.push(layout.len() as i32);
        if byte == b'\t' {
            let spaces = 4 - column % 4;
            layout.extend(std::iter::repeat_n(b' ', spaces));
            to_source.extend(std::iter::repeat_n(offset as i32, spaces));
            column += spaces;
        } else {
            layout.push(byte);
            to_source.push(offset as i32);
            if byte == b'\n' {
                column = 0;
            } else if byte & 0xc0 != 0x80 {
                column += 1;
            }
        }
        ensure!(
            layout.len() <= limit,
            "Expanded tab layout exceeds the enhanced-view size limit"
        );
    }
    to_layout.push(layout.len() as i32);
    to_source.push(text.len() as i32);
    Ok((String::from_utf8(layout)?.into(), to_layout, to_source))
}

/// Validate without building a Value (which would lose duplicate keys/number spellings).
fn pretty_json(source: &str, limit: usize) -> Result<String> {
    let mut parser = serde_json::Deserializer::from_str(source);
    serde::de::IgnoredAny::deserialize(&mut parser)?;
    parser.end()?;
    let mut output = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut previous = None;
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            output.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
        } else {
            match c {
                '"' => {
                    quoted = true;
                    output.push(c);
                }
                '{' | '[' => {
                    output.push(c);
                    depth += 1;
                    if chars.clone().find(|c| !c.is_ascii_whitespace())
                        != Some(if c == '{' { '}' } else { ']' })
                    {
                        newline(&mut output, depth);
                    }
                }
                '}' | ']' => {
                    depth = depth.saturating_sub(1);
                    if previous != Some(if c == '}' { '{' } else { '[' }) {
                        newline(&mut output, depth);
                    }
                    output.push(c);
                }
                ',' => {
                    output.push(c);
                    newline(&mut output, depth);
                }
                ':' => output.push_str(": "),
                c if c.is_ascii_whitespace() => continue,
                _ => output.push(c),
            }
        }
        previous = Some(c);
        ensure!(
            output.len() <= limit,
            "Formatted document exceeds the enhanced-view size limit"
        );
    }
    Ok(output)
}

fn newline(output: &mut String, depth: usize) {
    output.push('\n');
    for _ in 0..depth {
        output.push_str("  ");
    }
}

fn pretty_xml(source: &str, limit: usize) -> Result<String> {
    use quick_xml::{Reader, events::Event};
    // Keep raw ranges so preserved subtrees and lexical details are copied verbatim.
    struct Token {
        range: Range<usize>,
        depth: usize,
        start: bool,
        end: bool,
        whitespace: bool,
        preserve: bool,
        protected: bool,
    }
    let mut reader = Reader::from_str(source);
    reader.config_mut().enable_all_checks(true);
    let mut tokens: Vec<Token> = Vec::new();
    let mut stack: Vec<(usize, bool)> = Vec::new();
    let mut root_count = 0;
    loop {
        let from = reader.buffer_position() as usize;
        let event = reader.read_event()?;
        let to = reader.buffer_position() as usize;
        let mut preserve = stack.last().is_some_and(|(_, p)| *p);
        let mut start = false;
        let mut end = false;
        let mut whitespace = false;
        let mut protected = false;
        let mut depth = stack.len();
        match event {
            Event::Start(e) | Event::Empty(e) => {
                start = !source[from..to].ends_with("/>");
                if stack.is_empty() {
                    root_count += 1;
                }
                for attr in e.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"xml:space" && attr.value.as_ref() == b"preserve" {
                        protected = true;
                    }
                }
                if start {
                    stack.push((tokens.len(), preserve || protected));
                }
                ensure!(
                    stack.len() <= 128,
                    "XML nesting exceeds the formatting limit"
                );
            }
            Event::End(_) => {
                let (opening, inherited) = stack
                    .pop()
                    .ok_or_else(|| anyhow::anyhow!("Unexpected XML closing tag"))?;
                depth = stack.len();
                let whitespace_leaf = tokens.len() > opening + 1
                    && tokens[opening + 1..].iter().all(|t| t.whitespace);
                preserve = inherited || tokens[opening].protected || whitespace_leaf;
                if preserve {
                    for token in &mut tokens[opening + 1..] {
                        token.preserve = true;
                    }
                }
                end = true;
            }
            Event::Text(_) => {
                whitespace = source[from..to].bytes().all(|b| b.is_ascii_whitespace());
                if !whitespace {
                    ensure!(!stack.is_empty(), "Text outside the XML root element");
                    if let Some((opening, _)) = stack.last() {
                        tokens[*opening].protected = true;
                    }
                    preserve = true;
                }
            }
            Event::CData(_) | Event::GeneralRef(_) => {
                ensure!(!stack.is_empty(), "Content outside the XML root element");
                if let Some((opening, _)) = stack.last() {
                    tokens[*opening].protected = true;
                }
                preserve = true;
            }
            Event::Eof => break,
            _ => {}
        }
        tokens.push(Token {
            range: from..to,
            depth,
            start,
            end,
            whitespace,
            preserve,
            protected,
        });
        ensure!(
            tokens.len() <= SPAN_LIMIT,
            "XML document has too many formatting tokens"
        );
    }
    ensure!(
        stack.is_empty() && root_count == 1,
        "XML must contain one complete root element"
    );
    let mut output = String::new();
    let mut previous_start = false;
    for token in tokens {
        if token.whitespace && !token.preserve {
            continue;
        }
        if !output.is_empty() && !token.preserve && !(token.end && previous_start) {
            newline(&mut output, token.depth);
        }
        output.push_str(&source[token.range]);
        previous_start = token.start;
        ensure!(
            output.len() <= limit,
            "Formatted document exceeds the enhanced-view size limit"
        );
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(path: &str, text: &str, mode: Mode) -> Output {
        prepare(
            Path::new(path),
            text.into(),
            mode,
            false,
            VIEWER_LIMIT,
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn json_formatting_preserves_tokens_and_rejects_invalid_documents() {
        let source = r#" {"z":1e+9999,"z":-0.00,"a":"雪 \\ \" ","b": [ true, null,{} ]} "#;
        let pretty = pretty_json(source, VIEWER_LIMIT).unwrap();
        assert!(pretty.starts_with("{\n  \"z\": 1e+9999,\n  \"z\": -0.00,"));
        assert!(pretty.contains(r#""a": "雪 \\ \" ""#));
        assert_eq!(
            pretty_json("[{}, [], 1]", VIEWER_LIMIT).unwrap(),
            "[\n  {},\n  [],\n  1\n]"
        );
        for invalid in ["{", "[1,]", "true false", r#"{"a":}"#, ""] {
            assert!(pretty_json(invalid, VIEWER_LIMIT).is_err(), "{invalid}");
        }
        assert!(pretty_json("[1,2,3]", 5).is_err());
        assert_eq!(pretty_json("42", 10).unwrap(), "42");
    }

    #[test]
    fn xml_formatting_preserves_mixed_content_and_protected_whitespace() {
        let source = "<?xml version=\"1.0\"?><root xmlns=\"urn:test\"><item a='1'>text</item><empty/><!--note--><p>Hello <b>world</b>!</p><keep xml:space=\"preserve\">  <x/>\n </keep><space>  </space><![CDATA[<raw>]]></root>";
        let pretty = pretty_xml(source, VIEWER_LIMIT).unwrap();
        // Root CDATA is mixed content, so the entire root remains verbatim.
        assert_eq!(pretty, source.replace("?><root", "?>\n<root"));
        let source = "<root><item a='1'>text</item><empty/><p>Hello <b>world</b>!</p><keep xml:space='preserve'> <x/>\n </keep><space>  </space></root>";
        let pretty = pretty_xml(source, VIEWER_LIMIT).unwrap();
        assert!(
            pretty.starts_with("<root>\n  <item a='1'>text</item>\n  <empty/>\n"),
            "{pretty}"
        );
        assert!(pretty.contains("<p>Hello <b>world</b>!</p>"));
        assert!(pretty.contains("<keep xml:space='preserve'> <x/>\n </keep>"));
        assert!(pretty.contains("<space>  </space>"));
        assert!(pretty.ends_with("\n</root>"));
        for invalid in ["<root>", "<a></b>", "<a/><b/>", "text", ""] {
            assert!(pretty_xml(invalid, VIEWER_LIMIT).is_err(), "{invalid}");
        }
        assert!(pretty_xml("<root><child/></root>", 10).is_err());
    }

    #[test]
    fn highlighting_preserves_exact_unicode_crlf_tabs_and_trailing_lines() {
        let text = "{\r\n\t\"é雪😀\": \"\\u0061\",\r\n\r\n\t\"a\": true\r\n}\r\n";
        let output = render("file.json", text, Mode::Highlighted);
        assert_eq!(output.language, "JSON");
        assert!(output.pretty);
        let prepared = output.prepared.unwrap();
        assert_eq!(&*prepared.text, text);
        assert_eq!(prepared.lines.len(), 6);
        for line in &prepared.lines {
            let reconstructed: String = prepared.spans[line.spans.clone()]
                .iter()
                .map(|span| &prepared.text[span.range.clone()])
                .collect();
            assert_eq!(reconstructed, prepared.text[line.range.clone()]);
        }
        assert!(prepared.spans.windows(2).any(|s| s[0].color != s[1].color));
        assert!(!prepared.layout.contains('\t'));
        for (offset, _) in text
            .char_indices()
            .chain(std::iter::once((text.len(), '\0')))
        {
            let displayed = prepared.to_layout[offset] as usize;
            assert!(prepared.layout.is_char_boundary(displayed));
            assert_eq!(prepared.to_source[displayed] as usize, offset);
        }
    }

    #[test]
    fn bundled_languages_detect_without_opening_the_path() {
        for (name, source, language) in [
            ("Cargo.toml", "[package]\nname = \"hello\"", "TOML"),
            ("app.ts", "const value: string = 'a';", "TypeScript"),
            ("Dockerfile", "FROM alpine", "Dockerfile"),
            ("script", "#!/usr/bin/env python\nprint('hello')", "Python"),
            ("file.xml", "<root/>", "XML"),
        ] {
            assert_eq!(render(name, source, Mode::Highlighted).language, language);
        }
        let unknown = render("unknown.no_syntax", "ordinary text", Mode::Highlighted);
        assert_eq!(unknown.mode, Mode::Plain);
        assert!(unknown.prepared.is_none());
    }

    #[test]
    fn malformed_pretty_view_falls_back_to_highlighted_original() {
        for (path, source) in [("bad.json", "{bad"), ("bad.xml", "<root>")] {
            let output = render(path, source, Mode::Pretty);
            assert_eq!(output.mode, Mode::Highlighted);
            assert!(output.notice.contains("Cannot format"));
            assert_eq!(&*output.prepared.unwrap().text, source);
        }
        let long = format!("\"{}\"", "x".repeat(LINE_LIMIT));
        let output = render("long.json", &long, Mode::Highlighted);
        assert!(!output.notice.is_empty());
        assert_eq!(&*output.prepared.unwrap().text, long);
    }

    fn finish(state: &mut State, path: &str, text: &str, dark: bool) {
        let ctx = crate::ui_signal::UiSignal::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            state.update(Path::new(path), text, true, VIEWER_LIMIT, dark, &ctx);
            if !state.busy() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn modes_theme_and_retired_workers_never_replace_the_source() {
        let mut state = State::default();
        let source = "{\"value\":1}";
        finish(&mut state, "file.json", source, false);
        let light = state.prepared.as_ref().unwrap().background;
        state.select(Mode::Pretty.label());
        finish(&mut state, "file.json", source, false);
        assert_ne!(state.display(source), source);
        state.select(Mode::Plain.label());
        finish(&mut state, "file.json", source, false);
        assert_eq!(state.display(source), source);
        state.select(Mode::Highlighted.label());
        finish(&mut state, "file.json", source, true);
        assert_ne!(state.prepared.as_ref().unwrap().background, light);
        let ctx = crate::ui_signal::UiSignal::default();
        state.mode = Mode::Pretty;
        state.update(
            Path::new("old.json"),
            source,
            true,
            VIEWER_LIMIT,
            false,
            &ctx,
        );
        finish(&mut state, "new.xml", "<root/>", false);
        assert_eq!(state.language, "XML");
        assert_eq!(state.display("<root/>"), "<root/>");
        state.update(
            Path::new("file.json"),
            source,
            false,
            VIEWER_LIMIT,
            false,
            &ctx,
        );
        assert_eq!(state.mode, Mode::Plain);
        assert!(!state.pretty);
        assert_eq!(state.display(source), source);
        state.update(
            Path::new("file.json"),
            &"x".repeat(PREVIEW_LIMIT + 1),
            true,
            PREVIEW_LIMIT,
            false,
            &ctx,
        );
        assert_eq!(state.mode, Mode::Plain);
        assert!(!state.busy());
        finish(&mut state, "file.json", source, false);
        assert!(state.supported && state.pretty);
        assert_eq!(state.mode, Mode::Plain);
    }
}
