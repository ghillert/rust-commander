//! Format choices and validation shared by the Pack files dialog and worker.
use crate::archive::{self, Format};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

pub const FORMATS: [(Format, &str, &str); 9] = [
    (Format::Zip, "ZIP", ".zip"),
    (Format::SevenZ, "7z", ".7z"),
    (Format::Tar, "TAR", ".tar"),
    (Format::TarGz, "TAR.GZ", ".tar.gz"),
    (Format::TarBz2, "TAR.BZ2", ".tar.bz2"),
    (Format::TarXz, "TAR.XZ", ".tar.xz"),
    (Format::Gzip, "GZip (one file)", ".gz"),
    (Format::Bzip2, "BZip2 (one file)", ".bz2"),
    (Format::Xz, "XZ (one file)", ".xz"),
];

pub struct Options {
    pub format: Format,
    pub compression_level: Option<u8>,
    pub comment: String,
    pub background: bool,
    pub expanded: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            format: Format::Zip,
            compression_level: None,
            comment: String::new(),
            background: false,
            expanded: false,
        }
    }
}
impl Options {
    pub fn label(&self) -> &'static str {
        FORMATS
            .iter()
            .find(|(f, _, _)| *f == self.format)
            .unwrap()
            .1
    }
    pub fn set_format(&mut self, label: &str, destination: &mut String) -> Result<()> {
        self.format = FORMATS
            .iter()
            .find(|(_, name, _)| *name == label)
            .context("Choose a supported archive format.")?
            .0;
        if validate_level(self.format, self.compression_level).is_err() {
            self.compression_level = None;
        }
        if !destination.trim().is_empty() {
            *destination = self
                .destination(Path::new(destination))
                .display()
                .to_string();
        }
        Ok(())
    }
    pub fn destination(&self, path: &Path) -> PathBuf {
        if archive::format(path) == Some(self.format) {
            return path.to_owned();
        }
        let Some(name) = path.file_name() else {
            return path.to_owned();
        };
        let mut name = name.to_os_string();
        // Remove the complete compound suffix before changing formats.
        let lower = name.to_string_lossy().to_lowercase();
        for suffix in [
            ".tar.bz2", ".tar.gz", ".tar.xz", ".tbz2", ".tgz", ".txz", ".zip", ".7z", ".tar",
            ".gz", ".bz2", ".xz",
        ] {
            if lower.ends_with(suffix) {
                for _ in 0..if suffix.starts_with(".tar.") { 2 } else { 1 } {
                    name = Path::new(&name).file_stem().unwrap_or(&name).to_os_string();
                }
                break;
            }
        }
        name.push(
            FORMATS
                .iter()
                .find(|(f, _, _)| *f == self.format)
                .unwrap()
                .2,
        );
        path.with_file_name(name)
    }
    pub fn comment(&self) -> &str {
        if self.format == Format::Zip {
            &self.comment
        } else {
            ""
        }
    }
    pub fn validate(&self, sources: &[PathBuf]) -> Result<()> {
        ensure!(
            !sources.is_empty(),
            "Select at least one file or folder to pack."
        );
        validate_comment(self.format, self.comment())?;
        validate_level(self.format, self.compression_level)?;
        if matches!(self.format, Format::Gzip | Format::Bzip2 | Format::Xz) {
            ensure!(
                sources.len() == 1 && std::fs::symlink_metadata(&sources[0])?.is_file(),
                "Stream compression requires one regular file; use a TAR archive for folders or multiple files."
            );
        }
        Ok(())
    }
    pub fn validate_selection(&self, selection: &crate::drag_drop::Selection) -> Result<()> {
        selection.validate()?;
        validate_comment(self.format, self.comment())?;
        validate_level(self.format, self.compression_level)?;
        if matches!(self.format, Format::Gzip | Format::Bzip2 | Format::Xz) {
            ensure!(
                selection.entries.len() == 1
                    && !selection.entries[0].is_dir
                    && !selection.entries[0].is_symlink,
                "Stream compression requires one regular file; use a TAR archive for folders or multiple files."
            );
        }
        Ok(())
    }
}

/// Materialize a verified virtual selection privately, then pack to a local
/// destination. Only the finished destination participates in durable Undo.
pub fn pack_selection(
    selection: &crate::drag_drop::Selection,
    destination: &Path,
    comment: &str,
    compression_level: Option<u8>,
    task: &mut crate::operations::Task,
) -> Result<()> {
    let options = Options {
        format: archive::format(destination).context("Choose a supported archive format")?,
        compression_level,
        comment: comment.into(),
        ..Default::default()
    };
    options.validate_selection(selection)?;
    validate_comment(options.format, comment)?;
    ensure!(
        destination.is_absolute(),
        "Choose a local archive destination"
    );
    ensure!(
        !crate::operations::exists(destination),
        "Archive already exists. Choose a different name."
    );
    task.check()?;
    let stage = tempfile::Builder::new()
        .prefix("rust-commander-pack-")
        .tempdir()?;
    let exported = crate::drag_drop::transient(task, stage.path(), |task| {
        crate::drag_drop::export_in(selection, task, Some(stage.path()))
    })?;
    task.check()?;
    options.validate(&exported.paths)?;
    task.capture(destination)?;
    archive::pack_with_compression(
        &exported.paths,
        destination,
        comment,
        compression_level,
        task,
    )
}

/// Zero means storage for ZIP, no deflate for GZip, and the fastest preset
/// for 7z/XZ. BZip2 has no level zero; plain TAR has no compression settings.
pub fn validate_level(format: Format, level: Option<u8>) -> Result<()> {
    if let Some(level) = level {
        ensure!(
            !matches!(format, Format::Tar | Format::Extended),
            "This archive format has no compression levels."
        );
        let minimum = if matches!(format, Format::TarBz2 | Format::Bzip2) {
            1
        } else {
            0
        };
        ensure!(
            (minimum..=9).contains(&level),
            "Choose a compression level from {minimum} to 9."
        );
    }
    Ok(())
}

pub fn level_label(format: Format, level: Option<u8>) -> String {
    match level {
        None => "Default".into(),
        Some(0) => match format {
            Format::Zip => "0 — Store only".into(),
            Format::Gzip | Format::TarGz => "0 — No compression".into(),
            _ => "0 — Fastest".into(),
        },
        Some(1) if !matches!(format, Format::SevenZ | Format::Xz | Format::TarXz) => {
            "1 — Fastest".into()
        }
        Some(6) => "6 — Normal".into(),
        Some(9) => "9 — Best compression".into(),
        Some(level) => level.to_string(),
    }
}

pub fn level_choices(format: Format) -> Vec<String> {
    std::iter::once(None)
        .chain((0..=9).map(Some))
        .filter(|level| validate_level(format, *level).is_ok())
        .map(|level| level_label(format, level))
        .collect()
}

pub fn parse_level(format: Format, label: &str) -> Result<Option<u8>> {
    std::iter::once(None)
        .chain((0..=9).map(Some))
        .find(|level| {
            validate_level(format, *level).is_ok() && level_label(format, *level) == label
        })
        .context("Choose a supported compression level.")
}

pub fn validate_comment(format: Format, comment: &str) -> Result<()> {
    ensure!(
        comment.is_empty() || format == Format::Zip,
        "Archive comments are supported for ZIP only."
    );
    ensure!(
        comment.len() <= u16::MAX as usize,
        "ZIP comments must be at most 65,535 UTF-8 bytes."
    );
    ensure!(
        !comment
            .as_bytes()
            .windows(4)
            .any(|bytes| bytes == b"PK\x05\x06"),
        "ZIP comments cannot contain an end-of-directory signature."
    );
    Ok(())
}
