use rust_commander::{
    filesystem::{self, Location},
    operations::Task,
    remote::{self, Connection, Secrets},
    resumable,
    transfer::{self, ConflictPolicy, TransferOptions},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn mount(root: &Path, id: &str) -> Location {
    let connection = Connection {
        id: id.into(),
        name: "Disposable recovery fixture".into(),
        service: "fs".into(),
        root: root.display().to_string(),
        ..Default::default()
    };
    remote::connect(&connection, Secrets::default(), false).unwrap();
    Location::Remote {
        connection: connection.id,
        path: PathBuf::new(),
    }
}

// Invoked only by the parent test, with an isolated root. exit() deliberately
// bypasses destructors, matching a terminated worker rather than cancellation.
#[test]
#[ignore = "subprocess crash fixture, invoked by checkpoint_survives_process_termination"]
fn checkpoint_crash_child() {
    let root = PathBuf::from(
        std::env::var_os("RUST_COMMANDER_CRASH_FIXTURE").expect("parent fixture required"),
    );
    assert!(root.join("owned-recovery-fixture").is_file());
    let from = mount(&root.join("source"), "process-crash");
    let entries = filesystem::list(&from).unwrap();
    let to = Location::Local(root.join("target"));
    let mut task = Task::new(Arc::new(AtomicBool::new(false)), |progress| {
        if progress.bytes > 0 {
            std::process::exit(73);
        }
    });
    task = task.report_every(Duration::ZERO);
    resumable::run_in(
        &root.join("checkpoints"),
        "batch-process",
        &from,
        &entries,
        &to,
        &mut task,
    )
    .unwrap();
    panic!("child should exit during the first streamed chunk");
}

#[test]
fn checkpoint_survives_process_termination() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("owned-recovery-fixture"), b"fixture").unwrap();
    fs::create_dir(root.path().join("source")).unwrap();
    fs::create_dir(root.path().join("target")).unwrap();
    let bytes = vec![0x39; 2 * 1024 * 1024 + 29];
    fs::write(root.path().join("source/file.bin"), &bytes).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "checkpoint_crash_child",
            "--ignored",
            "--nocapture",
        ])
        .env("RUST_COMMANDER_CRASH_FIXTURE", root.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(73));
    let partial = root.path().join("checkpoints/batch-process/0.part");
    let offset = partial.metadata().unwrap().len();
    assert!(offset > 0 && offset < bytes.len() as u64);
    assert!(!root.path().join("target/file.bin").exists());
    let from = mount(&root.path().join("source"), "process-crash");
    let entries = filesystem::list(&from).unwrap();
    let message = resumable::run_in(
        &root.path().join("checkpoints"),
        "batch-process",
        &from,
        &entries,
        &Location::Local(root.path().join("target")),
        &mut Task::default(),
    )
    .unwrap();
    assert!(message.contains("recovered 1"));
    assert_eq!(
        fs::read(root.path().join("target/file.bin")).unwrap(),
        bytes
    );
    assert_eq!(
        fs::read(root.path().join("source/file.bin")).unwrap(),
        bytes
    );
    assert!(!root.path().join("checkpoints/batch-process").exists());
}

#[test]
fn interrupted_replacement_keeps_old_target_and_all_move_sources() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    let bytes = vec![0x22; 2 * 1024 * 1024];
    fs::write(source.join("file"), &bytes).unwrap();
    fs::write(target.join("file"), b"previous").unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let mut task = Task::new(cancel, move |progress| {
        if progress.bytes > 0 {
            stop.store(true, Ordering::Relaxed);
        }
    });
    task = task.report_every(Duration::ZERO);
    let result = transfer::transfer(
        &[source.join("file")],
        &target,
        true,
        TransferOptions {
            conflicts: ConflictPolicy::Replace,
            merge_folders: false,
        },
        &mut task,
    );
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    assert_eq!(fs::read(source.join("file")).unwrap(), bytes);
    assert_eq!(fs::read(target.join("file")).unwrap(), b"previous");
    assert_eq!(
        fs::read_dir(&target).unwrap().count(),
        1,
        "uncommitted staging must be removed"
    );
}

#[cfg(windows)]
#[test]
fn protected_windows_dacl_is_copied_without_enabling_inheritance() {
    fn permissions(path: &Path) -> serde_json::Value {
        let script = r#"
$ErrorActionPreference = 'Stop'
$acl = Get-Acl -LiteralPath $env:RUST_COMMANDER_ACL_TEST_PATH
$rules = @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]) | ForEach-Object {
    [pscustomobject]@{
        Sid = $_.IdentityReference.Value
        Rights = [int]$_.FileSystemRights
        Type = [int]$_.AccessControlType
        Inheritance = [int]$_.InheritanceFlags
        Propagation = [int]$_.PropagationFlags
        Inherited = $_.IsInherited
    }
})
[pscustomobject]@{ Protected = $acl.AreAccessRulesProtected; Rules = $rules } | ConvertTo-Json -Compress -Depth 4
"#;
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("RUST_COMMANDER_ACL_TEST_PATH", path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"source").unwrap();
    fs::write(&target, b"target").unwrap();
    let user = Command::new("whoami.exe").output().unwrap();
    assert!(user.status.success());
    let user = String::from_utf8(user.stdout).unwrap();
    let result = Command::new("icacls.exe")
        .arg(&source)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{}:(F)", user.trim()))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    rust_commander::file_metadata::copy(&source, &target).unwrap();
    // Windows can rewrite descriptor layout and auto-inheritance bookkeeping
    // when applying a DACL. Compare permissions through an independent API,
    // rather than requiring two different files' raw descriptors to match.
    let expected = permissions(&source);
    assert_eq!(expected["Protected"], true);
    assert!(!expected["Rules"].as_array().unwrap().is_empty());
    assert_eq!(permissions(&target), expected);
    assert_eq!(fs::read(&target).unwrap(), b"target");
    let before = rust_commander::file_metadata::digest(&target).unwrap();
    assert_eq!(
        rust_commander::file_metadata::digest(&target).unwrap(),
        before
    );
    let result = Command::new("icacls.exe")
        .arg(&target)
        .arg("/grant:r")
        .arg(format!("{}:(R)", user.trim()))
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_ne!(
        rust_commander::file_metadata::digest(&target).unwrap(),
        before
    );
    rust_commander::file_metadata::copy(&source, &target).unwrap();
}

#[test]
fn remote_delete_handles_empty_folders_and_overlapping_selections() {
    let root = tempfile::tempdir().unwrap();
    let storage = root.path().join("storage");
    fs::create_dir_all(storage.join("tree/child/empty")).unwrap();
    fs::write(storage.join("tree/child/file"), b"selected").unwrap();
    fs::write(storage.join("outside"), b"keep").unwrap();
    let location = mount(&storage, &format!("delete-{:016x}", rand::random::<u64>()));
    let Location::Remote { connection, .. } = location else {
        unreachable!()
    };
    let parent = remote::list(&connection, Path::new(""))
        .unwrap()
        .into_iter()
        .find(|entry| entry.name == "tree")
        .unwrap();
    let child = remote::list(&connection, Path::new("tree"))
        .unwrap()
        .remove(0);
    remote::delete(
        &connection,
        &[parent.clone(), child, parent],
        &mut Task::default(),
    )
    .unwrap();
    assert!(!storage.join("tree").exists());
    assert_eq!(fs::read(storage.join("outside")).unwrap(), b"keep");
}
