//! Platform metadata copied before committing a new local file.
use anyhow::Result;
use std::path::Path;

#[cfg(target_os = "macos")]
pub fn copy(source: &Path, target: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn copyfile(
            from: *const libc::c_char,
            to: *const libc::c_char,
            state: *mut libc::c_void,
            flags: u32,
        ) -> libc::c_int;
    }
    let from = CString::new(source.as_os_str().as_bytes())?;
    let to = CString::new(target.as_os_str().as_bytes())?;
    // ACL + xattrs include Finder tags, quarantine and resource forks. Do not
    // copy data again or follow links; mode/mtime are handled by the caller.
    if unsafe {
        copyfile(
            from.as_ptr(),
            to.as_ptr(),
            std::ptr::null_mut(),
            1 | 4 | (1 << 18) | (1 << 19),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn copy(source: &Path, target: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    let n = unsafe { libc::listxattr(source.as_ptr(), std::ptr::null_mut(), 0) };
    if n < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ENOTSUP) {
            return Ok(());
        }
        return Err(error.into());
    }
    let mut names = vec![0u8; n as usize];
    let n = unsafe { libc::listxattr(source.as_ptr(), names.as_mut_ptr().cast(), names.len()) };
    if n < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    for name in names[..n as usize]
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
    {
        // User metadata and POSIX ACLs are transferable by the owning user.
        // Kernel-managed security/trusted attributes require separate privileges.
        if !name.starts_with(b"user.") && !name.starts_with(b"system.posix_acl_") {
            continue;
        }
        let name = CString::new(name)?;
        let n = unsafe { libc::getxattr(source.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) };
        if n < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut value = vec![0u8; n as usize];
        let n = unsafe {
            libc::getxattr(
                source.as_ptr(),
                name.as_ptr(),
                value.as_mut_ptr().cast(),
                value.len(),
            )
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if unsafe {
            libc::setxattr(
                target.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                n as usize,
                0,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn copy(source: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GetFileSecurityW, GetSecurityDescriptorControl,
        PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED, SetFileSecurityW,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain([0]).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain([0]).collect();
    let mut needed = 0;
    unsafe {
        GetFileSecurityW(
            source.as_ptr(),
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            0,
            &mut needed,
        );
    }
    if needed == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // Pointer-sized storage keeps the SECURITY_DESCRIPTOR aligned.
    let mut descriptor = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {
        GetFileSecurityW(
            source.as_ptr(),
            DACL_SECURITY_INFORMATION,
            descriptor.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut control = 0;
    let mut revision = 0;
    if unsafe {
        GetSecurityDescriptorControl(descriptor.as_mut_ptr().cast(), &mut control, &mut revision)
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let inheritance = if control & SE_DACL_PROTECTED != 0 {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else {
        UNPROTECTED_DACL_SECURITY_INFORMATION
    };
    if unsafe {
        SetFileSecurityW(
            target.as_ptr(),
            DACL_SECURITY_INFORMATION | inheritance,
            descriptor.as_mut_ptr().cast(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn copy(_source: &Path, _target: &Path) -> Result<()> {
    Ok(())
}

/// Native metadata participates in Undo guards, so external tag/ACL changes
/// cannot be silently overwritten when restoring a journal snapshot.
pub fn digest(path: &Path) -> Result<Vec<u8>> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let path_c = CString::new(path.as_os_str().as_bytes())?;
        #[cfg(target_os = "macos")]
        let n = unsafe {
            libc::listxattr(
                path_c.as_ptr(),
                std::ptr::null_mut(),
                0,
                libc::XATTR_NOFOLLOW,
            )
        };
        #[cfg(target_os = "linux")]
        let n = unsafe { libc::listxattr(path_c.as_ptr(), std::ptr::null_mut(), 0) };
        if n < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ENOTSUP) {
                return Err(error.into());
            }
        } else {
            let mut names = vec![0u8; n as usize];
            #[cfg(target_os = "macos")]
            let n = unsafe {
                libc::listxattr(
                    path_c.as_ptr(),
                    names.as_mut_ptr().cast(),
                    names.len(),
                    libc::XATTR_NOFOLLOW,
                )
            };
            #[cfg(target_os = "linux")]
            let n =
                unsafe { libc::listxattr(path_c.as_ptr(), names.as_mut_ptr().cast(), names.len()) };
            if n < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let mut names = names[..n as usize]
                .split(|b| *b == 0)
                .filter(|n| !n.is_empty())
                .collect::<Vec<_>>();
            names.sort();
            for name in names {
                #[cfg(target_os = "linux")]
                if !name.starts_with(b"user.") && !name.starts_with(b"system.posix_acl_") {
                    continue;
                }
                let name_c = CString::new(name)?;
                #[cfg(target_os = "macos")]
                let n = unsafe {
                    libc::getxattr(
                        path_c.as_ptr(),
                        name_c.as_ptr(),
                        std::ptr::null_mut(),
                        0,
                        0,
                        libc::XATTR_NOFOLLOW,
                    )
                };
                #[cfg(target_os = "linux")]
                let n = unsafe {
                    libc::getxattr(path_c.as_ptr(), name_c.as_ptr(), std::ptr::null_mut(), 0)
                };
                if n < 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                hash.update((name.len() as u64).to_le_bytes());
                hash.update(name);
                hash.update((n as u64).to_le_bytes());
                #[cfg(target_os = "macos")]
                if name == b"com.apple.ResourceFork" {
                    use std::io::Read;
                    let mut file = std::fs::File::open(path.join("..namedfork/rsrc"))?;
                    let mut buffer = [0; 65536];
                    loop {
                        let n = file.read(&mut buffer)?;
                        if n == 0 {
                            break;
                        }
                        hash.update(&buffer[..n]);
                    }
                    continue;
                }
                let mut value = vec![0u8; n as usize];
                #[cfg(target_os = "macos")]
                let actual = unsafe {
                    libc::getxattr(
                        path_c.as_ptr(),
                        name_c.as_ptr(),
                        value.as_mut_ptr().cast(),
                        value.len(),
                        0,
                        libc::XATTR_NOFOLLOW,
                    )
                };
                #[cfg(target_os = "linux")]
                let actual = unsafe {
                    libc::getxattr(
                        path_c.as_ptr(),
                        name_c.as_ptr(),
                        value.as_mut_ptr().cast(),
                        value.len(),
                    )
                };
                if actual < 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                hash.update(&value[..actual as usize]);
            }
        }
        #[cfg(target_os = "macos")]
        {
            unsafe extern "C" {
                fn acl_get_file(path: *const libc::c_char, kind: libc::c_int) -> *mut libc::c_void;
                fn acl_to_text(
                    acl: *mut libc::c_void,
                    length: *mut libc::ssize_t,
                ) -> *mut libc::c_char;
                fn acl_free(ptr: *mut libc::c_void) -> libc::c_int;
            }
            let acl = unsafe { acl_get_file(path_c.as_ptr(), 0x100) };
            if !acl.is_null() {
                let mut length = 0;
                let text = unsafe { acl_to_text(acl, &mut length) };
                if !text.is_null() {
                    hash.update(unsafe {
                        std::slice::from_raw_parts(text.cast::<u8>(), length as usize)
                    });
                    unsafe {
                        acl_free(text.cast());
                    }
                }
                unsafe {
                    acl_free(acl);
                }
            }
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, GetFileSecurityW};
        let path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let mut needed = 0;
        unsafe {
            GetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                0,
                &mut needed,
            );
        }
        if needed == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut descriptor = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe {
            GetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION,
                descriptor.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        hash.update(unsafe {
            std::slice::from_raw_parts(descriptor.as_ptr().cast::<u8>(), needed as usize)
        });
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    let _ = path;
    Ok(hash.finalize().to_vec())
}
