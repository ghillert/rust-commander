use crate::{
    filesystem::{Location, safe_relative},
    operations::{self, Task},
};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum ZipEdit {
    Add {
        sources: Vec<PathBuf>,
        prefix: PathBuf,
    },
    Remove(Vec<PathBuf>),
    Rename {
        source: PathBuf,
        target: PathBuf,
    },
}

pub fn edit_zip(file: &Path, edit: &ZipEdit, task: &mut Task) -> Result<()> {
    if crate::archive::format(file) != Some(crate::archive::Format::Zip) {
        bail!("Archive editing requires a local ZIP file.");
    }
    if origin(file).is_some() {
        bail!("Extract nested or remote archives locally before editing them.");
    }
    if fs::symlink_metadata(file)?.file_type().is_symlink() {
        bail!("Edit the archive's actual file, not a symlink.");
    }
    let stamp = crate::history::fingerprint(file, task)?;
    let mut input = ZipArchive::new(File::open(file)?)?;
    if input.len() > 100_000 {
        bail!("Archive exceeds the entry limit.");
    }
    let mut paths = BTreeSet::new();
    let mut found = false;
    let mut mapping = Vec::new();
    match edit {
        ZipEdit::Add { prefix, .. } if !prefix.as_os_str().is_empty() => {
            safe_relative(prefix)?;
        }
        ZipEdit::Remove(selected) => {
            if selected.is_empty() {
                bail!("Select entries to remove.");
            }
            for path in selected {
                safe_relative(path)?;
            }
        }
        ZipEdit::Rename { source, target } => {
            safe_relative(source)?;
            safe_relative(target)?;
        }
        _ => {}
    }
    for index in 0..input.len() {
        let entry = input.by_index_raw(index)?;
        if matches!(edit, ZipEdit::Add { .. }) && entry.encrypted() {
            bail!("Adding entries to an encrypted ZIP requires creating a new encrypted archive.");
        }
        let path = safe_relative(Path::new(entry.name()))?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            bail!("Archive editing refuses symlinks.");
        }
        let mut target = Some(path.clone());
        match edit {
            ZipEdit::Remove(selected)
                if selected.iter().any(|selected| path.starts_with(selected)) =>
            {
                target = None;
                found = true;
            }
            ZipEdit::Rename {
                source,
                target: new,
            } if path.starts_with(source) => {
                let suffix = path.strip_prefix(source)?;
                target = Some(if suffix.as_os_str().is_empty() {
                    new.clone()
                } else {
                    new.join(suffix)
                });
                found = true;
            }
            _ => {}
        }
        let target = target.map(|path| {
            format!(
                "{}{}",
                path.to_str().unwrap_or("").replace('\\', "/"),
                if entry.is_dir() { "/" } else { "" }
            )
        });
        if let Some(name) = &target
            && (name.is_empty() || !paths.insert(name.trim_end_matches('/').to_string()))
        {
            bail!("ZIP edit would create duplicate or invalid names.");
        }
        mapping.push(target);
    }
    if matches!(edit, ZipEdit::Remove(_) | ZipEdit::Rename { .. }) && !found {
        bail!("Selected archive entry was not found.");
    }
    let mut temporary =
        tempfile::NamedTempFile::new_in(file.parent().context("Missing archive folder")?)?;
    {
        let mut output = ZipWriter::new(temporary.as_file_mut());
        for (index, target) in mapping.into_iter().enumerate() {
            task.check()?;
            if let Some(target) = target {
                let entry = input.by_index_raw(index)?;
                let old = entry.name().to_string();
                if target == old {
                    output.raw_copy_file(entry)?;
                } else {
                    // Changing names of encrypted raw entries is supported by the ZIP writer.
                    output.raw_copy_file_rename(entry, target)?;
                }
            }
        }
        if let ZipEdit::Add { sources, prefix } = edit {
            operations::validate_sources(sources)?;
            fn add(
                path: &Path,
                name: &Path,
                output: &mut ZipWriter<&mut File>,
                names: &mut BTreeSet<String>,
                task: &mut Task,
            ) -> Result<()> {
                task.check()?;
                let metadata = fs::symlink_metadata(path)?;
                if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir())
                {
                    bail!("ZIP additions refuse links and special files.");
                }
                let name = safe_relative(name)?
                    .to_str()
                    .context("ZIP filenames must be UTF-8")?
                    .replace('\\', "/");
                if !names.insert(name.clone()) {
                    bail!("Archive already contains {name}.");
                }
                let options = SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .large_file(true);
                if metadata.is_dir() {
                    output.add_directory(format!("{name}/"), options)?;
                    for entry in fs::read_dir(path)? {
                        let entry = entry?;
                        add(
                            &entry.path(),
                            &Path::new(&name).join(entry.file_name()),
                            output,
                            names,
                            task,
                        )?;
                    }
                } else {
                    output.start_file(name, options)?;
                    task.stream(&mut File::open(path)?, output, path)?;
                }
                Ok(())
            }
            for source in sources {
                if fs::symlink_metadata(source)?.is_dir()
                    && file.canonicalize()?.starts_with(source.canonicalize()?)
                {
                    bail!("Do not add a folder containing the archive itself.");
                }
                if source.canonicalize()? == file.canonicalize()? {
                    bail!("An archive cannot contain itself.");
                }
                add(
                    source,
                    &prefix.join(source.file_name().context("Missing filename")?),
                    &mut output,
                    &mut paths,
                    task,
                )?;
            }
        }
        output.finish()?;
    }
    // A file cannot also be the parent of another entry, even after a rename.
    {
        let mut verify = ZipArchive::new(temporary.reopen()?)?;
        let mut entries = BTreeMap::new();
        for index in 0..verify.len() {
            let entry = verify.by_index_raw(index)?;
            entries.insert(safe_relative(Path::new(entry.name()))?, entry.is_dir());
        }
        for path in entries.keys() {
            for parent in path
                .ancestors()
                .skip(1)
                .filter(|path| !path.as_os_str().is_empty())
            {
                if entries.get(parent) == Some(&false) {
                    bail!("ZIP edit would place an entry inside a regular file.");
                }
            }
        }
    }
    task.check()?;
    if crate::history::fingerprint(file, task)? != stamp {
        bail!("Archive changed during editing; the original was kept.");
    }
    temporary
        .as_file()
        .set_permissions(fs::metadata(file)?.permissions())?;
    temporary.as_file().sync_all()?;
    temporary.persist(file)?;
    Ok(())
}

struct Materialized {
    _directory: tempfile::TempDir,
    file: PathBuf,
    origin: Location,
    depth: usize,
}
static MATERIALIZED: OnceLock<Mutex<BTreeMap<PathBuf, Materialized>>> = OnceLock::new();

pub fn origin(file: &Path) -> Option<Location> {
    MATERIALIZED
        .get_or_init(Default::default)
        .lock()
        .ok()?
        .get(file)
        .map(|entry| entry.origin.clone())
}

pub fn materialize(location: &Location, path: &Path) -> Result<PathBuf> {
    const LIMIT: u64 = 128 * 1024 * 1024;
    let depth = match location {
        Location::Archive { file, .. } => MATERIALIZED
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| anyhow::anyhow!("Archive cache lock failed"))?
            .get(file)
            .map_or(1, |entry| entry.depth + 1),
        _ => 1,
    };
    if depth > 8 {
        bail!("Nested archives are limited to eight levels.");
    }
    let bytes = match location {
        Location::Archive { file, .. } => crate::archive::read_bytes(file, path, LIMIT)?,
        Location::Remote { connection, .. } => crate::remote::read_bytes(connection, path, LIMIT)?,
        Location::Local(_) => {
            let mut data = Vec::new();
            File::open(path)?.take(LIMIT + 1).read_to_end(&mut data)?;
            data
        }
    };
    if bytes.len() as u64 > LIMIT {
        bail!("Nested/remote archive exceeds 128 MiB.");
    }
    let directory = tempfile::Builder::new()
        .prefix("rust-commander-archive-")
        .tempdir()?;
    let file = directory
        .path()
        .join(path.file_name().context("Missing archive name")?);
    File::create(&file)?.write_all(&bytes)?;
    let mut cache = MATERIALIZED
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("Archive cache lock failed"))?;
    let total = cache.values().try_fold(0u64, |total, entry| {
        Ok::<_, std::io::Error>(total + fs::metadata(&entry.file)?.len())
    })?;
    if total + bytes.len() as u64 > 512 * 1024 * 1024 {
        bail!("The nested archive cache has reached 512 MiB. Close the application to clear it.");
    }
    let origin = match location {
        Location::Archive { file, .. } => Location::Archive {
            file: file.clone(),
            prefix: path.parent().unwrap_or(Path::new("")).into(),
        },
        Location::Remote { connection, .. } => Location::Remote {
            connection: connection.clone(),
            path: path.parent().unwrap_or(Path::new("")).into(),
        },
        Location::Local(_) => Location::Local(path.parent().unwrap_or(Path::new("/")).into()),
    };
    cache.insert(
        file.clone(),
        Materialized {
            _directory: directory,
            file: file.clone(),
            origin,
            depth,
        },
    );
    Ok(file)
}

pub fn clear_cache() {
    if let Ok(mut cache) = MATERIALIZED.get_or_init(Default::default).lock() {
        cache.clear();
    }
}

pub fn editable(file: &Path) -> bool {
    origin(file).is_none()
        && matches!(
            crate::archive::format(file),
            Some(
                crate::archive::Format::Zip
                    | crate::archive::Format::Tar
                    | crate::archive::Format::TarGz
                    | crate::archive::Format::TarBz2
                    | crate::archive::Format::TarXz
                    | crate::archive::Format::SevenZ
            )
        )
}
/// Rewrite TAR/7z and encrypted ZIP through a private, bounded extraction tree.
pub fn edit_archive(file: &Path, edit: &ZipEdit, task: &mut Task) -> Result<()> {
    if !editable(file) || fs::symlink_metadata(file)?.file_type().is_symlink() {
        bail!("Choose a local ZIP, TAR or 7z archive; extract cached archives locally first.");
    }
    let kind = crate::archive::format(file).context("Unknown archive format")?;
    let encrypted_zip = if kind == crate::archive::Format::Zip {
        let mut zip = ZipArchive::new(File::open(file)?)?;
        let mut encrypted = false;
        for i in 0..zip.len() {
            encrypted |= zip.by_index_raw(i)?.encrypted();
        }
        encrypted
    } else {
        false
    };
    if kind == crate::archive::Format::Zip && !encrypted_zip {
        return edit_zip(file, edit, task);
    }
    let password = crate::archive::password(file);
    if encrypted_zip && password.is_none() {
        bail!("Unlock the encrypted archive in Archive tools before editing it.");
    }
    let stamp = crate::history::fingerprint(file, task)?;
    let directory = tempfile::Builder::new()
        .prefix(".rust-commander-archive-edit-")
        .tempdir_in(file.parent().context("Missing archive folder")?)?;
    let tree = directory.path().join("contents");
    if crate::archive::list(file, Path::new(""))?.is_empty() {
        fs::create_dir(&tree)?;
    } else {
        crate::archive::extract(file, &[], Path::new(""), &tree, task)?;
    }
    match edit {
        ZipEdit::Add { sources, prefix } => {
            if sources.is_empty() {
                bail!("Select files to add");
            }
            let parent = if prefix.as_os_str().is_empty() {
                tree.clone()
            } else {
                tree.join(safe_relative(prefix)?)
            };
            fs::create_dir_all(&parent)?;
            operations::validate_sources(sources)?;
            for source in sources {
                if source.canonicalize()? == file.canonicalize()?
                    || directory.path().starts_with(source.canonicalize()?)
                {
                    bail!("An archive cannot contain itself or its working directory");
                }
                let name = source.file_name().context("Cannot add a filesystem root")?;
                let target = parent.join(name);
                if operations::exists(&target) {
                    bail!("Archive entry already exists: {}", name.to_string_lossy());
                }
                operations::copy_item(source, &target, task)?;
            }
        }
        ZipEdit::Remove(paths) => {
            if paths.is_empty() {
                bail!("Select entries to remove");
            }
            let mut paths = paths
                .iter()
                .map(|p| safe_relative(p))
                .collect::<Result<Vec<_>>>()?;
            paths.sort_by_key(|p| p.components().count());
            let mut removed: Vec<PathBuf> = Vec::new();
            for p in paths {
                if removed.iter().any(|r| p.starts_with(r)) {
                    continue;
                }
                let target = tree.join(&p);
                if !operations::exists(&target) {
                    bail!("Archive entry missing: {}", p.display());
                }
                operations::remove(&target)?;
                removed.push(p);
            }
        }
        ZipEdit::Rename { source, target } => {
            let source = tree.join(safe_relative(source)?);
            let target = tree.join(safe_relative(target)?);
            if target.starts_with(&source) {
                bail!("Cannot move an archive folder inside itself");
            }
            fs::create_dir_all(target.parent().context("Missing entry parent")?)?;
            crate::desktop::rename_exclusive(&source, &target)?;
        }
    }
    let sources = fs::read_dir(&tree)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    let output = directory
        .path()
        .join(file.file_name().context("Missing archive filename")?);
    if let Some(password) = password.as_deref() {
        crate::archive::pack_with_password(&sources, &output, password, task)?;
    } else {
        crate::archive::pack(&sources, &output, task)?;
    }
    // Validate the produced container before replacing the original.
    if let Some(password) = password.as_deref() {
        crate::archive::unlock(&output, password)?;
    } else {
        crate::archive::list(&output, Path::new(""))?;
    }
    if stamp != crate::history::fingerprint(file, task)? {
        bail!("Archive changed during editing; the original was preserved");
    }
    task.capture(file)?;
    fs::set_permissions(&output, fs::metadata(file)?.permissions())?;
    let mut temporary = tempfile::NamedTempFile::new_in(file.parent().unwrap())?;
    task.stream(&mut File::open(&output)?, &mut temporary, file)?;
    temporary
        .as_file()
        .set_permissions(fs::metadata(file)?.permissions())?;
    temporary.as_file().sync_all()?;
    temporary.persist(file)?;
    Ok(())
}
