//! A guarded, reviewable synchronization plan for two local directory trees.
use crate::{
    history::fingerprint,
    operations::{self, Task},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Choice {
    Skip,
    LeftToRight,
    RightToLeft,
    DeleteLeft,
    DeleteRight,
}
impl Choice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Skip => "Skip",
            Self::LeftToRight => "Copy left → right",
            Self::RightToLeft => "Copy right → left",
            Self::DeleteLeft => "Delete left",
            Self::DeleteRight => "Delete right",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub path: PathBuf,
    pub choice: Choice,
    pub conflict: bool,
    pub left: bool,
    pub right: bool,
    left_stamp: Option<String>,
    right_stamp: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default)]
    pub remote_roots: Option<(crate::filesystem::Location, crate::filesystem::Location)>,
    pub left: PathBuf,
    pub right: PathBuf,
    pub items: Vec<Item>,
    left_stamp: Option<String>,
    right_stamp: Option<String>,
}
impl Plan {
    /// Capability and source-presence checks apply to UI choices and restored queue plans.
    pub fn allows(&self, item: &Item, choice: Choice) -> bool {
        let (left, right) = self.remote_roots.clone().unwrap_or_else(|| {
            (
                crate::filesystem::Location::Local(self.left.clone()),
                crate::filesystem::Location::Local(self.right.clone()),
            )
        });
        match choice {
            Choice::Skip => true,
            Choice::LeftToRight => item.left && crate::drag_drop::writable(&right),
            Choice::RightToLeft => item.right && crate::drag_drop::writable(&left),
            Choice::DeleteLeft => item.left && crate::drag_drop::writable(&left),
            Choice::DeleteRight => item.right && crate::drag_drop::writable(&right),
        }
    }
}
fn scan(root: &Path, at: &Path, items: &mut BTreeMap<PathBuf, bool>, task: &Task) -> Result<()> {
    task.check()?;
    for entry in fs::read_dir(at)? {
        let entry = entry?;
        let meta = entry.file_type()?;
        if meta.is_symlink() || (!meta.is_file() && !meta.is_dir()) {
            bail!(
                "Synchronization refuses links and special files: {}",
                entry.path().display()
            );
        }
        let relative = entry.path().strip_prefix(root)?.to_owned();
        items.insert(relative, meta.is_dir());
        if items.len() > 100_000 {
            bail!("Synchronization exceeds 100,000 entries");
        }
        if meta.is_dir() {
            scan(root, &entry.path(), items, task)?;
        }
    }
    Ok(())
}
fn contents_equal(a: &Path, b: &Path, task: &Task) -> Result<bool> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    fn hash(path: &Path, task: &Task) -> Result<Vec<u8>> {
        let mut file = fs::File::open(path)?;
        let mut hash = Sha256::new();
        let mut buf = [0; 65536];
        loop {
            task.check()?;
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
        }
        Ok(hash.finalize().to_vec())
    }
    if fs::metadata(a)?.len() != fs::metadata(b)?.len() {
        return Ok(false);
    }
    Ok(hash(a, task)? == hash(b, task)?)
}
/// direction: 0 = two-way, 1 = left to right, 2 = right to left. Deletions start disabled.
pub fn preview(left: &Path, right: &Path, direction: u8, task: &Task) -> Result<Plan> {
    let left = left.canonicalize()?;
    let right = right.canonicalize()?;
    if !left.is_dir() || !right.is_dir() || left.starts_with(&right) || right.starts_with(&left) {
        bail!("Choose two separate, non-overlapping directories");
    }
    let left_stamp = fingerprint(&left, task)?;
    let right_stamp = fingerprint(&right, task)?;
    let mut l = BTreeMap::new();
    let mut r = BTreeMap::new();
    scan(&left, &left, &mut l, task)?;
    scan(&right, &right, &mut r, task)?;
    let keys = l
        .keys()
        .chain(r.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut items = Vec::new();
    let mut blocked = Vec::new();
    for path in keys {
        if blocked.iter().any(|p: &PathBuf| path.starts_with(p)) {
            continue;
        }
        let a = l.get(&path);
        let b = r.get(&path);
        let conflict = match (a, b) {
            (Some(x), Some(y)) if x != y => {
                blocked.push(path.clone());
                true
            }
            (Some(false), Some(false)) => {
                !contents_equal(&left.join(&path), &right.join(&path), task)?
            }
            _ => false,
        };
        if a == Some(&true) && b == Some(&true) {
            continue;
        }
        if a.is_some() && b.is_some() && !conflict {
            continue;
        }
        let choice = if conflict {
            Choice::Skip
        } else {
            match (direction, a.is_some(), b.is_some()) {
                (1, true, _) | (0, true, false) => Choice::LeftToRight,
                (2, _, true) | (0, false, true) => Choice::RightToLeft,
                _ => Choice::Skip,
            }
        };
        if (a == Some(&true) && b.is_none()) || (b == Some(&true) && a.is_none()) {
            blocked.push(path.clone());
        }
        let left_item_stamp = fingerprint(&left.join(&path), task)?;
        let right_item_stamp = fingerprint(&right.join(&path), task)?;
        items.push(Item {
            left_stamp: left_item_stamp,
            right_stamp: right_item_stamp,
            path,
            choice,
            conflict,
            left: a.is_some(),
            right: b.is_some(),
        });
    }
    if left_stamp != fingerprint(&left, task)? || right_stamp != fingerprint(&right, task)? {
        bail!("Directories changed during preview. Preview again.");
    }
    Ok(Plan {
        remote_roots: None,
        left,
        right,
        items,
        left_stamp,
        right_stamp,
    })
}
pub fn apply(plan: &Plan, task: &mut Task) -> Result<String> {
    anyhow::ensure!(
        plan.items.iter().all(|item| plan.allows(item, item.choice)),
        "The synchronization plan writes to a read-only folder or has a missing source. Preview again."
    );
    if plan.remote_roots.is_some() {
        return remote::apply(plan, task);
    }
    if fingerprint(&plan.left, task)? != plan.left_stamp
        || fingerprint(&plan.right, task)? != plan.right_stamp
    {
        task.undo_blocked = true;
        bail!("Synchronization preview is stale. Preview again before applying.");
    }
    // Capture entire roots so directory creation and partial cancellation remain reversible.
    if plan
        .items
        .iter()
        .any(|i| matches!(i.choice, Choice::RightToLeft | Choice::DeleteLeft))
    {
        task.capture(&plan.left)?;
    }
    if plan
        .items
        .iter()
        .any(|i| matches!(i.choice, Choice::LeftToRight | Choice::DeleteRight))
    {
        task.capture(&plan.right)?;
    }
    if fingerprint(&plan.left, task)? != plan.left_stamp
        || fingerprint(&plan.right, task)? != plan.right_stamp
    {
        task.undo_blocked = true;
        bail!("Directories changed while preparing recovery copies. Preview again.");
    }
    let mut count = 0;
    for item in &plan.items {
        task.check()?;
        let relative = crate::filesystem::portable_relative(&item.path)?;
        let (source, target) = match item.choice {
            Choice::Skip => continue,
            Choice::LeftToRight => (Some(plan.left.join(&relative)), plan.right.join(&relative)),
            Choice::RightToLeft => (Some(plan.right.join(&relative)), plan.left.join(&relative)),
            Choice::DeleteLeft => (None, plan.left.join(&relative)),
            Choice::DeleteRight => (None, plan.right.join(&relative)),
        };
        let (expected_source, expected_target) = match item.choice {
            Choice::LeftToRight => (item.left_stamp.as_ref(), item.right_stamp.as_ref()),
            Choice::RightToLeft => (item.right_stamp.as_ref(), item.left_stamp.as_ref()),
            Choice::DeleteLeft => (None, item.left_stamp.as_ref()),
            Choice::DeleteRight => (None, item.right_stamp.as_ref()),
            Choice::Skip => unreachable!(),
        };
        if fingerprint(&target, task)?.as_ref() != expected_target {
            task.undo_blocked = true;
            bail!("Synchronization target changed: {}", target.display());
        }
        if let Some(source) = source {
            if fingerprint(&source, task)?.as_ref() != expected_source {
                task.undo_blocked = true;
                bail!("Synchronization source changed: {}", source.display());
            }
            if !operations::exists(&source) {
                bail!("Source missing: {}", source.display());
            }
            if source.is_dir() && target.is_dir() {
                continue;
            }
            fs::create_dir_all(target.parent().context("Missing target parent")?)?;
            let stage = tempfile::tempdir_in(target.parent().unwrap())?;
            let copy = stage.path().join("copy");
            operations::copy_item(&source, &copy, task)?;
            if fingerprint(&source, task)?.as_ref() != expected_source
                || fingerprint(&target, task)?.as_ref() != expected_target
            {
                task.undo_blocked = true;
                bail!("Synchronization files changed while staging. Preview again.");
            }
            if operations::exists(&target) {
                crate::desktop::rename_exclusive(&target, &stage.path().join("previous"))?;
            }
            if let Err(error) = crate::desktop::rename_exclusive(&copy, &target) {
                if operations::exists(&stage.path().join("previous")) {
                    crate::desktop::rename_exclusive(&stage.path().join("previous"), &target)?;
                }
                return Err(error.into());
            }
        } else if operations::exists(&target) {
            operations::remove(&target)?;
        }
        count += 1;
    }
    Ok(format!("Synchronized {count} entries."))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conflicts_are_explicit_and_stale_plans_refuse_changes() {
        let d = tempfile::tempdir().unwrap();
        let l = d.path().join("l");
        let r = d.path().join("r");
        fs::create_dir(&l).unwrap();
        fs::create_dir(&r).unwrap();
        fs::write(l.join("a"), "left").unwrap();
        fs::write(r.join("a"), "right").unwrap();
        let mut p = preview(&l, &r, 0, &Task::default()).unwrap();
        assert!(p.items[0].conflict);
        assert_eq!(p.items[0].choice, Choice::Skip);
        p.items[0].choice = Choice::LeftToRight;
        fs::write(r.join("b"), "external").unwrap();
        assert!(apply(&p, &mut Task::default()).is_err());
        assert_eq!(fs::read_to_string(r.join("a")).unwrap(), "right");
    }
    #[test]
    fn one_way_copies_nested_and_preserves_destination_extras() {
        let d = tempfile::tempdir().unwrap();
        let l = d.path().join("l");
        let r = d.path().join("r");
        fs::create_dir_all(l.join("nested")).unwrap();
        fs::create_dir(&r).unwrap();
        fs::write(l.join("nested/a"), "a").unwrap();
        fs::write(r.join("extra"), "keep").unwrap();
        let p = preview(&l, &r, 1, &Task::default()).unwrap();
        apply(&p, &mut Task::default()).unwrap();
        assert_eq!(fs::read(r.join("nested/a")).unwrap(), b"a");
        assert!(r.join("extra").exists());
    }
}

mod remote;
pub fn preview_locations(
    left: &crate::filesystem::Location,
    right: &crate::filesystem::Location,
    direction: u8,
    task: &Task,
) -> Result<Plan> {
    if let (crate::filesystem::Location::Local(a), crate::filesystem::Location::Local(b)) =
        (left, right)
    {
        preview(a, b, direction, task)
    } else {
        remote::preview(left, right, direction, task)
    }
}
