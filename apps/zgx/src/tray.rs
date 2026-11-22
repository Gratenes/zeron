//! Closing Zeron's window on Windows keeps Zeron in the notification area
//! (Settings → General, "Keep Zeron in the notification area when you close
//! the window", on by default), as Windows apps that work in the background
//! do: the window hides, its icon by the clock shows it again (a click, or
//! its menu's Show Zeron), and its menu's Quit Zeron quits as the app's own
//! Quit does (`WindowFactory::quit`: the quit question, an update to
//! install, src/main.rs after `run`). Launching Zeron again while it's
//! hidden shows it too (`reopen`: zgx's one instance a session hands the
//! running app `ApplicationEvent::Reopen`).
//!
//! Off, or anywhere but Windows, nothing changes: closing the window quits.
//!
//! The setting is the scripts' (`settings_close_to_tray`,
//! ui/screens/settings/prefs.luau), told here as `app.close_to_tray`; the
//! icon's words are theirs too (`app.tray_words`, ui/screens/language.luau),
//! in the language the app shows. The icon itself: src/tray/win32.rs.

use zgx_desktop::WindowContext;
use zgx_script::ScriptHost;

#[cfg(windows)]
mod win32;

/// The main window's tray: its icon while the setting is on, its window
/// hidden rather than closed. The main window's alone (src/main.rs
/// `open_main`).
#[cfg(windows)]
pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    use std::{cell::RefCell, rc::Rc};
    use zgx_script::Value;

    let (window, windows) = (cx.window.clone(), cx.windows.clone());
    let shows = window.clone();
    let Some(icon) = win32::Icon::new(move |chosen| match chosen {
        win32::Chosen::Show => show(&shows),
        win32::Chosen::Quit => windows.quit(),
    }) else {
        eprintln!("zeron-zgx: no notification area icon: closing the window quits");
        return;
    };
    let icon = Rc::new(icon);
    MAIN.with(|main| *main.borrow_mut() = Some(window.clone()));
    // (What outlives the window holds the icon weakly: it goes with it.)
    let words = Rc::downgrade(&icon);
    host.action("app.tray_words", move |args| {
        let Some(icon) = words.upgrade() else { return };
        let defaults = win32::Words::default();
        let field = |key: &str| args.field(key).map(crate::auth::text).filter(|text| !text.is_empty());
        icon.set_words(win32::Words {
            tip: field("tip").unwrap_or(defaults.tip),
            show: field("show").unwrap_or(defaults.show),
            quit: field("quit").unwrap_or(defaults.quit),
        });
    });
    let on = host.shared("app.close_to_tray", Value::Bool(true)).signal();
    let follows = Rc::downgrade(&icon);
    let effect = cx.ui.runtime.effect(move || {
        let on = on.get().truthy();
        if let Some(icon) = follows.upgrade() {
            icon.set_shown(on);
        }
    });
    let hides = Rc::downgrade(&icon);
    let hidden = window.clone();
    cx.on_close_requested(move || {
        // In the notification area: hidden, to come back from there.
        if hides.upgrade().is_some_and(|icon| icon.shown()) {
            hidden.set_visible(false);
            return false;
        }
        true
    });
    let keep = RefCell::new(Some((icon, effect)));
    cx.on_closed(move || {
        keep.borrow_mut().take();
        MAIN.with(|main| main.borrow_mut().take());
    });
}

#[cfg(not(windows))]
pub fn install(_cx: &mut WindowContext, _host: &ScriptHost) {}

#[cfg(windows)]
thread_local! {
    /// The main window, while it's open (hidden or not).
    static MAIN: std::cell::RefCell<Option<zgx_desktop::WindowHandle>> = const { std::cell::RefCell::new(None) };
}

#[cfg(windows)]
fn show(window: &zgx_desktop::WindowHandle) {
    window.set_visible(true);
    window.request_focus();
}

/// Zeron launched again (`ApplicationEvent::Reopen`): its main window shown
/// and in front, hidden in the notification area or not. Elsewhere the
/// system does that itself.
pub fn reopen() {
    #[cfg(windows)]
    MAIN.with(|main| {
        if let Some(window) = &*main.borrow() {
            show(window);
        }
    });
}
