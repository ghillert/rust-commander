//! Locations and directory entries shared by the file panes and file operations.
//!
//! [`list`] reads entries from a [`Location`]. Use [`Location::enter`] and
//! [`Location::parent`] to navigate across filesystem and archive boundaries.
//! Keep the [`Entry::path`] for subsequent operations. Its display name can be
//! a lossy rendering of a native filename.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fs,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

/// A place a pane can display, including the lists used to discover remote resources.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Location {
    /// A directory or path on the host filesystem.
    Local(PathBuf),
    /// A directory inside an archive.
    Archive {
        /// Local path of the archive, including a materialized copy when applicable.
        file: PathBuf,
        /// Directory within the archive. An empty prefix selects its root.
        prefix: PathBuf,
    },
    /// A path served by a remote connection.
    Remote {
        /// ID of the [connection profile](crate::remote::Connection).
        connection: String,
        /// Path relative to the configured connection root. Empty selects the root.
        path: PathBuf,
    },
    /// Network discovery, either at the server list or inside one server.
    Network {
        /// The selected server's profile ID, or `None` for the server list.
        server: Option<String>,
    },
    /// The list of discoverable Docker containers.
    Containers,
}

impl Location {
    /// Returns the address shown in the pane, including the prefix inside an archive.
    /// Remote locations use the address built by [`crate::uri::address`].
    pub fn label(&self) -> String {
        match self {
            Self::Local(path) => path.display().to_string(),
            Self::Archive { file, prefix } => format!("{}!/{}", file.display(), prefix.display()),
            Self::Remote { connection, path } => crate::uri::address(connection, path),
            Self::Containers => "containers://".into(),
            Self::Network { server: None } => "network://".into(),
            Self::Network { server: Some(id) } => crate::remote::profile(id)
                .map_or_else(|| format!("network://{id}"), |profile| profile.endpoint),
        }
    }

    /// Returns a short tab title, using the connection name at a remote root
    /// and the archive filename at an archive root.
    pub fn title(&self) -> String {
        let path = match self {
            Self::Containers => return crate::i18n::text("Containers").into_owned(),
            Self::Network { server: None } => return crate::i18n::text("Network").into_owned(),
            Self::Network { server: Some(id) } => return crate::remote::connection_name(id),
            Self::Local(path) => path,
            Self::Archive { file, prefix } => {
                if prefix.as_os_str().is_empty() {
                    file
                } else {
                    prefix
                }
            }
            Self::Remote { connection, path } => {
                return if path.as_os_str().is_empty() {
                    crate::remote::connection_name(connection)
                } else {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                };
            }
        };
        path.file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    /// Returns the location reached by navigating up one level.
    ///
    /// At an archive root this returns the recorded origin of a materialized
    /// archive, or its local parent directory. At a remote root it returns the
    /// container list or parent network server when the profile identifies one.
    /// Local filesystem roots and the top-level discovery lists have no parent.
    ///
    /// # Examples
    ///
    /// Enter a ZIP file, list its contents, and navigate back to its containing directory.
    ///
    /// ```
    /// use rust_commander::archive;
    /// use rust_commander::filesystem::{self, Location};
    /// use rust_commander::operations::Task;
    /// use std::fs;
    ///
    /// let workspace = tempfile::tempdir()?;
    /// let note = workspace.path().join("notes.txt");
    /// let zip = workspace.path().join("notes.zip");
    /// fs::write(&note, "meeting notes")?;
    /// archive::pack(&[note], &zip, &mut Task::default())?;
    ///
    /// let folder = Location::Local(workspace.path().to_owned());
    /// let entry = filesystem::entry_from_path(&zip)?;
    /// let inside = folder.enter(&entry).expect("ZIP files can be opened as folders");
    /// let contents = filesystem::list(&inside)?;
    /// assert_eq!(contents.len(), 1);
    /// assert_eq!(contents[0].name, "notes.txt");
    /// assert_eq!(inside.parent(), Some(folder));
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn parent(&self) -> Option<Self> {
        match self {
            Self::Network { server: None } | Self::Containers => None,
            Self::Network { server: Some(_) } => Some(Self::Network { server: None }),
            Self::Local(path) => path.parent().map(|p| Self::Local(p.to_owned())),
            Self::Archive { file, prefix } if !prefix.as_os_str().is_empty() => {
                Some(Self::Archive {
                    file: file.clone(),
                    prefix: prefix.parent().unwrap_or(Path::new("")).to_owned(),
                })
            }
            Self::Archive { file, .. } => crate::archive_tools::origin(file)
                .or_else(|| file.parent().map(|p| Self::Local(p.to_owned()))),
            Self::Remote { connection, path } if path.as_os_str().is_empty() => {
                let profile = crate::remote::profile(connection)?;
                if profile.docker.is_some() {
                    Some(Self::Containers)
                } else {
                    profile
                        .network_parent
                        .map(|id| Self::Network { server: Some(id) })
                }
            }
            Self::Remote { connection, path } => path.parent().map(|p| Self::Remote {
                connection: connection.clone(),
                path: p.to_owned(),
            }),
        }
    }

    /// Borrows the native path of a [`Location::Local`] location. Other variants return `None`.
    pub fn local(&self) -> Option<&Path> {
        match self {
            Self::Local(path) => Some(path),
            _ => None,
        }
    }

    pub(crate) fn is_virtual_list(&self) -> bool {
        matches!(self, Self::Network { .. } | Self::Containers)
    }

    pub(crate) fn docker_target(&self) -> Option<crate::remote::DockerTarget> {
        let Self::Remote { connection, .. } = self else {
            return None;
        };
        let profile = crate::remote::profile(connection)?;
        if profile.service == "docker" {
            profile.docker
        } else {
            None
        }
    }

    /// Returns the location for a child directory or the root of a recognized local archive.
    ///
    /// Archive recognition uses the filename. Ordinary files and entries in
    /// the network and container discovery lists return `None`.
    pub fn enter(&self, entry: &Entry) -> Option<Self> {
        if entry.is_dir {
            Some(match self {
                Self::Local(_) => Self::Local(entry.path.clone()),
                Self::Network { .. } | Self::Containers => return None,
                Self::Archive { file, .. } => Self::Archive {
                    file: file.clone(),
                    prefix: entry.path.clone(),
                },
                Self::Remote { connection, .. } => Self::Remote {
                    connection: connection.clone(),
                    path: entry.path.clone(),
                },
            })
        } else if self.local().is_some() && crate::archive::format(&entry.path).is_some() {
            Some(Self::Archive {
                file: entry.path.clone(),
                prefix: PathBuf::new(),
            })
        } else {
            None
        }
    }
}

/// An entry as observed in a directory listing or by [`entry_from_path`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    /// Native local path, archive member path, or path relative to a remote root.
    /// Discovery lists use this field to identify the listed resource.
    pub path: PathBuf,
    /// Filename for display in the pane.
    pub name: String,
    /// Whether the entry is a directory. Local listings also set this for links to directories.
    pub is_dir: bool,
    /// Whether the listing identified this entry as a symbolic link.
    pub is_symlink: bool,
    /// Entry length in bytes when [`size_known`](Self::size_known) is true.
    pub size: u64,
    /// Whether the provider supplied a size. An unknown size may be represented by zero.
    #[serde(default = "known_size")]
    pub size_known: bool,
    /// Last modification time, when available.
    pub modified: Option<SystemTime>,
}
fn known_size() -> bool {
    true
}

impl Entry {
    /// Returns a display category, checking for a link before a folder or an archive.
    pub fn kind(&self) -> &'static str {
        if self.is_symlink {
            "Link"
        } else if self.is_dir {
            "Folder"
        } else if crate::archive::format(&self.path).is_some() {
            "Archive"
        } else {
            "File"
        }
    }
}

/// Primary sort key for a directory listing. Ties are broken by the display name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    /// Name folded to lowercase. This is the default.
    #[default]
    Name,
    /// Recorded entry length in bytes.
    Size,
    /// Last modification time, when available.
    Modified,
    /// Filename extension.
    Extension,
    /// The category returned by [`Entry::kind`].
    Kind,
    /// Owner label resolved from the path's metadata.
    Owner,
    /// Group label resolved from the path's metadata.
    Group,
    /// Unix permission bits or the Windows read-only flag.
    Permissions,
}

/// Lists the immediate entries at `location` in the order supplied by its provider.
/// Use [`sort_entries`] to apply a pane sort order.
///
/// # Errors
///
/// Propagates directory, archive, and remote access errors. A local listing
/// also fails if a child disappears before its metadata can be read.
pub fn list(location: &Location) -> Result<Vec<Entry>> {
    match location {
        Location::Containers => crate::uri_backend::docker::list_containers(&Default::default()),
        Location::Network { server: None } => crate::discovery::network_entries(|_| {}),
        Location::Network { server: Some(id) } => {
            let profile = crate::remote::profile(id).context("Reopen the server from Network.")?;
            let secrets = crate::remote::navigation_credentials(id, Default::default(), false)?;
            crate::discovery::server_entries(&profile, &secrets)
        }
        Location::Archive { file, prefix } => crate::archive::list(file, prefix),
        Location::Remote { connection, path } => crate::remote::list(connection, path),
        Location::Local(path) => list_local(path, false),
    }
}

/// A live folder view can omit children that disappeared after enumeration.
pub(crate) fn browse(location: &Location) -> Result<Vec<Entry>> {
    match location {
        Location::Local(path) => list_local(path, true),
        _ => list(location),
    }
}

fn list_local(path: &Path, skip_vanished: bool) -> Result<Vec<Entry>> {
    let children = fs::read_dir(path)
            .map_err(|error| {
                let mut message = format!("Cannot open {}", path.display());
                if cfg!(target_os = "macos")
                    && error.kind() == std::io::ErrorKind::PermissionDenied
                    && crate::native_files::network_mounts()
                        .iter()
                        .any(|(_, mount)| path.starts_with(mount))
                {
                    message.push_str(". Check Rust Commander’s Network Volumes access in System Settings → Privacy & Security → Files and Folders");
                }
                anyhow::Error::new(error).context(message)
            })?;
    collect_children(children, skip_vanished)
}

fn collect_children(
    children: impl Iterator<Item = std::io::Result<fs::DirEntry>>,
    skip_vanished: bool,
) -> Result<Vec<Entry>> {
    children.map(|child| child.and_then(local_entry))
        .filter(|entry| !matches!(entry, Err(e) if skip_vanished && e.kind() == std::io::ErrorKind::NotFound))
        .collect::<std::io::Result<_>>().map_err(Into::into)
}

fn local_entry(item: fs::DirEntry) -> std::io::Result<Entry> {
    let path = item.path();
    let metadata = fs::symlink_metadata(&path)?;
    let is_symlink = metadata.file_type().is_symlink();
    Ok(Entry {
        name: item.file_name().to_string_lossy().into_owned(),
        is_dir: if is_symlink {
            path.is_dir()
        } else {
            metadata.is_dir()
        },
        is_symlink,
        size_known: true,
        size: metadata.len(),
        modified: metadata.modified().ok(),
        path,
    })
}

/// Sorts a listing in place, keeping directories first even in descending order.
pub fn sort_entries(entries: &mut [Entry], sort: Sort, descending: bool) {
    sort_entries_with_options(entries, sort, descending, true);
}

/// Sorts a listing in place by `sort` and then by name. When `folders_first`
/// is set, `descending` only affects the ordering within each group.
pub fn sort_entries_with_options(
    entries: &mut [Entry],
    sort: Sort,
    descending: bool,
    folders_first: bool,
) {
    let permissions_cache = if sort == Sort::Permissions {
        entries
            .iter()
            .map(|e| (e.path.clone(), crate::metadata::permission_key(&e.path)))
            .collect::<std::collections::BTreeMap<_, _>>()
    } else {
        Default::default()
    };
    let names_cache = if matches!(sort, Sort::Owner | Sort::Group) {
        entries
            .iter()
            .map(|e| (e.path.clone(), crate::metadata::ownership_labels(&e.path)))
            .collect::<std::collections::BTreeMap<_, _>>()
    } else {
        Default::default()
    };
    entries.sort_by(|a, b| {
        let dirs = b.is_dir.cmp(&a.is_dir);
        if folders_first && dirs != Ordering::Equal {
            return dirs;
        }
        let order = match sort {
            Sort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Sort::Size => a.size.cmp(&b.size),
            Sort::Modified => a.modified.cmp(&b.modified),
            Sort::Extension => a.path.extension().cmp(&b.path.extension()),
            Sort::Kind => a.kind().cmp(b.kind()),
            Sort::Owner => names_cache
                .get(&a.path)
                .and_then(|m| m.0.as_ref())
                .cmp(&names_cache.get(&b.path).and_then(|m| m.0.as_ref())),
            Sort::Group => names_cache
                .get(&a.path)
                .and_then(|m| m.1.as_ref())
                .cmp(&names_cache.get(&b.path).and_then(|m| m.1.as_ref())),
            Sort::Permissions => permissions_cache
                .get(&a.path)
                .copied()
                .flatten()
                .cmp(&permissions_cache.get(&b.path).copied().flatten()),
        }
        .then_with(|| a.name.cmp(&b.name));
        if descending { order.reverse() } else { order }
    });
}

/// Returns the user's home directory, falling back to the working directory and then `/`.
pub fn home() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_owned())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
}

/// Resolves a local path against `base` and expands a leading `~` path component.
/// The path need not exist. Filesystem aliases and parent components are left for the caller.
pub fn resolve_path(text: &str, base: &Path) -> PathBuf {
    resolve_native_path(Path::new(text), base)
}

pub(crate) fn resolve_native_path(path: &Path, base: &Path) -> PathBuf {
    if let Ok(tail) = path.strip_prefix("~") {
        home().join(tail)
    } else if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    }
}

/// Parses a pane address, preserving `base` when `text` matches its current label.
///
/// A `remote://` address specifies a connection ID and a path within it. Other
/// input is resolved as a native path against a local base, or `/` for other
/// locations. An existing file with a recognized archive extension opens at its root.
/// See [`crate::uri::parse`] for service-specific addresses such as `sftp://`.
///
/// # Errors
///
/// Rejects malformed remote addresses and remote URLs with embedded credentials,
/// query strings, fragments, or invalid path components.
pub fn parse_location(text: &str, base: &Location) -> Result<Location> {
    if text == base.label() {
        return Ok(base.clone());
    }
    if text
        .split_once("://")
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("remote"))
    {
        let url = url::Url::parse(text)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("Remote locations cannot contain credentials, query strings or fragments.");
        }
        let connection = url
            .host_str()
            .context("Missing remote connection ID")?
            .to_owned();
        let path = crate::uri::remote_path(&url)?;
        return Ok(Location::Remote { connection, path });
    }
    let path = resolve_path(text, base.local().unwrap_or_else(|| Path::new("/")));
    if path.is_file() && crate::archive::format(&path).is_some() {
        Ok(Location::Archive {
            file: path,
            prefix: PathBuf::new(),
        })
    } else {
        Ok(Location::Local(path))
    }
}

/// Checks the syntax of a single filename or directory name.
///
/// # Errors
///
/// Rejects blank names, `.`, `..`, either path separator, and NUL. On Windows,
/// additional checks reject reserved punctuation and a trailing period or space.
pub fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty()
        || name.contains(['/', '\\', '\0'])
        || name == "."
        || name == ".."
        || Path::new(name).components().count() != 1
    {
        bail!("Enter a single file or folder name without path separators.");
    }
    #[cfg(target_os = "windows")]
    if name.contains(['<', '>', ':', '"', '|', '?', '*']) || name.ends_with(['.', ' ']) {
        bail!("This name contains characters that Windows cannot use.");
    }
    Ok(())
}

/// Encode a native relative path using archive separators without changing filename bytes.
/// Use this for locally assembled paths or already validated archive members.
/// Untrusted archive headers must go through `safe_relative` directly.
pub(crate) fn portable_relative(path: &Path) -> Result<PathBuf> {
    let mut name = std::ffi::OsString::new();
    for part in path.components() {
        match part {
            Component::Normal(component) => {
                if !name.is_empty() {
                    name.push("/");
                }
                name.push(component);
            }
            Component::CurDir => {}
            _ => bail!("Unsafe relative path: {}", path.display()),
        }
    }
    let name = PathBuf::from(name);
    safe_relative(&name)?;
    Ok(name)
}

/// Returns a relative archive member path with `.` components removed.
///
/// # Errors
///
/// Rejects an empty result, parent components, absolute paths, backslashes,
/// colons, and NUL. The same checks apply to archive names on every platform.
pub fn safe_relative(path: &Path) -> Result<PathBuf> {
    let text = path.to_string_lossy();
    if text.contains(['\\', ':', '\0']) || path.is_absolute() {
        bail!("Unsafe archive path: {}", path.display());
    }
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(name) => clean.push(name),
            Component::CurDir => {}
            _ => bail!("Unsafe archive path: {}", path.display()),
        }
    }
    if clean.as_os_str().is_empty() {
        bail!("Empty archive path");
    }
    Ok(clean)
}

/// Formats bytes in binary units from bytes through TiB, for example `1.5 KiB`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut n = bytes as f64;
    let mut unit = 0;
    while n >= 1024.0 && unit < UNITS.len() - 1 {
        n /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{n:.1} {}", UNITS[unit])
    }
}

/// Reads a local entry's own metadata. A symlink to a directory has
/// [`Entry::is_symlink`] set and [`Entry::is_dir`] clear in this snapshot.
///
/// # Errors
///
/// Fails if the entry metadata cannot be read or the path has no filename.
pub fn entry_from_path(path: &Path) -> Result<Entry> {
    let metadata = fs::symlink_metadata(path)?;
    Ok(Entry {
        path: path.into(),
        name: path
            .file_name()
            .context("Cannot copy a filesystem root")?
            .to_string_lossy()
            .into_owned(),
        is_dir: metadata.is_dir(),
        is_symlink: metadata.file_type().is_symlink(),
        size_known: true,
        size: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

/// Returns the numeric Unix user ID, group ID, and permission bits of an absolute path.
/// Numeric IDs avoid network name lookups. Symlinks report their own metadata.
/// Relative paths, unavailable metadata, and non-Unix platforms yield three `None` values.
pub fn ownership(path: &Path) -> (Option<u32>, Option<u32>, Option<u32>) {
    if !path.is_absolute() {
        return (None, None, None);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = fs::symlink_metadata(path) {
            return (
                Some(meta.uid()),
                Some(meta.gid()),
                Some(meta.mode() & 0o7777),
            );
        }
    }
    let _ = path;
    (None, None, None)
}

#[cfg(test)]
mod relative_path_tests {
    use super::*;

    #[test]
    fn browsing_skips_vanished_children_but_preserves_root_and_inventory_errors() {
        let directory = tempfile::tempdir().unwrap();
        let vanished = directory.path().join("vanished");
        fs::write(&vanished, "").unwrap();
        fs::write(directory.path().join("survivor"), "").unwrap();
        let enumerate = || fs::read_dir(directory.path()).unwrap().collect::<Vec<_>>();
        let browse_children = enumerate();
        let inventory_children = enumerate();
        fs::remove_file(&vanished).unwrap();
        let entries = collect_children(browse_children.into_iter(), true).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "survivor");
        assert!(collect_children(inventory_children.into_iter(), false).is_err());
        assert!(browse(&Location::Local(vanished)).is_err());
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(collect_children(std::iter::once(Err(denied)), true).is_err());
    }

    #[test]
    fn home_expansion_keeps_repeated_separators_inside_home() {
        let base = std::env::temp_dir();
        for text in ["~/nested/file", "~//nested/file"] {
            assert_eq!(resolve_path(text, &base), home().join("nested/file"));
        }
        assert_eq!(resolve_path("~", &base), home());
        assert_eq!(
            resolve_path("relative/file", &base),
            base.join("relative/file")
        );
        #[cfg(windows)]
        assert_eq!(
            resolve_path(r"~\nested\file", &base),
            home().join("nested/file")
        );
    }

    #[cfg(unix)]
    #[test]
    fn native_startup_paths_preserve_non_unicode_filename_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let name = std::ffi::OsString::from_vec(b"native-\xff".to_vec());
        let base = std::env::temp_dir();
        let expected = base.join(&name);
        assert_eq!(resolve_native_path(Path::new(&name), &base), expected);
        assert_eq!(resolve_native_path(&expected, &base), expected);
    }

    #[test]
    fn native_components_encode_as_portable_names_and_can_be_revalidated() {
        let native = Path::new("source").join("nested").join("café.txt");
        let portable = portable_relative(&native).unwrap();
        assert_eq!(portable.to_str(), Some("source/nested/café.txt"));
        let validated = safe_relative(&portable).unwrap();
        assert_eq!(validated, native);
        assert_eq!(portable_relative(&validated).unwrap(), portable);
        assert!(safe_relative(Path::new(r"source\nested")).is_err());
    }

    #[test]
    fn native_conversion_keeps_traversal_and_unsafe_names_rejected() {
        for path in [
            "",
            ".",
            "..",
            "source/../escape",
            "/absolute",
            "C:/escape",
            "name:stream",
            "bad\0name",
        ] {
            assert!(portable_relative(Path::new(path)).is_err(), "{path:?}");
        }
        #[cfg(windows)]
        for path in [
            r"..\escape",
            r"source\..\escape",
            r"\absolute",
            r"C:\escape",
            r"\\server\share",
            r"\\?\C:\escape",
        ] {
            assert!(portable_relative(Path::new(path)).is_err(), "{path:?}");
        }
        #[cfg(unix)]
        assert!(portable_relative(Path::new(r"literal\backslash")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn native_conversion_preserves_non_unicode_unix_filenames() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let path = Path::new("source").join(std::ffi::OsString::from_vec(vec![0xff]));
        assert_eq!(
            portable_relative(&path).unwrap().as_os_str().as_bytes(),
            b"source/\xff"
        );
    }
}
