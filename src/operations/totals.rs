//! Best-effort transfer totals from metadata, never from file contents.
use super::Task;
use crate::filesystem::{self, Entry, Location};
use anyhow::Result;
use std::{collections::BTreeSet, path::PathBuf};

pub(crate) fn local(paths: &[PathBuf], task: &Task) -> Result<Option<u64>> {
    let entries = paths
        .iter()
        .map(|path| filesystem::entry_from_path(path))
        .collect::<Result<Vec<_>>>();
    match entries {
        Ok(entries) => entries_total(&Location::Local(PathBuf::new()), &entries, task),
        Err(_) => {
            task.check()?;
            Ok(None)
        }
    }
}

pub(crate) fn entries_total(
    location: &Location,
    entries: &[Entry],
    task: &Task,
) -> Result<Option<u64>> {
    let mut stack = entries
        .iter()
        .cloned()
        .map(|entry| (entry, 0))
        .collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    let mut total = 0u64;
    while let Some((entry, depth)) = stack.pop() {
        task.check()?;
        if depth >= 64 || visited.len() >= 100_000 || !visited.insert(entry.path.clone()) {
            return Ok(None);
        }
        if entry.is_symlink {
            continue;
        }
        if entry.is_dir {
            match filesystem::list(&crate::directory_tree::at(location, &entry.path)) {
                Ok(children) => stack.extend(children.into_iter().map(|entry| (entry, depth + 1))),
                Err(_) => {
                    task.check()?;
                    return Ok(None);
                }
            }
        } else {
            // HTTP indexes may round sizes. Stat fetches Content-Length; SFTP,
            // SMB and other providers use their authoritative metadata as well.
            let fresh = match crate::remote::entry_at(location, &entry.path) {
                Ok(Some(fresh)) if fresh.size_known && !fresh.is_dir && !fresh.is_symlink => fresh,
                _ => {
                    task.check()?;
                    return Ok(None);
                }
            };
            let Some(sum) = total.checked_add(fresh.size) else {
                return Ok(None);
            };
            total = sum;
        }
    }
    Ok(Some(total))
}

pub(crate) fn add(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    left?.checked_add(right?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::Path,
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };

    #[test]
    fn progress_distinguishes_known_unknown_and_empty_totals() {
        let mut task = Task::default().report_every(Duration::ZERO);
        assert_eq!(task.progress.fraction(), None);
        task.plan_total(Some(100));
        task.tick(Path::new("file"), 25).unwrap();
        assert_eq!(task.progress.fraction(), Some(0.25));
        task.plan_total(Some(1)); // A nested copy cannot replace the budget.
        assert_eq!(task.progress.total_bytes, Some(100));
        task.advance_progress(25);
        assert_eq!(task.progress.fraction(), Some(0.5));
        assert_eq!(task.progress.bytes, 25);
        task.tick(Path::new("file"), 50).unwrap();
        assert_eq!(task.progress.fraction(), Some(1.0));
        task.tick(Path::new("file"), 1).unwrap();
        assert_eq!(task.progress.fraction(), None); // Size changed: no false percentage.
        let mut empty = Task::default();
        empty.plan_total(Some(0));
        assert_eq!(empty.progress.fraction(), Some(0.0));
        let legacy: super::super::Progress =
            serde_json::from_str(r#"{"current":"","bytes":4,"files":0}"#).unwrap();
        assert_eq!(legacy.fraction(), None);
    }

    #[test]
    fn tree_totals_skip_links_and_support_cancellation() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("a"), [0; 10]).unwrap();
        std::fs::write(root.path().join("nested/b"), [0; 20]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.path(), root.path().join("nested/loop")).unwrap();
        assert_eq!(
            local(&[root.path().into()], &Task::default()).unwrap(),
            Some(30)
        );
        let task = Task::new(Arc::new(AtomicBool::new(true)), |_| {});
        assert!(local(&[root.path().into()], &task).is_err());
    }
}
