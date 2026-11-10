//! Discover host-mounted network folders without probing their contents.
use std::path::PathBuf;

pub(crate) fn network_mounts() -> Vec<(String, PathBuf)> {
    let mut paths = network_paths();
    paths.sort_unstable();
    paths.dedup();
    paths.into_iter().map(named_mount).collect()
}

pub(super) fn named_mount(path: PathBuf) -> (String, PathBuf) {
    let name = path.file_name().unwrap_or(path.as_os_str());
    (name.to_string_lossy().into_owned(), path)
}

#[cfg(target_os = "macos")]
fn network_paths() -> Vec<PathBuf> {
    use std::{mem::MaybeUninit, os::unix::ffi::OsStrExt, ptr};

    // SAFETY: A null buffer and zero length request the mount count only.
    let count = unsafe { libc::getfsstat(ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if count <= 0 {
        return Vec::new();
    }
    // Leave space for mounts added between calls and bound the allocation.
    let mut mounts = vec![MaybeUninit::<libc::statfs>::uninit(); (count as usize + 8).min(16_384)];
    // SAFETY: The buffer is writable, properly aligned and sized in bytes.
    let count = unsafe {
        libc::getfsstat(
            mounts.as_mut_ptr().cast(),
            std::mem::size_of_val(mounts.as_slice()) as libc::c_int,
            libc::MNT_NOWAIT,
        )
    };
    if count <= 0 {
        return Vec::new();
    }

    mounts
        .into_iter()
        .take(count as usize)
        .filter_map(|mount| {
            // SAFETY: getfsstat initialized the returned records that fit the buffer.
            let mount = unsafe { mount.assume_init() };
            if mount.f_flags & (libc::MNT_LOCAL | libc::MNT_DONTBROWSE) as u32 != 0 {
                return None;
            }
            let bytes: Vec<_> = mount
                .f_mntonname
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8)
                .collect();
            let path = PathBuf::from(std::ffi::OsStr::from_bytes(&bytes));
            (path.is_absolute() && path.parent().is_some()).then_some(path)
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn network_paths() -> Vec<PathBuf> {
    use windows_sys::Win32::{
        Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives},
        System::WindowsProgramming::DRIVE_REMOTE,
    };

    // SAFETY: GetLogicalDrives has no arguments and returns a drive-letter bitmask.
    let drives = unsafe { GetLogicalDrives() };
    (0..26)
        .filter_map(|index| {
            if drives & (1 << index) == 0 {
                return None;
            }
            let letter = b'A' + index;
            let root = [u16::from(letter), u16::from(b':'), u16::from(b'\\'), 0];
            // SAFETY: root is a NUL-terminated UTF-16 drive root with a trailing slash.
            (unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_REMOTE)
                .then(|| PathBuf::from(format!("{}:\\", char::from(letter))))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn network_paths() -> Vec<PathBuf> {
    linux_mounts()
        .into_iter()
        .filter_map(|(path, network)| network.then_some(path))
        .collect()
}

#[cfg(target_os = "linux")]
pub(super) fn linux_mounts() -> Vec<(PathBuf, bool)> {
    std::fs::read("/proc/self/mountinfo").map_or_else(
        |_| Vec::new(),
        |data| {
            data.split(|&b| b == b'\n')
                .filter_map(linux_mount)
                .collect()
        },
    )
}

#[cfg(any(target_os = "linux", all(test, unix)))]
fn linux_mount(line: &[u8]) -> Option<(PathBuf, bool)> {
    use std::os::unix::ffi::OsStrExt;

    let mut fields = line
        .split(|b| b.is_ascii_whitespace())
        .filter(|f| !f.is_empty());
    let mount = fields.nth(4)?;
    fields.find(|&field| field == b"-")?;
    let kind = fields.next()?;
    let network = matches!(
        kind,
        b"nfs"
            | b"nfs4"
            | b"cifs"
            | b"smb3"
            | b"smbfs"
            | b"afs"
            | b"9p"
            | b"ceph"
            | b"fuse.ceph"
            | b"glusterfs"
            | b"fuse.glusterfs"
            | b"lustre"
            | b"fuse.sshfs"
            | b"davfs"
            | b"fuse.davfs"
    );
    // mountinfo escapes spaces, tabs, newlines and backslashes using octal.
    let mut bytes = Vec::with_capacity(mount.len());
    let mut rest = mount;
    while let Some((&byte, tail)) = rest.split_first() {
        let decoded = match rest.get(..4) {
            Some(b"\\040") => Some(b' '),
            Some(b"\\011") => Some(b'\t'),
            Some(b"\\012") => Some(b'\n'),
            Some(b"\\134") => Some(b'\\'),
            _ => None,
        };
        bytes.push(decoded.unwrap_or(byte));
        rest = if decoded.is_some() { &rest[4..] } else { tail };
    }
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(&bytes));
    path.is_absolute().then_some((path, network))
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn network_paths() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(all(test, unix))]
#[test]
fn linux_mounts_recognize_network_filesystems_and_decode_mount_paths_once() {
    let (path, network) =
        linux_mount(br"44 31 0:38 / /mnt/Team\040Share\134040 rw shared:7 - cifs //nas/team rw")
            .unwrap();
    assert_eq!(path, PathBuf::from(r"/mnt/Team Share\040"));
    assert!(network);
    let (_, network) = linux_mount(b"45 31 8:1 / /media/disk rw - ext4 /dev/sda1 rw").unwrap();
    assert!(!network);
    let (_, network) = linux_mount(b"46 31 0:32 / /home rw - autofs map rw").unwrap();
    assert!(!network);
}
