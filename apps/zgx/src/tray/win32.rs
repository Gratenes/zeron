//! The notification-area icon itself (`Shell_NotifyIconW`), Windows' alone:
//! a message-only window on the UI thread hears its clicks (winit's loop
//! dispatches every window's messages on that thread), a left click shows
//! Zeron, a right click opens its menu (Show Zeron, Quit Zeron). Only
//! `windows-sys`: what's chosen goes to `on`, the rest is the caller's.

use std::cell::{Cell, RefCell};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW},
        WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow,
            GWLP_USERDATA, GetCursorPos, GetSystemMetrics, GetWindowLongPtrW, HICON, HWND_MESSAGE, IMAGE_ICON,
            LR_DEFAULTCOLOR, LoadImageW, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassW, RegisterWindowMessageW,
            SM_CXSMICON, SM_CYSMICON, SetForegroundWindow, SetMenuDefaultItem, SetWindowLongPtrW, TPM_BOTTOMALIGN,
            TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
            WM_NULL, WM_RBUTTONUP, WNDCLASSW,
        },
    },
};

/// What a click on the icon or its menu asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chosen {
    Show,
    Quit,
}

/// The icon's words: its tooltip and its menu's two items.
#[derive(Clone, Debug, PartialEq)]
pub struct Words {
    pub tip: String,
    pub show: String,
    pub quit: String,
}

impl Default for Words {
    fn default() -> Self {
        Self { tip: "Zeron".into(), show: "Show Zeron".into(), quit: "Quit Zeron".into() }
    }
}

/// The message the icon's clicks come as.
const CALLBACK: u32 = WM_APP + 0x5a;
/// The menu's items' ids (`TPM_RETURNCMD` hands the one chosen back).
const SHOW: usize = 1;
const QUIT: usize = 2;
/// The exe's icon (dist/windows/zeron.rc: `1 ICON "zeron.ico"`).
const ICON_ID: usize = 1;

struct Inner {
    on: Box<dyn Fn(Chosen)>,
    words: RefCell<Words>,
    /// In the notification area now.
    shown: Cell<bool>,
    icon: HICON,
    /// Explorer, restarted, says so with this (every icon is added again).
    taskbar_created: u32,
}

/// The icon, in the notification area while `set_shown(true)`; removed, its
/// window destroyed, as it drops.
pub struct Icon {
    hwnd: HWND,
    inner: *mut Inner,
}

impl Icon {
    /// The icon's window, not yet shown. `None` if Windows refused one.
    pub fn new(on: impl Fn(Chosen) + 'static) -> Option<Icon> {
        let class: Vec<u16> = "ZeronTrayIcon\0".encode_utf16().collect();
        // SAFETY: plain Win32 calls with valid, NUL-terminated strings; the
        // window is message-only (`HWND_MESSAGE`), made and used on this
        // thread alone, and `inner` outlives it (`Drop` destroys the window
        // before freeing it).
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: module,
                hIcon: std::ptr::null_mut(),
                hCursor: std::ptr::null_mut(),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class.as_ptr(),
            };
            // (A second registration fails as the class is there: fine.)
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                module,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return None;
            }
            let icon = LoadImageW(
                module,
                ICON_ID as *const u16,
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            ) as HICON;
            let created: Vec<u16> = "TaskbarCreated\0".encode_utf16().collect();
            let inner = Box::into_raw(Box::new(Inner {
                on: Box::new(on),
                words: RefCell::new(Words::default()),
                shown: Cell::new(false),
                icon,
                taskbar_created: RegisterWindowMessageW(created.as_ptr()),
            }));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, inner as isize);
            Some(Icon { hwnd, inner })
        }
    }

    fn inner(&self) -> &Inner {
        // SAFETY: `inner` lives as long as `self` (freed in `Drop`).
        unsafe { &*self.inner }
    }

    /// In the notification area, or out of it.
    pub fn set_shown(&self, shown: bool) {
        let inner = self.inner();
        if inner.shown.get() == shown {
            return;
        }
        inner.shown.set(shown);
        notify(self.hwnd, inner, if shown { NIM_ADD } else { NIM_DELETE });
    }

    pub fn shown(&self) -> bool {
        self.inner().shown.get()
    }

    /// The tooltip and menu in other words (the app's language).
    pub fn set_words(&self, words: Words) {
        let inner = self.inner();
        if *inner.words.borrow() == words {
            return;
        }
        *inner.words.borrow_mut() = words;
        if inner.shown.get() {
            notify(self.hwnd, inner, NIM_MODIFY);
        }
    }
}

impl Drop for Icon {
    fn drop(&mut self) {
        self.set_shown(false);
        // SAFETY: the window is ours (made in `new`); once it is destroyed
        // no message reaches `proc` with `inner`, which is then ours to free.
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            DestroyWindow(self.hwnd);
            let inner = Box::from_raw(self.inner);
            if !inner.icon.is_null() {
                DestroyIcon(inner.icon);
            }
        }
    }
}

/// `text` as a NUL-terminated UTF-16 field of `N`, cut to fit.
fn wide<const N: usize>(text: &str) -> [u16; N] {
    let mut out = [0u16; N];
    for (slot, unit) in out.iter_mut().take(N - 1).zip(text.encode_utf16()) {
        *slot = unit;
    }
    out
}

fn notify(hwnd: HWND, inner: &Inner, message: u32) {
    // SAFETY: a zeroed NOTIFYICONDATAW is valid (all integers, pointers and
    // arrays); its size and fields are set as `Shell_NotifyIconW` reads them.
    unsafe {
        let mut data: NOTIFYICONDATAW = std::mem::zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        if message != NIM_DELETE {
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = CALLBACK;
            data.hIcon = inner.icon;
            data.szTip = wide(&inner.words.borrow().tip);
        }
        Shell_NotifyIconW(message, &data);
    }
}

/// The menu at the pointer: what's chosen, if anything.
fn menu(hwnd: HWND, words: &Words) -> Option<Chosen> {
    // SAFETY: a menu made, shown and destroyed here, on the thread that owns
    // `hwnd`; its strings live through the call that copies them.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return None;
        }
        let show: Vec<u16> = words.show.encode_utf16().chain([0]).collect();
        let quit: Vec<u16> = words.quit.encode_utf16().chain([0]).collect();
        AppendMenuW(menu, MF_STRING, SHOW, show.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, QUIT, quit.as_ptr());
        SetMenuDefaultItem(menu, SHOW as u32, 0);
        let mut at = POINT { x: 0, y: 0 };
        GetCursorPos(&mut at);
        // (Without the foreground the menu stays open after a click away,
        // and the WM_NULL after it lets it close the second time: the
        // documented way, KB135788.)
        SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            at.x,
            at.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        match chosen as usize {
            SHOW => Some(Chosen::Show),
            QUIT => Some(Chosen::Quit),
            _ => None,
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: GWLP_USERDATA is `Icon::inner` while the window lives (0
    // before it's set and as it goes), only read on this thread.
    let inner = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Inner).as_ref() };
    if let Some(inner) = inner {
        if message == CALLBACK {
            match lparam as u32 {
                WM_LBUTTONUP | WM_LBUTTONDBLCLK => (inner.on)(Chosen::Show),
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    // (The words copied out: the menu's loop runs other
                    // windows' messages, which may change them.)
                    let words = inner.words.borrow().clone();
                    if let Some(chosen) = menu(hwnd, &words) {
                        (inner.on)(chosen);
                    }
                }
                _ => {}
            }
            return 0;
        }
        if message == inner.taskbar_created && inner.taskbar_created != 0 && inner.shown.get() {
            notify(hwnd, inner, NIM_ADD);
            return 0;
        }
    }
    // SAFETY: the default handling of a message to our own window.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
