//! Native drop receiver: private selection tokens plus ordinary Shell file lists.
use super::*;
use std::{os::windows::ffi::OsStringExt, sync::Mutex};
use windows::Win32::{
    Graphics::Gdi::ScreenToClient,
    System::{
        DataExchange::RegisterClipboardFormatW,
        Ole::{IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop},
    },
    UI::Shell::{DragQueryFileW, HDROP},
};

pub struct DropTarget {
    window: HWND,
    _target: IDropTarget,
}
impl Drop for DropTarget {
    fn drop(&mut self) {
        unsafe {
            let _ = RevokeDragDrop(self.window);
        }
    }
}
#[derive(Clone)]
struct Offer {
    token: Option<String>,
    paths: Vec<PathBuf>,
}
#[implement(IDropTarget)]
struct Target {
    window: HWND,
    callback: crate::DropCallback,
    offer: Mutex<Option<Offer>>,
    token_format: u16,
}
impl Target {
    fn offer(&self, object: &IDataObject) -> Option<Offer> {
        unsafe {
            let format = FORMATETC {
                cfFormat: self.token_format,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            };
            if let Ok(mut medium) = object.GetData(&format) {
                let handle = medium.u.hGlobal;
                let length = GlobalSize(handle);
                let ptr = GlobalLock(handle);
                let token = if !ptr.is_null() && length <= 128 {
                    let bytes = std::slice::from_raw_parts(ptr.cast::<u8>(), length);
                    std::str::from_utf8(bytes)
                        .ok()
                        .map(|s| s.trim_end_matches('\0').to_owned())
                        .filter(|s| s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit()))
                } else {
                    None
                };
                if !ptr.is_null() {
                    let _ = GlobalUnlock(handle);
                }
                ReleaseStgMedium(&mut medium);
                if let Some(token) = token {
                    return Some(Offer {
                        token: Some(token),
                        paths: Vec::new(),
                    });
                }
            }
            let format = FORMATETC {
                cfFormat: CF_HDROP.0,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            };
            let mut medium = object.GetData(&format).ok()?;
            let drop = HDROP(medium.u.hGlobal.0 as isize);
            let count = DragQueryFileW(drop, u32::MAX, None);
            let mut paths = Vec::new();
            if count <= 100_000 {
                for index in 0..count {
                    let length = DragQueryFileW(drop, index, None);
                    if length == 0 || length > 32768 {
                        paths.clear();
                        break;
                    }
                    let mut name = vec![0u16; length as usize + 1];
                    let length = DragQueryFileW(drop, index, Some(&mut name));
                    paths.push(PathBuf::from(std::ffi::OsString::from_wide(
                        &name[..length as usize],
                    )));
                }
            }
            ReleaseStgMedium(&mut medium);
            (!paths.is_empty()).then_some(Offer { token: None, paths })
        }
    }
    fn emit(&self, point: POINTL, dropped: bool) {
        if let Some(offer) = self.offer.lock().unwrap().clone() {
            let mut point = POINT {
                x: point.x,
                y: point.y,
            };
            unsafe {
                ScreenToClient(self.window, &mut point);
            }
            // Physical coordinates; the owning Slint window applies its scale factor.
            let event = if dropped {
                crate::DropEvent::Drop {
                    x: point.x as f32,
                    y: point.y as f32,
                    token: offer.token,
                    paths: offer.paths,
                }
            } else {
                crate::DropEvent::Hover {
                    x: point.x as f32,
                    y: point.y as f32,
                    token: offer.token,
                    paths: offer.paths,
                }
            };
            (self.callback)(event);
        }
    }
}
#[allow(non_snake_case)]
impl IDropTarget_Impl for Target {
    fn DragEnter(
        &self,
        object: Option<&IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        *self.offer.lock().unwrap() = object.and_then(|o| self.offer(o));
        unsafe {
            *effect = if self.offer.lock().unwrap().is_some() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT(0)
            };
        }
        self.emit(*point, false);
        Ok(())
    }
    fn DragOver(
        &self,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        unsafe {
            *effect = if self.offer.lock().unwrap().is_some() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT(0)
            };
        }
        self.emit(*point, false);
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        self.offer.lock().unwrap().take();
        (self.callback)(crate::DropEvent::Leave);
        Ok(())
    }
    fn Drop(
        &self,
        _: Option<&IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        unsafe {
            *effect = if self.offer.lock().unwrap().is_some() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT(0)
            };
        }
        self.emit(*point, true);
        self.offer.lock().unwrap().take();
        Ok(())
    }
}
pub fn attach_drop_target(
    window: &impl HasWindowHandle,
    callback: crate::DropCallback,
) -> crate::Result<DropTarget> {
    init_ole();
    let RawWindowHandle::Win32(window) = window
        .window_handle()
        .map_err(|_| crate::Error::UnsupportedWindowHandle)?
        .as_raw()
    else {
        return Err(crate::Error::UnsupportedWindowHandle);
    };
    let window = HWND(window.hwnd.get());
    let target: IDropTarget = Target {
        window,
        callback,
        offer: Mutex::new(None),
        token_format: unsafe {
            RegisterClipboardFormatW(w!("application/x-rust-commander-selection")) as u16
        },
    }
    .into();
    unsafe {
        RevokeDragDrop(window)?;
        RegisterDragDrop(window, &target)?;
    }
    Ok(DropTarget {
        window,
        _target: target,
    })
}
