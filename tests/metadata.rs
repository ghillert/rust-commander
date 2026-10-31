use rust_commander::metadata;
use std::fs;

#[test]
fn modes_and_acl_entries_include_special_bits_and_reject_invalid_inputs() {
    for (mode, text) in [
        (0o755, "rwxr-xr-x"),
        (0o4644, "rwSr--r--"),
        (0o2755, "rwxr-sr-x"),
        (0o1700, "rwx-----T"),
        (0o7777, "rwsrwsrwt"),
    ] {
        assert_eq!(metadata::symbolic(mode), text);
        assert_eq!(metadata::parse_mode(&format!("0o{mode:o}")).unwrap(), mode);
    }
    for value in ["", "888", "-1", "10000", "rwxr-xr-x"] {
        assert!(metadata::parse_mode(value).is_err());
    }
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, bits) in [(1u16, 7u16), (4, 5), (16, 5), (32, 0)] {
        acl.extend(tag.to_le_bytes());
        acl.extend(bits.to_le_bytes());
        acl.extend(u32::MAX.to_le_bytes());
    }
    assert_eq!(
        metadata::decode_posix_acl(&acl).unwrap(),
        ["user::rwx", "group::r-x", "mask::r-x", "other::---"]
    );
    for broken in [vec![], vec![1, 0, 0, 0], acl[..acl.len() - 1].to_vec()] {
        assert!(metadata::decode_posix_acl(&broken).is_err());
    }
    acl[4] = 99;
    assert!(metadata::decode_posix_acl(&acl).is_err());
    acl[4] = 1;
    acl[6] = 8;
    assert!(metadata::decode_posix_acl(&acl).is_err());
}

#[test]
fn native_metadata_resolves_current_owner_and_reports_permissions_and_acl() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("café metadata.txt");
    fs::write(&file, b"fixture").unwrap();
    assert_eq!(
        metadata::ownership_labels(std::path::Path::new("relative-virtual-entry")),
        (None, None)
    );
    assert!(metadata::permission_key(std::path::Path::new("relative-virtual-entry")).is_none());
    let text = metadata::describe(&file).unwrap();
    assert!(text.contains("Owner: "), "{text}");
    assert!(text.contains("Group: "), "{text}");
    assert!(text.contains("ACL:"), "{text}");
    assert!(!text.contains("ACL: unavailable"), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert!(text.contains("Permissions: "));
        let raw = fs::metadata(&file).unwrap().uid().to_string();
        // Resolution is asynchronous in pane columns. Bound the fixture wait.
        let start = std::time::Instant::now();
        loop {
            let (owner, group) = metadata::ownership_labels(&file);
            assert!(group.is_some());
            if owner.as_deref() != Some(&raw) {
                break;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "{text}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    #[cfg(windows)]
    {
        assert!(text.contains("Read-only: false"));
        assert!(text.contains("Windows security descriptor (SDDL):"));
    }
}

#[cfg(unix)]
#[test]
fn permission_operations_preserve_special_bits_and_undo_the_complete_mode() {
    use rust_commander::{
        history,
        operations::{Operation, Task},
    };
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("file");
    fs::write(&file, b"fixture").unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
    let journal = root.path().join("history");
    let record = history::execute_in(
        Operation::Permissions {
            paths: vec![file.clone()],
            mode: 0o1750,
        },
        &mut Task::default(),
        &journal,
    )
    .unwrap();
    assert!(record.success, "{}", record.result);
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o7777,
        0o1750
    );
    let undone = history::execute_in(
        Operation::Undo(Box::new(record)),
        &mut Task::default(),
        &journal,
    )
    .unwrap();
    assert!(undone.success, "{}", undone.result);
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o7777,
        0o640
    );
    let invalid = history::execute_in(
        Operation::Permissions {
            paths: vec![file.clone()],
            mode: 0o10000,
        },
        &mut Task::default(),
        &journal,
    )
    .unwrap();
    assert!(!invalid.success);
    assert_eq!(fs::read(file).unwrap(), b"fixture");
}

#[cfg(target_os = "macos")]
#[test]
fn appkit_host_properties_show_a_real_extended_acl_without_following_symlinks() {
    use std::{os::unix::fs::symlink, process::Command};
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("acl.txt");
    fs::write(&file, b"fixture").unwrap();
    assert!(
        Command::new("chmod")
            .arg("+a")
            .arg("everyone allow read")
            .arg(&file)
            .status()
            .unwrap()
            .success()
    );
    let acl = metadata::acl_text(&file).unwrap().unwrap();
    assert!(acl.contains("everyone") && acl.contains("read"), "{acl}");
    let link = root.path().join("link");
    symlink(&file, &link).unwrap();
    assert!(!metadata::describe(&link).unwrap().contains("ACL:"));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_properties_show_a_real_named_acl_and_mask() {
    use std::os::unix::ffi::OsStrExt;
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("acl.txt");
    fs::write(&file, b"fixture").unwrap();
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, bits, id) in [
        (1u16, 6u16, u32::MAX),
        (2, 4, 123456),
        (4, 0, u32::MAX),
        (16, 4, u32::MAX),
        (32, 0, u32::MAX),
    ] {
        acl.extend(tag.to_le_bytes());
        acl.extend(bits.to_le_bytes());
        acl.extend(id.to_le_bytes());
    }
    let path = std::ffi::CString::new(file.as_os_str().as_bytes()).unwrap();
    assert_eq!(
        unsafe {
            libc::setxattr(
                path.as_ptr(),
                c"system.posix_acl_access".as_ptr(),
                acl.as_ptr().cast(),
                acl.len(),
                0,
            )
        },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    let text = metadata::acl_text(&file).unwrap().unwrap();
    assert!(
        text.contains("user:123456:r--") && text.contains("mask::r--"),
        "{text}"
    );
}

#[cfg(windows)]
#[test]
fn permission_column_sorts_by_the_displayed_windows_read_only_attribute() {
    use rust_commander::filesystem::{self, Location, Sort};
    let root = tempfile::tempdir().unwrap();
    let writable = root.path().join("z-writable");
    let readonly = root.path().join("a-readonly");
    fs::write(&writable, b"fixture").unwrap();
    fs::write(&readonly, b"fixture").unwrap();
    let original = fs::metadata(&readonly).unwrap().permissions();
    let mut permissions = original.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&readonly, permissions).unwrap();
    let mut entries = filesystem::list(&Location::Local(root.path().into())).unwrap();
    filesystem::sort_entries(&mut entries, Sort::Permissions, false);
    assert_eq!(entries[0].path, writable);
    filesystem::sort_entries(&mut entries, Sort::Permissions, true);
    assert_eq!(entries[0].path, readonly);
    fs::set_permissions(readonly, original).unwrap();
}
