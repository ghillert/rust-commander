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
    pub encrypted: bool,
    pub password: String,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            format: Format::Zip,
            compression_level: None,
            comment: String::new(),
            background: false,
            expanded: false,
            encrypted: false,
            password: String::new(),
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
        if !matches!(self.format, Format::Zip | Format::SevenZ) {
            self.encrypted = false;
            self.password.clear();
        }
        if !destination.trim().is_empty() {
            // A Windows PathBuf join inserts backslashes. Keep URI separators,
            // escaping and any query/fragment intact while replacing the filename.
            let value = destination.trim();
            let uri = value.split_once("://").filter(|(scheme, _)| {
                scheme.len() > 1
                    && scheme
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"+.-".contains(&c))
            });
            if let Some((scheme, _)) = uri {
                let boundary = value.find(['?', '#']).unwrap_or(value.len());
                if let Some(slash) = value[..boundary].rfind('/')
                    && slash >= scheme.len() + 3
                {
                    let name = percent_encoding::percent_decode_str(&value[slash + 1..boundary])
                        .decode_utf8()
                        .context("Archive URI filenames must contain valid UTF-8")?;
                    let normalized = self.destination(Path::new(name.as_ref()));
                    const URI_FILENAME: &percent_encoding::AsciiSet =
                        &percent_encoding::NON_ALPHANUMERIC
                            .remove(b'.')
                            .remove(b'-')
                            .remove(b'_')
                            .remove(b'~');
                    *destination = format!(
                        "{}/{}{}",
                        &value[..slash],
                        percent_encoding::utf8_percent_encode(
                            &normalized.to_string_lossy(),
                            URI_FILENAME
                        ),
                        &value[boundary..]
                    );
                }
            } else {
                *destination = self
                    .destination(Path::new(destination))
                    .display()
                    .to_string();
            }
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
        self.validate_encryption()?;
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
        self.validate_encryption()?;
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
    fn validate_encryption(&self) -> Result<()> {
        if self.encrypted {
            ensure!(
                matches!(self.format, Format::Zip | Format::SevenZ),
                "Encryption requires ZIP or 7z."
            );
            ensure!(!self.password.is_empty(), "Enter an archive password.");
        }
        Ok(())
    }
}

/// Resolve local paths or a URI belonging to an already configured connection.
/// Do not put inline URI credentials into queue recipes.
pub fn destination(
    value: &str,
    base: &Path,
    options: &Options,
    connections: &[crate::remote::Connection],
) -> Result<crate::filesystem::Location> {
    use crate::{filesystem::Location, uri::Input};
    let location = match crate::uri::parse(value, &Location::Local(base.into()))? {
        Input::Location(Location::Archive { file, prefix }) if prefix.as_os_str().is_empty() => {
            Location::Local(file)
        }
        Input::Location(location) => location,
        Input::Remote(target) => {
            ensure!(
                target.secrets.password.is_empty() && target.secrets.request_query.is_empty(),
                "Use a configured connection instead of credentials in the archive destination."
            );
            let matches: Vec<_> = connections
                .iter()
                .filter(|c| crate::uri::matches_profile(&target, c))
                .collect();
            ensure!(
                matches.len() == 1,
                "Choose an unambiguous configured remote connection for the archive destination."
            );
            Location::Remote {
                connection: matches[0].id.clone(),
                path: target.path,
            }
        }
    };
    Ok(match location {
        Location::Local(path) => Location::Local(options.destination(&path)),
        Location::Remote { connection, path } => {
            ensure!(
                crate::remote::profile(&connection).is_some_and(|c| c.writable()),
                "Choose a writable configured connection."
            );
            Location::Remote {
                connection,
                path: options.destination(&path),
            }
        }
        Location::Archive { .. } => {
            anyhow::bail!("Choose a local or remote archive destination, outside another archive.")
        }
    })
}

pub fn pack_to(
    selection: &crate::drag_drop::Selection,
    target: &crate::filesystem::Location,
    options: &Options,
    task: &mut crate::operations::Task,
) -> Result<()> {
    use crate::filesystem::Location;
    options.validate_selection(selection)?;
    let path = match target {
        Location::Local(path) | Location::Remote { path, .. } => path,
        _ => anyhow::bail!("Unsupported archive destination"),
    };
    crate::remote::ensure_absent(target, path)?;
    let stage = tempfile::Builder::new()
        .prefix("rust-commander-pack-")
        .tempdir()?;
    let export = if matches!(selection.from, Location::Local(_)) {
        None
    } else {
        Some(crate::drag_drop::transient(task, stage.path(), |task| {
            crate::drag_drop::export_in(selection, task, Some(stage.path()))
        })?)
    };
    let sources = export.as_ref().map_or_else(
        || selection.entries.iter().map(|e| e.path.clone()).collect(),
        |export| export.paths.clone(),
    );
    options.validate(&sources)?;
    let output_dir = stage.path().join("output");
    std::fs::create_dir(&output_dir)?;
    let output = output_dir.join(path.file_name().context("Enter an archive filename")?);
    let output = if let Location::Local(path) = target {
        task.capture(path)?;
        path.as_path()
    } else {
        output.as_path()
    };
    if options.encrypted {
        archive::pack_with_password_options(
            &sources,
            output,
            &options.password,
            options.comment(),
            options.compression_level,
            task,
        )?;
    } else {
        archive::pack_with_compression(
            &sources,
            output,
            options.comment(),
            options.compression_level,
            task,
        )?;
    }
    if let Location::Remote { connection, path } = target {
        let remote_stage = path.parent().unwrap_or(Path::new("")).join(format!(
            ".rust-commander-pack-{:032x}",
            rand::random::<u128>()
        ));
        let entry = crate::filesystem::entry_from_path(output)?;
        let result = (|| -> Result<()> {
            crate::remote::copy_entry(
                &Location::Local(stage.path().into()),
                &entry,
                target,
                &remote_stage,
                task,
            )?;
            let local = std::fs::File::open(output)?;
            let remote = crate::remote::read(connection, &remote_stage)?;
            ensure!(
                digest(local, path, task)? == digest(remote, path, task)?,
                "Uploaded archive failed integrity verification."
            );
            crate::remote::publish_checkpoint(connection, &remote_stage, path, task)
        })();
        if result.is_err()
            && let Ok(Some(entry)) = crate::remote::entry_at(target, &remote_stage)
        {
            let _ = crate::remote::remove_entry(
                target,
                &entry,
                &mut crate::operations::Task::default(),
            );
        }
        result?;
    }
    Ok(())
}
fn digest(
    mut input: impl std::io::Read,
    path: &Path,
    task: &mut crate::operations::Task,
) -> Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        task.check()?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        task.tick(path, n as u64)?;
    }
    Ok(hash.finalize().into())
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
