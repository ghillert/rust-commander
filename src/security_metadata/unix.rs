use super::*;
use anyhow::{Context, bail};
use std::{
    ffi::{CStr, CString},
    fs::{File, OpenOptions},
    mem::MaybeUninit,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
};

pub(super) fn open(path: &Path, _change: Option<(&Snapshot, &Change)>) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .context("Cannot open this object for guarded security editing")
}
pub(super) fn read(path: &Path, file: &File) -> Result<Snapshot> {
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() || meta.is_dir(),
        "Only regular files and directories support this editor"
    );
    Ok(Snapshot {
        path: path.into(),
        identity: format!("{}:{}", meta.dev(), meta.ino()),
        owner: meta.uid().to_string(),
        group: meta.gid().to_string(),
        acl: acl(file, meta.mode())?,
        mode: meta.mode() & 0o7777,
        directory: meta.is_dir(),
    })
}
fn id(value: &str, group: bool, current: &str) -> Result<u32> {
    let value = if value.trim().is_empty() {
        current
    } else {
        value.trim()
    };
    if let Ok(value) = value.parse::<u32>() {
        ensure!(value != u32::MAX, "Reserved identity ID");
        return Ok(value);
    }
    let name = CString::new(value)?;
    let mut size = 1024;
    while size <= 1024 * 1024 {
        let mut buffer = vec![0u8; size];
        if group {
            let mut entry = MaybeUninit::<libc::group>::uninit();
            let mut result = std::ptr::null_mut();
            let status = unsafe {
                libc::getgrnam_r(
                    name.as_ptr(),
                    entry.as_mut_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                )
            };
            if status == libc::ERANGE {
                size *= 2;
                continue;
            }
            ensure!(status == 0 && !result.is_null(), "Unknown group name");
            return Ok(unsafe { (*result).gr_gid });
        }
        let mut entry = MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwnam_r(
                name.as_ptr(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE {
            size *= 2;
            continue;
        }
        ensure!(status == 0 && !result.is_null(), "Unknown owner name");
        return Ok(unsafe { (*result).pw_uid });
    }
    bail!("Identity lookup exceeded its size limit")
}
pub(super) fn apply(file: &File, expected: &Snapshot, change: &Change) -> Result<()> {
    let uid = id(&change.owner, false, &expected.owner)?;
    let gid = id(&change.group, true, &expected.group)?;
    let before_uid = expected.owner.parse::<u32>()?;
    let before_gid = expected.group.parse::<u32>()?;
    validate_acl(&change.acl, expected.directory)?;
    let changed_acl = change.acl != expected.acl;
    if changed_acl {
        set_acl(file, &change.acl, expected.directory)?;
    }
    if uid != before_uid || gid != before_gid {
        // No privilege escalation: the kernel decides whether the current user
        // may change ownership. Restore ACLs if ownership is denied.
        let status = unsafe {
            libc::fchown(
                file.as_raw_fd(),
                if uid == before_uid { u32::MAX } else { uid },
                if gid == before_gid { u32::MAX } else { gid },
            )
        };
        if status != 0 {
            let error = std::io::Error::last_os_error();
            if changed_acl && let Err(rollback) = set_acl(file, &expected.acl, expected.directory) {
                bail!(
                    "Ownership failed ({error}); ACL rollback failed ({rollback}). Reload the actual metadata."
                );
            }
            if changed_acl {
                ensure!(
                    unsafe { libc::fchmod(file.as_raw_fd(), expected.mode as libc::mode_t) } == 0,
                    "Ownership failed and original mode could not be restored; reload security metadata"
                );
            }
            return Err(error)
                .context("Ownership change was denied; existing ACL rules were retained");
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn attribute(file: &File, name: &CStr) -> Result<Option<Vec<u8>>> {
    let size = unsafe { libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        let e = std::io::Error::last_os_error();
        if matches!(e.raw_os_error(), Some(libc::ENODATA | libc::ENOTSUP)) {
            return Ok(None);
        }
        return Err(e.into());
    }
    ensure!(size <= 64 * 1024, "ACL is too large");
    let mut bytes = vec![0u8; size as usize];
    let size = unsafe {
        libc::fgetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if size < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    bytes.truncate(size as usize);
    Ok(Some(bytes))
}
#[cfg(target_os = "linux")]
fn numeric_acl(bytes: &[u8], prefix: &str) -> Result<Vec<String>> {
    ensure!(
        bytes.len() >= 4 && (bytes.len() - 4).is_multiple_of(8) && bytes[..4] == 2u32.to_le_bytes(),
        "Invalid POSIX ACL storage"
    );
    bytes[4..]
        .as_chunks::<8>()
        .0
        .iter()
        .map(|entry| {
            let tag = u16::from_le_bytes(entry[..2].try_into()?);
            let bits = u16::from_le_bytes(entry[2..4].try_into()?);
            let id = u32::from_le_bytes(entry[4..].try_into()?);
            ensure!(bits <= 7, "Invalid ACL permissions");
            let subject = match tag {
                1 => "user:".into(),
                2 => format!("user:{id}"),
                4 => "group:".into(),
                8 => format!("group:{id}"),
                16 => "mask:".into(),
                32 => "other:".into(),
                _ => bail!("Unknown ACL entry"),
            };
            Ok(format!(
                "{prefix}{subject}:{}",
                &crate::metadata::symbolic(bits as u32)[6..]
            ))
        })
        .collect()
}
#[cfg(target_os = "linux")]
fn acl(file: &File, mode: u32) -> Result<String> {
    let mut lines = if let Some(bytes) = attribute(file, c"system.posix_acl_access")? {
        numeric_acl(&bytes, "")?
    } else {
        vec![
            format!("user::{}", &crate::metadata::symbolic(mode)[..3]),
            format!("group::{}", &crate::metadata::symbolic(mode)[3..6]),
            format!("other::{}", &crate::metadata::symbolic(mode)[6..]),
        ]
    };
    if let Some(bytes) = attribute(file, c"system.posix_acl_default")? {
        lines.extend(numeric_acl(&bytes, "default:")?);
    }
    Ok(lines.join("\n"))
}
#[cfg(target_os = "linux")]
fn parse_acl(text: &str, directory: bool) -> Result<[Option<Vec<u8>>; 2]> {
    let mut lists = [Vec::new(), Vec::new()];
    for line in text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        let (default, line) = line
            .strip_prefix("default:")
            .map_or((false, line), |s| (true, s));
        ensure!(!default || directory, "Default ACLs require a directory");
        let parts = line.split(':').collect::<Vec<_>>();
        ensure!(
            parts.len() == 3,
            "Use POSIX ACL entries such as user:name:rwx"
        );
        let (tag, qualifier) = match (parts[0], parts[1]) {
            ("user", "") => (1, u32::MAX),
            ("user", name) => (2, id(name, false, "")?),
            ("group", "") => (4, u32::MAX),
            ("group", name) => (8, id(name, true, "")?),
            ("mask", "") => (16, u32::MAX),
            ("other", "") => (32, u32::MAX),
            _ => bail!("Invalid ACL subject"),
        };
        let bytes = parts[2].as_bytes();
        ensure!(
            bytes.len() == 3
                && (bytes[0] == b'r' || bytes[0] == b'-')
                && (bytes[1] == b'w' || bytes[1] == b'-')
                && (bytes[2] == b'x' || bytes[2] == b'-'),
            "ACL permissions must use rwx or -"
        );
        let bits = u16::from(bytes[0] == b'r') * 4
            + u16::from(bytes[1] == b'w') * 2
            + u16::from(bytes[2] == b'x');
        lists[usize::from(default)].push((tag, qualifier, bits));
    }
    let mut result = [None, None];
    for (list, output) in lists.iter_mut().zip(&mut result) {
        if list.is_empty() {
            continue;
        }
        ensure!(list.len() <= 1024, "Too many ACL rules");
        list.sort_by_key(|e| (e.0, e.1));
        ensure!(
            !list
                .windows(2)
                .any(|w| w[0].0 == w[1].0 && w[0].1 == w[1].1),
            "Duplicate ACL entry"
        );
        for tag in [1, 4, 32] {
            ensure!(
                list.iter().any(|e| e.0 == tag),
                "ACL needs user::, group:: and other:: entries"
            );
        }
        ensure!(
            !list.iter().any(|e| matches!(e.0, 2 | 8)) || list.iter().any(|e| e.0 == 16),
            "Named ACL rules require an explicit mask:: entry"
        );
        let mut bytes = 2u32.to_le_bytes().to_vec();
        for &(tag, qualifier, bits) in list.iter() {
            bytes.extend((tag as u16).to_le_bytes());
            bytes.extend(bits.to_le_bytes());
            bytes.extend(qualifier.to_le_bytes());
        }
        *output = Some(bytes);
    }
    Ok(result)
}
#[cfg(target_os = "linux")]
fn validate_acl(text: &str, directory: bool) -> Result<()> {
    parse_acl(text, directory).map(|_| ())
}
#[cfg(target_os = "linux")]
fn put(file: &File, name: &CStr, value: Option<&[u8]>) -> Result<()> {
    let status = if let Some(bytes) = value {
        unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                bytes.as_ptr().cast(),
                bytes.len(),
                0,
            )
        }
    } else {
        unsafe { libc::fremovexattr(file.as_raw_fd(), name.as_ptr()) }
    };
    if status != 0 {
        let e = std::io::Error::last_os_error();
        if value.is_none() && e.raw_os_error() == Some(libc::ENODATA) {
            return Ok(());
        }
        return Err(e.into());
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn set_acl(file: &File, text: &str, directory: bool) -> Result<()> {
    let [access, default] = parse_acl(text, directory)?;
    let before = attribute(file, c"system.posix_acl_access")?;
    let before_mode = file.metadata()?.mode() & 0o7777;
    put(file, c"system.posix_acl_access", access.as_deref())?;
    if directory && let Err(error) = put(file, c"system.posix_acl_default", default.as_deref()) {
        if let Err(rollback) = put(file, c"system.posix_acl_access", before.as_deref()) {
            bail!(
                "Default ACL failed ({error}); access ACL rollback failed ({rollback}). Reload the actual metadata."
            );
        }
        if unsafe { libc::fchmod(file.as_raw_fd(), before_mode) } != 0 {
            bail!(
                "Default ACL failed ({error}); permission rollback failed: {}",
                std::io::Error::last_os_error()
            );
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod apple {
    use super::*;
    use std::ffi::c_void;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: libc::c_int, kind: libc::c_int) -> *mut c_void;
        fn acl_set_fd_np(fd: libc::c_int, acl: *mut c_void, kind: libc::c_int) -> libc::c_int;
        fn acl_from_text(text: *const libc::c_char) -> *mut c_void;
        fn acl_to_text(acl: *mut c_void, len: *mut libc::ssize_t) -> *mut libc::c_char;
        fn acl_init(count: libc::c_int) -> *mut c_void;
        fn acl_valid(acl: *mut c_void) -> libc::c_int;
        fn acl_free(value: *mut c_void) -> libc::c_int;
    }
    struct Acl(*mut c_void);
    impl Drop for Acl {
        fn drop(&mut self) {
            unsafe {
                acl_free(self.0);
            }
        }
    }
    fn parsed(text: &str) -> Result<Acl> {
        let acl = if text.trim().is_empty() {
            unsafe { acl_init(0) }
        } else {
            let text = CString::new(text)?;
            unsafe { acl_from_text(text.as_ptr()) }
        };
        ensure!(
            !acl.is_null(),
            "Invalid macOS ACL rules: {}",
            std::io::Error::last_os_error()
        );
        let acl = Acl(acl);
        ensure!(unsafe { acl_valid(acl.0) } == 0, "Invalid macOS ACL rules");
        Ok(acl)
    }
    pub(super) fn get(file: &File) -> Result<String> {
        let raw = unsafe { acl_get_fd_np(file.as_raw_fd(), 0x100) };
        if raw.is_null() {
            let e = std::io::Error::last_os_error();
            if matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ENOTSUP)) {
                return Ok(String::new());
            }
            return Err(e.into());
        }
        let acl = Acl(raw);
        let mut len = 0;
        let raw = unsafe { acl_to_text(acl.0, &mut len) };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let _text = Acl(raw.cast());
        Ok(unsafe { CStr::from_ptr(raw) }.to_str()?.to_owned())
    }
    pub(super) fn validate(text: &str) -> Result<()> {
        parsed(text).map(|_| ())
    }
    pub(super) fn set(file: &File, text: &str) -> Result<()> {
        let acl = parsed(text)?;
        if unsafe { acl_set_fd_np(file.as_raw_fd(), acl.0, 0x100) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
}
#[cfg(target_os = "macos")]
fn acl(file: &File, _mode: u32) -> Result<String> {
    apple::get(file)
}
#[cfg(target_os = "macos")]
fn validate_acl(text: &str, _directory: bool) -> Result<()> {
    apple::validate(text)
}
#[cfg(target_os = "macos")]
fn set_acl(file: &File, text: &str, _directory: bool) -> Result<()> {
    apple::set(file, text)
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_change_and_invalid_owner_never_change_file_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file");
        std::fs::write(&path, b"keep contents").unwrap();
        let snapshot = super::super::read(&path).unwrap();
        super::super::apply(&snapshot, &snapshot.change()).unwrap();
        let mut change = snapshot.change();
        change.owner = "rust-commander-nonexistent-user-fixture".into();
        change.acl.clear();
        assert!(super::super::apply(&snapshot, &change).is_err());
        assert_eq!(super::super::read(&path).unwrap(), snapshot);
        assert_eq!(std::fs::read(&path).unwrap(), b"keep contents");
    }
    #[test]
    fn replaced_objects_and_symlinks_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file");
        std::fs::write(&path, b"first").unwrap();
        let snapshot = super::super::read(&path).unwrap();
        std::fs::rename(&path, root.path().join("old")).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        assert!(super::super::apply(&snapshot, &snapshot.change()).is_err());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(root.path().join("old"), &path).unwrap();
        assert!(super::super::read(&path).is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn posix_acl_rules_round_trip_and_require_a_mask() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file");
        std::fs::write(&path, b"data").unwrap();
        let before = super::super::read(&path).unwrap();
        let mut change = before.change();
        change.acl = "user::rw-\nuser:65534:r--\ngroup::---\nmask::r--\nother::---".into();
        let after = super::super::apply(&before, &change).unwrap();
        assert!(after.acl.contains("user:65534:r--"));
        assert_eq!(std::fs::read(&path).unwrap(), b"data");
        change.acl = "user::rw-\nuser:65534:r--\ngroup::---\nother::---".into();
        assert!(super::super::apply(&after, &change).is_err());
        assert_eq!(super::super::read(&path).unwrap(), after);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_allow_rules_can_be_edited_and_removed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("acl.txt");
        std::fs::write(&path, b"unchanged").unwrap();
        assert!(
            std::process::Command::new("/bin/chmod")
                .args(["+a", "everyone allow readattr"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let before = super::super::read(&path).unwrap();
        assert!(before.acl.contains("readattr"));
        let mut change = before.change();
        change.acl = change.acl.replace("readattr", "readextattr");
        let after = super::super::apply(&before, &change).unwrap();
        assert!(after.acl.contains("readextattr"));
        change.acl.clear();
        let cleared = super::super::apply(&after, &change).unwrap();
        assert!(!cleared.acl.contains("readextattr"));
        assert_eq!(std::fs::read(&path).unwrap(), b"unchanged");
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn directory_default_acl_is_inherited_and_can_be_removed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("directory");
        std::fs::create_dir(&path).unwrap();
        let before = super::super::read(&path).unwrap();
        let mut change = before.change();
        change.acl = format!(
            "{}\ndefault:user::rwx\ndefault:user:65534:r--\ndefault:group::---\ndefault:mask::r--\ndefault:other::---",
            before.acl
        );
        let after = super::super::apply(&before, &change).unwrap();
        assert!(after.acl.contains("default:user:65534:r--"));
        let child = path.join("child");
        std::fs::write(&child, b"child").unwrap();
        assert!(
            super::super::read(&child)
                .unwrap()
                .acl
                .contains("user:65534:r--")
        );
        change.acl = before.acl;
        let cleared = super::super::apply(&after, &change).unwrap();
        assert!(!cleared.acl.contains("default:"));
    }
}
