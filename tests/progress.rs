use rust_commander::{
    operations::{self, Operation, Progress, Task},
    transfer::{self, ConflictPolicy, TransferOptions},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Duration,
};

#[test]
fn local_copy_and_native_move_report_whole_selection_progress() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    let moved = root.path().join("moved");
    std::fs::create_dir_all(source.join("nested")).unwrap();
    std::fs::create_dir(&destination).unwrap();
    std::fs::create_dir(&moved).unwrap();
    std::fs::write(source.join("a"), vec![0; 2 * 65536]).unwrap();
    std::fs::write(source.join("nested/b"), vec![1; 2 * 65536]).unwrap();
    let updates: Arc<Mutex<Vec<Progress>>> = Default::default();
    let collected = updates.clone();
    let mut task = Task::new(Arc::new(AtomicBool::new(false)), move |p| {
        collected.lock().unwrap().push(p)
    })
    .report_every(Duration::ZERO);
    operations::run(
        Operation::Copy {
            sources: vec![source.clone()],
            destination: destination.clone(),
        },
        &mut task,
    )
    .unwrap();
    assert_eq!(task.progress.total_bytes, Some(4 * 65536));
    assert_eq!(task.progress.completed_bytes, 4 * 65536);
    assert!(
        updates
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.fraction().is_some_and(|f| f > 0.0 && f < 1.0))
    );
    let mut task = Task::default();
    operations::run(
        Operation::Move {
            sources: vec![destination.join("source")],
            destination: moved.clone(),
        },
        &mut task,
    )
    .unwrap();
    assert_eq!(task.progress.total_bytes, Some(4 * 65536));
    assert_eq!(task.progress.completed_bytes, 4 * 65536);
    assert!(moved.join("source/nested/b").exists());
}

#[test]
fn local_merge_excludes_skipped_files_from_the_total() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir_all(destination.join("source")).unwrap();
    std::fs::write(source.join("skip"), [0; 10]).unwrap();
    std::fs::write(source.join("copy"), [0; 20]).unwrap();
    std::fs::write(destination.join("source/skip"), b"keep").unwrap();
    let mut task = Task::default();
    transfer::transfer_resolved(
        &[PathBuf::from(&source)],
        &destination,
        false,
        TransferOptions {
            conflicts: ConflictPolicy::Skip,
            merge_folders: true,
        },
        &Default::default(),
        &mut task,
    )
    .unwrap();
    assert_eq!(task.progress.total_bytes, Some(20));
    assert_eq!(task.progress.completed_bytes, 20);
    assert_eq!(
        std::fs::read(destination.join("source/skip")).unwrap(),
        b"keep"
    );
}
