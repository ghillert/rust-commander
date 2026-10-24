//! Shared bounded-memory search for local and remote files.
use crate::{
    desktop::{SearchOptions, SearchResult},
    filesystem::{self, Location},
    operations::Task,
    path_rules::PathRules,
};
use anyhow::{Context, Result, ensure};
use regex::bytes::{Regex, RegexBuilder};
use std::{io::Read, time::SystemTime};

#[derive(Clone, Debug)]
pub struct SearchFilters {
    pub name_case_sensitive: bool,
    pub content_regex: bool,
    pub content_case_sensitive: bool,
    pub exclude: String,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub modified_after: Option<SystemTime>,
    pub modified_before: Option<SystemTime>,
}
impl Default for SearchFilters {
    fn default() -> Self {
        Self {
            name_case_sensitive: false,
            content_regex: false,
            content_case_sensitive: true,
            exclude: String::new(),
            min_size: None,
            max_size: None,
            modified_after: None,
            modified_before: None,
        }
    }
}
pub fn parse_date(value: &str) -> Result<Option<SystemTime>> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(
        chrono::DateTime::parse_from_rfc3339(value.trim())
            .context("Enter an RFC 3339 date, for example 2026-10-23T00:00:00Z")?
            .into(),
    ))
}
pub fn date_label(value: Option<SystemTime>) -> String {
    value
        .map(|v| chrono::DateTime::<chrono::Utc>::from(v).to_rfc3339())
        .unwrap_or_default()
}
pub fn validate(options: &SearchOptions) -> Result<()> {
    let f = &options.filters;
    ensure!(
        (1..=100_000).contains(&options.max_results),
        "Maximum results must be between 1 and 100,000"
    );
    ensure!(
        f.min_size.zip(f.max_size).is_none_or(|(a, b)| a <= b),
        "Minimum size exceeds maximum size"
    );
    ensure!(
        f.modified_after
            .zip(f.modified_before)
            .is_none_or(|(a, b)| a <= b),
        "Date range is reversed"
    );
    rules(options).compile()?;
    Content::new(options)?;
    Ok(())
}
fn rules(options: &SearchOptions) -> PathRules {
    PathRules {
        include: options.pattern.clone(),
        exclude: options.filters.exclude.clone(),
        case_sensitive: options.filters.name_case_sensitive,
    }
}
struct Content {
    regex: Regex,
    literal_width: Option<usize>,
}
impl Content {
    fn new(options: &SearchOptions) -> Result<Option<Self>> {
        if options.content.is_empty() {
            return Ok(None);
        }
        ensure!(
            options.content.len() <= 16_384,
            "Content pattern exceeds 16 KiB"
        );
        let pattern = if options.filters.content_regex {
            options.content.clone()
        } else {
            regex::escape(&options.content)
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!options.filters.content_case_sensitive)
            .multi_line(true)
            .crlf(true)
            .size_limit(10 * 1024 * 1024)
            .build()
            .context("Invalid content pattern")?;
        Ok(Some(Self {
            regex,
            literal_width: (!options.filters.content_regex)
                .then(|| options.content.chars().count() * 4),
        }))
    }
    fn matches(
        &self,
        mut input: Box<dyn Read>,
        path: &std::path::Path,
        task: &mut Task,
    ) -> Result<bool> {
        let mut chunk = [0u8; 65_536];
        let mut pending = Vec::new();
        loop {
            task.check()?;
            let n = input.read(&mut chunk)?;
            if n == 0 {
                return Ok(self.regex.is_match(&pending));
            }
            task.tick(path, n as u64)?;
            if let Some(width) = self.literal_width {
                pending.extend_from_slice(&chunk[..n]);
                if self.regex.is_match(&pending) {
                    return Ok(true);
                }
                let keep = pending.len().min(width.saturating_sub(1));
                pending.drain(..pending.len() - keep);
            } else {
                // Regex is line-oriented. A bound on a single line keeps malformed or
                // binary input from allocating the entire file. Report this limit.
                for part in chunk[..n].split_inclusive(|b| *b == b'\n') {
                    ensure!(
                        pending.len() + part.len() <= 8 * 1024 * 1024,
                        "Regex search encountered a line over 8 MiB; use literal search for arbitrary-length lines"
                    );
                    pending.extend_from_slice(part);
                    if part.last() == Some(&b'\n') {
                        if self.regex.is_match(&pending) {
                            return Ok(true);
                        }
                        pending.clear();
                    }
                }
            }
        }
    }
}
pub fn search(
    location: &Location,
    options: &SearchOptions,
    task: &mut Task,
) -> Result<SearchResult> {
    validate(options)?;
    let rules = rules(options).compile()?;
    let content = Content::new(options)?;
    let mut result = SearchResult {
        paths: Vec::new(),
        errors: Vec::new(),
        truncated: false,
    };
    let mut folders = vec![(options.root.clone(), 0usize)];
    let mut visited = std::collections::BTreeSet::new();
    let mut count = 0usize;
    while let Some((folder, depth)) = folders.pop() {
        task.check()?;
        if !visited.insert(folder.clone()) {
            continue;
        }
        let entries = match entries(location, &folder) {
            Ok(entries) => entries,
            Err(e) => {
                record_error(&mut result, format!("{}: {e:#}", folder.display()));
                continue;
            }
        };
        for entry in entries {
            task.check()?;
            count += 1;
            if count > 100_000 || depth >= 256 {
                result.truncated = true;
                return Ok(result);
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    record_error(&mut result, format!("{}: {e:#}", folder.display()));
                    continue;
                }
            };
            let relative = entry
                .path
                .strip_prefix(&options.root)
                .context("Search entry is outside its root")?;
            if (!options.hidden && entry.name.starts_with('.')) || rules.excluded(relative) {
                continue;
            }
            if entry.is_dir && !entry.is_symlink {
                folders.push((entry.path.clone(), depth + 1));
            }
            if !rules.included(relative) {
                continue;
            }
            let f = &options.filters;
            if (f.min_size.is_some() || f.max_size.is_some())
                && (entry.is_dir
                    || !entry.size_known
                    || f.min_size.is_some_and(|s| entry.size < s)
                    || f.max_size.is_some_and(|s| entry.size > s))
            {
                continue;
            }
            if (f.modified_after.is_some() || f.modified_before.is_some())
                && entry.modified.is_none_or(|m| {
                    f.modified_after.is_some_and(|s| m < s)
                        || f.modified_before.is_some_and(|s| m > s)
                })
            {
                continue;
            }
            if let Some(matcher) = &content {
                if entry.is_dir || entry.is_symlink {
                    continue;
                }
                if matches!(location, Location::Local(_)) {
                    match std::fs::symlink_metadata(&entry.path) {
                        Ok(metadata) if metadata.is_file() => {}
                        Ok(_) => continue,
                        Err(e) => {
                            record_error(&mut result, format!("{}: {e}", entry.path.display()));
                            continue;
                        }
                    }
                }
                let matched = crate::storage::reader(location, &entry.path)
                    .and_then(|input| matcher.matches(input, &entry.path, task));
                task.check()?;
                match matched {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(e) => {
                        record_error(&mut result, format!("{}: {e:#}", entry.path.display()));
                        continue;
                    }
                }
            }
            result.paths.push(entry.path);
            if result.paths.len() >= options.max_results {
                result.truncated = true;
                return Ok(result);
            }
        }
    }
    result.paths.sort();
    Ok(result)
}
type Entries = Box<dyn Iterator<Item = Result<filesystem::Entry>>>;
fn entries(location: &Location, folder: &std::path::Path) -> Result<Entries> {
    if matches!(location, Location::Local(_)) {
        Ok(Box::new(
            std::fs::read_dir(folder)?.map(|entry| filesystem::entry_from_path(&entry?.path())),
        ))
    } else {
        Ok(Box::new(
            filesystem::list(&crate::directory_tree::at(location, folder))?
                .into_iter()
                .map(Ok),
        ))
    }
}
fn record_error(result: &mut SearchResult, message: String) {
    if result.errors.len() < 1000 {
        result.errors.push(message);
    } else {
        result.truncated = true;
    }
}
