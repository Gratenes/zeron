//! The app icon's badge (ui/screens/shell/badge, mounted on
//! ui/badge_fixture.luau, the shell with the engine's chats made up): how many
//! main chats wait for the person, on the taskbar button, the Dock tile and
//! where a Linux dock shows a count. These read it from the test desktop, as
//! `ui.badge` tells it (the platforms' own drawing is zgx's).
//!
//! `ZERON_ZGX_TEST_UI` points these at a scratch copy of ui/, to show them
//! failing on old code (as tests/reload.rs).

use std::path::PathBuf;

use zgx_script::{ScriptHost, testing::TestApp};

fn mount() -> TestApp {
    let ui = std::env::var_os("ZERON_ZGX_TEST_UI")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"), PathBuf::from);
    let path = ui.join("badge_fixture.luau");
    let app = TestApp::with_desktop(|runtime| ScriptHost::new(runtime, path));
    app.ui.scene.borrow_mut().resize(1200., 800.);
    app.frames(30);
    assert_eq!(app.error(), None);
    app
}

fn badge(app: &TestApp) -> Option<String> {
    app.desktop.as_ref().expect("a test desktop").badge()
}

fn run(app: &mut TestApp, command: &str) {
    assert!(app.host.run_command(command), "no command {command}");
    app.frames(10);
    assert_eq!(app.error(), None);
}

#[test]
fn the_count_is_the_main_chats_that_ask_finished_failed_or_woke() {
    let mut app = mount();
    // (Asks, finished, failed, woke: four. Not the one working, the one
    // waiting on work, the idle one, the archived one that finished, or the
    // side chat and the subagent under a chat that asks.)
    run(&mut app, "badge.mixed");
    assert_eq!(badge(&app).as_deref(), Some("4"));
}

#[test]
fn seeing_a_chat_takes_it_off_and_a_new_ask_puts_one_on() {
    let mut app = mount();
    run(&mut app, "badge.mixed");
    assert_eq!(badge(&app).as_deref(), Some("4"));
    run(&mut app, "badge.finished_seen");
    assert_eq!(badge(&app).as_deref(), Some("3"));
    run(&mut app, "badge.another_asks");
    assert_eq!(badge(&app).as_deref(), Some("4"));
}

#[test]
fn with_none_waiting_the_badge_is_taken_off() {
    let mut app = mount();
    run(&mut app, "badge.mixed");
    assert_eq!(badge(&app).as_deref(), Some("4"));
    run(&mut app, "badge.none");
    assert_eq!(badge(&app), None);
}

#[test]
fn an_engine_offline_leaves_the_badge_as_it_was() {
    let mut app = mount();
    run(&mut app, "badge.mixed");
    run(&mut app, "badge.offline");
    assert_eq!(badge(&app).as_deref(), Some("4"));
}

#[test]
fn turning_the_setting_off_takes_the_badge_down_and_on_puts_it_back() {
    let mut app = mount();
    run(&mut app, "badge.mixed");
    assert_eq!(badge(&app).as_deref(), Some("4"));
    run(&mut app, "badge.pref_off");
    assert_eq!(badge(&app), None);
    run(&mut app, "badge.pref_on");
    assert_eq!(badge(&app).as_deref(), Some("4"));
}

#[test]
fn needs_me_counts_only_the_chats_that_ask() {
    let mut app = mount();
    run(&mut app, "badge.mixed");
    run(&mut app, "badge.pref_needs_me");
    assert_eq!(badge(&app).as_deref(), Some("1"));
    run(&mut app, "badge.pref_all");
    assert_eq!(badge(&app).as_deref(), Some("4"));
}
