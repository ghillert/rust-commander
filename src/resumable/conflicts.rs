//! Fixed conflict plans and durable receipts for replacement cleanup.
use super::*;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Decision {
    #[default]
    New,
    Merge,
    Skip,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Original {
    relative: PathBuf,
    directory: bool,
    stamp: Option<String>,
    removing: bool,
    removed: bool,
    #[serde(default)]
    pub(super) size: u64,
}

fn snapshot(location: &Location, root: &Path, task: &Task) -> Result<Vec<Original>> {
    fn visit(
        location: &Location,
        path: &Path,
        relative: &Path,
        out: &mut Vec<Original>,
        task: &Task,
    ) -> Result<()> {
        task.check()?;
        ensure!(
            relative.components().count() < 64 && out.len() < 100_000,
            "Replacement tree exceeds inspection limits"
        );
        let entry = remote::entry_at(location, path)?
            .context("Replacement disappeared during inspection")?;
        ensure!(!entry.is_symlink, "Checkpointed replacements refuse links");
        out.push(Original {
            relative: relative.into(),
            directory: entry.is_dir,
            stamp: if entry.is_dir {
                None
            } else {
                storage::stamp(location, path, task)?
            },
            removing: false,
            removed: false,
            size: if entry.is_dir { 0 } else { entry.size },
        });
        if entry.is_dir {
            let mut children = filesystem::list(&crate::directory_tree::at(location, path))?;
            children.sort_by(|a, b| a.name.cmp(&b.name));
            for child in children {
                filesystem::validate_name(&child.name)?;
                ensure!(
                    child.path == path.join(&child.name),
                    "Replacement listing escaped its parent"
                );
                visit(
                    location,
                    &child.path,
                    &relative.join(&child.name),
                    out,
                    task,
                )?;
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if remote::entry_at(location, root)?.is_some() {
        visit(location, root, Path::new(""), &mut out, task)?;
    }
    Ok(out)
}

fn content(location: &Location, root: &Path, task: &Task) -> Result<String> {
    fn visit(
        location: &Location,
        path: &Path,
        relative: &Path,
        hash: &mut Sha256,
        task: &Task,
    ) -> Result<()> {
        task.check()?;
        ensure!(
            relative.components().count() < 64,
            "Recovery tree is too deep"
        );
        let entry = remote::entry_at(location, path)?.context("Recovery item disappeared")?;
        ensure!(!entry.is_symlink, "Recovery inspection refuses links");
        // Length-prefixed relative names make this independent of backup paths.
        hash.update(serde_json::to_vec(&(relative, entry.is_dir))?);
        if entry.is_dir {
            let mut entries = filesystem::list(&crate::directory_tree::at(location, path))?;
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            for child in entries {
                filesystem::validate_name(&child.name)?;
                ensure!(
                    child.path == path.join(&child.name),
                    "Recovery listing escaped its parent"
                );
                visit(
                    location,
                    &child.path,
                    &relative.join(&child.name),
                    hash,
                    task,
                )?;
            }
        } else {
            hash.update(storage::hash(location, path, task)?);
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    visit(location, root, Path::new(""), &mut hash, task)?;
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn plan(
    items: &mut [Item],
    to: &Location,
    options: TransferOptions,
    overrides: &BTreeMap<PathBuf, ConflictPolicy>,
    task: &Task,
) -> Result<()> {
    let root = crate::directory_tree::path(to);
    // Source scan is in pre-order. Remap each subtree once, retaining the chosen
    // name in the manifest so a resume never allocates a second keep-both name.
    let mut remaps: Vec<(PathBuf, PathBuf, bool)> = Vec::new();
    let mut skipped: Vec<PathBuf> = Vec::new();
    let mut reserved = BTreeSet::new();
    for item in items.iter_mut() {
        task.check()?;
        let source_relative = item.relative.clone();
        if skipped.iter().any(|p| source_relative.starts_with(p)) {
            item.decision = Decision::Skip;
            continue;
        }
        let mut new_parent = false;
        if let Some((before, after, fresh)) = remaps
            .iter()
            .rev()
            .find(|(before, _, _)| source_relative.starts_with(before))
        {
            item.relative = after.join(source_relative.strip_prefix(before)?);
            new_parent = *fresh;
        }
        let original_target = root.join(&source_relative);
        let policy = overrides
            .get(&original_target)
            .copied()
            .unwrap_or(options.conflicts);
        let target = root.join(&item.relative);
        let existing = if new_parent {
            None
        } else {
            remote::entry_at(to, &target)?
        };
        if let Some(existing) = existing {
            ensure!(
                !existing.is_symlink,
                "Checkpointed conflicts refuse destination links"
            );
            if policy == ConflictPolicy::Rename {
                let parent = item.relative.parent().context("Missing target parent")?;
                let stem = item.relative.file_stem().context("Missing target name")?;
                let mut chosen = None;
                for n in 1..=100_000 {
                    let mut name = stem.to_os_string();
                    name.push(format!(" ({n})"));
                    if let Some(extension) = item.relative.extension() {
                        name.push(".");
                        name.push(extension);
                    }
                    let candidate = parent.join(name);
                    if !reserved.contains(&candidate)
                        && remote::entry_at(to, &root.join(&candidate))?.is_none()
                    {
                        chosen = Some(candidate);
                        break;
                    }
                }
                item.relative = chosen.context("Could not find an unused transfer name")?;
            } else if options.merge_folders && item.directory && existing.is_dir {
                item.decision = Decision::Merge;
            } else {
                match policy {
                    ConflictPolicy::Refuse => {
                        anyhow::bail!("Destination already exists: {}", target.display())
                    }
                    ConflictPolicy::Skip => {
                        item.decision = Decision::Skip;
                        skipped.push(source_relative.clone());
                    }
                    ConflictPolicy::Replace => {
                        item.decision = Decision::Replace;
                        item.original = snapshot(to, &target, task)?;
                    }
                    ConflictPolicy::Rename => unreachable!(),
                }
            }
        }
        ensure!(
            reserved.insert(item.relative.clone()),
            "Duplicate transfer target"
        );
        if item.directory {
            remaps.push((
                source_relative,
                item.relative.clone(),
                item.decision != Decision::Merge,
            ));
        }
    }
    Ok(())
}

fn original_path(target: &Path, relative: &Path) -> Result<PathBuf> {
    Ok(if relative.as_os_str().is_empty() {
        target.into()
    } else {
        target.join(filesystem::portable_relative(relative)?)
    })
}

fn verify_remaining(manifest: &Manifest, i: usize, target: &Path, task: &Task) -> Result<()> {
    let fresh = snapshot(&manifest.to, target, task)?;
    let expected: Vec<_> = manifest.items[i]
        .original
        .iter()
        .filter(|n| !n.removed)
        .map(|n| {
            let mut node = n.clone();
            node.removing = false;
            node
        })
        .collect();
    ensure!(
        fresh == expected,
        "Replacement destination changed; recovery files retained for review: {}",
        target.display()
    );
    Ok(())
}

pub(super) fn prepare(
    manifest: &mut Manifest,
    i: usize,
    receipt: &Path,
    directory: &Path,
    target: &Path,
    task: &mut Task,
) -> Result<()> {
    if manifest.items[i].decision != Decision::Replace {
        return Ok(());
    }
    let local = Location::Local(directory.into());
    let backup = directory.join(format!("original-{i}"));
    if let Some(expected) = &manifest.items[i].backup_hash {
        ensure!(
            content(&local, &backup, task)? == *expected,
            "Replacement recovery copy changed; review required"
        );
        if manifest.items[i].cleared {
            return Ok(());
        }
    } else {
        verify_remaining(manifest, i, target, task)?;
        let expected = content(&manifest.to, target, task)?;
        if crate::operations::exists(&backup) {
            // An incomplete backup is disposable only while the original tree
            // still exactly matches the durable plan above.
            ensure!(
                !fs::symlink_metadata(&backup)?.file_type().is_symlink(),
                "Recovery copy is a link"
            );
            crate::operations::remove(&backup)?;
        }
        let entry = remote::entry_at(&manifest.to, target)?.context("Replacement disappeared")?;
        if matches!(manifest.to, Location::Local(_)) {
            crate::operations::copy_item(target, &backup, task)?;
        } else {
            remote::copy_entry(&manifest.to, &entry, &local, &backup, task)?;
        }
        ensure!(
            content(&local, &backup, task)? == expected,
            "Replacement backup failed verification"
        );
        verify_remaining(manifest, i, target, task)?;
        manifest.items[i].backup_hash = Some(expected);
        save(receipt, manifest)?;
    }
    // Normalize a deletion whose receipt was flushed before a crash. Missing
    // outputs are accepted only for nodes with a durable deletion intent.
    for n in 0..manifest.items[i].original.len() {
        let node = &manifest.items[i].original[n];
        if node.removing
            && !node.removed
            && remote::entry_at(&manifest.to, &original_path(target, &node.relative)?)?.is_none()
        {
            manifest.items[i].original[n].removed = true;
        }
    }
    verify_remaining(manifest, i, target, task)?;
    save(receipt, manifest)?;
    for n in (0..manifest.items[i].original.len()).rev() {
        task.check()?;
        let node = &manifest.items[i].original[n];
        if node.removed {
            continue;
        }
        let path = original_path(target, &node.relative)?;
        let entry = remote::entry_at(&manifest.to, &path)?
            .context("Replacement destination disappeared")?;
        ensure!(
            entry.is_dir == node.directory && !entry.is_symlink,
            "Replacement type changed"
        );
        if entry.is_dir {
            ensure!(
                filesystem::list(&crate::directory_tree::at(&manifest.to, &path))?.is_empty(),
                "Replacement folder changed; recovery retained"
            );
        } else {
            ensure!(
                storage::stamp(&manifest.to, &path, task)? == node.stamp,
                "Replacement file changed; recovery retained"
            );
        }
        manifest.items[i].original[n].removing = true;
        save(receipt, manifest)?;
        remote::remove_checkpoint_leaf(&manifest.to, &entry, task)?;
        task.tick(&path, 0)?;
        manifest.items[i].original[n].removed = true;
        save(receipt, manifest)?;
    }
    manifest.items[i].cleared = true;
    save(receipt, manifest)
}
