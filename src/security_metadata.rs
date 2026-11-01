//! Guarded security metadata editing. Operations stay on an opened object, never
//! follow its final symlink, and refuse stale identity/permission snapshots.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub path: PathBuf,
    pub identity: String,
    pub owner: String,
    pub group: String,
    pub acl: String,
    pub mode: u32,
    pub directory: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub owner: String,
    pub group: String,
    pub acl: String,
}
impl Snapshot {
    pub fn change(&self) -> Change {
        Change {
            owner: self.owner.clone(),
            group: self.group.clone(),
            acl: self.acl.clone(),
        }
    }
}
pub fn read(path: &Path) -> Result<Snapshot> {
    ensure!(
        path.is_absolute(),
        "Security metadata requires an absolute local path"
    );
    ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Edit the symlink target explicitly"
    );
    let file = platform::open(path, None)?;
    platform::read(path, &file)
}
pub fn apply(expected: &Snapshot, change: &Change) -> Result<Snapshot> {
    ensure!(change.acl.len() <= 64 * 1024, "ACL rules exceed 64 KiB");
    let file = platform::open(&expected.path, Some((expected, change)))?;
    ensure!(
        platform::read(&expected.path, &file)? == *expected,
        "The file or its security metadata changed. Reload it before applying changes."
    );
    platform::apply(&file, expected, change)?;
    platform::read(&expected.path, &file)
}
pub fn syntax() -> &'static str {
    if cfg!(target_os = "windows") {
        "DACL in Windows SDDL format, starting with D:. Owner/group accept account names or SIDs. A null DACL is refused; inheritance flags are retained as entered."
    } else if cfg!(target_os = "macos") {
        "macOS extended ACL text. Edit the displayed allow/deny rules; blank removes the extended ACL. Owner/group accept names or numeric IDs."
    } else {
        "POSIX ACL: user::rwx, user:name:r-x, group::r-x, mask::r-x, other::---; prefix directory defaults with default:. Named entries need an explicit mask. Blank removes extended/default ACLs."
    }
}
