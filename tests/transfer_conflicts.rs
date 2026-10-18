use rust_commander::{
    operations::Task,
    transfer::{self, ConflictPolicy, TransferOptions},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    (root, source, destination)
}
fn options(conflicts: ConflictPolicy, merge_folders: bool) -> TransferOptions {
    TransferOptions {
        conflicts,
        merge_folders,
    }
}
fn recovery(destination: &Path) -> PathBuf {
    fs::read_dir(destination)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".rust-commander-replaced-")
        })
        .unwrap()
}

#[test]
fn keep_both_preserves_extensions_and_reserves_names_across_batch() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("report.txt"), "new").unwrap();
    fs::write(source.join("report (1).txt"), "second").unwrap();
    fs::write(destination.join("report.txt"), "old").unwrap();
    transfer::transfer(
        &[source.join("report.txt"), source.join("report (1).txt")],
        &destination,
        false,
        options(ConflictPolicy::Rename, false),
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("report.txt")).unwrap(),
        "old"
    );
    assert_eq!(
        fs::read_to_string(destination.join("report (1).txt")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(destination.join("report (1) (1).txt")).unwrap(),
        "second"
    );
}

#[test]
fn skip_move_keeps_conflicting_source_and_moves_other_items() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("a"), "new").unwrap();
    fs::write(source.join("b"), "other").unwrap();
    fs::write(destination.join("a"), "old").unwrap();
    let message = transfer::transfer(
        &[source.join("a"), source.join("b")],
        &destination,
        true,
        options(ConflictPolicy::Skip, false),
        &mut Task::default(),
    )
    .unwrap();
    assert!(message.contains("skipped 1"));
    assert_eq!(fs::read_to_string(source.join("a")).unwrap(), "new");
    assert_eq!(fs::read_to_string(destination.join("a")).unwrap(), "old");
    assert!(!source.join("b").exists());
    assert_eq!(fs::read_to_string(destination.join("b")).unwrap(), "other");
}

#[test]
fn replace_retains_originals_with_paths_in_reported_recovery_folder() {
    let (_root, source, destination) = fixture();
    // Exercise an equivalent path spelling; transfers canonicalize destinations.
    let destination = destination.join("..").join("destination");
    fs::write(source.join("a"), "new").unwrap();
    fs::write(destination.join("a"), "old").unwrap();
    let message = transfer::transfer(
        &[source.join("a")],
        &destination,
        true,
        options(ConflictPolicy::Replace, false),
        &mut Task::default(),
    )
    .unwrap();
    let recovery = recovery(&destination).canonicalize().unwrap();
    assert!(
        message.contains(&recovery.display().to_string()),
        "Recovery path {} missing from transfer result: {message}",
        recovery.display()
    );
    assert_eq!(fs::read_to_string(recovery.join("items/a")).unwrap(), "old");
    assert_eq!(fs::read_to_string(destination.join("a")).unwrap(), "new");
    assert!(!source.join("a").exists());
    assert!(recovery.join("RECOVERY.txt").exists());
}

#[test]
fn merge_replace_keeps_unrelated_contents_and_backs_up_nested_conflicts() {
    let (_root, source, destination) = fixture();
    fs::create_dir_all(source.join("folder/nested")).unwrap();
    fs::create_dir_all(destination.join("folder/nested")).unwrap();
    fs::write(source.join("folder/nested/a"), "new").unwrap();
    fs::write(source.join("folder/b"), "added").unwrap();
    fs::write(destination.join("folder/nested/a"), "old").unwrap();
    fs::write(destination.join("folder/keep"), "unrelated").unwrap();
    transfer::transfer(
        &[source.join("folder")],
        &destination,
        true,
        options(ConflictPolicy::Replace, true),
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("folder/nested/a")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(destination.join("folder/b")).unwrap(),
        "added"
    );
    assert_eq!(
        fs::read_to_string(destination.join("folder/keep")).unwrap(),
        "unrelated"
    );
    assert_eq!(
        fs::read_to_string(recovery(&destination).join("items/folder/nested/a")).unwrap(),
        "old"
    );
    assert!(!source.join("folder").exists());
}

#[test]
fn merge_skip_retains_whole_source_folder_on_partial_move() {
    let (_root, source, destination) = fixture();
    fs::create_dir(source.join("folder")).unwrap();
    fs::create_dir(destination.join("folder")).unwrap();
    fs::write(source.join("folder/a"), "new").unwrap();
    fs::write(source.join("folder/b"), "added").unwrap();
    fs::write(destination.join("folder/a"), "old").unwrap();
    transfer::transfer(
        &[source.join("folder")],
        &destination,
        true,
        options(ConflictPolicy::Skip, true),
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(source.join("folder/a")).unwrap(), "new");
    assert_eq!(
        fs::read_to_string(source.join("folder/b")).unwrap(),
        "added"
    );
    assert_eq!(
        fs::read_to_string(destination.join("folder/b")).unwrap(),
        "added"
    );
    assert_eq!(
        fs::read_to_string(destination.join("folder/a")).unwrap(),
        "old"
    );
}

#[test]
fn deep_conflicts_are_found_before_any_batch_output_is_written() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("first"), "new").unwrap();
    fs::create_dir_all(source.join("folder/deep")).unwrap();
    fs::create_dir_all(destination.join("folder/deep")).unwrap();
    fs::write(source.join("folder/deep/conflict"), "new").unwrap();
    fs::write(destination.join("folder/deep/conflict"), "old").unwrap();
    assert!(
        transfer::transfer(
            &[source.join("first"), source.join("folder")],
            &destination,
            false,
            options(ConflictPolicy::Refuse, true),
            &mut Task::default()
        )
        .is_err()
    );
    assert!(!destination.join("first").exists());
    assert_eq!(
        fs::read_to_string(destination.join("folder/deep/conflict")).unwrap(),
        "old"
    );
}

#[test]
fn replace_whole_folder_preserves_old_tree_instead_of_merging_it() {
    let (_root, source, destination) = fixture();
    fs::create_dir(source.join("folder")).unwrap();
    fs::create_dir(destination.join("folder")).unwrap();
    fs::write(source.join("folder/new"), "new").unwrap();
    fs::write(destination.join("folder/old"), "old").unwrap();
    transfer::transfer(
        &[source.join("folder")],
        &destination,
        false,
        options(ConflictPolicy::Replace, false),
        &mut Task::default(),
    )
    .unwrap();
    assert!(!destination.join("folder/old").exists());
    assert_eq!(
        fs::read_to_string(recovery(&destination).join("items/folder/old")).unwrap(),
        "old"
    );
}

#[test]
fn cancellation_before_replacement_keeps_original_and_no_recovery_debris() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("a"), "new").unwrap();
    fs::write(destination.join("a"), "old").unwrap();
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = cancelled.clone();
    let mut task = Task::new(cancelled, move |_| {
        flag.store(true, std::sync::atomic::Ordering::Relaxed)
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        transfer::transfer(
            &[source.join("a")],
            &destination,
            true,
            options(ConflictPolicy::Replace, false),
            &mut task
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(source.join("a")).unwrap(), "new");
    assert_eq!(fs::read_to_string(destination.join("a")).unwrap(), "old");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
}

#[test]
fn refuses_destination_tree_containing_any_selected_source() {
    let (root, source, destination) = fixture();
    fs::create_dir_all(destination.join("folder/inside")).unwrap();
    fs::create_dir(source.join("folder")).unwrap();
    let nested = destination.join("folder/inside/file");
    fs::write(&nested, "keep").unwrap();
    assert!(
        transfer::transfer(
            &[source.join("folder"), nested.clone()],
            &destination,
            true,
            options(ConflictPolicy::Replace, false),
            &mut Task::default()
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(nested).unwrap(), "keep");
    assert!(root.path().join("source/folder").is_dir());
}

#[test]
fn destination_change_during_staging_is_preserved_and_refused() {
    let (_root, source, destination) = fixture();
    fs::write(source.join("a"), "new").unwrap();
    fs::write(destination.join("a"), "old").unwrap();
    let target = destination.join("a");
    let mut task = Task::new(Default::default(), move |_| {
        fs::write(&target, "external edit during transfer").unwrap();
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    let error = transfer::transfer(
        &[source.join("a")],
        &destination,
        true,
        options(ConflictPolicy::Replace, false),
        &mut task,
    )
    .unwrap_err();
    assert!(error.to_string().contains("Destination changed"));
    assert_eq!(
        fs::read_to_string(destination.join("a")).unwrap(),
        "external edit during transfer"
    );
    assert_eq!(fs::read_to_string(source.join("a")).unwrap(), "new");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
}

#[test]
fn source_change_during_merge_move_keeps_originals() {
    let (_root, source, destination) = fixture();
    fs::create_dir(source.join("folder")).unwrap();
    fs::create_dir(destination.join("folder")).unwrap();
    fs::write(source.join("folder/a"), "new").unwrap();
    let changing = source.join("folder/a");
    let mut task = Task::new(Default::default(), move |_| {
        fs::write(&changing, "changed during transfer").unwrap();
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    let error = transfer::transfer(
        &[source.join("folder")],
        &destination,
        true,
        options(ConflictPolicy::Refuse, true),
        &mut task,
    )
    .unwrap_err();
    assert!(error.to_string().contains("source changed"));
    assert_eq!(
        fs::read_to_string(source.join("folder/a")).unwrap(),
        "changed during transfer"
    );
    assert!(destination.join("folder/a").exists());
}

#[cfg(unix)]
#[test]
fn merge_never_follows_destination_symlinks() {
    let (root, source, destination) = fixture();
    fs::create_dir(source.join("folder")).unwrap();
    fs::write(source.join("folder/a"), "new").unwrap();
    let external = root.path().join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("a"), "old").unwrap();
    std::os::unix::fs::symlink(&external, destination.join("folder")).unwrap();
    transfer::transfer(
        &[source.join("folder")],
        &destination,
        false,
        options(ConflictPolicy::Replace, true),
        &mut Task::default(),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(external.join("a")).unwrap(), "old");
    assert!(
        !fs::symlink_metadata(destination.join("folder"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(recovery(&destination).join("items/folder")).unwrap(),
        external
    );
}
