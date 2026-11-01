//! Win32 menus use the OS Alt/F10, mnemonic, arrow-key and accessibility behavior.
//! Subclass callbacks only enqueue command IDs; application code runs later.
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::*,
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
#[derive(Clone, PartialEq)]
pub struct Entry {
    pub action: String,
    pub label: String,
    pub enabled: bool,
}
pub type Model = Vec<(String, Vec<Entry>)>;
struct State {
    alive: Cell<bool>,
    events: RefCell<VecDeque<usize>>,
}
pub struct Menu {
    hwnd: HWND,
    menu: HMENU,
    state: Box<State>,
    model: Model,
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
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
    if msg == WM_COMMAND && l == 0 && w >> 16 == 0 && w >= 1000 {
        state.events.borrow_mut().push_back(w - 1000);
        return 0;
    }
    if msg == WM_NCDESTROY {
        state.alive.set(false);
        unsafe {
            RemoveWindowSubclass(hwnd, Some(subclass), 0x52434d);
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, w, l) }
}
impl Menu {
    /// Must be called on the owning GUI thread, outside controller borrows:
    /// attaching a menu can synchronously resize the host window.
    pub fn attach(hwnd: HWND) -> Option<Self> {
        let state = Box::new(State {
            alive: Cell::new(true),
            events: RefCell::new(VecDeque::new()),
        });
        let menu = unsafe { CreateMenu() };
        if menu.is_null() {
            return None;
        }
        if unsafe {
            SetWindowSubclass(
                hwnd,
                Some(subclass),
                0x52434d,
                (&*state as *const State) as usize,
            )
        } == 0
        {
            unsafe {
                DestroyMenu(menu);
            }
            return None;
        }
        if unsafe { SetMenu(hwnd, menu) } == 0 {
            unsafe {
                RemoveWindowSubclass(hwnd, Some(subclass), 0x52434d);
                DestroyMenu(menu);
            }
            return None;
        }
        Some(Self {
            hwnd,
            menu,
            state,
            model: Vec::new(),
        })
    }
    pub fn update(&mut self, model: Model) {
        if self.model == model || !self.state.alive.get() {
            return;
        }
        // Keep the attached menu handle stable while rebuilding its contents.
        unsafe {
            while GetMenuItemCount(self.menu) > 0 {
                DeleteMenu(self.menu, 0, MF_BYPOSITION);
            }
            let mut id = 1000;
            for (title, entries) in &model {
                let popup = CreatePopupMenu();
                if popup.is_null() {
                    continue;
                }
                for entry in entries {
                    let text = wide(&entry.label);
                    AppendMenuW(
                        popup,
                        MF_STRING | if entry.enabled { MF_ENABLED } else { MF_GRAYED },
                        id,
                        text.as_ptr(),
                    );
                    id += 1;
                }
                AppendMenuW(self.menu, MF_POPUP, popup as usize, wide(title).as_ptr());
            }
            DrawMenuBar(self.hwnd);
        }
        self.model = model;
    }
    pub fn poll(&self) -> Vec<String> {
        let entries = self.model.iter().flat_map(|(_, e)| e).collect::<Vec<_>>();
        self.state
            .events
            .borrow_mut()
            .drain(..)
            .filter_map(|id| {
                entries
                    .get(id)
                    .filter(|e| e.enabled)
                    .map(|e| e.action.clone())
            })
            .collect()
    }
}
impl Drop for Menu {
    fn drop(&mut self) {
        if self.state.alive.get() {
            unsafe {
                RemoveWindowSubclass(self.hwnd, Some(subclass), 0x52434d);
                SetMenu(self.hwnd, null_mut());
                DestroyMenu(self.menu);
            }
        }
        // On WM_NCDESTROY Windows owns and destroys the attached menu tree.
    }
}
