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
    pub comment: String,
    pub background: bool,
    pub expanded: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            format: Format::Zip,
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
        if matches!(self.format, Format::Gzip | Format::Bzip2 | Format::Xz) {
            ensure!(
                sources.len() == 1 && std::fs::symlink_metadata(&sources[0])?.is_file(),
                "Stream compression requires one regular file; use a TAR archive for folders or multiple files."
            );
        }
        Ok(())
    }
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
