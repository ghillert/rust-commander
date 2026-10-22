//! Recursive, read-only comparison across local folders and connected remote stores.
use crate::{
    desktop::Comparison,
    filesystem::{self, Entry, Location},
    operations::Task,
    storage,
};
use anyhow::{Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

fn inventory(location: &Location, task: &mut Task) -> Result<BTreeMap<PathBuf, Entry>> {
    let root = crate::directory_tree::path(location);
    let mut pending = vec![(location.clone(), 0usize)];
    let mut items = BTreeMap::new();
    while let Some((folder, depth)) = pending.pop() {
        task.check()?;
        ensure!(depth < 64, "Comparison tree exceeds 64 levels");
        for entry in filesystem::list(&folder)? {
            task.tick(&entry.path, 0)?;
            let relative = filesystem::portable_relative(entry.path.strip_prefix(root)?)?;
            ensure!(!relative.as_os_str().is_empty(), "Invalid comparison entry");
            ensure!(!items.contains_key(&relative), "Duplicate comparison entry");
            if entry.is_dir && !entry.is_symlink {
                pending.push((crate::directory_tree::at(location, &entry.path), depth + 1));
            }
            items.insert(relative, entry);
            ensure!(items.len() <= 100_000, "Comparison exceeds 100,000 entries");
        }
    }
    Ok(items)
}

/// Links are shown explicitly and never followed; unavailable sizes never imply equality.
pub fn compare(left: &Location, right: &Location, task: &mut Task) -> Result<Vec<Comparison>> {
    ensure!(
        !matches!(left, Location::Archive { .. }) && !matches!(right, Location::Archive { .. }),
        "Choose local folders or connected remote folders to compare"
    );
    let a = inventory(left, task)?;
    let b = inventory(right, task)?;
    let names: BTreeSet<_> = a.keys().chain(b.keys()).collect();
    let mut result = Vec::new();
    for name in names {
        task.check()?;
        let status = match (a.get(name), b.get(name)) {
            (Some(_), None) => "Only left",
            (None, Some(_)) => "Only right",
            (Some(a), Some(b)) if a.is_symlink || b.is_symlink => "Link (not compared)",
            (Some(a), Some(b)) if a.is_dir != b.is_dir => "Different type",
            (Some(a), Some(_)) if a.is_dir => "Folder",
            (Some(a), Some(b)) => {
                // Listing metadata can be approximate (HTTP indexes) or missing.
                // Hash both streams, so a rounded size can never produce a false result.
                if storage::hash(left, &a.path, task)? == storage::hash(right, &b.path, task)? {
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
