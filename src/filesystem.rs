use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fs,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Location {
    Local(PathBuf),
    Archive { file: PathBuf, prefix: PathBuf },
    Remote { connection: String, path: PathBuf },
}

impl Location {
    pub fn label(&self) -> String {
        match self {
            Self::Local(path) => path.display().to_string(),
            Self::Archive { file, prefix } => format!("{}!/{}", file.display(), prefix.display()),
            Self::Remote { connection, path } => format!(
                "remote://{connection}/{}",
                crate::remote::display_path(path)
            ),
        }
    }

    pub fn title(&self) -> String {
        let path = match self {
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

    pub fn parent(&self) -> Option<Self> {
        match self {
            Self::Local(path) => path.parent().map(|p| Self::Local(p.to_owned())),
            Self::Archive { file, prefix } if !prefix.as_os_str().is_empty() => {
                Some(Self::Archive {
                    file: file.clone(),
                    prefix: prefix.parent().unwrap_or(Path::new("")).to_owned(),
                })
            }
            Self::Archive { file, .. } => crate::archive_tools::origin(file)
                .or_else(|| file.parent().map(|p| Self::Local(p.to_owned()))),
            Self::Remote { connection, path } => path.parent().map(|p| Self::Remote {
                connection: connection.clone(),
                path: p.to_owned(),
            }),
        }
    }

    pub fn local(&self) -> Option<&Path> {
        match self {
            Self::Local(path) => Some(path),
            _ => None,
        }
    }

    pub fn enter(&self, entry: &Entry) -> Option<Self> {
        if entry.is_dir {
            Some(match self {
                Self::Local(_) => Self::Local(entry.path.clone()),
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Entry {
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    #[default]
    Name,
    Size,
    Modified,
    Extension,
    Kind,
    Owner,
    Group,
    Permissions,
}

pub fn list(location: &Location) -> Result<Vec<Entry>> {
    match location {
        Location::Archive { file, prefix } => crate::archive::list(file, prefix),
        Location::Remote { connection, path } => crate::remote::list(connection, path),
        Location::Local(path) => fs::read_dir(path)
            .with_context(|| format!("Cannot open {}", path.display()))?
            .map(|item| {
                let item = item?;
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
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                    path,
                })
            })
            .collect(),
    }
}

pub fn sort_entries(entries: &mut [Entry], sort: Sort, descending: bool) {
    sort_entries_with_options(entries, sort, descending, true);
}

pub fn sort_entries_with_options(
    entries: &mut [Entry],
    sort: Sort,
    descending: bool,
    folders_first: bool,
) {
    let ownership_cache = if matches!(sort, Sort::Owner | Sort::Group | Sort::Permissions) {
        entries
            .iter()
            .map(|e| (e.path.clone(), ownership(&e.path)))
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
            Sort::Owner => ownership_cache
                .get(&a.path)
                .and_then(|m| m.0)
                .cmp(&ownership_cache.get(&b.path).and_then(|m| m.0)),
            Sort::Group => ownership_cache
                .get(&a.path)
                .and_then(|m| m.1)
                .cmp(&ownership_cache.get(&b.path).and_then(|m| m.1)),
            Sort::Permissions => ownership_cache
                .get(&a.path)
                .and_then(|m| m.2)
                .cmp(&ownership_cache.get(&b.path).and_then(|m| m.2)),
        }
        .then_with(|| a.name.cmp(&b.name));
        if descending { order.reverse() } else { order }
    });
}

pub fn home() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_owned())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
}

pub fn resolve_path(text: &str, base: &Path) -> PathBuf {
    if text == "~" {
        home()
    } else if let Some(tail) = text.strip_prefix("~/") {
        home().join(tail)
    } else {
        let path = PathBuf::from(text);
        if path.is_absolute() {
            path
        } else {
            base.join(path)
        }
    }
}

pub fn parse_location(text: &str, base: &Location) -> Result<Location> {
    if text == base.label() {
        return Ok(base.clone());
    }
    if text.starts_with("remote://") {
        let url = url::Url::parse(text)?;
        if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
            bail!("Remote locations cannot contain credentials or query strings.");
        }
        let connection = url
            .host_str()
            .context("Missing remote connection ID")?
            .to_owned();
        let text = percent_encoding::percent_decode_str(url.path().trim_start_matches('/'))
            .decode_utf8()?;
        let path = if text.is_empty() {
            PathBuf::new()
        } else {
            safe_relative(Path::new(text.as_ref()))?
        };
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

/// Archive paths are interpreted portably, including Windows drive/UNC paths.
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
        size: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

/// Numeric IDs avoid blocking directory rendering on network name services.
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
