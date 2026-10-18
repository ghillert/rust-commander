use crate::{
    filesystem::{Entry, safe_relative},
    operations::{Task, exists},
};
use anyhow::{Context, Result, bail};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    SevenZ,
    Gzip,
    Bzip2,
    Xz,
    Extended,
}

pub fn format(path: &Path) -> Option<Format> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if name.ends_with(".zip") {
        Some(Format::Zip)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Some(Format::TarGz)
    } else if name.ends_with(".tar") {
        Some(Format::Tar)
    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
        Some(Format::TarBz2)
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        Some(Format::TarXz)
    } else if name.ends_with(".7z") {
        Some(Format::SevenZ)
    } else if name.ends_with(".gz") {
        Some(Format::Gzip)
    } else if name.ends_with(".bz2") {
        Some(Format::Bzip2)
    } else if name.ends_with(".xz") {
        Some(Format::Xz)
    } else if [".rar", ".iso", ".nrg", ".ar", ".deb", ".cpio", ".rpm"]
        .iter()
        .any(|ext| name.ends_with(ext))
    {
        Some(Format::Extended)
    } else {
        None
    }
}

struct Member {
    path: PathBuf,
    size: u64,
    directory: bool,
    link: bool,
    regular: bool,
    mode: Option<u32>,
    modified: Option<SystemTime>,
}

fn decompressor(file: &Path, kind: Format) -> Result<Box<dyn Read>> {
    let input = File::open(file)?;
    Ok(match kind {
        Format::TarGz | Format::Gzip => Box::new(GzDecoder::new(input)),
        Format::TarBz2 | Format::Bzip2 => Box::new(bzip2::read::BzDecoder::new(input)),
        Format::TarXz | Format::Xz => Box::new(xz2::read::XzDecoder::new(input)),
        _ => Box::new(input),
    })
}

fn visit(file: &Path, mut callback: impl FnMut(Member, &mut dyn Read) -> Result<()>) -> Result<()> {
    match format(file).context("Supported archives: .zip, .tar, .tar.gz and .tgz")? {
        Format::Extended => visit_extended(file, callback)?,
        Format::Zip => {
            let mut archive = ZipArchive::new(File::open(file)?)?;
            for i in 0..archive.len() {
                let password = password(file);
                let mut entry = match password.as_deref() {
                    Some(password) => archive.by_index_decrypt(i, password.as_bytes())?,
                    None => archive.by_index(i)?,
                };
                let path = safe_relative(Path::new(entry.name()))?;
                let directory = entry.is_dir();
                let link = entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000);
                let member = Member {
                    path,
                    size: entry.size(),
                    directory,
                    link,
                    regular: !directory && !link,
                    mode: entry.unix_mode(),
                    modified: entry.last_modified().and_then(|date| {
                        chrono::NaiveDate::from_ymd_opt(
                            date.year() as i32,
                            date.month() as u32,
                            date.day() as u32,
                        )?
                        .and_hms_opt(
                            date.hour() as u32,
                            date.minute() as u32,
                            date.second() as u32,
                        )?
                        .and_local_timezone(chrono::Local)
                        .earliest()
                        .map(SystemTime::from)
                    }),
                };
                callback(member, &mut entry)?;
            }
        }
        Format::SevenZ => {
            let mut reader = sevenz_rust::SevenZReader::open(
                file,
                sevenz_rust::Password::from(password(file).as_deref().unwrap_or("")),
            )?;
            reader.for_each_entries(|entry, input| {
                let result = (|| -> Result<()> {
                    let path = safe_relative(Path::new(&entry.name))?;
                    callback(
                        Member {
                            path,
                            size: entry.size,
                            directory: entry.is_directory,
                            link: false,
                            regular: !entry.is_directory && !entry.is_anti_item,
                            mode: None,
                            modified: None,
                        },
                        input,
                    )
                })();
                result
                    .map(|()| true)
                    .map_err(|e| sevenz_rust::Error::other(e.to_string()))
            })?;
        }
        kind @ (Format::Gzip | Format::Bzip2 | Format::Xz) => {
            let limit = 10 * 1024 * 1024 * 1024;
            let size = std::io::copy(
                &mut decompressor(file, kind)?.take(limit + 1),
                &mut std::io::sink(),
            )?;
            if size > limit {
                bail!("Compressed stream exceeds the 10 GiB extraction limit.");
            }
            let path = safe_relative(Path::new(
                file.file_stem().context("Missing stream filename")?,
            ))?;
            callback(
                Member {
                    path,
                    size,
                    directory: false,
                    link: false,
                    regular: true,
                    mode: None,
                    modified: None,
                },
                &mut decompressor(file, kind)?,
            )?;
        }
        kind => {
            let reader = decompressor(file, kind)?;
            let mut archive = tar::Archive::new(reader);
            for entry in archive.entries()? {
                let mut entry = entry?;
                let raw = entry.path()?.into_owned();
                // A root '.' directory is a common TAR convention.
                if raw == Path::new(".") || raw == Path::new("./") {
                    continue;
                }
                let path = safe_relative(&raw)?;
                let kind = entry.header().entry_type();
                let member = Member {
                    path,
                    size: entry.size(),
                    directory: kind.is_dir(),
                    link: kind.is_symlink() || kind.is_hard_link(),
                    regular: kind.is_file(),
                    mode: entry.header().mode().ok(),
                    modified: entry.header().mtime().ok().and_then(|seconds| {
                        SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
                    }),
                };
                callback(member, &mut entry)?;
            }
        }
    }
    Ok(())
}

pub fn list(file: &Path, prefix: &Path) -> Result<Vec<Entry>> {
    let mut entries: BTreeMap<PathBuf, Entry> = BTreeMap::new();
    visit(file, |member, _| {
        if let Ok(relative) = member.path.strip_prefix(prefix)
            && let Some(first) = relative.components().next()
        {
            let path = prefix.join(first.as_os_str());
            let directory = relative.components().count() > 1 || member.directory;
            let item = entries.entry(path.clone()).or_insert_with(|| Entry {
                name: first.as_os_str().to_string_lossy().into_owned(),
                path: path.clone(),
                is_dir: directory,
                is_symlink: member.link,
                size: if directory { 0 } else { member.size },
                modified: if member.path == path {
                    member.modified
                } else {
                    None
                },
            });
            if directory {
                item.is_dir = true;
                item.size = 0;
            }
        }
        Ok(())
    })?;
    Ok(entries.into_values().collect())
}

fn included(path: &Path, selected: &[PathBuf], prefix: &Path) -> bool {
    path.starts_with(prefix)
        && (selected.is_empty() || selected.iter().any(|s| path == s || path.starts_with(s)))
}

/// Extracts into a new directory only. No archive links or existing files are followed.
pub fn extract(
    file: &Path,
    selected: &[PathBuf],
    prefix: &Path,
    destination: &Path,
    task: &mut Task,
) -> Result<()> {
    if exists(destination) {
        bail!("Extraction folder already exists. Choose a new folder name.");
    }
    if !prefix.as_os_str().is_empty() {
        safe_relative(prefix)?;
    }
    for path in selected {
        safe_relative(path)?;
    }
    let mut count = 0usize;
    let mut bytes = 0u64;
    visit(file, |member, _| {
        task.check()?;
        if !included(&member.path, selected, prefix) {
            return Ok(());
        }
        if member.link || (!member.directory && !member.regular) {
            bail!(
                "Archive links and special files cannot be extracted: {}",
                member.path.display()
            );
        }
        count += 1;
        bytes = bytes
            .checked_add(member.size)
            .context("Archive size overflow")?;
        if count > 100_000 || bytes > 10 * 1024 * 1024 * 1024 {
            bail!("Archive exceeds the extraction limit (100,000 entries or 10 GiB).");
        }
        Ok(())
    })?;
    if count == 0 {
        bail!("No archive entries matched the selection.");
    }
    fs::create_dir(destination).context("Cannot create the new extraction folder")?;
    let mut extracted_count = 0usize;
    let mut extracted_bytes = 0u64;
    let mut directories = Vec::new();
    let result = visit(file, |member, input| {
        task.check()?;
        if !included(&member.path, selected, prefix) {
            return Ok(());
        }
        if member.link || (!member.directory && !member.regular) {
            bail!("Archive changed during extraction.");
        }
        extracted_count += 1;
        extracted_bytes = extracted_bytes
            .checked_add(member.size)
            .context("Archive size overflow")?;
        if extracted_count > 100_000 || extracted_bytes > 10 * 1024 * 1024 * 1024 {
            bail!("Archive exceeds the extraction limit (100,000 entries or 10 GiB).");
        }
        let relative = member.path.strip_prefix(prefix)?;
        if relative.as_os_str().is_empty() {
            return Ok(());
        }
        let target = destination.join(safe_relative(relative)?);
        if member.directory {
            fs::create_dir_all(&target)?;
            directories.push((target, member.mode));
        } else {
            fs::create_dir_all(target.parent().context("Missing extraction parent")?)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            let mut bounded = input.take(member.size.saturating_add(1));
            let size = task.stream(&mut bounded, &mut output, &member.path)?;
            if size != member.size {
                bail!("Archive member size does not match its header.");
            }
            output.sync_all()?;
            if let Some(modified) = member.modified {
                output.set_times(fs::FileTimes::new().set_modified(modified))?;
            }
            restore_permissions(&target, member.mode)?;
        }
        Ok(())
    });
    let result = result.and_then(|()| {
        directories.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
        for (path, mode) in directories {
            restore_permissions(&path, mode)?;
        }
        Ok(())
    });
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn restore_permissions(path: &Path, mode: Option<u32>) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(mode) = mode {
            fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn walk(
    source: &Path,
    name: &Path,
    task: &mut Task,
    callback: &mut dyn FnMut(&Path, &Path, bool, &mut Task) -> Result<()>,
) -> Result<()> {
    task.tick(source, 0)?;
    safe_relative(name)?;
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        bail!(
            "Archive creation does not follow symlinks: {}",
            source.display()
        );
    }
    if !metadata.is_file() && !metadata.is_dir() {
        bail!("Cannot archive special files: {}", source.display());
    }
    callback(source, name, metadata.is_dir(), task)?;
    if metadata.is_dir() {
        let mut children = fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
        children.sort_by_key(|e| e.file_name());
        for child in children {
            walk(&child.path(), &name.join(child.file_name()), task, callback)?;
        }
    }
    Ok(())
}

fn pack_zip<W: Write + Seek>(
    writer: W,
    sources: &[PathBuf],
    password: Option<&str>,
    task: &mut Task,
) -> Result<W> {
    let mut archive = ZipWriter::new(writer);
    for source in sources {
        let name = Path::new(
            source
                .file_name()
                .context("Cannot archive a filesystem root")?,
        );
        walk(source, name, task, &mut |source, name, directory, task| {
            let name = name
                .to_str()
                .context("ZIP creation requires UTF-8 filenames")?
                .replace('\\', "/");
            let options = SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .large_file(true);
            let options = if let Some(password) = password {
                options.with_aes_encryption(zip::AesMode::Aes256, password)
            } else {
                options
            };
            #[cfg(unix)]
            let options = {
                use std::os::unix::fs::PermissionsExt;
                options.unix_permissions(fs::metadata(source)?.permissions().mode() & 0o777)
            };
            if directory {
                archive.add_directory(format!("{name}/"), options)?;
            } else {
                archive.start_file(name, options)?;
                task.stream(&mut File::open(source)?, &mut archive, source)?;
            }
            Ok(())
        })?;
    }
    Ok(archive.finish()?)
}

struct CancellableReader<'a> {
    input: File,
    task: &'a mut Task,
    path: &'a Path,
}
impl Read for CancellableReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.task.check().map_err(std::io::Error::other)?;
        let n = self.input.read(buffer)?;
        self.task
            .tick(self.path, n as u64)
            .map_err(std::io::Error::other)?;
        Ok(n)
    }
}

fn pack_tar<W: Write>(writer: W, sources: &[PathBuf], task: &mut Task) -> Result<W> {
    let mut archive = tar::Builder::new(writer);
    for source in sources {
        let name = Path::new(
            source
                .file_name()
                .context("Cannot archive a filesystem root")?,
        );
        walk(source, name, task, &mut |source, name, directory, task| {
            if directory {
                archive.append_dir(name, source)?;
            } else {
                let mut header = tar::Header::new_gnu();
                header.set_metadata(&fs::metadata(source)?);
                header.set_cksum();
                let reader = CancellableReader {
                    input: File::open(source)?,
                    task,
                    path: source,
                };
                archive.append_data(&mut header, name, reader)?;
                task.progress.files += 1;
            }
            Ok(())
        })?;
    }
    Ok(archive.into_inner()?)
}

fn pack_impl(
    sources: &[PathBuf],
    destination: &Path,
    password: Option<&str>,
    task: &mut Task,
) -> Result<()> {
    if exists(destination) {
        bail!("Archive already exists. Choose a different name.");
    }
    let kind = format(destination).context("Use a .zip, .tar, .tar.gz or .tgz filename")?;
    if kind == Format::Extended {
        bail!(
            "This archive format is available for reading only. Create a ZIP, 7z or TAR archive instead."
        );
    }
    let parent = destination
        .parent()
        .context("Missing archive parent folder")?
        .canonicalize()?;
    let mut names = Vec::new();
    for source in sources {
        let name = source
            .file_name()
            .context("Cannot archive a filesystem root")?;
        if names.contains(&name) {
            bail!("Selected items have duplicate names.");
        }
        names.push(name);
        if fs::symlink_metadata(source)?.is_dir() && parent.starts_with(source.canonicalize()?) {
            bail!("Create the archive outside the selected source folder.");
        }
    }
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    match kind {
        Format::Extended => unreachable!(),
        Format::Zip => {
            pack_zip(&mut output, sources, password, task)?;
        }
        Format::Tar => {
            pack_tar(&mut output, sources, task)?;
        }
        Format::TarGz => {
            pack_tar(
                GzEncoder::new(&mut output, Compression::default()),
                sources,
                task,
            )?
            .finish()?;
        }
        Format::TarBz2 => {
            pack_tar(
                bzip2::write::BzEncoder::new(&mut output, bzip2::Compression::default()),
                sources,
                task,
            )?
            .finish()?;
        }
        Format::TarXz => {
            pack_tar(xz2::write::XzEncoder::new(&mut output, 6), sources, task)?.finish()?;
        }
        Format::SevenZ => {
            let mut archive = sevenz_rust::SevenZWriter::new(&mut output)?;
            if let Some(password) = password {
                archive.set_content_methods(vec![
                    sevenz_rust::AesEncoderOptions::new(sevenz_rust::Password::from(password))
                        .into(),
                    sevenz_rust::SevenZMethod::LZMA2.into(),
                ]);
            }
            for source in sources {
                let name = Path::new(source.file_name().context("Missing filename")?);
                walk(source, name, task, &mut |source, name, directory, task| {
                    let entry = sevenz_rust::SevenZArchiveEntry::from_path(
                        source,
                        name.to_str()
                            .context("7z creation requires UTF-8 filenames")?
                            .replace('\\', "/"),
                    );
                    if directory {
                        archive.push_archive_entry::<File>(entry, None)?;
                    } else {
                        archive.push_archive_entry(
                            entry,
                            Some(CancellableReader {
                                input: File::open(source)?,
                                task,
                                path: source,
                            }),
                        )?;
                        task.progress.files += 1;
                    }
                    Ok(())
                })?;
            }
            archive.finish()?;
        }
        kind @ (Format::Gzip | Format::Bzip2 | Format::Xz) => {
            if sources.len() != 1 || !fs::symlink_metadata(&sources[0])?.is_file() {
                bail!(
                    "Stream compression requires one regular file; use a TAR archive for folders or multiple files."
                );
            }
            let mut input = File::open(&sources[0])?;
            match kind {
                Format::Gzip => {
                    let mut encoder = GzEncoder::new(&mut output, Compression::default());
                    task.stream(&mut input, &mut encoder, &sources[0])?;
                    encoder.finish()?;
                }
                Format::Bzip2 => {
                    let mut encoder =
                        bzip2::write::BzEncoder::new(&mut output, bzip2::Compression::default());
                    task.stream(&mut input, &mut encoder, &sources[0])?;
                    encoder.finish()?;
                }
                Format::Xz => {
                    let mut encoder = xz2::write::XzEncoder::new(&mut output, 6);
                    task.stream(&mut input, &mut encoder, &sources[0])?;
                    encoder.finish()?;
                }
                _ => unreachable!(),
            }
        }
    }
    task.check()?;
    output.as_file().sync_all()?;
    output.persist_noclobber(destination).map_err(|e| e.error)?;
    Ok(())
}

struct ArchiveEntryReader<'a> {
    archive: &'a mut compress_tools::ArchiveIterator<File>,
    buffer: std::io::Cursor<Vec<u8>>,
    ended: bool,
}
impl Read for ArchiveEntryReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let count = self.buffer.read(output)?;
            if count != 0 || self.ended || output.is_empty() {
                return Ok(count);
            }
            match self.archive.next() {
                Some(compress_tools::ArchiveContents::DataChunk(bytes)) => {
                    self.buffer = std::io::Cursor::new(bytes)
                }
                Some(compress_tools::ArchiveContents::EndOfEntry) | None => self.ended = true,
                Some(compress_tools::ArchiveContents::Err(error)) => {
                    return Err(std::io::Error::other(error));
                }
                _ => return Err(std::io::Error::other("Unexpected archive entry boundary")),
            }
        }
    }
}

fn visit_extended(
    file: &Path,
    mut callback: impl FnMut(Member, &mut dyn Read) -> Result<()>,
) -> Result<()> {
    let mut archive = compress_tools::ArchiveIteratorBuilder::new(File::open(file)?)
        .mtree_format(false)
        .build()?;
    while let Some(header) = archive.next_header() {
        match header {
            compress_tools::ArchiveContents::StartOfEntry(name, stat) => {
                if name == "." || name == "./" {
                    continue;
                }
                let path = safe_relative(Path::new(&name))?;
                // libc uses u16 for st_mode on macOS/Windows and u32 on Linux.
                #[allow(clippy::unnecessary_cast)]
                let mode = stat.st_mode as u32;
                let directory = mode & 0o170000 == 0o040000;
                let regular = mode & 0o170000 == 0o100000;
                let link = mode & 0o170000 == 0o120000 || (regular && stat.st_nlink > 1);
                let member = Member {
                    path,
                    size: u64::try_from(stat.st_size).context("Invalid archive member size")?,
                    directory,
                    regular,
                    link,
                    mode: Some(mode),
                    modified: u64::try_from(stat.st_mtime)
                        .ok()
                        .and_then(|s| SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(s))),
                };
                let mut reader = ArchiveEntryReader {
                    archive: &mut archive,
                    buffer: std::io::Cursor::new(Vec::new()),
                    ended: false,
                };
                callback(member, &mut reader)?;
            }
            compress_tools::ArchiveContents::Err(error) => return Err(error.into()),
            _ => bail!("Unexpected archive header"),
        }
    }
    Ok(())
}

pub fn read_bytes(file: &Path, path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut result = None;
    visit(file, |member, input| {
        if member.path == path {
            if member.directory || member.link || !member.regular {
                bail!("Select a regular text file.");
            }
            if member.size > limit {
                bail!("This archive entry is too large for the text viewer.");
            }
            let mut bytes = Vec::new();
            input.take(limit + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > limit {
                bail!("This entry is too large for the viewer.");
            }
            result = Some(bytes);
        }
        Ok(())
    })?;
    result.context("Archive entry no longer exists")
}

pub fn read_text(file: &Path, path: &Path, limit: u64) -> Result<String> {
    let bytes = read_bytes(file, path, limit)?;
    if bytes.contains(&0) {
        bail!("This archive entry is binary.");
    }
    String::from_utf8(bytes).context("This archive entry is not UTF-8")
}

static PASSWORDS: std::sync::OnceLock<
    std::sync::RwLock<std::collections::BTreeMap<PathBuf, String>>,
> = std::sync::OnceLock::new();
pub(crate) fn password(path: &Path) -> Option<String> {
    PASSWORDS
        .get_or_init(Default::default)
        .read()
        .ok()?
        .get(path)
        .cloned()
}
pub fn unlock(path: &Path, secret: &str) -> Result<()> {
    let passwords = PASSWORDS.get_or_init(Default::default);
    let previous = passwords
        .write()
        .map_err(|_| anyhow::anyhow!("Archive password lock failed"))?
        .insert(path.into(), secret.into());
    let mut count = 0usize;
    let mut bytes = 0u64;
    let result = visit(path, |member, input| {
        count += 1;
        bytes = bytes.saturating_add(member.size);
        if count > 100_000 || bytes > 10 * 1024 * 1024 * 1024 {
            bail!("Archive exceeds the validation limit.");
        }
        if member.regular {
            let copied = std::io::copy(
                &mut input.take(member.size.saturating_add(1)),
                &mut std::io::sink(),
            )?;
            if copied != member.size {
                bail!("Archive entry has an invalid size.");
            }
        }
        Ok(())
    });
    if let Err(error) = result {
        let mut passwords = passwords
            .write()
            .map_err(|_| anyhow::anyhow!("Archive password lock failed"))?;
        if let Some(previous) = previous {
            passwords.insert(path.into(), previous);
        } else {
            passwords.remove(path);
        }
        return Err(error);
    }
    Ok(())
}
pub fn pack(sources: &[PathBuf], destination: &Path, task: &mut Task) -> Result<()> {
    pack_impl(sources, destination, None, task)
}
pub fn pack_with_password(
    sources: &[PathBuf],
    destination: &Path,
    secret: &str,
    task: &mut Task,
) -> Result<()> {
    if secret.is_empty() {
        bail!("Enter an archive password.");
    }
    if !matches!(format(destination), Some(Format::Zip | Format::SevenZ)) {
        bail!("Encryption is supported for ZIP and 7z archives.");
    }
    pack_impl(sources, destination, Some(secret), task)
}
pub fn extract_with_password(
    file: &Path,
    selected: &[PathBuf],
    prefix: &Path,
    destination: &Path,
    secret: &str,
    task: &mut Task,
) -> Result<()> {
    unlock(file, secret)?;
    extract(file, selected, prefix, destination, task)
}
