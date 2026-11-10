//! A press on the composer pill's hairline border (its 1-point edge, under
//! the draft's line) gives the draft the keys, as one on its padding does
//! (ds/composer's press layer sits inside the border, so the pill takes the
//! press too). Stock zgx's Luau `app:click` presses a target's middle only,
//! so it's pressed here at a point. On the shell's fixture, a session.

use zgx::input::{InputEvent, Modifiers, PointerButton};
use zgx_script::{ScriptHost, testing::TestApp};

fn mount() -> TestApp {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/shell_fixture.luau");
    let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, path));
    app.ui.scene.borrow_mut().resize(1240., 820.);
    app.frames(30);
    assert_eq!(app.error(), None, "the shell failed");
    app
}

fn settle(app: &TestApp) {
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(15));
        app.frames(2);
    }
}

/// Whether the draft has the keys (the tree marks what's focused).
fn draft_has_keys(app: &mut TestApp) -> bool {
    app.tree().lines().find(|l| l.contains(" focused")).is_some_and(|l| l.contains("textbox \"Message\""))
}

fn press_at(app: &mut TestApp, x: f32, y: f32) {
    let (button, m) = (PointerButton::Primary, Modifiers::default());
    app.ui.input.forget_clicks();
    app.ui.dispatch_with_modifiers(InputEvent::PointerMove { x, y }, m);
    app.ui.dispatch_with_modifiers(InputEvent::PointerDown { x, y, button }, m);
    app.ui.dispatch_with_modifiers(InputEvent::PointerUp { x, y, button }, m);
    app.ui.input.forget_clicks();
    app.settle();
    settle(app);
}

#[test]
fn a_press_on_the_pills_foot_hairline_gives_the_draft_the_keys() {
    let mut app = mount();
    app.click("#shell-sidebar >> role:button \"Fix the build\"");
    settle(&app);
    settle(&app);
    let pill = app.bounds("#composer-pill");
    // The pill's last point: the 1-point border under its padding.
    let x = pill.x + pill.width / 2.;
    for up in [6., 3., 0.5] {
        // (The keys away from the draft first: a click in it, then Escape.)
        app.click("#chat-composer >> role:textbox");
        app.key(zgx::input::Key::Escape);
        settle(&app);
        assert!(!draft_has_keys(&mut app), "the keys should be away from the draft");
        press_at(&mut app, x, pill.y + pill.height - up);
        assert!(draft_has_keys(&mut app), "a press {up} above the pill's foot should give the draft the keys");
    }
}
