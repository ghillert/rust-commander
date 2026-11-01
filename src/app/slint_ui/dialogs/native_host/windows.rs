//! System Win32/common controls on their own UI thread/process. No WinUI preview
//! runtime or nested Slint loop is needed. V6 visual styles and system fonts are
//! selected by the app manifest; colors and DPI follow Windows.
use super::*;
#[cfg(feature = "screenshot")]
use windows_sys::Win32::Storage::Xps::{PW_CLIENTONLY, PrintWindow};

use std::{
    cell::RefCell,
    collections::HashMap,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn tr(s: &str) -> String {
    crate::app::tr(s).into_owned()
}
fn file_picker(
    owner: HWND,
    picker: Picker,
    current: &str,
) -> ::windows::core::Result<Option<std::path::PathBuf>> {
    use ::windows::{
        Win32::{
            Foundation::HWND as Owner,
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize,
            },
            UI::Shell::{
                FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PICKFOLDERS, FileOpenDialog,
                FileSaveDialog, IFileDialog, IShellItem, SHCreateItemFromParsingName,
                SIGDN_FILESYSPATH,
            },
        },
        core::HSTRING,
    };
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _apartment = Apartment;
        let dialog: IFileDialog = CoCreateInstance(
            if picker == Picker::SaveFile {
                &FileSaveDialog
            } else {
                &FileOpenDialog
            },
            None,
            CLSCTX_INPROC_SERVER,
        )?;
        let mut options = dialog.GetOptions()? | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR;
        if matches!(picker, Picker::Directory | Picker::NewDirectory) {
            options |= FOS_PICKFOLDERS;
        }
        dialog.SetOptions(options)?;
        dialog.SetTitle(&HSTRING::from(tr(if picker == Picker::NewDirectory {
            "Choose parent folder"
        } else {
            "Browse"
        })))?;
        let path = std::path::Path::new(current);
        let folder = if picker == Picker::Directory {
            Some(path)
        } else {
            path.parent()
        };
        if let Some(folder) = folder.filter(|path| path.is_absolute()) {
            let shell: ::windows::core::Result<IShellItem> = SHCreateItemFromParsingName(
                &HSTRING::from_wide(&folder.as_os_str().encode_wide().collect::<Vec<_>>()),
                None,
            );
            if let Ok(shell) = shell {
                let _ = dialog.SetDefaultFolder(&shell);
            }
        }
        if picker == Picker::SaveFile
            && let Some(name) = path.file_name()
        {
            dialog.SetFileName(&HSTRING::from_wide(&name.encode_wide().collect::<Vec<_>>()))?;
        }
        if let Err(error) = dialog.Show(Some(Owner(owner))) {
            if error.code().0 as u32 == 0x800704c7 {
                return Ok(None);
            }
            return Err(error);
        }
        let selected = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let value = std::ffi::OsString::from_wide(selected.as_wide());
        CoTaskMemFree(Some(selected.0.cast()));
        Ok(Some(value.into()))
    }
}
struct Control {
    id: String,
    hwnd: HWND,
    kind: i32,
    choices: Vec<String>,
}
struct View {
    window: HWND,
    body: HWND,
    tabs: HWND,
    subtabs: HWND,
    error: HWND,
    review: HWND,
    busy: HWND,
    fields: Vec<Control>,
    buttons: HashMap<usize, (String, HWND)>,
    model: Model,
    font: HFONT,
    dpi: u32,
    page: i32,
    main_page: i32,
    folder_page: i32,
    appearance_page: i32,
    scroll: i32,
    total: i32,
    updating: bool,
    next_id: usize,
    structure: String,
}
impl View {
    #[cfg(feature = "screenshot")]
    unsafe fn capture(&self, path: &str) -> bool {
        unsafe {
            let mut bounds: RECT = std::mem::zeroed();
            GetClientRect(self.window, &mut bounds);
            let (w, h) = (bounds.right, bounds.bottom);
            if w <= 0 || h <= 0 {
                return false;
            }
            let screen = GetDC(self.window);
            let dc = CreateCompatibleDC(screen);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut data = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut data, null_mut(), 0);
            if bitmap.is_null() {
                DeleteDC(dc);
                ReleaseDC(self.window, screen);
                return false;
            }
            let previous = SelectObject(dc, bitmap as HGDIOBJ);
            RedrawWindow(
                self.window,
                null(),
                null_mut(),
                RDW_INVALIDATE | RDW_UPDATENOW | RDW_ALLCHILDREN,
            );
            // Read only our visible client area. PrintWindow omits some child
            // controls on alternative Windows implementations and remote desktops.
            let ok = BitBlt(dc, 0, 0, w, h, screen, 0, 0, SRCCOPY) != 0
                || PrintWindow(self.window, dc, PW_CLIENTONLY) != 0;
            let mut pixels =
                std::slice::from_raw_parts(data as *const u8, w as usize * h as usize * 4).to_vec();
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
                pixel[3] = 255;
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap as HGDIOBJ);
            DeleteDC(dc);
            ReleaseDC(self.window, screen);
            ok && image::save_buffer(path, &pixels, w as u32, h as u32, image::ColorType::Rgba8)
                .is_ok()
        }
    }
    #[cfg(feature = "screenshot")]
    unsafe fn smoke(&mut self, callback: &str, value: &str) -> bool {
        unsafe {
            match callback {
                "native-field" => {
                    let Ok((id, value)) = serde_json::from_str::<(String, String)>(value) else {
                        return false;
                    };
                    let Some(control) = self.fields.iter().find(|f| f.id == id) else {
                        return false;
                    };
                    let hwnd = control.hwnd;
                    if matches!(control.kind, 2 | 8) {
                        SendMessageW(hwnd, BM_SETCHECK, usize::from(value == "true"), 0);
                        emit(&Event::Field(id, value));
                    } else if control.kind == 3 {
                        let Some(index) = control.choices.iter().position(|v| v == &value) else {
                            return false;
                        };
                        SendMessageW(hwnd, CB_SETCURSEL, index, 0);
                        emit(&Event::Field(id, value));
                    } else {
                        SetWindowTextW(hwnd, wide(&value).as_ptr());
                        emit(&Event::Field(id, value));
                    }
                    true
                }
                "native-action" if value == "close" => {
                    PostMessageW(self.window, WM_CLOSE, 0, 0) != 0
                }
                "native-action" => {
                    if value == "return"
                        && self.model.form.busy
                        && self.model.form.kind == "prompt"
                        && self.model.form.default_action
                        && self.model.form.error.is_empty()
                    {
                        emit(&Event::Action("accept".into()));
                        return true;
                    }
                    let id = if value == "return" { "accept" } else { value };
                    let Some((_, button)) = self
                        .buttons
                        .values()
                        .find(|(key, button)| key == id && IsWindowEnabled(*button) != 0)
                    else {
                        return false;
                    };
                    PostMessageW(*button, BM_CLICK, 0, 0) != 0
                }
                "native-check" => {
                    self.model.form.error.contains(value)
                        && IsWindowVisible(self.window) != 0
                        && self
                            .fields
                            .iter()
                            .all(|field| IsWindowVisible(field.hwnd) != 0)
                        && self
                            .buttons
                            .values()
                            .filter(|(id, _)| !id.starts_with("browse:"))
                            .all(|(_, button)| IsWindowVisible(*button) != 0)
                }
                _ => false,
            }
        }
    }
    fn px(&self, n: i32) -> i32 {
        ((n as i64 * self.dpi as i64 + 48) / 96) as i32
    }
    #[allow(clippy::too_many_arguments)] // Mirrors CreateWindowEx: style, owner and bounds.
    unsafe fn child(
        &mut self,
        class: &str,
        text: &str,
        style: u32,
        parent: HWND,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> (usize, HWND) {
        unsafe {
            let id = self.next_id;
            self.next_id += 1;
            let hwnd = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                self.px(x),
                self.px(y),
                self.px(w),
                self.px(h),
                parent,
                id as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            SendMessageW(hwnd, WM_SETFONT, self.font as usize, 1);
            (id, hwnd)
        }
    }
    #[allow(clippy::too_many_arguments)] // Native button label, owner and bounds.
    unsafe fn button(
        &mut self,
        id: &str,
        title: &str,
        enabled: bool,
        parent: HWND,
        x: i32,
        y: i32,
        w: i32,
    ) -> HWND {
        unsafe {
            let (key, hwnd) = self.child(
                "BUTTON",
                &tr(title),
                WS_TABSTOP
                    | if id == "accept" && self.model.form.default_action {
                        BS_DEFPUSHBUTTON as u32
                    } else {
                        BS_PUSHBUTTON as u32
                    },
                parent,
                x,
                y,
                w,
                28,
            );
            EnableWindow(hwnd, enabled as i32);
            self.buttons.insert(key, (id.into(), hwnd));
            hwnd
        }
    }
    unsafe fn set_font(&mut self) {
        unsafe {
            if !self.font.is_null() {
                DeleteObject(self.font as HGDIOBJ);
            }
            let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
            metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
            self.font = if SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS,
                metrics.cbSize,
                &mut metrics as *mut _ as *mut _,
                0,
                self.dpi,
            ) != 0
            {
                CreateFontIndirectW(&metrics.lfMessageFont)
            } else {
                let mut fallback: LOGFONTW = std::mem::zeroed();
                GetObjectW(
                    GetStockObject(DEFAULT_GUI_FONT),
                    std::mem::size_of::<LOGFONTW>() as i32,
                    &mut fallback as *mut _ as *mut _,
                );
                CreateFontIndirectW(&fallback)
            };
        }
    }
    unsafe fn tabs(&mut self) {
        unsafe {
            for window in [self.tabs, self.subtabs] {
                if !window.is_null() {
                    DestroyWindow(window);
                }
            }
            self.tabs = null_mut();
            self.subtabs = null_mut();
            if self.model.form.kind != "preferences" {
                return;
            }
            self.tabs = self
                .child(
                    "SysTabControl32",
                    "",
                    WS_TABSTOP,
                    self.window,
                    16,
                    12,
                    760,
                    28,
                )
                .1;
            for (i, title) in [
                "General",
                "Folders",
                "Appearance",
                "Shortcuts",
                "Mail",
                "Advanced",
            ]
            .iter()
            .enumerate()
            {
                self.tab_item(self.tabs, i, title);
            }
            SendMessageW(self.tabs, TCM_SETCURSEL, self.main_page as usize, 0);
            let names: &[&str] = match self.main_page {
                1 => &["Browsing", "Columns"],
                2 => &["Theme and icons", "Toolbar", "Command bar"],
                _ => &[],
            };
            if !names.is_empty() {
                self.subtabs = self
                    .child(
                        "SysTabControl32",
                        "",
                        WS_TABSTOP,
                        self.window,
                        24,
                        50,
                        744,
                        28,
                    )
                    .1;
                for (i, name) in names.iter().enumerate() {
                    self.tab_item(self.subtabs, i, name);
                }
                SendMessageW(
                    self.subtabs,
                    TCM_SETCURSEL,
                    if self.main_page == 1 {
                        self.folder_page
                    } else {
                        self.appearance_page
                    } as usize,
                    0,
                );
            }
        }
    }
    unsafe fn tab_item(&self, tabs: HWND, index: usize, title: &str) {
        unsafe {
            let mut text = wide(&tr(title));
            let mut item: TCITEMW = std::mem::zeroed();
            item.mask = TCIF_TEXT;
            item.pszText = text.as_mut_ptr();
            SendMessageW(tabs, TCM_INSERTITEMW, index, &item as *const _ as isize);
        }
    }
    fn selected_page(&self) -> i32 {
        match self.main_page {
            1 => {
                if self.folder_page == 0 {
                    1
                } else {
                    8
                }
            }
            2 => match self.appearance_page {
                0 => 2,
                1 => 6,
                _ => 7,
            },
            page => page,
        }
    }
    unsafe fn show_page(&mut self, page: i32) {
        unsafe {
            match page {
                6 | 7 => {
                    self.main_page = 2;
                    self.appearance_page = page - 5;
                }
                8 => {
                    self.main_page = 1;
                    self.folder_page = 1;
                }
                0..=5 => {
                    self.main_page = page;
                    if page == 1 {
                        self.folder_page = 0;
                    }
                    if page == 2 {
                        self.appearance_page = 0;
                    }
                }
                _ => return,
            }
            self.page = self.selected_page();
            self.scroll = 0;
            self.tabs();
            self.build();
        }
    }
    fn fingerprint(model: &Model, page: i32) -> String {
        // Editable values, including passwords, are excluded. This stays in memory only.
        let fields = model
            .form
            .fields
            .iter()
            .filter(|f| model.form.kind != "preferences" || f.page == page)
            .map(|f| {
                (
                    &f.id,
                    &f.title,
                    f.kind,
                    f.picker,
                    if matches!(f.kind, 6 | 9) {
                        f.value.clone()
                    } else {
                        String::new()
                    },
                    if f.kind == 3 {
                        f.choices.clone()
                    } else if f.kind == 11 {
                        vec![f.choices[1].clone()]
                    } else {
                        Vec::new()
                    },
                )
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&(
            model.language,
            &model.form.description,
            fields,
            &model.form.rows,
            model
                .form
                .buttons
                .iter()
                .map(|(id, title, _)| (id, title))
                .collect::<Vec<_>>(),
        ))
        .unwrap()
    }
    unsafe fn build(&mut self) {
        unsafe {
            self.updating = true;
            let focus = GetFocus();
            let editing = self
                .fields
                .iter()
                .find(|f| f.hwnd == focus && matches!(f.kind, 0 | 1 | 4 | 5))
                .map(|f| {
                    let (mut a, mut b) = (0u32, 0u32);
                    SendMessageW(
                        f.hwnd,
                        EM_GETSEL,
                        &mut a as *mut _ as usize,
                        &mut b as *mut _ as isize,
                    );
                    (f.id.clone(), a, b)
                });
            for f in self.fields.drain(..) {
                DestroyWindow(f.hwnd);
            }
            for (_, (_, hwnd)) in self.buttons.drain() {
                DestroyWindow(hwnd);
            }
            let mut child = GetWindow(self.body, GW_CHILD);
            while !child.is_null() {
                let next = GetWindow(child, GW_HWNDNEXT);
                DestroyWindow(child);
                child = next;
            }
            self.next_id = 100;
            let mut y = 12;
            if self.model.form.kind != "preferences" && !self.model.form.description.is_empty() {
                let description = self.model.form.description.clone();
                let height = if description.len() > 300 { 120 } else { 48 };
                self.child(
                    if height == 120 { "EDIT" } else { "STATIC" },
                    &description,
                    if height == 120 {
                        WS_TABSTOP
                            | WS_VSCROLL
                            | ES_MULTILINE as u32
                            | ES_READONLY as u32
                            | ES_AUTOVSCROLL as u32
                    } else {
                        0
                    },
                    self.body,
                    12,
                    y,
                    700,
                    height,
                );
                y += height + 8;
            }
            let fields = self
                .model
                .form
                .fields
                .iter()
                .filter(|f| self.model.form.kind != "preferences" || f.page == self.page)
                .cloned()
                .collect::<Vec<_>>();
            for field in fields {
                let title = tr(&field.title);
                match field.kind {
                    10 => {
                        self.child("STATIC", &title, 0, self.body, 12, y, 700, 26);
                        y += 34;
                    }
                    6 | 9 => {
                        if !title.is_empty() {
                            self.child("STATIC", &title, 0, self.body, 12, y, 700, 24);
                            y += 28;
                        }
                        if field.kind == 6 && field.value.len() > 300 {
                            let hwnd = self
                                .child(
                                    "EDIT",
                                    &field.value,
                                    WS_TABSTOP
                                        | WS_BORDER
                                        | WS_VSCROLL
                                        | ES_MULTILINE as u32
                                        | ES_AUTOVSCROLL as u32
                                        | ES_READONLY as u32,
                                    self.body,
                                    12,
                                    y,
                                    700,
                                    180,
                                )
                                .1;
                            self.fields.push(Control {
                                id: field.id,
                                hwnd,
                                kind: 6,
                                choices: Vec::new(),
                            });
                            y += 190;
                            continue;
                        }
                        self.child("STATIC", &field.value, 0, self.body, 12, y, 700, 52);
                        y += 60;
                    }
                    12 => {
                        self.child(
                            "STATIC",
                            &format!(
                                "{}                         {}                         {}",
                                field.title, field.value, field.choices[0]
                            ),
                            0,
                            self.body,
                            12,
                            y,
                            700,
                            24,
                        );
                        y += 30;
                    }
                    2 | 8 => {
                        let hwnd = self
                            .child(
                                "BUTTON",
                                &title,
                                WS_TABSTOP
                                    | if field.kind == 8 {
                                        BS_PUSHBUTTON as u32
                                    } else {
                                        BS_AUTOCHECKBOX as u32
                                    },
                                self.body,
                                12,
                                y,
                                700,
                                26,
                            )
                            .1;
                        SendMessageW(hwnd, BM_SETCHECK, usize::from(field.value == "true"), 0);
                        EnableWindow(hwnd, (field.id != "column-Name") as i32);
                        self.fields.push(Control {
                            id: field.id,
                            hwnd,
                            kind: field.kind,
                            choices: Vec::new(),
                        });
                        y += 34;
                    }
                    0 | 1 | 3 | 4 | 5 | 11 => {
                        self.child("STATIC", &title, 0, self.body, 12, y + 4, 195, 32);
                        let width = if field.kind == 11 {
                            238
                        } else if field.picker.is_some() {
                            394
                        } else {
                            500
                        };
                        let (class, style, height) = if field.kind == 3 {
                            (
                                "COMBOBOX",
                                WS_TABSTOP
                                    | WS_VSCROLL
                                    | CBS_DROPDOWNLIST as u32
                                    | CBS_HASSTRINGS as u32,
                                240,
                            )
                        } else {
                            (
                                "EDIT",
                                WS_TABSTOP
                                    | WS_BORDER
                                    | ES_AUTOHSCROLL as u32
                                    | if field.kind == 1 {
                                        ES_PASSWORD as u32
                                    } else if field.kind == 4 {
                                        ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | WS_VSCROLL
                                    } else {
                                        0
                                    },
                                if field.kind == 4 { 100 } else { 26 },
                            )
                        };
                        let hwnd = self
                            .child(
                                class,
                                if field.kind == 3 { "" } else { &field.value },
                                style,
                                self.body,
                                218,
                                y,
                                width,
                                height,
                            )
                            .1;
                        if field.kind == 3 {
                            for choice in &field.choices {
                                SendMessageW(
                                    hwnd,
                                    CB_ADDSTRING,
                                    0,
                                    wide(&tr(choice)).as_ptr() as isize,
                                );
                            }
                            SendMessageW(
                                hwnd,
                                CB_SETCURSEL,
                                field
                                    .choices
                                    .iter()
                                    .position(|v| v == &field.value)
                                    .unwrap_or(usize::MAX),
                                0,
                            );
                        }
                        self.fields.push(Control {
                            id: field.id,
                            hwnd,
                            kind: field.kind,
                            choices: field.choices.clone(),
                        });
                        if field.picker.is_some() {
                            self.button(
                                &format!("browse:{}", self.fields.last().unwrap().id),
                                "Browse…",
                                true,
                                self.body,
                                620,
                                y,
                                98,
                            );
                        }
                        if field.kind == 11 {
                            let hwnd = self
                                .child(
                                    "EDIT",
                                    &field.choices[0],
                                    WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                                    self.body,
                                    480,
                                    y,
                                    238,
                                    26,
                                )
                                .1;
                            self.fields.push(Control {
                                id: field.choices[1].clone(),
                                hwnd,
                                kind: 0,
                                choices: Vec::new(),
                            });
                        }
                        y += if field.kind == 4 { 112 } else { 38 };
                    }
                    _ => {}
                }
            }
            let rows = self
                .model
                .form
                .rows
                .iter()
                .filter(|r| self.model.form.kind != "preferences" || r.page == self.page)
                .cloned()
                .collect::<Vec<_>>();
            for row in rows {
                self.child("STATIC", &row.title, 0, self.body, 12, y + 4, 480, 24);
                for (i, (id, title, enabled)) in row.actions.iter().enumerate() {
                    self.button(id, title, *enabled, self.body, 510 + i as i32 * 102, y, 98);
                }
                y += 36;
            }
            self.total = y + 12;
            let buttons = self.model.form.buttons.clone();
            let mut x = 764;
            // Windows puts dismissal after the affirmative actions.
            let mut ordered = buttons.iter().collect::<Vec<_>>();
            ordered.sort_by_key(|(id, _, _)| match id.as_str() {
                "cancel" => 0,
                "accept" => 1,
                "apply" => 2,
                "defaults" => 99,
                _ => 3,
            });
            for (id, title, enabled) in ordered {
                let width = (title.chars().count() as i32 * 7 + 28).max(80);
                let left = if id == "defaults" { 16 } else { x - width };
                self.button(id, title, *enabled, self.window, left, 572, width);
                if id != "defaults" {
                    x -= width + 8;
                }
            }
            self.structure = Self::fingerprint(&self.model, self.page);
            self.layout();
            self.update_values();
            self.updating = false;
            if let Some((id, a, b)) = editing
                && let Some(f) = self.fields.iter().find(|f| f.id == id)
            {
                SetFocus(f.hwnd);
                SendMessageW(f.hwnd, EM_SETSEL, a as usize, b as isize);
            }
        }
    }
    unsafe fn layout(&mut self) {
        unsafe {
            let mut client: RECT = std::mem::zeroed();
            GetClientRect(self.window, &mut client);
            let width = client.right * 96 / self.dpi as i32;
            let height = client.bottom * 96 / self.dpi as i32;
            let top = if self.model.form.kind == "preferences" {
                if self.subtabs.is_null() { 52 } else { 90 }
            } else {
                12
            };
            MoveWindow(
                self.body,
                self.px(16),
                self.px(top),
                self.px(width - 32),
                self.px((height - top - 140).max(40)),
                1,
            );
            if !self.tabs.is_null() {
                MoveWindow(
                    self.tabs,
                    self.px(16),
                    self.px(12),
                    self.px(width - 32),
                    self.px(30),
                    1,
                );
            }
            if !self.subtabs.is_null() {
                MoveWindow(
                    self.subtabs,
                    self.px(24),
                    self.px(50),
                    self.px(width - 48),
                    self.px(30),
                    1,
                );
            }
            MoveWindow(
                self.error,
                self.px(16),
                self.px(height - 132),
                self.px(width - 32),
                self.px(40),
                1,
            );
            MoveWindow(
                self.review,
                self.px(16),
                self.px(height - 74),
                self.px(190),
                self.px(26),
                1,
            );
            MoveWindow(
                self.busy,
                self.px(16),
                self.px(height - 74),
                self.px(190),
                self.px(16),
                1,
            );
            let mut footer = self.model.form.buttons.iter().collect::<Vec<_>>();
            footer.sort_by_key(|(id, _, _)| match id.as_str() {
                "cancel" => 0,
                "accept" => 1,
                "apply" => 2,
                "defaults" => 99,
                _ => 3,
            });
            let mut right = width - 16;
            for (id, title, _) in footer {
                if let Some((_, button)) = self.buttons.values().find(|(key, _)| key == id) {
                    let bw = (title.chars().count() as i32 * 7 + 28).max(80);
                    let bx = if id == "defaults" { 16 } else { right - bw };
                    MoveWindow(
                        *button,
                        self.px(bx),
                        self.px(height - 40),
                        self.px(bw),
                        self.px(28),
                        1,
                    );
                    if id != "defaults" {
                        right -= bw + 8;
                    }
                }
            }
            let mut info: SCROLLINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
            info.fMask = SIF_ALL;
            info.nMin = 0;
            info.nMax = self.px(self.total);
            info.nPage = self.px((height - top - 140).max(40)) as u32;
            info.nPos = self.scroll;
            SetScrollInfo(self.body, SB_VERT, &info, 1);
        }
    }
    unsafe fn update_values(&mut self) {
        unsafe {
            self.updating = true;
            SetWindowTextW(self.window, wide(&self.model.form.title).as_ptr());
            SetWindowTextW(self.error, wide(&self.model.form.error).as_ptr());
            ShowWindow(
                self.busy,
                if self.model.form.busy && self.model.form.error.is_empty() {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            SendMessageW(
                self.busy,
                PBM_SETMARQUEE,
                usize::from(self.model.form.busy),
                30,
            );
            ShowWindow(
                self.review,
                if self.model.form.error_page.is_some() {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
            for control in &self.fields {
                let value = self
                    .model
                    .form
                    .fields
                    .iter()
                    .find(|f| f.id == control.id)
                    .map(|f| f.value.as_str())
                    .or_else(|| {
                        self.model
                            .form
                            .fields
                            .iter()
                            .find(|f| f.kind == 11 && f.choices.get(1) == Some(&control.id))
                            .map(|f| f.choices[0].as_str())
                    });
                let Some(value) = value else {
                    continue;
                };
                if control.kind == 3 {
                    SendMessageW(
                        control.hwnd,
                        CB_SETCURSEL,
                        control
                            .choices
                            .iter()
                            .position(|v| v == value)
                            .unwrap_or(usize::MAX),
                        0,
                    );
                } else if control.kind == 8 {
                    let title = self
                        .model
                        .form
                        .fields
                        .iter()
                        .find(|f| f.id == control.id)
                        .map(|f| tr(&f.title))
                        .unwrap_or_default();
                    SetWindowTextW(
                        control.hwnd,
                        wide(&format!(
                            "{} {}",
                            if value == "true" { "−" } else { "+" },
                            title
                        ))
                        .as_ptr(),
                    );
                } else if control.kind == 2 {
                    SendMessageW(control.hwnd, BM_SETCHECK, usize::from(value == "true"), 0);
                } else if text(control.hwnd) != value {
                    SetWindowTextW(control.hwnd, wide(value).as_ptr());
                }
            }
            for (id, _, enabled) in self
                .model
                .form
                .buttons
                .iter()
                .chain(self.model.form.rows.iter().flat_map(|r| r.actions.iter()))
            {
                if let Some((_, hwnd)) = self.buttons.values().find(|(key, _)| key == id) {
                    EnableWindow(*hwnd, *enabled as i32);
                }
            }
            self.updating = false;
        }
    }
    unsafe fn command(&mut self, wparam: usize, lparam: isize) {
        unsafe {
            if self.updating {
                return;
            }
            let id = wparam & 0xffff;
            let notification = (wparam >> 16) & 0xffff;
            if let Some((action, _)) = self.buttons.get(&id) {
                if let Some(field_id) = action.strip_prefix("browse:") {
                    emit(&Event::Editing);
                    if let Some(field) = self.model.form.fields.iter().find(|f| f.id == field_id)
                        && let Some(picker) = field.picker
                    {
                        let current = self
                            .fields
                            .iter()
                            .find(|f| f.id == field_id)
                            .map(|f| text(f.hwnd))
                            .unwrap_or_else(|| field.value.clone());
                        match file_picker(self.window, picker, &current) {
                            Ok(Some(path)) => emit(&Event::Field(
                                field_id.into(),
                                picker.selected(path, &current),
                            )),
                            Ok(None) => {}
                            Err(_) => {
                                MessageBoxW(self.window,wide("The system file picker could not be opened. You can enter the path in the form.").as_ptr(),wide("Rust Commander").as_ptr(),MB_OK|MB_ICONERROR);
                            }
                        }
                    }
                    return;
                }
                emit(&Event::Action(action.clone()));
                return;
            }
            if lparam as HWND == self.review {
                if let Some(page) = self.model.form.error_page {
                    self.show_page(page);
                }
                return;
            }
            if let Some(control) = self.fields.iter().find(|f| f.hwnd == lparam as HWND) {
                let value = match control.kind {
                    8 if notification == BN_CLICKED as usize => self
                        .model
                        .form
                        .fields
                        .iter()
                        .find(|f| f.id == control.id)
                        .map(|f| (f.value != "true").to_string()),
                    2 if notification == BN_CLICKED as usize => {
                        Some((SendMessageW(control.hwnd, BM_GETCHECK, 0, 0) != 0).to_string())
                    }
                    3 if notification == CBN_SELCHANGE as usize => control
                        .choices
                        .get(SendMessageW(control.hwnd, CB_GETCURSEL, 0, 0) as usize)
                        .cloned(),
                    0 | 1 | 4 | 5 | 11 if notification == EN_CHANGE as usize => {
                        Some(text(control.hwnd))
                    }
                    _ => None,
                };
                if let Some(value) = value {
                    emit(&Event::Field(control.id.clone(), value));
                }
            }
        }
    }
}
unsafe fn text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut text = vec![0; len + 1];
        let count = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&text[..count])
    }
}
unsafe extern "system" fn body_proc(hwnd: HWND, msg: u32, w: usize, l: isize) -> isize {
    unsafe {
        let parent = GetParent(hwnd);
        if matches!(msg, WM_COMMAND | WM_NOTIFY | WM_VSCROLL | WM_MOUSEWHEEL) {
            return SendMessageW(parent, msg, w, l);
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, w: usize, l: isize) -> isize {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RefCell<View>;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, w, l);
        }
        let Ok(mut view) = (&*ptr).try_borrow_mut() else {
            return DefWindowProcW(hwnd, msg, w, l);
        };
        match msg {
            WM_GETMINMAXINFO => {
                let info = &mut *(l as *mut MINMAXINFO);
                let mut bounds = RECT {
                    left: 0,
                    top: 0,
                    right: view.px(780),
                    bottom: view.px(420),
                };
                AdjustWindowRectExForDpi(
                    &mut bounds,
                    WS_OVERLAPPEDWINDOW,
                    0,
                    WS_EX_CONTROLPARENT,
                    view.dpi,
                );
                info.ptMinTrackSize = POINT {
                    x: bounds.right - bounds.left,
                    y: bounds.bottom - bounds.top,
                };
                0
            }
            WM_COMMAND => {
                view.command(w, l);
                0
            }
            WM_CLOSE => {
                emit(&Event::Action("cancel".into()));
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            WM_SIZE => {
                view.layout();
                0
            }
            WM_NOTIFY => {
                let header = &*(l as *const NMHDR);
                if header.code == TCN_SELCHANGE {
                    if header.hwndFrom == view.tabs {
                        view.main_page = SendMessageW(view.tabs, TCM_GETCURSEL, 0, 0) as i32;
                        view.page = view.selected_page();
                        view.scroll = 0;
                        view.tabs();
                        view.build();
                    } else if header.hwndFrom == view.subtabs {
                        let n = SendMessageW(view.subtabs, TCM_GETCURSEL, 0, 0) as i32;
                        if view.main_page == 1 {
                            view.folder_page = n;
                        } else {
                            view.appearance_page = n;
                        }
                        view.page = view.selected_page();
                        view.scroll = 0;
                        view.build();
                    }
                }
                0
            }
            WM_VSCROLL | WM_MOUSEWHEEL => {
                let mut info: SCROLLINFO = std::mem::zeroed();
                info.cbSize = std::mem::size_of::<SCROLLINFO>() as u32;
                info.fMask = SIF_ALL;
                GetScrollInfo(view.body, SB_VERT, &mut info);
                let next = if msg == WM_MOUSEWHEEL {
                    view.scroll - ((w >> 16) as u16 as i16 as i32 / 120) * view.px(80)
                } else {
                    match (w & 0xffff) as i32 {
                        SB_LINEUP => view.scroll - view.px(32),
                        SB_LINEDOWN => view.scroll + view.px(32),
                        SB_PAGEUP => view.scroll - info.nPage as i32,
                        SB_PAGEDOWN => view.scroll + info.nPage as i32,
                        SB_THUMBTRACK => info.nTrackPos,
                        _ => view.scroll,
                    }
                };
                let next = next.clamp(0, (info.nMax - info.nPage as i32).max(0));
                ScrollWindowEx(
                    view.body,
                    0,
                    view.scroll - next,
                    null(),
                    null(),
                    null_mut(),
                    null_mut(),
                    SW_SCROLLCHILDREN | SW_INVALIDATE,
                );
                view.scroll = next;
                SetScrollPos(view.body, SB_VERT, next, 1);
                0
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE => {
                // Common controls redraw with the updated system colors. Keep
                // the current form and focused editor intact during a theme change.
                RedrawWindow(
                    hwnd,
                    null(),
                    null_mut(),
                    RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN,
                );
                DefWindowProcW(hwnd, msg, w, l)
            }
            WM_DPICHANGED => {
                view.dpi = (w & 0xffff) as u32;
                view.set_font();
                let r = &*(l as *const RECT);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                view.tabs();
                view.build();
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}
pub(super) fn run(model: Model, incoming: Incoming) -> i32 {
    unsafe {
        let Parent::Windows(owner) = model.parent else {
            emit(&Event::Failed);
            return 2;
        };
        crate::i18n::set(model.language);
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_TAB_CLASSES | ICC_STANDARD_CLASSES | ICC_PROGRESS_CLASS,
        };
        InitCommonControlsEx(&controls);
        let instance = GetModuleHandleW(null());
        let name = wide("RustCommanderNativeForm");
        let body_name = wide("RustCommanderNativeFormBody");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: name.as_ptr(),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..std::mem::zeroed()
        };
        RegisterClassW(&class);
        let body_class = WNDCLASSW {
            lpfnWndProc: Some(body_proc),
            lpszClassName: body_name.as_ptr(),
            ..class
        };
        RegisterClassW(&body_class);
        let mut owner_rect: RECT = std::mem::zeroed();
        GetWindowRect(owner as HWND, &mut owner_rect);
        let dpi = GetDpiForWindow(owner as HWND).max(96);
        let mut initial_bounds = RECT {
            left: 0,
            top: 0,
            right: ((if model.form.kind == "preferences" {
                840
            } else {
                780
            }) * dpi
                / 96) as i32,
            bottom: (620 * dpi / 96) as i32,
        };
        AdjustWindowRectExForDpi(
            &mut initial_bounds,
            WS_OVERLAPPEDWINDOW,
            0,
            WS_EX_CONTROLPARENT,
            dpi,
        );
        let window = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            name.as_ptr(),
            wide(&model.form.title).as_ptr(),
            WS_OVERLAPPEDWINDOW,
            owner_rect.left + 40,
            owner_rect.top + 40,
            initial_bounds.right - initial_bounds.left,
            initial_bounds.bottom - initial_bounds.top,
            owner as HWND,
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            emit(&Event::Failed);
            return 2;
        }
        let mut initial = Box::new(View {
            window,
            body: null_mut(),
            tabs: null_mut(),
            subtabs: null_mut(),
            error: null_mut(),
            review: null_mut(),
            busy: null_mut(),
            fields: Vec::new(),
            buttons: HashMap::new(),
            model,
            font: null_mut(),
            dpi: GetDpiForWindow(window).max(96),
            page: 0,
            main_page: 0,
            folder_page: 0,
            appearance_page: 0,
            scroll: 0,
            total: 0,
            updating: true,
            next_id: 10,
            structure: String::new(),
        });
        initial.set_font();
        initial.body = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            body_name.as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_VSCROLL,
            16,
            12,
            760,
            500,
            window,
            null_mut(),
            instance,
            null(),
        );
        initial.error = initial.child("STATIC", "", 0, window, 16, 520, 740, 40).1;
        initial.review = initial
            .child(
                "BUTTON",
                &tr("Review invalid setting"),
                WS_TABSTOP,
                window,
                16,
                548,
                190,
                26,
            )
            .1;
        initial.busy = initial
            .child(
                "msctls_progress32",
                "",
                PBS_MARQUEE,
                window,
                16,
                548,
                190,
                16,
            )
            .1;
        let data = Box::new(RefCell::new(*initial));
        SetWindowLongPtrW(
            window,
            GWLP_USERDATA,
            &*data as *const RefCell<View> as isize,
        );
        {
            let mut initial = data.borrow_mut();
            initial.tabs();
            initial.build();
            ShowWindow(window, SW_SHOW);
            UpdateWindow(window);
            emit(&Event::Ready);
        }
        let mut message: MSG = std::mem::zeroed();
        let mut running = true;
        while running {
            while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                if message.message == WM_QUIT {
                    running = false;
                    break;
                }
                if message.message == WM_KEYDOWN && message.wParam == 27 {
                    emit(&Event::Action("cancel".into()));
                    continue;
                }
                if message.message == WM_KEYDOWN && message.wParam == 13 {
                    let view = data.borrow();
                    let focus = GetFocus();
                    let editing = view.fields.iter().find(|f| f.hwnd == focus);
                    let keep = editing.is_some_and(|f| {
                        f.kind == 4
                            || (f.kind == 3 && SendMessageW(f.hwnd, CB_GETDROPPEDSTATE, 0, 0) != 0)
                            || f.id == "shortcut-filter"
                    });
                    if !keep
                        && view.model.form.default_action
                        && view.model.form.error.is_empty()
                        && view.model.form.buttons.iter().any(|(id, _, enabled)| {
                            id == "accept"
                                && (*enabled
                                    || (view.model.form.kind == "prompt" && view.model.form.busy))
                        })
                    {
                        emit(&Event::Action("accept".into()));
                        continue;
                    }
                }
                if IsDialogMessageW(window, &message) == 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            for request in incoming.try_iter() {
                match request {
                    Request::Close => running = false,
                    Request::Update(model) => {
                        let mut view = data.borrow_mut();
                        crate::i18n::set(model.language);
                        let changed = View::fingerprint(&model, view.page) != view.structure;
                        view.model = *model;
                        if changed {
                            view.tabs();
                            view.build();
                        } else {
                            view.update_values();
                        }
                    }
                    #[cfg(feature = "screenshot")]
                    Request::Page(page) => {
                        data.borrow_mut().show_page(page);
                        emit(&Event::Checked(true));
                    }
                    #[cfg(feature = "screenshot")]
                    Request::Capture(path) => emit(&Event::Captured(data.borrow().capture(&path))),
                    #[cfg(feature = "screenshot")]
                    Request::Smoke(callback, value) => {
                        emit(&Event::Checked(data.borrow_mut().smoke(&callback, &value)))
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        DestroyWindow(window);
        let font = data.borrow().font;
        if !font.is_null() {
            DeleteObject(font as HGDIOBJ);
        }
        0
    }
}
