//! A standard Win32 BUTTON child, owned by the originating winit window.
use super::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::*,
        Input::KeyboardAndMouse::*,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
const SUBCLASS: usize = 0x52435042;
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
struct State {
    parent: HWND,
    button: HWND,
    alive: Cell<bool>,
    clicked: Rc<Cell<bool>>,
}
unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    let state = unsafe { &*(data as *const State) };
    if hwnd == state.parent
        && msg == WM_COMMAND
        && l == state.button as isize
        && (w >> 16) == BN_CLICKED as usize
    {
        state.clicked.set(true);
        return 0;
    }
    if hwnd == state.button
        && msg == WM_KEYDOWN
        && (w == VK_TAB as usize || w == VK_ESCAPE as usize)
    {
        unsafe {
            SetFocus(state.parent);
        }
        return 0;
    }
    if hwnd == state.parent && msg == WM_NCDESTROY {
        state.alive.set(false);
    }
    unsafe { DefSubclassProc(hwnd, msg, w, l) }
}
pub(super) struct Platform {
    state: Box<State>,
    font: HFONT,
    scale: f32,
}
impl Platform {
    pub fn new(ui: &CommanderWindow, clicked: Rc<Cell<bool>>) -> Option<Self> {
        let handle = ui.window().window_handle();
        let handle = handle.window_handle().ok()?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return None;
        };
        let parent = handle.hwnd.get() as HWND;
        unsafe {
            let button = CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide(&tr("Open externally")).as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
                0,
                0,
                1,
                1,
                parent,
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            );
            if button.is_null() {
                return None;
            }
            let state = Box::new(State {
                parent,
                button,
                alive: Cell::new(true),
                clicked,
            });
            let data = (&*state as *const State) as usize;
            if SetWindowSubclass(parent, Some(subclass), SUBCLASS, data) == 0
                || SetWindowSubclass(button, Some(subclass), SUBCLASS, data) == 0
            {
                RemoveWindowSubclass(parent, Some(subclass), SUBCLASS);
                DestroyWindow(button);
                return None;
            }
            Some(Self {
                state,
                font: null_mut(),
                scale: 0.0,
            })
        }
    }
    pub fn sync(&mut self, ui: &CommanderWindow, _dark: bool) {
        if !self.state.alive.get() {
            return;
        }
        let scale = ui.window().scale_factor();
        unsafe {
            if self.font.is_null() || self.scale != scale {
                let mut metrics = NONCLIENTMETRICSW {
                    cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32,
                    ..Default::default()
                };
                let font = if SystemParametersInfoForDpi(
                    SPI_GETNONCLIENTMETRICS,
                    metrics.cbSize,
                    &mut metrics as *mut _ as *mut _,
                    0,
                    (96.0 * scale).round() as u32,
                ) != 0
                {
                    CreateFontIndirectW(&metrics.lfMessageFont)
                } else {
                    let mut fallback = LOGFONTW::default();
                    GetObjectW(
                        GetStockObject(DEFAULT_GUI_FONT),
                        std::mem::size_of::<LOGFONTW>() as i32,
                        &mut fallback as *mut _ as *mut _,
                    );
                    CreateFontIndirectW(&fallback)
                };
                SendMessageW(self.state.button, WM_SETFONT, font as usize, 1);
                if !self.font.is_null() {
                    DeleteObject(self.font as _);
                }
                self.font = font;
                self.scale = scale;
            }
            SetWindowTextW(self.state.button, wide(&tr("Open externally")).as_ptr());
            EnableWindow(self.state.button, i32::from(ui.get_preview_can_open()));
            MoveWindow(
                self.state.button,
                (ui.get_preview_button_x() * scale).round() as i32,
                (ui.get_preview_button_y() * scale).round() as i32,
                (ui.get_preview_button_width() * scale).round() as i32,
                (32.0 * scale).round() as i32,
                1,
            );
        }
    }
    pub fn focus(&self) {
        if self.state.alive.get() {
            unsafe {
                SetFocus(self.state.button);
            }
        }
    }
}
impl Drop for Platform {
    fn drop(&mut self) {
        unsafe {
            if self.state.alive.get() {
                RemoveWindowSubclass(self.state.parent, Some(subclass), SUBCLASS);
                RemoveWindowSubclass(self.state.button, Some(subclass), SUBCLASS);
                if GetFocus() == self.state.button {
                    SetFocus(self.state.parent);
                }
                DestroyWindow(self.state.button);
            }
            if !self.font.is_null() {
                DeleteObject(self.font as _);
            }
        }
    }
}
