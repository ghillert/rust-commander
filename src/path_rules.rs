//! Portable, shared include/exclude rules for recursive tools.
use anyhow::{Context, Result, ensure};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PathRules {
    pub include: String,
    pub exclude: String,
    pub case_sensitive: bool,
}
impl Default for PathRules {
    fn default() -> Self {
        Self {
            include: "*".into(),
            exclude: String::new(),
            case_sensitive: false,
        }
    }
}
impl PathRules {
    pub fn filtered(&self) -> bool {
        !matches!(self.include.trim(), "" | "*" | "**") || !self.exclude.trim().is_empty()
    }
    pub fn compile(&self) -> Result<CompiledRules> {
        fn compile(text: &str, sensitive: bool) -> Result<GlobSet> {
            ensure!(text.len() <= 16_384, "Path rules are too long");
            let mut set = GlobSetBuilder::new();
            for pattern in text
                .split([';', '\n'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let pattern = pattern.trim_end_matches('/');
                set.add(
                    GlobBuilder::new(pattern)
                        .case_insensitive(!sensitive)
                        .literal_separator(true)
                        .backslash_escape(false)
                        .build()
                        .with_context(|| format!("Invalid path pattern: {pattern}"))?,
                );
            }
            Ok(set.build()?)
        }
        Ok(CompiledRules {
            include: compile(
                if self.include.trim().is_empty() {
                    "*"
                } else {
                    &self.include
                },
                self.case_sensitive,
            )?,
            exclude: compile(&self.exclude, self.case_sensitive)?,
        })
    }
}
pub struct CompiledRules {
    include: GlobSet,
    exclude: GlobSet,
}
impl CompiledRules {
    pub fn excluded(&self, relative: &Path) -> bool {
        // Excluding a directory also excludes all descendants, including direct callers.
        relative
            .ancestors()
            .filter(|p| !p.as_os_str().is_empty())
            .any(|p| {
                self.exclude.is_match(p)
                    || p.file_name()
                        .is_some_and(|n| self.exclude.is_match(Path::new(n)))
            })
    }
    pub fn included(&self, relative: &Path) -> bool {
        !self.excluded(relative)
            && (self.include.is_match(relative)
                || relative
                    .file_name()
                    .is_some_and(|n| self.include.is_match(Path::new(n))))
    }
}
