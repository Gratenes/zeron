//! Windows: a shown, active window of ours gets the keyboard focus if it has
//! none.
//!
//! zgx reopens a saved maximized frame by maximizing the window while it is
//! still hidden; winit then shows it (SW_MAXIMIZE activates and focuses) and
//! hides it again, which leaves this thread's focus NULL, and the real show
//! finds the window already active, so nothing focuses it. Keys then come as
//! WM_SYSKEYDOWN/WM_SYSCHAR and Windows dings on each one until the window is
//! reactivated. Remove once zgx's winit doesn't maximize hidden windows
//! (window_state.rs `apply_diff`).

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetActiveWindow, GetFocus, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND, GA_ROOT, GetAncestor, IsWindowVisible, OBJID_WINDOW,
    WINEVENT_OUTOFCONTEXT,
};

/// Focus `hwnd` if it is this thread's active, visible top-level window and
/// nothing on this thread has the focus.
fn refocus(hwnd: HWND) {
    // SAFETY: plain queries of this thread's input state and of a window; the
    // focus is only given to this thread's own active window.
    unsafe {
        if hwnd.is_null() || GetAncestor(hwnd, GA_ROOT) != hwnd {
            return;
        }
        if GetActiveWindow() == hwnd && GetFocus().is_null() && IsWindowVisible(hwnd) != 0 {
            SetFocus(hwnd);
        }
    }
}

unsafe extern "system" fn heard(_: HWINEVENTHOOK, _: u32, hwnd: HWND, object: i32, _: i32, _: u32, _: u32) {
    if object == OBJID_WINDOW {
        refocus(hwnd);
    }
}

/// Watch this process's windows showing and coming to the front. Call on the
/// UI thread: the hooks are out of context, so `heard` runs on this thread's
/// message loop, where `GetActiveWindow`/`SetFocus` see the windows' own state.
pub(crate) fn install() {
    let pid = std::process::id();
    // SAFETY: out-of-context hooks for this process alone; they last as the app does.
    unsafe {
        SetWinEventHook(EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW, std::ptr::null_mut(), Some(heard), pid, 0, WINEVENT_OUTOFCONTEXT);
        SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, std::ptr::null_mut(), Some(heard), pid, 0, WINEVENT_OUTOFCONTEXT);
        // Installed after the window showed: put it right now as well.
        refocus(GetActiveWindow());
    }
}
