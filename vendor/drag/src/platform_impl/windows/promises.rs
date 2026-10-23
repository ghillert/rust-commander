//! Shell virtual files: descriptors on hover, streams only after the accepted drop.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use windows::Win32::{
    System::DataExchange::RegisterClipboardFormatW,
    UI::Shell::{
        IDataObjectAsyncCapability, IDataObjectAsyncCapability_Impl, SHCreateStdEnumFmtEtc,
        SHCreateStreamOnFileEx, FD_ATTRIBUTES, FD_FILESIZE, FILEDESCRIPTORW,
    },
};

#[implement(IDataObject, IDataObjectAsyncCapability)]
struct VirtualFiles {
    files: crate::PromisedFiles,
    manifest: OnceLock<std::result::Result<Vec<crate::PromiseEntry>, String>>,
    dropped: Arc<AtomicBool>,
    descriptors: u16,
    contents: u16,
    operation: Arc<AsyncOperation>,
}
impl VirtualFiles {
    fn entries(&self) -> Result<&Vec<crate::PromiseEntry>> {
        self.manifest
            .get_or_init(|| (self.files.list)())
            .as_ref()
            .map_err(|_| Error::from(E_FAIL))
    }
    fn formats(&self) -> Vec<FORMATETC> {
        let mut formats = vec![
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
        ];
        if self.files.internal_token.is_some() {
            formats.push(FORMATETC {
                cfFormat: internal_format(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0 as u32,
                ..Default::default()
            });
        }
        formats
    }
}
#[allow(non_snake_case)]
impl IDataObject_Impl for VirtualFiles {
    fn GetData(&self, format: *const FORMATETC) -> Result<STGMEDIUM> {
        if self.QueryGetData(format).is_err() {
            return Err(Error::from(DV_E_FORMATETC));
        }
        let format = unsafe { &*format };
        if format.cfFormat == internal_format() {
            let token = self
                .files
                .internal_token
                .as_ref()
                .ok_or_else(|| Error::from(DV_E_FORMATETC))?;
            return global_medium(token.as_bytes());
        }
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
        let path = responsive_read(self.files.read.clone(), entry.path.clone())?;
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
        if f.cfFormat == internal_format()
            && self.files.internal_token.is_some()
            && f.tymed & TYMED_HGLOBAL.0 as u32 != 0
        {
            return S_OK;
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
        let operation = Arc::new(AsyncOperation::new(dropped.clone(), Box::new(finished)));
        let object: IDataObject = VirtualFiles {
            files,
            manifest: OnceLock::new(),
            dropped: dropped.clone(),
            descriptors: RegisterClipboardFormatW(w!("FileGroupDescriptorW")) as u16,
            contents: RegisterClipboardFormatW(w!("FileContents")) as u16,
            operation: operation.clone(),
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
        // Tell compatible Shell receivers to extract on their background worker.
        object
            .cast::<IDataObjectAsyncCapability>()?
            .SetAsyncMode(true)?;
        let outcome = DoDragDrop(&object, &source, DROPEFFECT_COPY, &mut effect);
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        operation.drag_ended(
            outcome == DRAGDROP_S_DROP && effect & DROPEFFECT_COPY != DROPEFFECT(0),
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
        init_ole();
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
                internal_token: None,
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
            operation: Arc::new(AsyncOperation::new(dropped.clone(), Box::new(|_, _| {}))),
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

fn internal_format() -> u16 {
    unsafe { RegisterClipboardFormatW(w!("application/x-rust-commander-selection")) as u16 }
}
fn global_medium(bytes: &[u8]) -> Result<STGMEDIUM> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            let _ = GlobalFree(memory);
            return Err(Error::from(E_OUTOFMEMORY));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
        let _ = GlobalUnlock(memory);
        Ok(STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: memory },
            pUnkForRelease: std::mem::ManuallyDrop::new(None),
        })
    }
}

// Receiver completion can arrive before or after DoDragDrop returns. Finish once,
// retaining the accepted content gate for the entire asynchronous extraction.
type Finished = Box<dyn Fn(DragResult, CursorPosition) + Send>;
struct AsyncState {
    running: bool,
    drag_ended: bool,
    accepted: bool,
    failed: bool,
    point: CursorPosition,
    finished: Option<Finished>,
}
struct AsyncOperation {
    mode: AtomicBool,
    dropped: Arc<AtomicBool>,
    state: Mutex<AsyncState>,
}
impl AsyncOperation {
    fn new(dropped: Arc<AtomicBool>, finished: Finished) -> Self {
        Self {
            mode: AtomicBool::new(false),
            dropped,
            state: Mutex::new(AsyncState {
                running: false,
                drag_ended: false,
                accepted: false,
                failed: false,
                point: CursorPosition { x: 0, y: 0 },
                finished: Some(finished),
            }),
        }
    }
    fn drag_ended(&self, accepted: bool, point: CursorPosition) {
        {
            let mut state = self.state.lock().unwrap();
            state.drag_ended = true;
            state.accepted = accepted;
            state.point = point;
        }
        self.complete();
    }
    fn complete(&self) {
        let result = {
            let mut state = self.state.lock().unwrap();
            if !state.drag_ended || state.running {
                return;
            }
            let Some(finished) = state.finished.take() else {
                return;
            };
            let accepted = state.accepted && !state.failed;
            self.dropped.store(false, Ordering::Release);
            (
                finished,
                if accepted {
                    DragResult::Dropped
                } else {
                    DragResult::Cancel
                },
                state.point.clone(),
            )
        };
        (result.0)(result.1, result.2);
    }
}
#[allow(non_snake_case)]
impl IDataObjectAsyncCapability_Impl for VirtualFiles {
    fn SetAsyncMode(&self, enabled: BOOL) -> Result<()> {
        self.operation
            .mode
            .store(enabled.as_bool(), Ordering::Release);
        Ok(())
    }
    fn GetAsyncMode(&self) -> Result<BOOL> {
        Ok(self.operation.mode.load(Ordering::Acquire).into())
    }
    fn StartOperation(&self, _: Option<&IBindCtx>) -> Result<()> {
        if !self.dropped.load(Ordering::Acquire) || !self.operation.mode.load(Ordering::Acquire) {
            return Err(Error::from(E_UNEXPECTED));
        }
        let mut state = self.operation.state.lock().unwrap();
        if state.running || state.finished.is_none() {
            return Err(Error::from(E_UNEXPECTED));
        }
        state.running = true;
        Ok(())
    }
    fn InOperation(&self) -> Result<BOOL> {
        Ok(self.operation.state.lock().unwrap().running.into())
    }
    fn EndOperation(&self, result: HRESULT, _: Option<&IBindCtx>, _: u32) -> Result<()> {
        {
            let mut state = self.operation.state.lock().unwrap();
            if !state.running {
                return Err(Error::from(E_UNEXPECTED));
            }
            state.running = false;
            state.failed |= result.is_err();
        }
        self.operation.complete();
        Ok(())
    }
}

// Synchronous receivers remain supported. The export runs off-thread, while the
// source thread pumps window messages so repaint, progress, and Cancel stay available.
fn responsive_read(reader: crate::PromiseRead, path: PathBuf) -> Result<PathBuf> {
    use windows::Win32::System::Threading::{CreateEventW, SetEvent};
    use windows::Win32::UI::WindowsAndMessaging::*;
    struct Event(isize);
    impl Drop for Event {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(HANDLE(self.0));
            }
        }
    }
    let event = Arc::new(Event(unsafe { CreateEventW(None, true, false, None)?.0 }));
    let notify = event.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("rust-commander-ole-export".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader(&path)))
                .unwrap_or_else(|_| Err("Virtual file export failed".into()));
            let _ = sender.send(result);
            unsafe {
                let _ = SetEvent(HANDLE(notify.0));
            }
        })
        .map_err(|_| Error::from(E_FAIL))?;
    unsafe {
        loop {
            let ready = MsgWaitForMultipleObjectsEx(
                Some(&[HANDLE(event.0)]),
                u32::MAX,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
            if ready == WAIT_OBJECT_0 {
                break;
            }
            if ready == WAIT_FAILED {
                return Err(Error::from_win32());
            }
            if ready.0 != WAIT_OBJECT_0.0 + 1 {
                return Err(Error::from(E_FAIL));
            }
            // COM's modal wait can leave ordinary posted messages undispatched.
            // Process one at a time so a busy queue cannot starve export completion.
            let mut message = MSG::default();
            if PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if message.message == WM_QUIT {
                    PostQuitMessage(message.wParam.0 as i32);
                    return Err(Error::from(E_ABORT));
                }
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    receiver
        .recv()
        .map_err(|_| Error::from(E_FAIL))?
        .map_err(|_| Error::from(E_FAIL))
}

#[cfg(test)]
mod async_tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    #[test]
    fn async_extraction_outlives_drag_loop_and_completes_exactly_once() {
        for end_first in [false, true] {
            let gate = Arc::new(AtomicBool::new(true));
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = calls.clone();
            let op = AsyncOperation::new(
                gate.clone(),
                Box::new(move |result, _| {
                    assert!(matches!(result, DragResult::Dropped));
                    counter.fetch_add(1, Ordering::Relaxed);
                }),
            );
            op.state.lock().unwrap().running = true;
            if end_first {
                op.state.lock().unwrap().running = false;
                op.complete();
                assert_eq!(calls.load(Ordering::Relaxed), 0);
            }
            op.drag_ended(true, CursorPosition { x: 1, y: 2 });
            if !end_first {
                assert!(gate.load(Ordering::Acquire));
                assert_eq!(calls.load(Ordering::Relaxed), 0);
                op.state.lock().unwrap().running = false;
                op.complete();
            }
            assert!(!gate.load(Ordering::Acquire));
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            op.complete();
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        }
    }
    #[test]
    fn async_failure_is_reported_as_cancellation() {
        let gate = Arc::new(AtomicBool::new(true));
        let op = AsyncOperation::new(
            gate.clone(),
            Box::new(|result, _| assert!(matches!(result, DragResult::Cancel))),
        );
        op.state.lock().unwrap().failed = true;
        op.drag_ended(true, CursorPosition { x: 0, y: 0 });
        assert!(!gate.load(Ordering::Acquire));
    }
    #[test]
    fn synchronous_export_runs_off_thread_and_dispatches_window_messages() {
        use std::sync::Condvar;
        use windows::Win32::UI::WindowsAndMessaging::*;
        static HANDLED: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
        unsafe extern "system" fn procedure(
            window: HWND,
            message: u32,
            w: WPARAM,
            l: LPARAM,
        ) -> LRESULT {
            if message == WM_APP {
                *HANDLED.0.lock().unwrap() = true;
                HANDLED.1.notify_one();
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(window, message, w, l) }
        }
        unsafe {
            OleInitialize(None).unwrap();
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                lpszClassName: w!("RustCommanderResponsiveExportTest"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let window = CreateWindowExW(
                Default::default(),
                class.lpszClassName,
                w!(""),
                Default::default(),
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                None,
                None,
                None,
            );
            assert_ne!(window.0, 0);
            let receiver = std::thread::current().id();
            let hwnd = window.0;
            let result = responsive_read(
                Arc::new(move |_| {
                    assert_ne!(receiver, std::thread::current().id());
                    PostMessageW(HWND(hwnd), WM_APP, WPARAM(0), LPARAM(0)).unwrap();
                    let (handled, _) = HANDLED
                        .1
                        .wait_timeout_while(
                            HANDLED.0.lock().unwrap(),
                            std::time::Duration::from_secs(5),
                            |handled| !*handled,
                        )
                        .unwrap();
                    assert!(
                        *handled,
                        "export wait must keep the source window responsive"
                    );
                    Ok("fixture.txt".into())
                }),
                "fixture.txt".into(),
            );
            DestroyWindow(window).unwrap();
            windows::Win32::System::Ole::OleUninitialize();
            assert_eq!(result.unwrap(), PathBuf::from("fixture.txt"));
        }
    }
    #[test]
    fn synchronous_export_preserves_quit_message() {
        use windows::Win32::UI::WindowsAndMessaging::*;
        let (release, blocked) = std::sync::mpsc::channel();
        let blocked = Mutex::new(blocked);
        unsafe {
            OleInitialize(None).unwrap();
            PostQuitMessage(42);
            let result = responsive_read(
                Arc::new(move |_| {
                    let _ = blocked
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(5));
                    Ok("fixture.txt".into())
                }),
                "fixture.txt".into(),
            );
            let _ = release.send(());
            let mut message = MSG::default();
            let queued = PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool();
            windows::Win32::System::Ole::OleUninitialize();
            assert_eq!(result.unwrap_err().code(), E_ABORT);
            assert!(queued);
            assert_eq!(message.message, WM_QUIT);
            assert_eq!(message.wParam.0, 42);
        }
    }
}
