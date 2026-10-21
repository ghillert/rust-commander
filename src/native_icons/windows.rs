use super::{ICON_SIZE, IconKind, Key, NativeIcon};
use std::{
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    ptr,
};
use windows_sys::Win32::{
    Graphics::Gdi::{
        self, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
    },
    System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    },
    UI::{
        Shell::{
            SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_SYSICONINDEX, SHGFI_USEFILEATTRIBUTES,
            SHGetFileInfoW,
        },
        WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON},
    },
};

pub(super) struct Provider {
    generic: Option<i32>,
}
impl Provider {
    pub fn new() -> Option<Self> {
        // The dedicated worker has no previous apartment. Both S_OK and S_FALSE
        // require a matching CoUninitialize when the worker exits.
        (unsafe {
            CoInitializeEx(
                ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        } >= 0)
            .then(|| {
                let name: Vec<u16> = "file.rust-commander-unregistered-6c9e42ba7d3f"
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                let mut info: SHFILEINFOW = unsafe { zeroed() };
                // USEFILEATTRIBUTES requests a type icon without touching disk.
                let result = unsafe {
                    SHGetFileInfoW(
                        name.as_ptr(),
                        windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
                        &mut info,
                        size_of::<SHFILEINFOW>() as u32,
                        SHGFI_SYSICONINDEX | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
                    )
                };
                Self {
                    generic: (result != 0).then_some(info.iIcon),
                }
            })
    }

    pub fn load(&self, key: &Key) -> Option<NativeIcon> {
        let path: Vec<u16> = key.path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut info: SHFILEINFOW = unsafe { zeroed() };
        // The NUL-terminated path and output structure remain valid for the
        // synchronous call. SHGFI_ICON transfers ownership of hIcon to us.
        if unsafe {
            SHGetFileInfoW(
                path.as_ptr(),
                0,
                &mut info,
                size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            )
        } == 0
            || info.hIcon.is_null()
        {
            return None;
        }
        let icon = Icon(info.hIcon);
        let bitmap = Bitmap::new()?;
        // Render against black and white to recover transparency for both
        // modern alpha icons and older icons that use an AND mask.
        let black = bitmap.draw(icon.0, 0)?;
        let white = bitmap.draw(icon.0, 255)?;
        let mut rgba = image::RgbaImage::new(ICON_SIZE, ICON_SIZE);
        for ((out, black), white) in rgba
            .pixels_mut()
            .zip(black.as_chunks::<4>().0)
            .zip(white.as_chunks::<4>().0)
        {
            let alpha = 255
                - (0..3)
                    .map(|c| white[c].saturating_sub(black[c]))
                    .max()
                    .unwrap_or(255);
            let straight = |c: usize| {
                if alpha == 0 {
                    0
                } else {
                    (u32::from(black[c]) * 255 / u32::from(alpha)).min(255) as u8
                }
            };
            *out = image::Rgba([straight(2), straight(1), straight(0), alpha]);
        }
        let kind = match self.generic {
            Some(index) if !key.directory && index == info.iIcon => IconKind::Generic,
            Some(_) => IconKind::Specific,
            None => IconKind::Unknown,
        };
        Some(NativeIcon::new(rgba, kind))
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        unsafe { DestroyIcon(self.0) };
    }
}

struct Bitmap {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut u8,
}
impl Bitmap {
    fn new() -> Option<Self> {
        // The RAII object releases partially initialized resources on failures.
        let mut output = Self {
            dc: unsafe { Gdi::CreateCompatibleDC(ptr::null_mut()) },
            bitmap: ptr::null_mut(),
            previous: ptr::null_mut(),
            pixels: ptr::null_mut(),
        };
        if output.dc.is_null() {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: ICON_SIZE as i32,
                biHeight: -(ICON_SIZE as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..unsafe { zeroed() }
            },
            ..unsafe { zeroed() }
        };
        let mut pixels = ptr::null_mut();
        output.bitmap = unsafe {
            Gdi::CreateDIBSection(
                output.dc,
                &info,
                DIB_RGB_COLORS,
                &mut pixels,
                ptr::null_mut(),
                0,
            )
        };
        if output.bitmap.is_null() || pixels.is_null() {
            return None;
        }
        output.pixels = pixels.cast();
        output.previous = unsafe { Gdi::SelectObject(output.dc, output.bitmap) };
        if output.previous.is_null() {
            return None;
        }
        Some(output)
    }
    fn draw(&self, icon: HICON, background: u8) -> Option<Vec<u8>> {
        let length = (ICON_SIZE * ICON_SIZE * 4) as usize;
        // The top-down DIB owns length initialized bytes; GDI exclusively draws
        // into it here. Flush before reading its pixels on this worker thread.
        unsafe {
            ptr::write_bytes(self.pixels, background, length);
            if DrawIconEx(
                self.dc,
                0,
                0,
                icon,
                ICON_SIZE as i32,
                ICON_SIZE as i32,
                0,
                ptr::null_mut(),
                DI_NORMAL,
            ) == 0
            {
                return None;
            }
            Gdi::GdiFlush();
            Some(std::slice::from_raw_parts(self.pixels, length).to_vec())
        }
    }
}
impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            if !self.previous.is_null() {
                Gdi::SelectObject(self.dc, self.previous);
            }
            if !self.bitmap.is_null() {
                Gdi::DeleteObject(self.bitmap);
            }
            if !self.dc.is_null() {
                Gdi::DeleteDC(self.dc);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_file_uses_generic_index_but_folders_remain_specific() {
        std::thread::spawn(|| {
            let root = tempfile::tempdir().unwrap();
            let unknown = root
                .path()
                .join("file.rust-commander-unregistered-6c9e42ba7d3f");
            std::fs::write(&unknown, "fixture").unwrap();
            let provider = Provider::new().expect("COM initializes on a fresh thread");
            assert!(provider.generic.is_some());
            for (path, directory, expected) in [
                (unknown, false, IconKind::Generic),
                (root.path().to_path_buf(), true, IconKind::Specific),
            ] {
                let icon = provider
                    .load(&Key {
                        path,
                        directory,
                        modified: None,
                    })
                    .unwrap();
                assert_eq!(icon.kind, expected);
            }
        })
        .join()
        .unwrap();
    }
}
