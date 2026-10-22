//! Shell virtual files: descriptors on hover, streams only after the accepted drop.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};
use windows::Win32::{
    System::DataExchange::RegisterClipboardFormatW,
    UI::Shell::{
        SHCreateStdEnumFmtEtc, SHCreateStreamOnFileEx, FD_ATTRIBUTES, FD_FILESIZE, FILEDESCRIPTORW,
    },
};

#[implement(IDataObject)]
struct VirtualFiles {
    files: crate::PromisedFiles,
    manifest: OnceLock<std::result::Result<Vec<crate::PromiseEntry>, String>>,
    dropped: Arc<AtomicBool>,
    descriptors: u16,
    contents: u16,
}
impl VirtualFiles {
    fn entries(&self) -> Result<&Vec<crate::PromiseEntry>> {
        self.manifest
            .get_or_init(|| (self.files.list)())
            .as_ref()
            .map_err(|_| Error::from(E_FAIL))
    }
    fn formats(&self) -> [FORMATETC; 2] {
        [
            FORMATETC {
                cfFormat: self.descriptors,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            },
            FORMATETC {
                cfFormat: self.contents,
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_ISTREAM.0 as u32,
                ..Default::default()
            },
        ]
    }
}
#[allow(non_snake_case)]
impl IDataObject_Impl for VirtualFiles {
    fn GetData(&self, format: *const FORMATETC) -> Result<STGMEDIUM> {
        if self.QueryGetData(format).is_err() {
            return Err(Error::from(DV_E_FORMATETC));
        }
        let format = unsafe { &*format };
        if format.cfFormat == self.descriptors {
            let entries = self.entries()?;
            let size = 4 + entries.len() * std::mem::size_of::<FILEDESCRIPTORW>();
            let mut bytes = vec![0u8; size];
            bytes[..4].copy_from_slice(&(entries.len() as u32).to_le_bytes());
            for (index, entry) in entries.iter().enumerate() {
                let name = entry.path.as_os_str().encode_wide().collect::<Vec<_>>();
                if name.is_empty()
                    || name.len() >= 260
                    || name.contains(&0)
                    || entry.path.is_absolute()
                    || entry
                        .path
                        .components()
                        .any(|v| !matches!(v, std::path::Component::Normal(_)))
                {
                    return Err(Error::from(E_INVALIDARG));
                }
                let mut descriptor = FILEDESCRIPTORW {
                    dwFlags: FD_ATTRIBUTES.0 as u32,
                    dwFileAttributes: if entry.directory { 0x10 } else { 0x80 },
                    ..Default::default()
                };
                let mut filename = [0u16; 260];
                filename[..name.len()].copy_from_slice(&name);
                descriptor.cFileName = filename;
                if let Some(size) = entry.size.filter(|_| !entry.directory) {
                    descriptor.dwFlags |= FD_FILESIZE.0 as u32;
                    descriptor.nFileSizeHigh = (size >> 32) as u32;
                    descriptor.nFileSizeLow = size as u32;
                }
                unsafe {
                    bytes
                        .as_mut_ptr()
                        .add(4 + index * std::mem::size_of::<FILEDESCRIPTORW>())
                        .cast::<FILEDESCRIPTORW>()
                        .write_unaligned(descriptor);
                }
            }
            let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len())? };
            let pointer = unsafe { GlobalLock(memory) };
            if pointer.is_null() {
                unsafe {
                    let _ = GlobalFree(memory);
                }
                return Err(Error::from(E_OUTOFMEMORY));
            }
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
                let _ = GlobalUnlock(memory);
            }
            return Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: memory },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            });
        }
        // A target may probe formats and descriptors during DragEnter/DragOver.
        // No content callback is reachable until QueryContinueDrag commits Drop.
        if !self.dropped.load(Ordering::Acquire) {
            return Err(Error::from(HRESULT(0x8000000Au32 as i32)));
        }
        let entry = usize::try_from(format.lindex)
            .ok()
            .and_then(|i| self.entries().ok()?.get(i))
            .ok_or_else(|| Error::from(DV_E_LINDEX))?;
        if entry.directory {
            return Err(Error::from(DV_E_LINDEX));
        }
        let path = (self.files.read)(&entry.path).map_err(|_| Error::from(E_FAIL))?;
        let name = path
            .as_os_str()
            .encode_wide()
            .chain(once(0))
            .collect::<Vec<_>>();
        let stream =
            unsafe { SHCreateStreamOnFileEx(PCWSTR(name.as_ptr()), 0x20, 0, false, None)? };
        Ok(STGMEDIUM {
            tymed: TYMED_ISTREAM.0 as u32,
            u: STGMEDIUM_0 {
                pstm: std::mem::ManuallyDrop::new(Some(stream)),
            },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        })
    }
    fn QueryGetData(&self, format: *const FORMATETC) -> HRESULT {
        let Some(f) = (unsafe { format.as_ref() }) else {
            return E_INVALIDARG;
        };
        if f.dwAspect != DVASPECT_CONTENT.0 {
            return DV_E_DVASPECT;
        }
        if (f.cfFormat == self.descriptors && f.tymed & TYMED_HGLOBAL.0 as u32 != 0)
            || (f.cfFormat == self.contents && f.tymed & TYMED_ISTREAM.0 as u32 != 0)
        {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }
    fn EnumFormatEtc(&self, direction: u32) -> Result<IEnumFORMATETC> {
        if direction != DATADIR_GET.0 as u32 {
            return Err(Error::from(E_NOTIMPL));
        }
        unsafe { SHCreateStdEnumFmtEtc(&self.formats()) }
    }
    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if !out.is_null() {
            unsafe {
                (*out).ptd = std::ptr::null_mut();
            }
        }
        E_NOTIMPL
    }
    fn SetData(&self, _: *const FORMATETC, _: *const STGMEDIUM, _: BOOL) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
    fn DAdvise(&self, _: *const FORMATETC, _: u32, _: Option<&IAdviseSink>) -> Result<u32> {
        Err(Error::from(OLE_E_ADVISENOTSUPPORTED))
    }
    fn DUnadvise(&self, _: u32) -> Result<()> {
        Err(Error::from(OLE_E_ADVISENOTSUPPORTED))
    }
    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(Error::from(OLE_E_ADVISENOTSUPPORTED))
    }
}
#[implement(IDropSource)]
struct VirtualSource {
    dropped: Arc<AtomicBool>,
    accepted: AtomicBool,
}
#[allow(non_snake_case)]
impl IDropSource_Impl for VirtualSource {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            self.dropped.store(false, Ordering::Release);
            DRAGDROP_S_CANCEL
        } else if keys & MK_LBUTTON == MODIFIERKEYS_FLAGS(0) {
            let accepted = self.accepted.load(Ordering::Acquire);
            self.dropped.store(accepted, Ordering::Release);
            if accepted {
                DRAGDROP_S_DROP
            } else {
                DRAGDROP_S_CANCEL
            }
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, effect: DROPEFFECT) -> HRESULT {
        self.accepted
            .store(effect & DROPEFFECT_COPY != DROPEFFECT(0), Ordering::Release);
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}
pub(super) fn start(
    files: crate::PromisedFiles,
    image: Image,
    finished: impl Fn(DragResult, CursorPosition) + Send + 'static,
) -> crate::Result<()> {
    init_ole();
    unsafe {
        #[allow(static_mut_refs)]
        if let Err(e) = &OLE_RESULT {
            return Err(e.clone().into());
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let object: IDataObject = VirtualFiles {
            files,
            manifest: OnceLock::new(),
            dropped: dropped.clone(),
            descriptors: RegisterClipboardFormatW(w!("FileGroupDescriptorW")) as u16,
            contents: RegisterClipboardFormatW(w!("FileContents")) as u16,
        }
        .into();
        let source: IDropSource = VirtualSource {
            dropped: dropped.clone(),
            accepted: AtomicBool::new(false),
        }
        .into();
        if let Some(image) = get_drag_image(image) {
            if let Ok(helper) = create_instance::<IDragSourceHelper>(&CLSID_DragDropHelper) {
                let _ = helper.InitializeFromBitmap(&image, &object);
            }
        }
        let mut effect = DROPEFFECT::default();
        let outcome = DoDragDrop(&object, &source, DROPEFFECT_COPY, &mut effect);
        dropped.store(false, Ordering::Release);
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        finished(
            if outcome == DRAGDROP_S_DROP && effect & DROPEFFECT_COPY != DROPEFFECT(0) {
                DragResult::Dropped
            } else {
                DragResult::Cancel
            },
            CursorPosition {
                x: point.x,
                y: point.y,
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_descriptors_are_safe_on_hover_and_contents_require_an_accepted_drop() {
        use std::sync::atomic::AtomicUsize;
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("fixture.txt");
        std::fs::write(&file, b"deferred bytes").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let dropped = Arc::new(AtomicBool::new(false));
        let entry = crate::PromiseEntry {
            path: "fixture.txt".into(),
            directory: false,
            size: Some(14),
        };
        let listing = entry.clone();
        let object = VirtualFiles {
            files: crate::PromisedFiles {
                entries: vec![entry],
                list: Arc::new(move || Ok(vec![listing.clone()])),
                read: Arc::new(move |_| {
                    counter.fetch_add(1, Ordering::Relaxed);
                    Ok(file.clone())
                }),
                write: Arc::new(|_, _| unreachable!()),
            },
            manifest: OnceLock::new(),
            dropped: dropped.clone(),
            descriptors: 49100,
            contents: 49101,
        };
        let mut descriptor = object.formats()[0];
        assert!(object.QueryGetData(&descriptor).is_ok());
        let mut medium = object.GetData(&descriptor).unwrap();
        unsafe {
            windows::Win32::System::Ole::ReleaseStgMedium(&mut medium);
        }
        descriptor = object.formats()[1];
        descriptor.lindex = 0;
        assert!(object.QueryGetData(&descriptor).is_ok());
        assert!(object.GetData(&descriptor).is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        let source = VirtualSource {
            dropped: dropped.clone(),
            accepted: AtomicBool::new(false),
        };
        let _ = source.GiveFeedback(DROPEFFECT_COPY);
        assert_eq!(
            source.QueryContinueDrag(BOOL(0), MODIFIERKEYS_FLAGS(0)),
            DRAGDROP_S_DROP
        );
        let mut medium = object.GetData(&descriptor).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        unsafe {
            windows::Win32::System::Ole::ReleaseStgMedium(&mut medium);
        }
        let _ = source.QueryContinueDrag(BOOL(1), MK_LBUTTON);
        assert!(object.GetData(&descriptor).is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}
