use super::*;
use crate::{
    filesystem::{self, Location},
    remote, storage,
};
fn scan(
    location: &Location,
    root: &Path,
    at: &Path,
    items: &mut BTreeMap<PathBuf, bool>,
    task: &Task,
    depth: usize,
) -> Result<()> {
    task.check()?;
    anyhow::ensure!(depth < 64, "Remote synchronization tree is too deep");
    for entry in filesystem::list(&crate::directory_tree::at(location, at))? {
        if [
            ".rust-commander-replaced-",
            ".rust-commander-sync-",
            ".rust-commander-batch-",
        ]
        .iter()
        .any(|prefix| entry.name.starts_with(prefix))
        {
            continue;
        }
        anyhow::ensure!(!entry.is_symlink, "Synchronization refuses links");
        let relative = filesystem::portable_relative(entry.path.strip_prefix(root)?)?;
        items.insert(relative, entry.is_dir);
        anyhow::ensure!(
            items.len() <= 100_000,
            "Synchronization exceeds 100,000 entries"
        );
        if entry.is_dir {
            scan(location, root, &entry.path, items, task, depth + 1)?;
        }
    }
    Ok(())
}
pub(super) fn preview(
    left: &Location,
    right: &Location,
    direction: u8,
    task: &Task,
) -> Result<Plan> {
    anyhow::ensure!(
        !matches!(left, Location::Archive { .. }) && !matches!(right, Location::Archive { .. }),
        "Archive synchronization is unavailable"
    );
    let a = crate::directory_tree::path(left);
    let b = crate::directory_tree::path(right);
    let same = matches!((left,right),(Location::Remote{connection:a,..},Location::Remote{connection:b,..}) if a==b);
    anyhow::ensure!(
        !(same && (a.starts_with(b) || b.starts_with(a))),
        "Choose non-overlapping remote folders"
    );
    let left_stamp = storage::stamp(left, a, task)?;
    let right_stamp = storage::stamp(right, b, task)?;
    anyhow::ensure!(
        left_stamp.is_some() && right_stamp.is_some(),
        "Choose two existing folders"
    );
    let mut l = BTreeMap::new();
    let mut r = BTreeMap::new();
    scan(left, a, a, &mut l, task, 0)?;
    scan(right, b, b, &mut r, task, 0)?;
    let keys = l
        .keys()
        .chain(r.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut blocked = Vec::<PathBuf>::new();
    let mut items = Vec::new();
    for path in keys {
        if blocked.iter().any(|p| path.starts_with(p)) {
            continue;
        }
        let x = l.get(&path);
        let y = r.get(&path);
        if x == Some(&true) && y == Some(&true) {
            continue;
        }
        let conflict = match (x, y) {
            (Some(x), Some(y)) if x != y => {
                blocked.push(path.clone());
                true
            }
            (Some(false), Some(false)) => {
                storage::hash(left, &a.join(&path), task)?
                    != storage::hash(right, &b.join(&path), task)?
            }
            _ => false,
        };
        if x.is_some() && y.is_some() && !conflict {
            continue;
        }
        let choice = if conflict {
            Choice::Skip
        } else {
            match (direction, x.is_some(), y.is_some()) {
                (1, true, _) | (0, true, false) => Choice::LeftToRight,
                (2, _, true) | (0, false, true) => Choice::RightToLeft,
                _ => Choice::Skip,
            }
        };
        if (x == Some(&true) && y.is_none()) || (y == Some(&true) && x.is_none()) {
            blocked.push(path.clone());
        }
        items.push(Item {
            path: path.clone(),
            choice,
            conflict,
            left: x.is_some(),
            right: y.is_some(),
            left_stamp: storage::stamp(left, &a.join(&path), task)?,
            right_stamp: storage::stamp(right, &b.join(&path), task)?,
        });
        anyhow::ensure!(
            items.len() <= 5000,
            "Too many differences to review; synchronize a smaller folder"
        );
    }
    anyhow::ensure!(
        storage::stamp(left, a, task)? == left_stamp
            && storage::stamp(right, b, task)? == right_stamp,
        "Folders changed during preview; preview again"
    );
    let mut plan = Plan {
        left: a.into(),
        right: b.into(),
        remote_roots: Some((left.clone(), right.clone())),
        items,
        left_stamp,
        right_stamp,
    };
    for index in 0..plan.items.len() {
        if !plan.allows(&plan.items[index], plan.items[index].choice) {
            plan.items[index].choice = Choice::Skip;
        }
    }
    Ok(plan)
}
pub(super) fn apply(plan: &Plan, task: &mut Task) -> Result<String> {
    let (left, right) = plan
        .remote_roots
        .as_ref()
        .context("Missing synchronization roots")?;
    task.undo_blocked = true; // Remote writes use recovery copies, never a misleading local-only Undo.
    anyhow::ensure!(
        storage::stamp(left, &plan.left, task)? == plan.left_stamp
            && storage::stamp(right, &plan.right, task)? == plan.right_stamp,
        "Synchronization preview is stale; preview again"
    );
    let mut count = 0;
    let mut messages = Vec::new();
    for item in &plan.items {
        task.check()?;
        let relative = filesystem::portable_relative(&item.path)?;
        let (source, to, source_root, target_root, source_stamp, target_stamp) = match item.choice {
            Choice::Skip => continue,
            Choice::LeftToRight => (
                Some(left),
                right,
                &plan.left,
                &plan.right,
                &item.left_stamp,
                &item.right_stamp,
            ),
            Choice::RightToLeft => (
                Some(right),
                left,
                &plan.right,
                &plan.left,
                &item.right_stamp,
                &item.left_stamp,
            ),
            Choice::DeleteLeft => (None, left, &plan.left, &plan.left, &None, &item.left_stamp),
            Choice::DeleteRight => (
                None,
                right,
                &plan.right,
                &plan.right,
                &None,
                &item.right_stamp,
            ),
        };
        let target = target_root.join(&relative);
        anyhow::ensure!(
            storage::stamp(to, &target, task)? == *target_stamp,
            "Synchronization target changed: {}",
            target.display()
        );
        if let Some(from) = source {
            let path = source_root.join(&relative);
            anyhow::ensure!(
                storage::stamp(from, &path, task)? == *source_stamp,
                "Synchronization source changed: {}",
                path.display()
            );
            let entry =
                remote::entry_at(from, &path)?.context("Synchronization source disappeared")?;
            let destination =
                crate::directory_tree::at(to, target.parent().context("Missing target folder")?);
            messages.push(remote::transfer_resolved(
                from,
                &[entry],
                &destination,
                false,
                crate::transfer::TransferOptions {
                    conflicts: crate::transfer::ConflictPolicy::Replace,
                    merge_folders: false,
                },
                &Default::default(),
                task,
            )?);
        } else if let Some(entry) = remote::entry_at(to, &target)? {
            let backup = target
                .parent()
                .context("Missing target folder")?
                .join(format!(
                    ".rust-commander-sync-{:016x}-{}",
                    rand::random::<u64>(),
                    entry.name
                ));
            remote::copy_entry(to, &entry, to, &backup, task)?;
            anyhow::ensure!(
                storage::stamp(to, &target, task)? == *target_stamp,
                "Deletion target changed; recovery retained at {}",
                backup.display()
            );
            remote::remove_entry(to, &entry, task)?;
            messages.push(format!("Recovery copy: {}", backup.display()));
        }
        count += 1;
    }
    Ok(format!(
        "Synchronized {count} entries.\n{}",
        messages.join("\n")
    ))
}
