use crate::{
    filesystem,
    operations::{self, Task},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

pub fn rename_exclusive(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let source = std::ffi::CString::new(source.as_os_str().as_bytes())?;
        let destination = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
        // Both C strings are NUL-terminated, alive for the call, and owned here.
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let result = -1;
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // Zero flags deliberately excludes MOVEFILE_REPLACE_EXISTING.
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                0,
            )
        } != 0
        {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RenameOptions {
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub template: String,
    pub start: usize,
    pub width: usize,
}
impl Default for RenameOptions {
    fn default() -> Self {
        Self {
            find: String::new(),
            replace: String::new(),
            regex: false,
            template: "{name}{ext}".into(),
            start: 1,
            width: 3,
        }
    }
}

pub fn rename_plan(
    sources: &[PathBuf],
    options: &RenameOptions,
) -> Result<Vec<(PathBuf, PathBuf)>> {
    let regex = if options.regex {
        Some(regex::Regex::new(&options.find)?)
    } else {
        None
    };
    let mut targets = BTreeSet::new();
    let mut plan = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let original = source
            .file_name()
            .context("Cannot rename a filesystem root")?
            .to_str()
            .context("Batch rename requires UTF-8 names")?;
        let replaced = if let Some(regex) = &regex {
            regex
                .replace_all(original, options.replace.as_str())
                .into_owned()
        } else if options.find.is_empty() {
            original.into()
        } else {
            original.replace(&options.find, &options.replace)
        };
        let path = Path::new(&replaced);
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let extension = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let name = options
            .template
            .replace("{name}", &stem)
            .replace("{ext}", &extension)
            .replace(
                "{n}",
                &format!(
                    "{:0width$}",
                    options
                        .start
                        .checked_add(index)
                        .context("Numbering overflow")?,
                    width = options.width.min(12)
                ),
            );
        filesystem::validate_name(&name)?;
        let target = source.parent().context("Missing source parent")?.join(name);
        if !targets.insert(target.clone()) {
            bail!(
                "The rename would create duplicate names: {}",
                target.display()
            );
        }
        if operations::exists(&target) && !sources.contains(&target) {
            bail!("A target already exists: {}", target.display());
        }
        plan.push((source.clone(), target));
    }
    Ok(plan)
}

pub fn batch_rename(plan: &[(PathBuf, PathBuf)], task: &mut Task) -> Result<()> {
    if plan.is_empty() {
        bail!("Select files to rename.");
    }
    let parent = plan[0].0.parent().context("Missing source parent")?;
    let sources: Vec<_> = plan.iter().map(|(s, _)| s.clone()).collect();
    let mut targets = BTreeSet::new();
    for (source, target) in plan {
        if source.parent() != Some(parent) || target.parent() != Some(parent) {
            bail!("Batch rename requires one folder.");
        }
        filesystem::validate_name(
            target
                .file_name()
                .context("Missing target name")?
                .to_str()
                .context("Invalid target name")?,
        )?;
        if !targets.insert(target) {
            bail!("Duplicate rename targets.");
        }
        if operations::exists(target) && !sources.contains(target) {
            bail!("A target already exists: {}", target.display());
        }
    }
    let staging = tempfile::Builder::new()
        .prefix(".rust-commander-rename-")
        .tempdir_in(parent)?
        .keep();
    let mut staged = BTreeSet::new();
    let mut committed = BTreeSet::new();
    let result = (|| -> Result<()> {
        for (index, (source, _)) in plan.iter().enumerate() {
            task.tick(source, 0)?;
            rename_exclusive(source, &staging.join(index.to_string()))?;
            staged.insert(index);
        }
        for (index, (_, target)) in plan.iter().enumerate() {
            task.tick(target, 0)?;
            rename_exclusive(&staging.join(index.to_string()), target)?;
            committed.insert(index);
        }
        Ok(())
    })();
    if let Err(error) = result {
        for &index in &committed {
            rename_exclusive(&plan[index].1, &staging.join(index.to_string())).with_context(
                || {
                    format!(
                        "Rollback interrupted. Recover staged originals from {}",
                        staging.display()
                    )
                },
            )?;
        }
        for &index in &staged {
            rename_exclusive(&staging.join(index.to_string()), &plan[index].0).with_context(
                || {
                    format!(
                        "Rollback interrupted. Recover staged originals from {}",
                        staging.display()
                    )
                },
            )?;
        }
        fs::remove_dir(&staging)?;
        return Err(error);
    }
    fs::remove_dir(staging)?;
    Ok(())
}

#[derive(Clone)]
pub struct SearchOptions {
    pub root: PathBuf,
    pub pattern: String,
    pub content: String,
    pub hidden: bool,
    pub max_results: usize,
}
pub struct SearchResult {
    pub paths: Vec<PathBuf>,
    pub errors: Vec<String>,
    pub truncated: bool,
}
pub fn search(options: &SearchOptions, task: &mut Task) -> Result<SearchResult> {
    let matcher = globset::GlobBuilder::new(&options.pattern)
        .case_insensitive(true)
        .build()?
        .compile_matcher();
    let mut result = SearchResult {
        paths: Vec::new(),
        errors: Vec::new(),
        truncated: false,
    };
    let mut folders = vec![options.root.clone()];
    while let Some(folder) = folders.pop() {
        task.tick(&folder, 0)?;
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) => {
                result.errors.push(format!("{}: {error}", folder.display()));
                continue;
            }
        };
        for item in entries {
            task.check()?;
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    result.errors.push(error.to_string());
                    continue;
                }
            };
            let name = item.file_name();
            let path = item.path();
            if !options.hidden && name.to_string_lossy().starts_with('.') {
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    result.errors.push(e.to_string());
                    continue;
                }
            };
            if metadata.is_dir() {
                folders.push(path.clone());
            }
            if !matcher.is_match(Path::new(&name)) {
                continue;
            }
            if !options.content.is_empty() {
                if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
                    continue;
                }
                let mut bytes = Vec::new();
                if File::open(&path)
                    .and_then(|f| f.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes))
                    .is_err()
                {
                    continue;
                }
                if !String::from_utf8_lossy(&bytes).contains(&options.content) {
                    continue;
                }
            }
            result.paths.push(path);
            if result.paths.len() >= options.max_results {
                result.truncated = true;
                return Ok(result);
            }
        }
    }
    result.paths.sort();
    Ok(result)
}

#[derive(Clone)]
pub struct Comparison {
    pub name: String,
    pub status: String,
}
pub fn compare(left: &Path, right: &Path, task: &mut Task) -> Result<Vec<Comparison>> {
    fn inventory(root: &Path, task: &mut Task) -> Result<BTreeMap<PathBuf, (bool, u64)>> {
        let mut stack = vec![root.to_owned()];
        let mut items = BTreeMap::new();
        while let Some(folder) = stack.pop() {
            for item in fs::read_dir(folder)? {
                let path = item?.path();
                task.tick(&path, 0)?;
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    stack.push(path.clone());
                }
                items.insert(
                    path.strip_prefix(root)?.to_owned(),
                    (metadata.is_dir(), metadata.len()),
                );
            }
        }
        Ok(items)
    }
    let a = inventory(left, task)?;
    let b = inventory(right, task)?;
    let names: BTreeSet<_> = a.keys().chain(b.keys()).collect();
    let mut result = Vec::new();
    for name in names {
        let status = match (a.get(name), b.get(name)) {
            (Some(_), None) => "Only left",
            (None, Some(_)) => "Only right",
            (Some((ad, al)), Some((bd, bl))) => {
                if ad != bd {
                    "Different type"
                } else if *ad {
                    "Folder"
                } else if al != bl {
                    "Different"
                } else if digest(&left.join(name), task)? == digest(&right.join(name), task)? {
                    "Equal"
                } else {
                    "Different"
                }
            }
            _ => unreachable!(),
        };
        result.push(Comparison {
            name: name.display().to_string(),
            status: status.into(),
        });
    }
    Ok(result)
}
fn digest(path: &Path, task: &mut Task) -> Result<Vec<u8>> {
    use sha2::Digest;
    let mut hash = sha2::Sha256::new();
    let mut input = File::open(path)?;
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
    Ok(hash.finalize().to_vec())
}

pub fn split(source: &Path, destination: &Path, chunk_size: u64, task: &mut Task) -> Result<()> {
    if chunk_size == 0 {
        bail!("Chunk size must be greater than zero.");
    }
    let mut input = File::open(source)?;
    fs::create_dir(destination)?;
    let result = (|| -> Result<()> {
        let mut index = 1;
        let mut buffer = [0; 64 * 1024];
        loop {
            task.check()?;
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            let mut output = File::create(destination.join(format!("part-{index:06}")))?;
            let first = (n as u64).min(chunk_size) as usize;
            output.write_all(&buffer[..first])?;
            // Seek back any bytes not consumed by this part.
            if first < n {
                use std::io::Seek;
                input.seek(std::io::SeekFrom::Current(-((n - first) as i64)))?;
            }
            task.tick(source, first as u64)?;
            task.stream(
                &mut (&mut input).take(chunk_size - first as u64),
                &mut output,
                source,
            )?;
            output.sync_all()?;
            index += 1;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}
pub fn combine(sources: &[PathBuf], destination: &Path, task: &mut Task) -> Result<()> {
    if sources.is_empty() {
        bail!("Select the parts to combine.");
    }
    let mut output = tempfile::NamedTempFile::new_in(
        destination.parent().context("Missing destination folder")?,
    )?;
    for source in sources {
        task.stream(&mut File::open(source)?, &mut output, source)?;
    }
    output.as_file().sync_all()?;
    output.persist_noclobber(destination).map_err(|e| e.error)?;
    Ok(())
}

pub fn properties(paths: &[PathBuf], task: &mut Task) -> Result<String> {
    let mut total = 0u64;
    let mut files = 0;
    let mut folders = 0;
    let mut stack = paths.to_vec();
    while let Some(path) = stack.pop() {
        task.tick(&path, 0)?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            folders += 1;
            for entry in fs::read_dir(&path)? {
                stack.push(entry?.path());
            }
        } else {
            files += 1;
            total = total.checked_add(metadata.len()).context("Size overflow")?;
        }
    }
    let mut text = format!(
        "{}\n\n{files} files · {folders} folders\n{} ({total} bytes)",
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        filesystem::human_size(total)
    );
    if paths.len() == 1 {
        let metadata = fs::symlink_metadata(&paths[0])?;
        if let Ok(time) = metadata.modified() {
            text.push_str(&format!(
                "\nModified: {}",
                chrono::DateTime::<chrono::Local>::from(time).format("%Y-%m-%d %H:%M:%S")
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            text.push_str(&format!(
                "\nPermissions: {:o}\nUID: {} · GID: {}",
                metadata.mode() & 0o7777,
                metadata.uid(),
                metadata.gid()
            ));
        }
    }
    Ok(text)
}

pub fn change_modified(paths: &[PathBuf], modified: SystemTime) -> Result<()> {
    for path in paths {
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            bail!("Change the symlink target directly.");
        }
        File::open(path)?.set_times(fs::FileTimes::new().set_modified(modified))?;
    }
    Ok(())
}

pub fn open_terminal(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-a", "Terminal"])
            .arg(path)
            .spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("x-terminal-emulator")
            .current_dir(path)
            .spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd.exe")
            .current_dir(path)
            .creation_flags(0x10)
            .spawn()?;
    }
    Ok(())
}
