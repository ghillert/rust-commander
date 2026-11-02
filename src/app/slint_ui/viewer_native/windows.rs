//! Real Win32/common controls, parented to the winit HWND. No owner drawing or
//! replacement window procedure: subclasses only route this host's notifications.
use super::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::Cell,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
use windows_sys::Win32::{
    System::SystemServices::{MK_LBUTTON, SS_ENDELLIPSIS, SS_LEFT},
    UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
};
const SUBCLASS: usize = 0x52435650;
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
unsafe fn text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd).max(0).min(65536) as usize;
        let mut buffer = vec![0; len + 1];
        let count = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        String::from_utf16_lossy(&buffer[..count.max(0) as usize])
    }
}
struct Item {
    hwnd: HWND,
    spec: Control,
    tool: bool,
}
struct State {
    parent: Cell<HWND>,
    popup: Cell<HWND>,
    events: Events,
    document: u64,
    items: RefCell<Vec<Item>>,
    updating: Cell<bool>,
    shortcuts: RefCell<Vec<crate::preferences::Shortcut>>,
    scale: Cell<f32>,
    font: Cell<HFONT>,
    ui: slint::Weak<CommanderWindow>,
}
impl State {
    fn send(&self, action: Action) {
        self.events.borrow_mut().push(Event {
            document: self.document,
            action,
        });
    }
    unsafe fn collect(&self) {
        unsafe {
            for item in self
                .items
                .borrow()
                .iter()
                .filter(|i| matches!(i.spec.kind, Kind::Field | Kind::Search))
            {
                self.send(Action::Change(item.spec.id.clone(), text(item.hwnd)));
            }
        }
    }
    unsafe fn notification(&self, hwnd: HWND, code: u32) {
        unsafe {
            if self.updating.get() {
                return;
            }
            let items = self.items.borrow();
            let Some(item) = items.iter().find(|i| i.hwnd == hwnd) else {
                return;
            };
            match item.spec.kind {
                Kind::Field | Kind::Search if code == EN_CHANGE => {
                    self.send(Action::Change(item.spec.id.clone(), text(hwnd)))
                }
                Kind::Choice if code == CBN_SELCHANGE => {
                    let index = SendMessageW(hwnd, CB_GETCURSEL, 0, 0) as usize;
                    if let Some(value) = item.spec.choices.get(index) {
                        self.send(Action::Change(item.spec.id.clone(), value.clone()));
                    }
                }
                Kind::Modes if code == TCN_SELCHANGE => {
                    let index = SendMessageW(hwnd, TCM_GETCURSEL, 0, 0) as usize;
                    if let Some(value) = item.spec.choices.get(index) {
                        self.send(Action::Change(item.spec.id.clone(), value.clone()));
                    }
                }
                Kind::Button if code == BN_CLICKED => {
                    let id = item.spec.id.clone();
                    drop(items);
                    self.collect();
                    self.send(Action::Activate(id));
                }
                _ => {}
            }
        }
    }
    unsafe fn key(&self, hwnd: HWND, key: usize) -> bool {
        unsafe {
            let mut event = slint::language::KeyEvent::default();
            event.modifiers.control = GetKeyState(VK_CONTROL as i32) < 0;
            event.modifiers.shift = GetKeyState(VK_SHIFT as i32) < 0;
            event.modifiers.alt = GetKeyState(VK_MENU as i32) < 0;
            event.text = match key as u16 {
                VK_ESCAPE => slint::platform::Key::Escape.into(),
                VK_F1..=VK_F24 => char::from_u32(0xF704 + key as u32 - VK_F1 as u32)
                    .unwrap()
                    .to_string()
                    .into(),
                _ => char::from_u32(MapVirtualKeyW(key as u32, MAPVK_VK_TO_CHAR) & 0x7fff_ffff)
                    .unwrap_or_default()
                    .to_lowercase()
                    .to_string()
                    .into(),
            };
            if intercept(&event, &self.shortcuts.borrow()) {
                self.send(Action::Key(event));
                return true;
            }
            if key == VK_TAB as usize {
                if hwnd == self.parent.get()
                    && self.ui.upgrade().is_some_and(|ui| {
                        !ui.get_viewer_editable() && ui.get_viewer_document_focused()
                    })
                    && !event.modifiers.control
                    && !event.modifiers.alt
                {
                    let items = self.items.borrow();
                    let mut controls = items
                        .iter()
                        .filter(|i| !i.tool && i.spec.enabled && i.spec.kind != Kind::Label)
                        .collect::<Vec<_>>();
                    if event.modifiers.shift {
                        controls.reverse();
                    }
                    if let Some(item) = controls.first() {
                        SetFocus(item.hwnd);
                        return true;
                    }
                }
                let items = self.items.borrow();
                let tool = !self.popup.get().is_null();
                let controls = items
                    .iter()
                    .filter(|i| i.tool == tool && i.spec.kind != Kind::Label && i.spec.enabled)
                    .collect::<Vec<_>>();
                if let Some(index) = controls.iter().position(|i| i.hwnd == hwnd) {
                    let next = if event.modifiers.shift {
                        index.checked_sub(1)
                    } else {
                        Some(index + 1)
                    };
                    if let Some(item) = next.and_then(|i| controls.get(i)) {
                        SetFocus(item.hwnd);
                    } else if tool {
                        if let Some(item) = if event.modifiers.shift {
                            controls.last()
                        } else {
                            controls.first()
                        } {
                            SetFocus(item.hwnd);
                        }
                    } else {
                        SetFocus(self.parent.get());
                        self.send(Action::Activate("focus-document".into()));
                    }
                    return true;
                }
            }
            if key == VK_RETURN as usize {
                let items = self.items.borrow();
                if let Some(item) = items.iter().find(|i| i.hwnd == hwnd) {
                    let id = match item.spec.id.as_str() {
                        "query" => "find-next",
                        "hex" => "hex-find",
                        "offset" => "jump",
                        "page" => "page-go",
                        id if item.spec.kind == Kind::Button => id,
                        _ => return false,
                    }
                    .to_string();
                    drop(items);
                    self.collect();
                    self.send(Action::Activate(id));
                    return true;
                }
            }
            false
        }
    }
}
unsafe extern "system" fn subclass(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let state = &*(data as *const State);
        match message {
            WM_COMMAND => state.notification(l as HWND, ((w >> 16) & 0xffff) as u32),
            WM_NOTIFY if l != 0 => {
                let notification = &*(l as *const NMHDR);
                state.notification(notification.hwndFrom, notification.code);
            }
            WM_KEYDOWN | WM_SYSKEYDOWN if hwnd != state.parent.get() => {
                if state.key(hwnd, w) {
                    return 0;
                }
            }
            WM_KEYDOWN if hwnd == state.parent.get() && w == VK_TAB as usize => {
                if state.key(hwnd, w) {
                    return 0;
                }
            }
            WM_LBUTTONDOWN if hwnd == state.parent.get() => {
                if !state.popup.get().is_null() {
                    state.send(Action::Activate("tools-closed".into()));
                }
                SetFocus(hwnd);
            }
            WM_NCDESTROY => {
                RemoveWindowSubclass(hwnd, Some(subclass), SUBCLASS);
                if hwnd == state.parent.get() {
                    state.parent.set(null_mut());
                }
            }
            _ => {}
        }
        DefSubclassProc(hwnd, message, w, l)
    }
}
unsafe extern "system" fn popup_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(l as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        if let Some(state) = state.as_ref() {
            match message {
                WM_COMMAND => {
                    state.notification(l as HWND, ((w >> 16) & 0xffff) as u32);
                    return 0;
                }
                WM_NOTIFY if l != 0 => {
                    let note = &*(l as *const NMHDR);
                    state.notification(note.hwndFrom, note.code);
                    return 0;
                }
                WM_ACTIVATE if (w & 0xffff) == WA_INACTIVE as usize => {
                    state.send(Action::Activate("tools-closed".into()))
                }
                WM_CLOSE => {
                    state.send(Action::Activate("tools-closed".into()));
                    return 0;
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, w, l)
    }
}
pub(super) struct Platform {
    state: Box<State>,
    find_focus: i32,
    document_focus: i32,
    native_focused: bool,
}
impl Platform {
    pub fn new(ui: &CommanderWindow, document: u64, events: Events) -> Option<Self> {
        unsafe {
            let handle = ui.window().window_handle();
            let handle = handle.window_handle().ok()?;
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                return None;
            };
            let parent = handle.hwnd.get() as HWND;
            let state = Box::new(State {
                parent: Cell::new(parent),
                popup: Cell::new(null_mut()),
                events,
                document,
                items: Default::default(),
                updating: Cell::new(false),
                shortcuts: Default::default(),
                scale: Cell::new(1.0),
                font: Cell::new(null_mut()),
                ui: ui.as_weak(),
            });
            if SetWindowSubclass(
                parent,
                Some(subclass),
                SUBCLASS,
                &*state as *const State as usize,
            ) == 0
            {
                return None;
            }
            let controls = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_TAB_CLASSES | ICC_STANDARD_CLASSES,
            };
            InitCommonControlsEx(&controls);
            Some(Self {
                state,
                find_focus: ui.get_viewer_native_find_focus(),
                document_focus: ui.get_viewer_native_document_focus(),
                native_focused: false,
            })
        }
    }
    fn px(&self, n: f32) -> i32 {
        (n * self.state.scale.get()).round() as i32
    }
    unsafe fn reconcile(&self, specs: &[Control], tool: bool) {
        unsafe {
            let state = &self.state;
            state.updating.set(true);
            let mut items = state.items.borrow_mut();
            items.retain(|i| {
                let keep = i.tool != tool
                    || specs
                        .iter()
                        .any(|c| c.id == i.spec.id && c.kind == i.spec.kind);
                if !keep && IsWindow(i.hwnd) != 0 {
                    DestroyWindow(i.hwnd);
                }
                keep
            });
            let parent = if tool {
                state.popup.get()
            } else {
                state.parent.get()
            };
            for spec in specs {
                if !items.iter().any(|i| i.spec.id == spec.id) {
                    let (class, style) = match spec.kind {
                        Kind::Button => ("BUTTON", BS_PUSHBUTTON as u32 | WS_TABSTOP),
                        Kind::Modes => (
                            "SysTabControl32",
                            TCS_BUTTONS | TCS_FIXEDWIDTH | TCS_SINGLELINE | WS_TABSTOP,
                        ),
                        Kind::Choice => (
                            "COMBOBOX",
                            CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
                        ),
                        Kind::Field | Kind::Search => {
                            ("EDIT", WS_BORDER | ES_AUTOHSCROLL as u32 | WS_TABSTOP)
                        }
                        Kind::Label => ("STATIC", SS_LEFT as u32 | SS_ENDELLIPSIS as u32),
                    };
                    let hwnd = CreateWindowExW(
                        0,
                        wide(class).as_ptr(),
                        wide(&spec.label).as_ptr(),
                        WS_CHILD | WS_VISIBLE | style,
                        0,
                        0,
                        1,
                        1,
                        parent,
                        null_mut(),
                        GetModuleHandleW(null()),
                        null(),
                    );
                    if hwnd.is_null() {
                        continue;
                    }
                    SetWindowSubclass(
                        hwnd,
                        Some(subclass),
                        SUBCLASS,
                        &**state as *const State as usize,
                    );
                    SendMessageW(hwnd, WM_SETFONT, state.font.get() as usize, 1);
                    items.push(Item {
                        hwnd,
                        spec: Control::new(&spec.id, spec.kind.clone(), "", [0.0; 4]),
                        tool,
                    });
                }
                let Some(item) = items.iter_mut().find(|i| i.spec.id == spec.id) else {
                    continue;
                };
                let [x, y, w, h] = spec.frame;
                MoveWindow(
                    item.hwnd,
                    self.px(x),
                    self.px(y),
                    self.px(w),
                    self.px(if spec.kind == Kind::Choice {
                        h + 180.0
                    } else {
                        h
                    }),
                    1,
                );
                EnableWindow(item.hwnd, i32::from(spec.enabled));
                SendMessageW(item.hwnd, WM_SETFONT, state.font.get() as usize, 1);
                match spec.kind {
                    Kind::Button | Kind::Label => {
                        if text(item.hwnd) != spec.label {
                            SetWindowTextW(item.hwnd, wide(&spec.label).as_ptr());
                        }
                    }
                    Kind::Field | Kind::Search => {
                        if text(item.hwnd) != spec.value {
                            SetWindowTextW(item.hwnd, wide(&spec.value).as_ptr());
                        }
                        SendMessageW(
                            item.hwnd,
                            EM_SETCUEBANNER,
                            0,
                            wide(&spec.label).as_ptr() as isize,
                        );
                    }
                    Kind::Choice => {
                        if item.spec.choices != spec.choices {
                            SendMessageW(item.hwnd, CB_RESETCONTENT, 0, 0);
                            for value in &spec.choices {
                                SendMessageW(
                                    item.hwnd,
                                    CB_ADDSTRING,
                                    0,
                                    wide(value).as_ptr() as isize,
                                );
                            }
                        }
                        if let Some(index) = spec.choices.iter().position(|s| s == &spec.value) {
                            SendMessageW(item.hwnd, CB_SETCURSEL, index, 0);
                        }
                    }
                    Kind::Modes => {
                        if item.spec.choices != spec.choices {
                            SendMessageW(item.hwnd, TCM_DELETEALLITEMS, 0, 0);
                            for (index, value) in spec.choices.iter().enumerate() {
                                let label = match value.as_str() {
                                    "Plain original" => "Original",
                                    "Highlighted original" => "Highlighted",
                                    _ => "Pretty",
                                };
                                let mut label = wide(label);
                                let entry = TCITEMW {
                                    mask: TCIF_TEXT,
                                    pszText: label.as_mut_ptr(),
                                    ..Default::default()
                                };
                                SendMessageW(
                                    item.hwnd,
                                    TCM_INSERTITEMW,
                                    index,
                                    &entry as *const _ as isize,
                                );
                            }
                        }
                        let width = self.px(w / spec.choices.len().max(1) as f32);
                        let height = self.px(h - 4.0);
                        SendMessageW(
                            item.hwnd,
                            TCM_SETITEMSIZE,
                            0,
                            ((width & 0xffff) | ((height & 0xffff) << 16)) as isize,
                        );
                        if let Some(index) = spec.choices.iter().position(|s| s == &spec.value) {
                            SendMessageW(item.hwnd, TCM_SETCURSEL, index, 0);
                        }
                    }
                }
                item.spec = spec.clone();
            }
            state.updating.set(false);
        }
    }
    unsafe fn close_popup(&self) {
        unsafe {
            let state = &self.state;
            state.updating.set(true);
            state.items.borrow_mut().retain(|i| {
                if i.tool && IsWindow(i.hwnd) != 0 {
                    DestroyWindow(i.hwnd);
                }
                !i.tool
            });
            let popup = state.popup.replace(null_mut());
            if !popup.is_null() {
                DestroyWindow(popup);
            }
            state.updating.set(false);
        }
    }
    pub fn sync(
        &mut self,
        ui: &CommanderWindow,
        controls: &[Control],
        tools: &[Control],
        height: f32,
        _dark: bool,
        shortcuts: &[crate::preferences::Shortcut],
    ) {
        unsafe {
            *self.state.shortcuts.borrow_mut() = shortcuts.to_vec();
            let scale = ui.window().scale_factor();
            if self.state.font.get().is_null() || self.state.scale.get() != scale {
                let mut metrics = NONCLIENTMETRICSW::default();
                metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
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
                let old = self.state.font.replace(font);
                for item in self.state.items.borrow().iter() {
                    SendMessageW(item.hwnd, WM_SETFONT, font as usize, 1);
                }
                if !old.is_null() {
                    DeleteObject(old as HGDIOBJ);
                }
                self.state.scale.set(scale);
            }
            self.reconcile(controls, false);
            if ui.get_viewer_native_tools_open() {
                if self.state.popup.get().is_null() {
                    let name = wide("RustCommanderViewerTools");
                    let class = WNDCLASSW {
                        lpfnWndProc: Some(popup_proc),
                        hInstance: GetModuleHandleW(null()),
                        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                        hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
                        lpszClassName: name.as_ptr(),
                        ..Default::default()
                    };
                    RegisterClassW(&class);
                    let mut point = POINT {
                        x: self.px(ui.get_viewer_window_width() - 384.0),
                        y: self.px(82.0),
                    };
                    ClientToScreen(self.state.parent.get(), &mut point);
                    let hwnd = CreateWindowExW(
                        WS_EX_TOOLWINDOW,
                        name.as_ptr(),
                        wide("Viewer tools").as_ptr(),
                        WS_POPUP | WS_BORDER,
                        point.x,
                        point.y,
                        self.px(354.0),
                        self.px(height + 2.0),
                        self.state.parent.get(),
                        null_mut(),
                        GetModuleHandleW(null()),
                        &*self.state as *const State as *const _,
                    );
                    self.state.popup.set(hwnd);
                    if !hwnd.is_null() {
                        ShowWindow(hwnd, SW_SHOW);
                    }
                }
                if !self.state.popup.get().is_null() {
                    self.reconcile(tools, true);
                }
            } else if !self.state.popup.get().is_null() {
                self.close_popup();
            }
            if self.find_focus != ui.get_viewer_native_find_focus() {
                self.find_focus = ui.get_viewer_native_find_focus();
                if let Some(item) = self
                    .state
                    .items
                    .borrow()
                    .iter()
                    .find(|i| i.spec.id == "query")
                {
                    SetFocus(item.hwnd);
                    SendMessageW(item.hwnd, EM_SETSEL, 0, -1);
                }
            }
            if self.document_focus != ui.get_viewer_native_document_focus() {
                self.document_focus = ui.get_viewer_native_document_focus();
                SetFocus(self.state.parent.get());
            }
            let focus = GetFocus();
            let native_focused = !self.state.popup.get().is_null()
                || self
                    .state
                    .items
                    .borrow()
                    .iter()
                    .any(|i| i.hwnd == focus || IsChild(i.hwnd, focus) != 0);
            if native_focused {
                ui.invoke_focus_viewer_chrome();
            } else if self.native_focused {
                ui.invoke_restore_viewer_focus();
            }
            self.native_focused = native_focused;
        }
    }
    #[cfg(feature = "screenshot")]
    pub fn smoke(&self, id: &str, value: &str) {
        unsafe {
            if id == "assert-tools-layout" {
                let popup = self.state.popup.get();
                assert!(!popup.is_null());
                let mut bounds = RECT::default();
                GetClientRect(popup, &mut bounds);
                for item in self.state.items.borrow().iter().filter(|i| i.tool) {
                    let mut rect = RECT::default();
                    GetWindowRect(item.hwnd, &mut rect);
                    let mut top = POINT {
                        x: rect.left,
                        y: rect.top,
                    };
                    let mut bottom = POINT {
                        x: rect.right,
                        y: rect.bottom,
                    };
                    ScreenToClient(popup, &mut top);
                    ScreenToClient(popup, &mut bottom);
                    let inset = self.px(12.0);
                    assert!(
                        top.x >= inset
                            && top.y >= inset
                            && bottom.x <= bounds.right - inset
                            && bottom.y <= bounds.bottom - inset,
                        "Native popover clips {}",
                        item.spec.id
                    );
                }
                return;
            }
            if id == "assert-document-focus" {
                assert_eq!(GetFocus(), self.state.parent.get());
                return;
            }
            if id == "assert-query-focus" {
                assert!(
                    self.state
                        .items
                        .borrow()
                        .iter()
                        .any(|i| i.spec.id == "query" && i.hwnd == GetFocus())
                );
                return;
            }
            if id == "document-click" {
                let y = self
                    .state
                    .items
                    .borrow()
                    .iter()
                    .find(|i| i.spec.id == "query")
                    .map_or(110.0, |i| i.spec.frame[1] + i.spec.frame[3] + 30.0);
                let point = ((self.px(80.0) & 0xffff) | ((self.px(y) & 0xffff) << 16)) as isize;
                for message in [WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP] {
                    PostMessageW(
                        self.state.parent.get(),
                        message,
                        if message == WM_LBUTTONDOWN {
                            MK_LBUTTON as usize
                        } else {
                            0
                        },
                        point,
                    );
                }
                return;
            }
            if id == "key" {
                let items = self.state.items.borrow();
                let tool = !self.state.popup.get().is_null();
                let fallback = items
                    .iter()
                    .find(|i| i.tool == tool && i.spec.kind != Kind::Label)
                    .expect("Native keyboard target must exist")
                    .hwnd;
                let focus = GetFocus();
                let target =
                    if focus == self.state.parent.get() || items.iter().any(|i| i.hwnd == focus) {
                        focus
                    } else {
                        fallback
                    };
                drop(items);
                let mut keys = [0u8; 256];
                GetKeyboardState(keys.as_mut_ptr());
                let previous = keys;
                if value == "Command+F" {
                    keys[VK_CONTROL as usize] = 0x80;
                    keys[VK_LCONTROL as usize] = 0x80;
                    SetKeyboardState(keys.as_ptr());
                }
                SendMessageW(
                    target,
                    WM_KEYDOWN,
                    match value {
                        "Escape" => VK_ESCAPE as usize,
                        "Return" => VK_RETURN as usize,
                        "X" => b'X' as usize,
                        _ => b'F' as usize,
                    },
                    0,
                );
                SetKeyboardState(previous.as_ptr());
                if value == "X" {
                    PostMessageW(target, WM_CHAR, b'x' as usize, 0);
                }
                return;
            }
            let items = self.state.items.borrow();
            let item = items
                .iter()
                .find(|i| i.spec.id == id)
                .expect("Native viewer control must exist");
            let hwnd = item.hwnd;
            match item.spec.kind {
                Kind::Button => {
                    drop(items);
                    SendMessageW(hwnd, BM_CLICK, 0, 0);
                }
                Kind::Modes => {
                    let index = item.spec.choices.iter().position(|s| s == value).unwrap();
                    drop(items);
                    SendMessageW(hwnd, TCM_SETCURSEL, index, 0);
                    self.state.notification(hwnd, TCN_SELCHANGE);
                }
                Kind::Choice => {
                    let index = item.spec.choices.iter().position(|s| s == value).unwrap();
                    drop(items);
                    SendMessageW(hwnd, CB_SETCURSEL, index, 0);
                    self.state.notification(hwnd, CBN_SELCHANGE);
                }
                _ => {
                    drop(items);
                    SetWindowTextW(hwnd, wide(value).as_ptr());
                }
            }
        }
    }
    #[cfg(feature = "screenshot")]
    pub fn snapshot(&self, ui: &CommanderWindow, path: &Path) -> bool {
        unsafe fn capture(hwnd: HWND) -> Option<::image::RgbaImage> {
            unsafe {
                let mut bounds = RECT::default();
                GetClientRect(hwnd, &mut bounds);
                let (width, height) = (bounds.right, bounds.bottom);
                if width <= 0 || height <= 0 {
                    return None;
                }
                let screen = GetDC(hwnd);
                let dc = CreateCompatibleDC(screen);
                let mut info = BITMAPINFO::default();
                info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
                info.bmiHeader.biWidth = width;
                info.bmiHeader.biHeight = -height;
                info.bmiHeader.biPlanes = 1;
                info.bmiHeader.biBitCount = 32;
                info.bmiHeader.biCompression = BI_RGB;
                let mut data = null_mut();
                let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut data, null_mut(), 0);
                if bitmap.is_null() {
                    DeleteDC(dc);
                    ReleaseDC(hwnd, screen);
                    return None;
                }
                let previous = SelectObject(dc, bitmap as HGDIOBJ);
                SendMessageW(
                    hwnd,
                    WM_PRINT,
                    dc as usize,
                    (PRF_CLIENT | PRF_CHILDREN | PRF_ERASEBKGND) as isize,
                );
                let mut pixels = std::slice::from_raw_parts(
                    data as *const u8,
                    width as usize * height as usize * 4,
                )
                .to_vec();
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                    pixel[3] = 255;
                }
                SelectObject(dc, previous);
                DeleteObject(bitmap as HGDIOBJ);
                DeleteDC(dc);
                ReleaseDC(hwnd, screen);
                ::image::RgbaImage::from_raw(width as u32, height as u32, pixels)
            }
        }
        let Ok(buffer) = ui.window().take_snapshot() else {
            return false;
        };
        let Some(mut image) = ::image::RgbaImage::from_raw(
            buffer.width(),
            buffer.height(),
            buffer.as_bytes().to_vec(),
        ) else {
            return false;
        };
        unsafe {
            for item in self.state.items.borrow().iter().filter(|i| !i.tool) {
                if let Some(native) = capture(item.hwnd) {
                    ::image::imageops::overlay(
                        &mut image,
                        &native,
                        self.px(item.spec.frame[0]) as i64,
                        self.px(item.spec.frame[1]) as i64,
                    );
                }
            }
            let popup = self.state.popup.get();
            if !popup.is_null()
                && let Some(native) = capture(popup)
            {
                let mut point = POINT::default();
                ClientToScreen(popup, &mut point);
                ScreenToClient(self.state.parent.get(), &mut point);
                ::image::imageops::overlay(&mut image, &native, point.x as i64, point.y as i64);
            }
        }
        image.save(path).is_ok()
    }
}
impl Drop for Platform {
    fn drop(&mut self) {
        unsafe {
            self.close_popup();
            self.state.updating.set(true);
            for item in self.state.items.borrow_mut().drain(..) {
                if IsWindow(item.hwnd) != 0 {
                    DestroyWindow(item.hwnd);
                }
            }
            let parent = self.state.parent.get();
            if !parent.is_null() && IsWindow(parent) != 0 {
                RemoveWindowSubclass(parent, Some(subclass), SUBCLASS);
            }
            let font = self.state.font.get();
            if !font.is_null() {
                DeleteObject(font as HGDIOBJ);
            }
        }
    }
}
