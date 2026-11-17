//! Every plugin consent and review path, asked of the engine by each kind of
//! caller that can reach it: this device's own app, a peer the edge verified
//! (`fromDevice`), a peer the edge named no device for (the account's own
//! self-hosted edge, which doesn't stamp it), and an agent's token. The engine
//! under test has no window of its own, as a headless server hasn't: whatever
//! works here works on one.
//!
//! What should work works for all three device callers (a person decides on
//! another of their devices' sheet), and what must stay this device's app's
//! (signing with this device's key, templates, reading a file by path) or
//! never an agent's stays refused, for each.

use std::sync::Arc;

use base64::Engine as _;
use serde_json::{Value, json};
use zeron_rpc::Caller;

use super::identity_tests::{Fixture, call, fixture, item, known, package, package_file};
use super::keys::tests::FakeDirectory;
use super::*;

fn device_callers() -> [Caller; 3] {
    [Caller::Local, Caller::Peer { device: Some("desk".into()) }, Caller::Peer { device: None }]
}

fn name(caller: &Caller) -> String {
    format!("{caller:?}")
}

async fn as_<T>(caller: &Caller, f: impl std::future::Future<Output = T>) -> T {
    zeron_rpc::with_caller(caller.clone(), f).await
}

async fn review_as(f: &Fixture, caller: &Caller, id: &str) -> Result<Value, String> {
    let shown = item(&f.plugins, id).await;
    as_(caller, call(&f.plugins, methods::REVIEW_PLUGIN, json!({ "id": id, "hash": shown["hash"] }))).await.map(|r| r["review"].clone())
}

/// Reviewed, then allowed, as the consent sheet does it from `caller`.
async fn allow_as(f: &Fixture, caller: &Caller, id: &str) -> Result<Value, String> {
    let review = review_as(f, caller, id).await?;
    let shown = item(&f.plugins, id).await;
    as_(caller, call(&f.plugins, methods::DECIDE_PLUGIN, json!({ "id": id, "allow": true, "hash": shown["hash"], "review": review }))).await
}

async fn decline_as(f: &Fixture, caller: &Caller, id: &str) -> Result<Value, String> {
    let shown = item(&f.plugins, id).await;
    as_(caller, call(&f.plugins, methods::DECIDE_PLUGIN, json!({ "id": id, "allow": false, "hash": shown["hash"] }))).await
}

/// A verified author's package of `id` (asking `gh`), installed from a file
/// here, waiting.
async fn waiting(f: &Fixture, id: &str) -> (SigningKey, PathBuf) {
    let fake = Arc::new(FakeDirectory::default());
    let ada = known(f, &fake, Some("ada"), false);
    let path = package_file(f, &format!("{id}.zeron-plugin"), &package(&ada, id, "1.0.0", json!({ "permissions": ["commands"], "commands": ["gh"] })));
    let installed = call(&f.plugins, methods::INSTALL_PLUGIN, json!({ "path": path })).await.unwrap();
    assert_eq!(installed["plugin"]["status"], "pending", "{installed}");
    (ada, path)
}

async fn status_of(f: &Fixture, id: &str) -> String {
    item(&f.plugins, id).await["status"].as_str().unwrap().to_owned()
}

// ── ReviewPlugin / DecidePlugin allow ──────────────────────────────────────

/// (a) the app, (b) an unverified peer, (c) a verified peer: each reviews and
/// allows a waiting plugin; (d) an agent's token does neither.
#[tokio::test]
async fn review_and_allow_work_for_every_device_and_never_for_an_agent() {
    for caller in device_callers() {
        let f = fixture();
        waiting(&f, "focus").await;
        let allowed = allow_as(&f, &caller, "focus").await.unwrap_or_else(|e| panic!("{}: {e}", name(&caller)));
        assert_eq!(allowed["status"], "on", "{}: {allowed}", name(&caller));
    }
    let f = fixture();
    waiting(&f, "focus").await;
    let agent = Caller::Agent { chat: Some("c".into()) };
    assert!(review_as(&f, &agent, "focus").await.is_err(), "an agent minted a review");
    // A review the app minted is no good to an agent either.
    let review = review_as(&f, &Caller::Local, "focus").await.unwrap();
    let shown = item(&f.plugins, "focus").await;
    let refused = as_(&agent, call(&f.plugins, methods::DECIDE_PLUGIN, json!({ "id": "focus", "allow": true, "hash": shown["hash"], "review": review }))).await;
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(status_of(&f, "focus").await, "pending");
    // …and the transport keeps an agent's token away from all of it.
    assert!(!zeron_rpc::agent_allowed(methods::REVIEW_PLUGIN));
    assert!(!zeron_rpc::agent_allowed(methods::DECIDE_PLUGIN));
    assert!(!zeron_rpc::agent_allowed(methods::SIGN_PLUGIN));
    assert!(!zeron_rpc::agent_allowed(methods::STAGE_PLUGIN_PACKAGE));
    assert!(!zeron_rpc::agent_allowed(methods::UPDATE_PLUGIN));
    assert!(!zeron_rpc::agent_allowed(methods::REMOVE_PLUGIN));
    assert!(!zeron_rpc::agent_allowed(methods::SET_PLUGIN_OFF));
}

/// Without a review nobody allows, whoever they are; a review is one use.
#[tokio::test]
async fn an_allow_without_its_review_stays_refused_for_every_caller() {
    for caller in device_callers() {
        let f = fixture();
        waiting(&f, "focus").await;
        let shown = item(&f.plugins, "focus").await;
        let bare = as_(&caller, call(&f.plugins, methods::DECIDE_PLUGIN, json!({ "id": "focus", "allow": true, "hash": shown["hash"] }))).await;
        assert!(bare.as_ref().is_err_and(|e| e.starts_with("review:")), "{}: {bare:?}", name(&caller));
        let review = review_as(&f, &caller, "focus").await.unwrap();
        let ok = as_(&caller, call(&f.plugins, methods::DECIDE_PLUGIN, json!({ "id": "focus", "allow": true, "hash": shown["hash"], "review": review }))).await;
        assert!(ok.is_ok(), "{}: {ok:?}", name(&caller));
        let again = as_(&caller, call(&f.plugins, methods::DECIDE_PLUGIN, json!({ "id": "focus", "allow": true, "hash": shown["hash"], "review": review }))).await;
        assert!(again.is_err(), "{}: a review was used twice", name(&caller));
    }
}

// ── Decline, reopening a declined plugin (#1326) ───────────────────────────

/// "Not now" needs no review and works from each device; the plugin stays off
/// and asking, and its sheet opens again: a review and an allow then turn it
/// on, from any device.
#[tokio::test]
async fn a_declined_plugin_reopens_and_is_allowed_from_any_device() {
    for decliner in device_callers() {
        for reopener in device_callers() {
            let f = fixture();
            waiting(&f, "focus").await;
            decline_as(&f, &decliner, "focus").await.unwrap_or_else(|e| panic!("{} declined: {e}", name(&decliner)));
            let shown = item(&f.plugins, "focus").await;
            assert_eq!(shown["status"], "off", "{shown}");
            assert!(shown["ask"].as_array().is_some_and(|a| !a.is_empty()), "still asking, so its sheet reopens: {shown}");
            // Turning a declined plugin on without the sheet is refused.
            let off = as_(&reopener, call(&f.plugins, methods::SET_PLUGIN_OFF, json!({ "id": "focus", "off": false }))).await;
            assert!(off.is_err(), "{off:?}");
            let allowed = allow_as(&f, &reopener, "focus").await.unwrap_or_else(|e| panic!("{} reopened {}'s decline: {e}", name(&reopener), name(&decliner)));
            assert_eq!(allowed["status"], "on", "{allowed}");
        }
    }
}

// ── SignPlugin ("It's mine…") ──────────────────────────────────────────────

/// Signing is this device's key: the app's alone. A peer (verified or not)
/// and an agent are refused, and nothing is signed.
#[tokio::test]
async fn signing_is_the_apps_alone() {
    for caller in [Caller::Peer { device: Some("desk".into()) }, Caller::Peer { device: None }, Caller::Agent { chat: None }] {
        let f = fixture();
        let dir = f.plugins.plugins_dir().join("mine");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("init.luau"), "return 1").unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), signing::pretty(&json!({ "id": "mine", "name": "mine", "version": "1.0.0", "entry": "init.luau" }))).unwrap();
        f.plugins.rescan().await;
        // The review an app would have minted, even: it is bound to the app.
        let review = review_as(&f, &Caller::Local, "mine").await.unwrap();
        let refused = as_(&caller, call(&f.plugins, methods::SIGN_PLUGIN, json!({ "id": "mine", "review": review }))).await;
        assert!(refused.as_ref().is_err_and(|e| e.contains("consent window")), "{}: {refused:?}", name(&caller));
        assert!(!f.plugins.entry("mine").unwrap().item.own, "{}", name(&caller));
    }
    // The same through the app works (the one place "It's mine…" can).
    let f = fixture();
    let dir = f.plugins.plugins_dir().join("mine");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("init.luau"), "return 1").unwrap();
    std::fs::write(dir.join(MANIFEST_FILE), signing::pretty(&json!({ "id": "mine", "name": "mine", "version": "1.0.0", "entry": "init.luau" }))).unwrap();
    f.plugins.rescan().await;
    let review = review_as(&f, &Caller::Local, "mine").await.unwrap();
    call(&f.plugins, methods::SIGN_PLUGIN, json!({ "id": "mine", "review": review })).await.unwrap();
    assert!(f.plugins.entry("mine").unwrap().item.own);
}

// ── SetPluginOff, RemovePlugin ─────────────────────────────────────────────

#[tokio::test]
async fn turning_off_and_removing_work_from_every_device() {
    for caller in device_callers() {
        let f = fixture();
        waiting(&f, "focus").await;
        allow_as(&f, &Caller::Local, "focus").await.unwrap();
        let off = as_(&caller, call(&f.plugins, methods::SET_PLUGIN_OFF, json!({ "id": "focus", "off": true }))).await.unwrap_or_else(|e| panic!("{}: {e}", name(&caller)));
        assert_eq!(off["status"], "off", "{off}");
        let on = as_(&caller, call(&f.plugins, methods::SET_PLUGIN_OFF, json!({ "id": "focus", "off": false }))).await.unwrap();
        assert_eq!(on["status"], "on", "{on}");
        as_(&caller, call(&f.plugins, methods::REMOVE_PLUGIN, json!({ "id": "focus" }))).await.unwrap_or_else(|e| panic!("{}: {e}", name(&caller)));
        assert!(f.plugins.entry("focus").is_none(), "{}", name(&caller));
    }
}

// ── InstallPlugin: path / url / upload, StagePluginPackage ─────────────────

async fn stage(f: &Fixture, caller: &Caller, raw: &[u8]) -> Result<String, String> {
    let data = base64::engine::general_purpose::STANDARD.encode(raw);
    as_(caller, call(&f.plugins, methods::STAGE_PLUGIN_PACKAGE, json!({ "data": data }))).await.map(|r| r["upload"].as_str().unwrap().to_owned())
}

/// A file sent from another device installs waiting, whether or not the edge
/// named the device: staged, then installed by the same caller; and only by
/// them.
#[tokio::test]
async fn a_plugin_file_sent_from_any_device_installs_waiting() {
    for caller in device_callers() {
        let f = fixture();
        let fake = Arc::new(FakeDirectory::default());
        let ada = known(&f, &fake, Some("ada"), false);
        let raw = package(&ada, "focus", "1.0.0", json!({ "permissions": ["commands"], "commands": ["gh"] }));
        let upload = stage(&f, &caller, &raw).await.unwrap_or_else(|e| panic!("{} couldn't send a file: {e}", name(&caller)));
        // Another sort of caller can't take it (and asking used it up).
        let other = if caller == Caller::Local { Caller::Peer { device: None } } else { Caller::Local };
        let taken = as_(&other, call(&f.plugins, methods::INSTALL_PLUGIN, json!({ "upload": upload, "name": "focus.zeron-plugin" }))).await;
        assert!(taken.is_err(), "{} installed {}'s file", name(&other), name(&caller));
        let second = stage(&f, &caller, &raw).await.unwrap();
        let installed = as_(&caller, call(&f.plugins, methods::INSTALL_PLUGIN, json!({ "upload": second, "name": "C:\\Users\\a\\focus.zeron-plugin" }))).await.unwrap();
        assert_eq!(installed["plugin"]["status"], "pending", "{installed}");
        assert_eq!(installed["plugin"]["source"]["kind"], "file");
        assert!(installed["plugin"]["source"].get("path").is_none(), "never the other device's path");
        let f2 = fixture();
        let fake = Arc::new(FakeDirectory::default());
        let ada = known(&f2, &fake, Some("ada"), false);
        let raw = package(&ada, "focus", "1.0.0", json!({ "permissions": ["commands"], "commands": ["gh"] }));
        let upload = stage(&f2, &caller, &raw).await.unwrap();
        let installed = as_(&caller, call(&f2.plugins, methods::INSTALL_PLUGIN, json!({ "upload": upload, "name": "focus.zeron-plugin" }))).await.unwrap();
        assert_eq!((installed["plugin"]["status"].as_str(), installed["waiting"].as_bool()), (Some("pending"), Some(true)), "{installed}");
        // Installed from a file, it still waits for a review and an allow.
        assert_eq!(allow_as(&f2, &caller, "focus").await.unwrap()["status"], "on");
    }
}

/// An agent's token can't send a file; a file path on another device is
/// never read here (the transport's `peer_refused`), but a link is.
#[tokio::test]
async fn an_agent_sends_no_file_and_a_peer_names_no_path() {
    let f = fixture();
    let agent = Caller::Agent { chat: None };
    let staged = stage(&f, &agent, b"x").await;
    assert!(staged.as_ref().is_err_and(|e| e.contains("one of your devices")), "{staged:?}");
    let upload = as_(&agent, call(&f.plugins, methods::INSTALL_PLUGIN, json!({ "upload": "u1" }))).await;
    assert!(upload.is_err(), "{upload:?}");
    assert!(zeron_rpc::peer_refused(methods::INSTALL_PLUGIN, &json!({ "path": "/home/x/a.zeron-plugin" })));
    assert!(!zeron_rpc::peer_refused(methods::INSTALL_PLUGIN, &json!({ "url": "https://example.com/a" })));
    assert!(!zeron_rpc::peer_refused(methods::INSTALL_PLUGIN, &json!({ "upload": "u" })));
    assert!(!zeron_rpc::peer_refused(methods::STAGE_PLUGIN_PACKAGE, &json!({ "data": "" })));
    assert!(zeron_rpc::agent_allowed(methods::INSTALL_PLUGIN) && zeron_rpc::agent_allowed(methods::CHECK_PLUGIN_UPDATES));
    // The app's own engine forwards a file or an allow only for the app.
    assert!(zeron_rpc::app_forward_only(methods::STAGE_PLUGIN_PACKAGE, &json!({})));
    assert!(zeron_rpc::app_forward_only(methods::INSTALL_PLUGIN, &json!({ "upload": "u" })));
    assert!(!zeron_rpc::app_forward_only(methods::INSTALL_PLUGIN, &json!({ "url": "https://example.com/a" })));
}

// ── Reinstall of a changed plugin, updates with a permission upgrade ───────

/// Changed since it was allowed: never allowed as it is; installed again from
/// where it came from, from any device, it waits and is then allowed.
#[tokio::test]
async fn a_changed_plugin_is_installed_again_and_allowed_from_any_device() {
    for caller in device_callers() {
        let f = fixture();
        waiting(&f, "focus").await;
        allow_as(&f, &Caller::Local, "focus").await.unwrap();
        std::fs::write(f.plugins.plugins_dir().join("focus/init.luau"), "changed()").unwrap();
        assert_eq!(status_of(&f, "focus").await, "tampered");
        // Never allowed as it is: not by any device's sheet.
        let refused = allow_as(&f, &caller, "focus").await;
        assert!(refused.as_ref().is_err_and(|e| e.contains("changed after you allowed it")), "{}: {refused:?}", name(&caller));
        let again = as_(&caller, call(&f.plugins, methods::INSTALL_PLUGIN, json!({ "reinstall": "focus" }))).await.unwrap_or_else(|e| panic!("{}: {e}", name(&caller)));
        assert_eq!(again["plugin"]["status"], "pending", "{again}");
        assert_eq!(allow_as(&f, &caller, "focus").await.unwrap()["status"], "on", "{}", name(&caller));
    }
}

/// An update that asks for more waits for a review, from any device: checked,
/// updated, then allowed again by a sheet on another device.
#[tokio::test]
async fn an_update_with_a_permission_upgrade_asks_again_on_any_device() {
    for caller in device_callers() {
        let f = fixture();
        let (ada, path) = waiting(&f, "focus").await;
        allow_as(&f, &caller, "focus").await.unwrap();
        std::fs::write(&path, package(&ada, "focus", "1.1.0", json!({ "permissions": ["commands", "native"], "commands": ["gh"] }))).unwrap();
        let checked = as_(&caller, call(&f.plugins, methods::CHECK_PLUGIN_UPDATES, json!({}))).await.unwrap();
        assert_eq!(checked["updates"].as_array().map(Vec::len), Some(1), "{}: {checked}", name(&caller));
        let updated = as_(&caller, call(&f.plugins, methods::UPDATE_PLUGIN, json!({ "id": "focus" }))).await.unwrap_or_else(|e| panic!("{}: {e}", name(&caller)));
        assert_eq!((updated["waiting"].as_bool(), updated["plugin"]["status"].as_str()), (Some(true), Some("pending")), "{updated}");
        assert_eq!(updated["plugin"]["ask"], json!(["native"]), "{updated}");
        assert_eq!(allow_as(&f, &caller, "focus").await.unwrap()["status"], "on", "{}", name(&caller));
        // An agent checks for updates (it only reads), but doesn't take them
        // through its own scoped token; the engine itself doesn't refuse it.
    }
}

// ── What stays this device's ───────────────────────────────────────────────

#[tokio::test]
async fn templates_and_the_device_key_stay_this_devices() {
    for caller in [Caller::Peer { device: Some("desk".into()) }, Caller::Peer { device: None }, Caller::Agent { chat: None }] {
        let f = fixture();
        let template = as_(&caller, call(&f.plugins, methods::REVIEW_TEMPLATE, json!({ "pending": "midnight", "hash": "h" }))).await;
        assert!(template.as_ref().is_err_and(|e| e.contains("consent window")), "{}: {template:?}", name(&caller));
    }
    assert!(zeron_rpc::peer_refused(methods::SIGN_PLUGIN, &json!({})));
    assert!(zeron_rpc::peer_refused(methods::REVIEW_TEMPLATE, &json!({})));
    assert!(zeron_rpc::peer_refused(methods::DECIDE_TEMPLATE, &json!({})));
    assert!(zeron_rpc::peer_refused(methods::PLUGIN_SIGNING_KEY, &json!({})));
}
