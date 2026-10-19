//! Bounded, lazy directory tree; never follows symlinks while expanding.
use crate::{
    filesystem::{self, Location},
    operations::Task,
};
use anyhow::Result;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub path: PathBuf,
    pub title: String,
    pub depth: usize,
    pub expanded: bool,
    pub error: String,
}
pub fn at(root: &Location, path: &Path) -> Location {
    match root {
        Location::Local(_) => Location::Local(path.into()),
        Location::Remote { connection, .. } => Location::Remote {
            connection: connection.clone(),
            path: path.into(),
        },
        Location::Archive { file, .. } => Location::Archive {
            file: file.clone(),
            prefix: path.into(),
        },
    }
}
pub fn path(location: &Location) -> &Path {
    match location {
        Location::Local(p)
        | Location::Remote { path: p, .. }
        | Location::Archive { prefix: p, .. } => p,
    }
}
pub fn root(location: &Location) -> Location {
    let path = path(location);
    let base = if matches!(location, Location::Local(_)) {
        path.ancestors().last().unwrap_or(path)
    } else {
        Path::new("")
    };
    at(location, base)
}
pub fn build(
    root: &Location,
    expanded: &BTreeSet<PathBuf>,
    hidden: bool,
    task: &Task,
) -> Result<Vec<Node>> {
    fn visit(
        root: &Location,
        path: &Path,
        depth: usize,
        expanded: &BTreeSet<PathBuf>,
        hidden: bool,
        task: &Task,
        nodes: &mut Vec<Node>,
    ) -> Result<()> {
        task.check()?;
        if nodes.len() >= 5000 || depth >= 64 {
            return Ok(());
        }
        let index = nodes.len();
        nodes.push(Node {
            path: path.into(),
            title: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| {
                    if path.as_os_str().is_empty() {
                        root.label()
                    } else {
                        path.display().to_string()
                    }
                }),
            depth,
            expanded: expanded.contains(path),
            error: String::new(),
        });
        if expanded.contains(path) {
            match filesystem::list(&at(root, path)) {
                Ok(mut entries) => {
                    entries.retain(|e| {
                        e.is_dir
                            && !e.is_symlink
                            && (hidden || !e.name.starts_with('.') || expanded.contains(&e.path))
                    });
                    entries.sort_by_key(|e| e.name.to_lowercase());
                    for entry in entries {
                        visit(root, &entry.path, depth + 1, expanded, hidden, task, nodes)?;
                    }
                }
                Err(e) => nodes[index].error = e.to_string(),
            }
        }
        Ok(())
    }
    let mut nodes = Vec::new();
    visit(root, path(root), 0, expanded, hidden, task, &mut nodes)?;
    Ok(nodes)
}
