//! Readable identities and permission/ACL presentation. Name-service lookups
//! requested by pane columns run off the UI thread and fall back to IDs.
use anyhow::{Result, ensure};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
};
#[cfg(windows)]
mod windows;
type Key = (bool, String);
#[derive(Default)]
struct Names {
    values: HashMap<Key, Option<String>>,
    pending: HashSet<Key>,
}
static NAMES: OnceLock<Mutex<Names>> = OnceLock::new();
static REVISION: AtomicU64 = AtomicU64::new(0);
fn names() -> &'static Mutex<Names> {
    NAMES.get_or_init(Default::default)
}
pub fn names_revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}
fn queue() -> &'static SyncSender<Key> {
    static QUEUE: OnceLock<SyncSender<Key>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (send, receive) = mpsc::sync_channel::<Key>(128);
        std::thread::spawn(move || {
            while let Ok(key) = receive.recv() {
                let value = resolve_identity(&key);
                let mut cache = names().lock().unwrap_or_else(|e| e.into_inner());
                if cache.values.len() >= 4096 {
                    cache.values.clear();
                }
                cache.pending.remove(&key);
                cache.values.insert(key, value);
                REVISION.fetch_add(1, Ordering::Relaxed);
            }
        });
        send
    })
}
fn identity(key: Key, blocking: bool) -> String {
    {
        let mut cache = names().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = cache.values.get(&key) {
            return value.clone().unwrap_or_else(|| key.1.clone());
        }
        if !blocking {
            if cache.pending.len() < 128
                && cache.pending.insert(key.clone())
                && queue().try_send(key.clone()).is_err()
            {
                cache.pending.remove(&key);
            }
            return key.1;
        }
    }
    resolve_identity(&key).unwrap_or(key.1)
}
#[cfg(unix)]
fn resolve_identity(key: &Key) -> Option<String> {
    use std::{ffi::CStr, mem::MaybeUninit};
    let id = key.1.parse::<u32>().ok()?;
    let mut size = 1024;
    while size <= 1024 * 1024 {
        let mut buffer = vec![0u8; size];
        if key.0 {
            let mut entry = MaybeUninit::<libc::group>::uninit();
            let mut result = std::ptr::null_mut();
            let error = unsafe {
                libc::getgrgid_r(
                    id,
                    entry.as_mut_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                    &mut result,
                )
            };
            if error == libc::ERANGE {
                size *= 2;
                continue;
            }
            if error != 0 || result.is_null() {
                return None;
            }
            return unsafe { CStr::from_ptr((*result).gr_name) }
                .to_str()
                .ok()
                .map(str::to_owned);
        }
        let mut entry = MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let error = unsafe {
            libc::getpwuid_r(
                id,
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if error == libc::ERANGE {
            size *= 2;
            continue;
        }
        if error != 0 || result.is_null() {
            return None;
        }
        return unsafe { CStr::from_ptr((*result).pw_name) }
            .to_str()
            .ok()
            .map(str::to_owned);
    }
    None
}
#[cfg(windows)]
fn resolve_identity(key: &Key) -> Option<String> {
    windows::account_name(&key.1)
}
#[cfg(not(any(unix, windows)))]
fn resolve_identity(_key: &Key) -> Option<String> {
    None
}

pub fn ownership_labels(path: &Path) -> (Option<String>, Option<String>) {
    if !path.is_absolute() {
        return (None, None);
    }
    #[cfg(unix)]
    {
        let (owner, group, _) = crate::filesystem::ownership(path);
        (
            owner.map(|id| identity((false, id.to_string()), false)),
            group.map(|id| identity((true, id.to_string()), false)),
        )
    }
    #[cfg(windows)]
    {
        windows::ownership(path)
            .map(|(owner, group)| {
                (
                    owner.map(|id| identity((false, id), false)),
                    group.map(|id| identity((true, id), false)),
                )
            })
            .unwrap_or_default()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        (None, None)
    }
}
pub fn permission_key(path: &Path) -> Option<u32> {
    if !path.is_absolute() {
        return None;
    }
    #[cfg(unix)]
    {
        crate::filesystem::ownership(path).2
    }
    #[cfg(windows)]
    {
        std::fs::symlink_metadata(path)
            .ok()
            .map(|m| u32::from(m.permissions().readonly()))
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}
pub fn parse_mode(value: &str) -> Result<u32> {
    let value = value.trim().strip_prefix("0o").unwrap_or(value.trim());
    ensure!(
        !value.is_empty() && value.chars().all(|c| ('0'..='7').contains(&c)),
        "Enter octal permissions, for example 0644 or 0755."
    );
    let mode = u32::from_str_radix(value, 8)?;
    ensure!(mode <= 0o7777, "Permissions must be between 0000 and 7777.");
    Ok(mode)
}
pub const PERMISSION_BITS: [(&str, u32); 12] = [
    ("Owner: Read", 0o400),
    ("Owner: Write", 0o200),
    ("Owner: Execute", 0o100),
    ("Group: Read", 0o040),
    ("Group: Write", 0o020),
    ("Group: Execute", 0o010),
    ("Other: Read", 0o004),
    ("Other: Write", 0o002),
    ("Other: Execute", 0o001),
    ("Set user ID", 0o4000),
    ("Set group ID", 0o2000),
    ("Sticky bit", 0o1000),
];
pub fn symbolic(mode: u32) -> String {
    let mut output = String::new();
    for (shift, special, letter) in [(6, 0o4000, 's'), (3, 0o2000, 's'), (0, 0o1000, 't')] {
        let bits = (mode >> shift) & 7;
        output.push(if bits & 4 != 0 { 'r' } else { '-' });
        output.push(if bits & 2 != 0 { 'w' } else { '-' });
        output.push(if mode & special != 0 {
            if bits & 1 != 0 {
                letter
            } else {
                letter.to_ascii_uppercase()
            }
        } else if bits & 1 != 0 {
            'x'
        } else {
            '-'
        });
    }
    output
}
pub fn describe(path: &Path) -> Result<String> {
    let mut lines = Vec::new();
    let metadata = std::fs::symlink_metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        lines.push(format!(
            "Owner: {} ({})",
            identity((false, metadata.uid().to_string()), true),
            metadata.uid()
        ));
        lines.push(format!(
            "Group: {} ({})",
            identity((true, metadata.gid().to_string()), true),
            metadata.gid()
        ));
        lines.push(format!(
            "Permissions: {} ({:04o})",
            symbolic(metadata.mode()),
            metadata.mode() & 0o7777
        ));
    }
    #[cfg(windows)]
    {
        if let Ok((owner, group)) = windows::ownership(path) {
            if let Some(id) = owner {
                lines.push(format!(
                    "Owner: {} ({id})",
                    identity((false, id.clone()), true)
                ));
            }
            if let Some(id) = group {
                lines.push(format!(
                    "Group: {} ({id})",
                    identity((true, id.clone()), true)
                ));
            }
        }
        lines.push(format!("Read-only: {}", metadata.permissions().readonly()));
    }
    if !metadata.file_type().is_symlink() {
        match acl_text(path) {
            Ok(Some(acl)) => lines.push(format!("ACL:\n{acl}")),
            Ok(None) => lines.push("ACL: no additional entries".into()),
            Err(error) => lines.push(format!("ACL: unavailable ({error})")),
        }
    }
    Ok(lines.join("\n"))
}
#[cfg(target_os = "macos")]
pub fn acl_text(path: &Path) -> Result<Option<String>> {
    use std::{
        ffi::{CStr, CString},
        os::unix::ffi::OsStrExt,
    };
    unsafe extern "C" {
        fn acl_get_link_np(path: *const libc::c_char, kind: libc::c_int) -> *mut libc::c_void;
        fn acl_to_text(acl: *mut libc::c_void, len: *mut libc::ssize_t) -> *mut libc::c_char;
        fn acl_free(value: *mut libc::c_void) -> libc::c_int;
    }
    let native_path = CString::new(path.as_os_str().as_bytes())?;
    let acl = unsafe { acl_get_link_np(native_path.as_ptr(), 0x100) };
    if acl.is_null() {
        let error = std::io::Error::last_os_error();
        // Darwin returns ENOENT for an existing object without an extended ACL.
        // A missing object still reports the original error.
        if error.raw_os_error() == Some(libc::ENOENT) && std::fs::symlink_metadata(path).is_ok() {
            return Ok(None);
        }
        return Err(error.into());
    }
    let mut len = 0;
    let text = unsafe { acl_to_text(acl, &mut len) };
    let value = if text.is_null() {
        None
    } else {
        Some(
            unsafe { CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned(),
        )
    };
    unsafe {
        if !text.is_null() {
            acl_free(text.cast());
        }
        acl_free(acl);
    }
    Ok(value.filter(|text| {
        text.lines()
            .any(|line| !line.trim().is_empty() && !line.starts_with('!'))
    }))
}
#[cfg(target_os = "linux")]
pub fn acl_text(path: &Path) -> Result<Option<String>> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut lines = Vec::new();
    for (name, prefix) in [
        (c"system.posix_acl_access", ""),
        (c"system.posix_acl_default", "default:"),
    ] {
        let n = unsafe { libc::lgetxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) };
        if n < 0 {
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::ENODATA | libc::ENOTSUP)) {
                continue;
            }
            return Err(error.into());
        }
        ensure!(n <= 1024 * 1024, "ACL metadata is too large");
        let mut bytes = vec![0u8; n as usize];
        let n = unsafe {
            libc::lgetxattr(
                path.as_ptr(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        for line in decode_posix_acl(&bytes[..n as usize])? {
            lines.push(format!("{prefix}{line}"));
        }
    }
    Ok((!lines.is_empty()).then(|| lines.join("\n")))
}
#[cfg(windows)]
pub fn acl_text(path: &Path) -> Result<Option<String>> {
    windows::acl_text(path).map(Some)
}
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn acl_text(_path: &Path) -> Result<Option<String>> {
    Ok(None)
}
pub fn decode_posix_acl(bytes: &[u8]) -> Result<Vec<String>> {
    ensure!(
        bytes.len() >= 4
            && (bytes.len() - 4).is_multiple_of(8)
            && u32::from_le_bytes(bytes[..4].try_into()?) == 2,
        "Unrecognized POSIX ACL format"
    );
    bytes[4..]
        .as_chunks::<8>()
        .0
        .iter()
        .map(|entry| {
            let tag = u16::from_le_bytes(entry[..2].try_into()?);
            let bits = u16::from_le_bytes(entry[2..4].try_into()?);
            let id = u32::from_le_bytes(entry[4..].try_into()?);
            ensure!(bits <= 7, "Invalid ACL permission bits");
            let subject = match tag {
                1 => "user:".into(),
                2 => format!("user:{}", identity((false, id.to_string()), true)),
                4 => "group:".into(),
                8 => format!("group:{}", identity((true, id.to_string()), true)),
                16 => "mask:".into(),
                32 => "other:".into(),
                _ => anyhow::bail!("Unrecognized POSIX ACL entry"),
            };
            Ok(format!("{subject}:{}", &symbolic(bits as u32)[6..]))
        })
        .collect()
}
