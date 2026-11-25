//! User-selected application rules. Programs and paths are never shell-expanded.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub pattern: String,
    pub application: PathBuf,
}
impl Rule {
    /// Checks that this filename rule can be saved with an installed application.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty or invalid glob, a pattern over 256 bytes, or an
    /// application path that is relative or missing.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.pattern.is_empty() && self.pattern.len() <= 256,
            "Enter a filename pattern"
        );
        globset::Glob::new(&self.pattern)?;
        ensure!(
            self.application.is_absolute() && self.application.exists(),
            "Choose an installed application"
        );
        Ok(())
    }
    /// Matches the final filename against this rule's case-insensitive glob. An invalid
    /// glob or a path without a filename yields `false`.
    pub fn matches(&self, path: &Path) -> bool {
        path.file_name().is_some_and(|name| {
            globset::GlobBuilder::new(&self.pattern)
                .case_insensitive(true)
                .build()
                .is_ok_and(|g| g.compile_matcher().is_match(Path::new(name)))
        })
    }
}
/// Returns the application from the first rule matching the filename. Rule order determines
/// precedence.
pub fn application<'a>(rules: &'a [Rule], path: &Path) -> Option<&'a Path> {
    rules
        .iter()
        .find(|r| r.matches(path))
        .map(|r| r.application.as_path())
}
/// Lists application candidates from the platform's standard installation or launcher
/// directories, sorted by filename. Unreadable locations are skipped.
pub fn discover() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    roots.extend([
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        crate::filesystem::home().join("Applications"),
    ]);
    #[cfg(target_os = "linux")]
    {
        roots.push(
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| crate::filesystem::home().join(".local/share"))
                .join("applications"),
        );
        for root in std::env::var("XDG_DATA_DIRS")
            .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
            .split(':')
        {
            roots.push(Path::new(root).join("applications"));
        }
    }
    #[cfg(windows)]
    for key in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(key) {
            roots.push(PathBuf::from(root));
        }
    }
    let mut result = Vec::new();
    let mut stack = roots.into_iter().map(|p| (p, 0)).collect::<Vec<_>>();
    let mut visited = 0;
    while let Some((root, depth)) = stack.pop() {
        if visited >= 20_000 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > 20_000 {
                break;
            }
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let extension = path.extension().unwrap_or_default();
            if (cfg!(target_os = "macos") && extension == "app")
                || (cfg!(target_os = "linux") && extension == "desktop")
                || (cfg!(windows) && extension == "exe")
            {
                result.push(path);
            } else if kind.is_dir() && depth < 2 {
                stack.push((path, depth + 1));
            }
        }
    }
    result.sort_by_key(|p| {
        p.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });
    result.dedup();
    result
}
