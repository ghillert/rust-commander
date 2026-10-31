use anyhow::{Result, ensure};
use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER,
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
            ConvertStringSidToSidW,
        },
        DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, GetAce, GetFileSecurityW,
        GetSecurityDescriptorDacl, GetSecurityDescriptorGroup, GetSecurityDescriptorOwner,
        LookupAccountSidW, OWNER_SECURITY_INFORMATION,
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
fn string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut n = 0;
    unsafe {
        while n < 1024 * 1024 && *ptr.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, n))
    }
}
fn descriptor(path: &Path) -> Result<Vec<usize>> {
    ensure!(
        !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Inspect the symlink target explicitly for security metadata."
    );
    let path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut needed = 0;
    let flags = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    unsafe {
        GetFileSecurityW(path.as_ptr(), flags, std::ptr::null_mut(), 0, &mut needed);
    }
    ensure!(
        needed > 0,
        "Cannot read the file security descriptor: {}",
        std::io::Error::last_os_error()
    );
    let mut bytes = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {
        GetFileSecurityW(
            path.as_ptr(),
            flags,
            bytes.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(bytes)
}
pub(super) fn ownership(path: &Path) -> Result<(Option<String>, Option<String>)> {
    let descriptor = descriptor(path)?;
    let mut owner = std::ptr::null_mut();
    let mut group = std::ptr::null_mut();
    let mut defaulted = 0;
    let pointer = descriptor.as_ptr().cast_mut().cast();
    unsafe {
        if GetSecurityDescriptorOwner(pointer, &mut owner, &mut defaulted) == 0
            || GetSecurityDescriptorGroup(pointer, &mut group, &mut defaulted) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok((sid_text(owner), sid_text(group)))
}
fn sid_text(sid: *mut c_void) -> Option<String> {
    if sid.is_null() {
        return None;
    }
    let mut text = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return None;
    }
    let _memory = Local(text.cast());
    Some(string(text))
}

pub(super) fn account_name(id: &str) -> Option<String> {
    let id: Vec<u16> = id.encode_utf16().chain([0]).collect();
    let mut sid = std::ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(id.as_ptr(), &mut sid) } == 0 {
        return None;
    }
    let _sid = Local(sid);
    let mut n = 0;
    let mut d = 0;
    let mut usage = 0;
    unsafe {
        LookupAccountSidW(
            std::ptr::null(),
            sid,
            std::ptr::null_mut(),
            &mut n,
            std::ptr::null_mut(),
            &mut d,
            &mut usage,
        );
    }
    if n == 0 || n > 65536 || d > 65536 {
        return None;
    }
    let mut name = vec![0u16; n as usize];
    let mut domain = vec![0u16; d as usize + 1];
    if unsafe {
        LookupAccountSidW(
            std::ptr::null(),
            sid,
            name.as_mut_ptr(),
            &mut n,
            domain.as_mut_ptr(),
            &mut d,
            &mut usage,
        )
    } == 0
    {
        return None;
    }
    let name = string(name.as_ptr());
    let domain = string(domain.as_ptr());
    Some(if domain.is_empty() {
        name
    } else {
        format!("{domain}\\{name}")
    })
}
pub(super) fn acl_text(path: &Path) -> Result<String> {
    let descriptor = descriptor(path)?;
    let mut text = std::ptr::null_mut();
    let flags = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    if unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.as_ptr().cast_mut().cast(),
            1,
            flags,
            &mut text,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let _text = Local(text.cast());
    Ok(format!(
        "{}\nWindows security descriptor (SDDL):\n{}",
        describe_dacl(descriptor.as_ptr().cast_mut().cast())?,
        string(text)
    ))
}

// Called with a Windows-owned or GetFileSecurity-validated descriptor whose
// storage remains alive. These are stored access rules, not an effective-access calculation.
fn describe_dacl(descriptor: *mut c_void) -> Result<String> {
    let mut present = 0;
    let mut acl = std::ptr::null_mut();
    let mut defaulted = 0;
    if unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    if present == 0 || acl.is_null() {
        return Ok("Access rules: unrestricted (no DACL)".into());
    }
    let count = unsafe { (*acl).AceCount };
    if count == 0 {
        return Ok("Access rules: empty DACL (no access granted)".into());
    }
    let mut lines = vec![
        "Stored access rules (effective access also depends on group membership):".to_string(),
    ];
    for index in 0..u32::from(count) {
        let mut ace = std::ptr::null_mut();
        if unsafe { GetAce(acl, index, &mut ace) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if !matches!(header.AceType, 0 | 1)
            || usize::from(header.AceSize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        {
            lines.push(format!(
                "Rule {}: type {} (see full SDDL below)",
                index + 1,
                header.AceType
            ));
            continue;
        }
        let entry = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = std::ptr::addr_of!(entry.SidStart).cast_mut().cast();
        let id = sid_text(sid).unwrap_or_else(|| "Unknown SID".into());
        let name = account_name(&id).unwrap_or(id);
        let mut flags = Vec::new();
        for (bit, label) in [
            (1, "files inherit"),
            (2, "folders inherit"),
            (4, "no propagation"),
            (8, "inherit only"),
            (16, "inherited"),
        ] {
            if header.AceFlags & bit != 0 {
                flags.push(label);
            }
        }
        lines.push(format!(
            "{} {}: {}{}",
            if header.AceType == 0 { "Allow" } else { "Deny" },
            name,
            rights(entry.Mask),
            if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(", "))
            }
        ));
    }
    Ok(lines.join("\n"))
}
fn rights(mask: u32) -> String {
    if mask & 0x001f01ff == 0x001f01ff {
        return "Full control".into();
    }
    let mut labels = Vec::new();
    for (bits, label) in [
        (0x80000000, "Generic read"),
        (0x40000000, "Generic write"),
        (0x20000000, "Generic execute"),
        (0x10000000, "Generic all"),
        (1, "Read/list"),
        (2, "Write/create"),
        (4, "Append"),
        (8, "Read extended attributes"),
        (16, "Write extended attributes"),
        (32, "Execute/traverse"),
        (64, "Delete children"),
        (128, "Read attributes"),
        (256, "Write attributes"),
        (0x10000, "Delete"),
        (0x20000, "Read permissions"),
        (0x40000, "Change permissions"),
        (0x80000, "Take ownership"),
        (0x100000, "Synchronize"),
    ] {
        if mask & bits != 0 {
            labels.push(label);
        }
    }
    format!("{} (0x{mask:08x})", labels.join(", "))
}
#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    fn describe(sddl: &str) -> String {
        let input: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
        let mut descriptor = std::ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    input.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let _memory = Local(descriptor);
        describe_dacl(descriptor).unwrap()
    }
    #[test]
    fn native_dacl_decoder_explains_grants_denials_inheritance_and_empty_rules() {
        let text = describe("D:(D;;FW;;;WD)(A;OICIID;FA;;;SY)");
        assert!(
            text.contains("Deny ") && text.contains("Write/create"),
            "{text}"
        );
        assert!(
            text.contains("Allow ") && text.contains("Full control"),
            "{text}"
        );
        assert!(
            text.contains("files inherit, folders inherit, inherited"),
            "{text}"
        );
        assert!(describe("D:").contains("empty DACL"));
        // Build a null DACL explicitly. Some Wine versions interpret the SDDL
        // NO_ACCESS_CONTROL token as an empty DACL, unlike native Windows.
        use windows_sys::Win32::Security::{
            InitializeSecurityDescriptor, SECURITY_DESCRIPTOR, SetSecurityDescriptorDacl,
        };
        let mut descriptor = std::mem::MaybeUninit::<SECURITY_DESCRIPTOR>::uninit();
        let pointer = descriptor.as_mut_ptr().cast();
        assert_ne!(unsafe { InitializeSecurityDescriptor(pointer, 1) }, 0);
        assert_ne!(
            unsafe { SetSecurityDescriptorDacl(pointer, 1, std::ptr::null(), 0) },
            0
        );
        assert!(describe_dacl(pointer).unwrap().contains("unrestricted"));
    }
}
