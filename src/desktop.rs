//! Local file tools for batch rename, directory comparison, split and combine,
//! and metadata changes. Use [`rename_plan`] to review targets before [`batch_rename`].

use crate::{
    filesystem,
    operations::{self, Task},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Renames a local path using the platform's exclusive rename operation so an occupied
/// destination cannot be overwritten.
///
/// # Errors
///
/// Returns the filesystem error if the destination exists, the paths are invalid, or the
/// rename fails.
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
    /// Pattern to replace in each original filename before applying the template.
    pub find: String,
    /// Replacement text. In regex mode it can refer to captured groups.
    pub replace: String,
    /// Whether to interpret the find pattern as a regular expression.
    pub regex: bool,
    /// Filename template. `{name}` inserts the replaced stem, `{ext}` its extension including
    /// the dot, and `{n}` the sequence number.
    pub template: String,
    /// Sequence number assigned to the first source for the `{n}` placeholder.
    pub start: usize,
    /// Minimum zero-padded width for `{n}`, capped at 12.
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

/// Previews source and target paths in input order. Find and replace is applied before
/// expanding `{name}`, `{ext}`, and `{n}` in the template. Targets may reuse names
/// belonging to other sources in the plan.
///
/// # Errors
///
/// Returns an error for invalid filenames, patterns or numbering, duplicate targets, or
/// collisions with entries outside the selection.
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

/// Applies a rename plan within one directory. Moves entries through temporary names so
/// swaps can succeed and attempts rollback after a failure.
///
/// # Errors
///
/// Returns an error for an empty or invalid plan, an occupied target outside the selection,
/// cancellation, or a filesystem or rollback failure.
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
    /// Directory at which the search starts, using filesystem or connection-relative paths as
    /// appropriate.
    pub root: PathBuf,
    /// Include globs matched against the root-relative path or its final component.
    pub pattern: String,
    /// Content query for regular files. An empty string disables content matching.
    pub content: String,
    /// Whether to include dot-prefixed names and descend into dot-prefixed folders.
    pub hidden: bool,
    /// Maximum matches to return, from 1 through 100,000.
    pub max_results: usize,
    /// Case sensitivity, exclusions, and metadata bounds applied to the search.
    pub filters: crate::search::SearchFilters,
}
impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::new(),
            pattern: "*".into(),
            content: String::new(),
            hidden: false,
            max_results: 10_000,
            filters: Default::default(),
        }
    }
}
pub struct SearchResult {
    /// Matching entry paths as returned by the location's listing.
    pub paths: Vec<PathBuf>,
    /// Up to 1,000 diagnostics for entries or directories the search could not inspect.
    pub errors: Vec<String>,
    /// Whether a result or traversal limit stopped the search, or additional diagnostics
    /// were omitted.
    pub truncated: bool,
}
/// Runs the recursive [`search`](crate::search::search) from the local root in `options`.
/// Inspect the result for individual read errors and truncation.
///
/// # Errors
///
/// Returns an error for invalid options, cancellation, or an invalid entry path.
pub fn search(options: &SearchOptions, task: &mut Task) -> Result<SearchResult> {
    crate::search::search(
        &crate::filesystem::Location::Local(options.root.clone()),
        options,
        task,
    )
}

#[derive(Clone)]
pub struct Comparison {
    /// Display form of the entry's path relative to the two roots.
    pub name: String,
    /// Display label describing a difference, a match, or an unverified result.
    pub status: String,
    /// Relative path used to locate the entry under either comparison root.
    pub path: PathBuf,
    /// Whether both sides list a non-directory, non-symlink entry eligible for a text diff.
    pub can_diff: bool,
}
/// Compares two local directory trees with the default content-verifying
/// [`comparison`](crate::comparison) options.
///
/// # Errors
///
/// Returns an error if the comparison cannot traverse or read an entry, exceeds traversal
/// limits, or is cancelled.
pub fn compare(left: &Path, right: &Path, task: &mut Task) -> Result<Vec<Comparison>> {
    crate::comparison::compare(
        &crate::filesystem::Location::Local(left.into()),
        &crate::filesystem::Location::Local(right.into()),
        task,
    )
}

/// Splits a file into numbered parts of at most `chunk_size` bytes in a new directory.
/// Numbering starts at `part-000001`. An empty input creates an empty directory.
///
/// # Errors
///
/// Returns an error for a zero chunk size, an occupied destination, cancellation, or I/O
/// failure. A failed split attempts to remove the partial directory.
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
/// Concatenates parts in the order supplied and publishes the completed file at a new
/// destination. Use the original part order when reversing [`split`].
///
/// # Errors
///
/// Returns an error for an empty selection, an occupied destination, cancellation, or I/O
/// failure.
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

/// Summarizes counts and byte sizes across local paths, traversing directories without
/// following symlinks. A single selected path also gets its own metadata description.
///
/// # Errors
///
/// Returns an error if an entry cannot be inspected, the byte count overflows, or the task
/// is cancelled.
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
        text.push('\n');
        text.push_str(&crate::metadata::describe(&paths[0])?);
    }
    Ok(text)
}

/// Sets the modification time on each selected path in order.
///
/// # Errors
///
/// Returns an error for a symlink or a failed metadata update. Paths processed before a
/// failure may already have the new time.
pub fn change_modified(paths: &[PathBuf], modified: SystemTime) -> Result<()> {
    for path in paths {
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            bail!("Change the symlink target directly.");
        }
        #[cfg(windows)]
        let file = {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_WRITE_ATTRIBUTES,
            };
            // SetFileTime requires metadata write access; the flag also permits directories.
            File::options()
                .access_mode(FILE_WRITE_ATTRIBUTES)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)?
        };
        #[cfg(not(windows))]
        let file = File::open(path)?;
        file.set_times(fs::FileTimes::new().set_modified(modified))?;
    }
    Ok(())
}

/// Launches the platform's external terminal for the supplied directory.
///
/// # Errors
///
/// Returns an error if the platform terminal process cannot be started.
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
