use super::*;
use anyhow::Context;
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{Authorization::*, *},
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, GetFileInformationByHandle,
        READ_CONTROL, WRITE_DAC, WRITE_OWNER,
    },
};
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
fn string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0;
    unsafe {
        while len < 1024 * 1024 && *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}
fn check(status: u32) -> Result<()> {
    if status != 0 {
        Err(std::io::Error::from_raw_os_error(status as i32).into())
    } else {
        Ok(())
    }
}
pub(super) fn open(path: &Path, change: Option<(&Snapshot, &Change)>) -> Result<File> {
    let mut rights = READ_CONTROL | FILE_READ_ATTRIBUTES;
    if let Some((old, new)) = change {
        if !new.owner.trim().is_empty() && new.owner != old.owner
            || !new.group.trim().is_empty() && new.group != old.group
        {
            rights |= WRITE_OWNER;
        }
        if new.acl != old.acl {
            rights |= WRITE_DAC;
        }
    }
    OpenOptions::new()
        .access_mode(rights)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .context("Cannot open this object with the requested security access")
}
fn sid_text(sid: PSID) -> Result<String> {
    let mut text = null_mut();
    ensure!(
        unsafe { ConvertSidToStringSidW(sid, &mut text) } != 0,
        "Cannot format Windows identity"
    );
    let _text = Local(text.cast());
    Ok(string(text))
}
fn descriptor(file: &File) -> Result<Local> {
    let mut pointer = null_mut();
    check(unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut pointer,
        )
    })?;
    Ok(Local(pointer))
}
pub(super) fn read(path: &Path, file: &File) -> Result<Snapshot> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "Edit the reparse-point target explicitly"
    );
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    ensure!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0,
        "Cannot read file identity"
    );
    let sd = descriptor(file)?;
    let (mut owner, mut group, mut text) = (null_mut(), null_mut(), null_mut());
    let mut defaulted = 0;
    ensure!(
        unsafe { GetSecurityDescriptorOwner(sd.0, &mut owner, &mut defaulted) } != 0
            && unsafe { GetSecurityDescriptorGroup(sd.0, &mut group, &mut defaulted) } != 0,
        "Cannot read Windows owner/group"
    );
    ensure!(
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd.0,
                1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                null_mut(),
            )
        } != 0,
        "Cannot format Windows DACL"
    );
    let _text = Local(text.cast());
    Ok(Snapshot {
        path: path.into(),
        identity: format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ),
        owner: sid_text(owner)?,
        group: if group.is_null() {
            String::new()
        } else {
            sid_text(group)?
        },
        acl: string(text),
        mode: 0,
        directory: metadata.is_dir(),
    })
}
fn account(value: &str) -> Result<Local> {
    let value = wide(value);
    let mut sid = null_mut();
    if unsafe { ConvertStringSidToSidW(value.as_ptr(), &mut sid) } != 0 {
        return Ok(Local(sid));
    }
    let (mut size, mut domain_size, mut usage) = (0, 0, 0);
    unsafe {
        LookupAccountNameW(
            null(),
            value.as_ptr(),
            null_mut(),
            &mut size,
            null_mut(),
            &mut domain_size,
            &mut usage,
        );
    }
    ensure!(
        size > 0 && size <= 65536 && domain_size <= 65536,
        "Unknown Windows account"
    );
    // LocalAlloc supplies aligned storage and the same RAII ownership as SID parsing.
    let sid = unsafe { windows_sys::Win32::System::Memory::LocalAlloc(0, size as usize) };
    ensure!(!sid.is_null(), "Cannot allocate Windows identity");
    let sid = Local(sid);
    let mut domain = vec![0; domain_size as usize + 1];
    ensure!(
        unsafe {
            LookupAccountNameW(
                null(),
                value.as_ptr(),
                sid.0,
                &mut size,
                domain.as_mut_ptr(),
                &mut domain_size,
                &mut usage,
            )
        } != 0,
        "Cannot resolve Windows account"
    );
    Ok(sid)
}
pub(super) fn apply(file: &File, old: &Snapshot, new: &Change) -> Result<()> {
    let mut flags = 0;
    let mut owner = None;
    let mut group = None;
    let mut acl = None;
    let mut dacl = null_mut();
    if !new.owner.trim().is_empty() && new.owner != old.owner {
        owner = Some(account(new.owner.trim())?);
        flags |= OWNER_SECURITY_INFORMATION;
    }
    if !new.group.trim().is_empty() && new.group != old.group {
        group = Some(account(new.group.trim())?);
        flags |= GROUP_SECURITY_INFORMATION;
    }
    if new.acl != old.acl {
        ensure!(
            new.acl.starts_with("D:")
                && !new.acl.contains("O:")
                && !new.acl.contains("G:")
                && !new.acl.contains("S:"),
            "Enter a DACL only, beginning with D:"
        );
        let mut pointer = null_mut();
        ensure!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide(&new.acl).as_ptr(),
                    1,
                    &mut pointer,
                    null_mut(),
                )
            } != 0,
            "Invalid Windows DACL SDDL: {}",
            std::io::Error::last_os_error()
        );
        let sd = Local(pointer);
        let (mut present, mut defaulted) = (0, 0);
        ensure!(
            unsafe { GetSecurityDescriptorDacl(sd.0, &mut present, &mut dacl, &mut defaulted) }
                != 0
                && present != 0
                && !dacl.is_null(),
            "A null DACL is refused because it grants unrestricted access"
        );
        let (mut control, mut revision) = (0, 0);
        ensure!(
            unsafe { GetSecurityDescriptorControl(sd.0, &mut control, &mut revision) } != 0,
            "Invalid security descriptor"
        );
        flags |= DACL_SECURITY_INFORMATION
            | if control & SE_DACL_PROTECTED != 0 {
                PROTECTED_DACL_SECURITY_INFORMATION
            } else {
                UNPROTECTED_DACL_SECURITY_INFORMATION
            };
        acl = Some(sd);
    }
    if flags == 0 {
        return Ok(());
    }
    let _acl = acl;
    check(unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            flags,
            owner.as_ref().map_or(null_mut(), |v| v.0),
            group.as_ref().map_or(null_mut(), |v| v.0),
            dacl,
            null_mut(),
        )
    })
    .context("Windows denied the security change; no privilege escalation was requested")
}
