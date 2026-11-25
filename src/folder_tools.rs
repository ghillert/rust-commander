//! Reviewable folder comparison and bounded, read-only text differences.
use crate::{
    comparison::{self, Options},
    desktop::Comparison,
    filesystem::Location,
    operations::Task,
    ui_signal::UiSignal,
};
use anyhow::{Result, ensure};
use std::{
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

/// State of the folder comparison dialog and its background workers.
///
/// [`Self::start`] begins a comparison, [`Self::start_diff`] opens a text diff for an
/// eligible row, and [`Self::poll`] collects results.
pub struct Form {
    /// Directory shown on the left side of the comparison.
    pub left: Location,
    /// Directory shown on the right side of the comparison.
    pub right: Location,
    /// Path rules and comparison mode for the next comparison.
    pub options: Options,
    /// Editable name used to save or select a preset.
    pub preset_name: String,
    /// Rows from the most recently completed comparison, when available.
    pub result: Option<Vec<Comparison>>,
    /// Receiver for the outstanding directory comparison.
    pub pending: Option<Receiver<Result<Vec<Comparison>, String>>>,
    /// Cancellation flag shared with the active comparison or diff worker.
    pub cancel: Arc<AtomicBool>,
    /// Diagnostic from a failed comparison or text diff, when available.
    pub error: Option<String>,
    /// Completed side-by-side diff for a selected comparison row.
    pub diff: Option<Diff>,
    /// Receiver for the outstanding text diff.
    pub diff_pending: Option<Receiver<Result<Diff, String>>>,
}
impl Drop for Form {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
impl Form {
    /// Creates an idle comparison between the two roots with default comparison options.
    pub fn new(left: Location, right: Location) -> Self {
        Self {
            left,
            right,
            options: Default::default(),
            preset_name: String::new(),
            result: None,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            error: None,
            diff: None,
            diff_pending: None,
        }
    }
    /// Cancels active work and clears pending comparisons, diff rows, and errors.
    pub fn invalidate(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.pending = None;
        self.diff_pending = None;
        self.diff = None;
        self.result = None;
        self.error = None;
    }
    /// Starts a background comparison using the current roots and options. The worker
    /// signals the UI when [`Self::poll`] can collect the result.
    pub fn start(&mut self, ctx: &UiSignal) {
        self.invalidate();
        self.cancel = Arc::new(AtomicBool::new(false));
        let (left, right, options, cancel) = (
            self.left.clone(),
            self.right.clone(),
            self.options.clone(),
            self.cancel.clone(),
        );
        let (sender, receiver) = mpsc::channel();
        self.pending = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = comparison::compare_with_options(
                &left,
                &right,
                &options,
                &mut Task::new(cancel, |_| {}),
            )
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    /// Starts a text diff for the comparison row at `index`. Ignores the request if that
    /// row cannot be diffed or another diff is already running.
    pub fn start_diff(&mut self, index: usize, ctx: &UiSignal) {
        if self.diff_pending.is_some() {
            return;
        }
        let Some(item) = self
            .result
            .as_ref()
            .and_then(|r| r.get(index))
            .filter(|r| r.can_diff)
        else {
            return;
        };
        let path = item.path.clone();
        let (left, right) = (self.left.clone(), self.right.clone());
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (sender, receiver) = mpsc::channel();
        self.diff_pending = Some(receiver);
        self.error = None;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = text_diff(&left, &right, &path, &Task::new(cancel, |_| {}))
                .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    /// Collects any completed comparison or text diff without blocking and records a worker
    /// failure in the dialog's error field.
    pub fn poll(&mut self) {
        if let Some(result) = crate::background::poll(&mut self.pending) {
            match result {
                Ok(result) => self.result = Some(result),
                Err(e) => self.error = Some(e),
            }
        }
        if let Some(result) = crate::background::poll(&mut self.diff_pending) {
            match result {
                Ok(result) => self.diff = Some(result),
                Err(e) => self.error = Some(e),
            }
        }
    }
}
/// One pair of aligned display rows in a side-by-side text diff.
pub struct DiffRow {
    /// Formatted left line with its one-based line number, or an empty string for an insertion.
    pub left: String,
    /// Formatted right line with its one-based line number, or an empty string for a deletion.
    pub right: String,
    /// Whether this row represents an insertion, deletion, or replacement.
    pub changed: bool,
}
/// Side-by-side text diff for the same relative file path on both sides of a comparison.
pub struct Diff {
    /// Display form of the compared file's root-relative path.
    pub name: String,
    /// Aligned display rows in document order.
    pub rows: Vec<DiffRow>,
}
/// Loads the same relative path from both roots and aligns its decoded text. Each side must
/// be a regular file of at most 1 MiB.
///
/// # Errors
///
/// Returns an error if a path is invalid, a file cannot be read or decoded, its listing
/// metadata changes, or alignment fails or is cancelled.
pub fn text_diff(left: &Location, right: &Location, relative: &Path, task: &Task) -> Result<Diff> {
    let relative = crate::filesystem::portable_relative(relative)?;
    fn load(root: &Location, relative: &Path, task: &Task) -> Result<String> {
        let path = crate::directory_tree::path(root).join(relative);
        let entry = crate::remote::entry_at(root, &path)?
            .ok_or_else(|| anyhow::anyhow!("File disappeared"))?;
        ensure!(
            !entry.is_dir && !entry.is_symlink,
            "Text comparison requires two regular files"
        );
        if matches!(root, Location::Local(_)) {
            ensure!(
                std::fs::symlink_metadata(&path)?.is_file(),
                "Text comparison requires regular files"
            );
        }
        ensure!(
            !entry.size_known || entry.size <= 1024 * 1024,
            "Text comparison supports files up to 1 MiB per side"
        );
        let mut input = crate::storage::reader(root, &path)?;
        let mut bytes = Vec::new();
        let mut buffer = [0; 65_536];
        loop {
            task.check()?;
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            ensure!(
                bytes.len() + n <= 1024 * 1024,
                "Text comparison supports files up to 1 MiB per side"
            );
            bytes.extend_from_slice(&buffer[..n]);
        }
        let after = crate::remote::entry_at(root, &path)?
            .ok_or_else(|| anyhow::anyhow!("File disappeared"))?;
        ensure!(
            entry.size == after.size
                && entry.modified == after.modified
                && !after.is_dir
                && !after.is_symlink,
            "File metadata changed during text comparison; compare again"
        );
        let document = crate::text_document::Document::detect(bytes)?;
        ensure!(
            !document.text.contains('\0'),
            "Binary files cannot be compared as text"
        );
        Ok(document.text)
    }
    let a = load(left, &relative, task)?;
    let b = load(right, &relative, task)?;
    Ok(Diff {
        name: relative.display().to_string(),
        rows: align(&a, &b, task)?,
    })
}
/// Aligns lines by their longest common subsequence and pairs intervening edits into
/// side-by-side rows. Line terminators participate in matching.
///
/// # Errors
///
/// Returns an error on cancellation or when either input exceeds 4,000 lines or the
/// alignment table exceeds 4,100,000 cells.
pub fn align(left: &str, right: &str, task: &Task) -> Result<Vec<DiffRow>> {
    let a: Vec<_> = left.split_inclusive('\n').collect();
    let b: Vec<_> = right.split_inclusive('\n').collect();
    ensure!(
        a.len() <= 4000 && b.len() <= 4000 && (a.len() + 1) * (b.len() + 1) <= 4_100_000,
        "Too many lines for text comparison (up to 4,000 per side and 4 million alignment cells)"
    );
    let width = b.len() + 1;
    let mut lengths = vec![0u16; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        task.check()?;
        for j in (0..b.len()).rev() {
            lengths[i * width + j] = if a[i] == b[j] {
                1 + lengths[(i + 1) * width + j + 1]
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    let show = |index: usize, line: &str, changed: bool| {
        format!(
            "{} {:>4} {}",
            if changed { "!" } else { " " },
            index + 1,
            line.trim_end_matches(['\r', '\n'])
        )
    };
    let (mut i, mut j) = (0, 0);
    let mut rows = Vec::new();
    while i < a.len() || j < b.len() {
        task.check()?;
        if i < a.len() && j < b.len() && a[i] == b[j] {
            rows.push(DiffRow {
                left: show(i, a[i], false),
                right: show(j, b[j], false),
                changed: false,
            });
            i += 1;
            j += 1;
        } else {
            let (mut removed, mut added) = (Vec::new(), Vec::new());
            while (i < a.len() || j < b.len()) && !(i < a.len() && j < b.len() && a[i] == b[j]) {
                if i < a.len()
                    && (j == b.len() || lengths[(i + 1) * width + j] >= lengths[i * width + j + 1])
                {
                    removed.push(i);
                    i += 1;
                } else {
                    added.push(j);
                    j += 1;
                }
            }
            for n in 0..removed.len().max(added.len()) {
                rows.push(DiffRow {
                    left: removed
                        .get(n)
                        .map(|i| show(*i, a[*i], true))
                        .unwrap_or_default(),
                    right: added
                        .get(n)
                        .map(|j| show(*j, b[*j], true))
                        .unwrap_or_default(),
                    changed: true,
                });
            }
        }
    }
    Ok(rows)
}
