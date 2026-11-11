//! The live Claude login against a identity file other CLIs write to: a chat's
//! CLI stamping its own account over it must never read as a switch.

use std::path::Path;

use super::super::*;
use super::refresh_token;

fn accounts(root: &Path) -> AgentAccounts {
    let endpoints = ProbeEndpoints {
        claude_token: "http://127.0.0.1:9/never".to_owned(),
        ..Default::default()
    };
    AgentAccounts::with_endpoints(
        AgentAccountsConfig::isolated(root),
        endpoints,
        Default::default(),
    )
}

/// The CLI's identity file alone: what a sign-in writes, and what a chat's
/// CLI writes too.
fn write_identity(root: &Path, uuid: &str) {
    let config = AgentAccountsConfig::isolated(root);
    std::fs::write(
        &config.claude_config_file,
        serde_json::json!({ "oauthAccount": {
            "accountUuid": uuid, "emailAddress": format!("{uuid}@example.com"),
        } })
        .to_string(),
    )
    .unwrap();
}

fn write_secret(root: &Path, access: &str, refresh: &str) {
    let config = AgentAccountsConfig::isolated(root);
    std::fs::create_dir_all(&config.claude_config_dir).unwrap();
    std::fs::write(
        config.claude_creds_file(),
        serde_json::json!({ "claudeAiOauth": {
            "accessToken": access, "refreshToken": refresh, "expiresAt": now_ms() + 3_600_000,
        } })
        .to_string(),
    )
    .unwrap();
}

/// A sign-in: the identity and the secret change together.
fn sign_in(root: &Path, uuid: &str, access: &str, refresh: &str) {
    write_identity(root, uuid);
    write_secret(root, access, refresh);
}

fn claude_slot(uuid: &str, access: &str, refresh: &str) -> Slot {
    Slot {
        id: slot_id_for(HarnessId::ClaudeCode, uuid),
        harness: HarnessId::ClaudeCode,
        account_key: uuid.into(),
        profile: SlotProfile {
            email: format!("{uuid}@example.com"),
            display_name: None,
            organization: None,
            plan: None,
            auth_kind: AgentAuthKind::Oauth,
        },
        credentials: serde_json::json!({
            "claudeAiOauth": { "accessToken": access, "refreshToken": refresh, "expiresAt": now_ms() + 60_000 },
        }),
        claude_config: Some(serde_json::json!({ "oauthAccount": {
            "accountUuid": uuid, "emailAddress": format!("{uuid}@example.com"),
        } })),
        saved_at: now_ms(),
        created_at: None,
        store_key: None,
    }
}

fn refresh_of(accounts: &AgentAccounts, uuid: &str) -> Option<String> {
    accounts
        .read_slot(
            HarnessId::ClaudeCode,
            &slot_id_for(HarnessId::ClaudeCode, uuid),
        )
        .and_then(|slot| refresh_token(&slot.credentials))
}

async fn live_key(accounts: &AgentAccounts) -> Option<String> {
    accounts.live_account_key(HarnessId::ClaudeCode, None).await
}

#[tokio::test]
async fn a_chats_cli_stamping_its_account_under_an_unchanged_login_switches_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let accounts = accounts(tmp.path());
    let b = claude_slot("b", "at-b", "rt-b");
    accounts.write_slot(&b).unwrap();
    accounts.ensure_login_dir(&b).unwrap();
    sign_in(tmp.path(), "a", "at-a", "rt-a");
    accounts.list(false).await.unwrap();
    assert_eq!(refresh_of(&accounts, "a").as_deref(), Some("rt-a"));

    // A chat on account b: its CLI writes b's identity, the secret stays a's.
    write_identity(tmp.path(), "b");
    accounts.list(false).await.unwrap();
    accounts.list(false).await.unwrap();

    // b keeps its own login and its folder (chats run on it); a is still live.
    assert_eq!(refresh_of(&accounts, "b").as_deref(), Some("rt-b"));
    assert!(accounts.login_dir(&b.id).exists());
    assert_eq!(live_key(&accounts).await.as_deref(), Some("a"));
}

#[tokio::test]
async fn a_real_sign_in_changes_the_tokens_with_the_identity_and_is_taken_as_one() {
    let tmp = tempfile::tempdir().unwrap();
    let accounts = accounts(tmp.path());
    sign_in(tmp.path(), "a", "at-a", "rt-a");
    accounts.list(false).await.unwrap();
    accounts
        .write_slot(&claude_slot("b", "at-b", "rt-b"))
        .unwrap();

    sign_in(tmp.path(), "b", "at-b2", "rt-b2");
    accounts.list(false).await.unwrap();

    assert_eq!(refresh_of(&accounts, "b").as_deref(), Some("rt-b2"));
    assert_eq!(refresh_of(&accounts, "a").as_deref(), Some("rt-a"));
    assert_eq!(live_key(&accounts).await.as_deref(), Some("b"));
}

#[tokio::test]
async fn after_a_restart_a_login_saved_under_one_account_is_that_accounts_whatever_the_file_says() {
    let tmp = tempfile::tempdir().unwrap();
    let before = accounts(tmp.path());
    before
        .write_slot(&claude_slot("a", "at-a", "rt-a"))
        .unwrap();
    // The engine starts to a file that already names another account.
    sign_in(tmp.path(), "b", "at-a", "rt-a");
    let restarted = accounts(tmp.path());
    restarted.list(false).await.unwrap();

    assert!(
        restarted
            .read_slot(
                HarnessId::ClaudeCode,
                &slot_id_for(HarnessId::ClaudeCode, "b")
            )
            .is_none(),
        "the login was saved under the account that merely stamped the file"
    );
    assert_eq!(live_key(&restarted).await.as_deref(), Some("a"));
}

#[tokio::test]
async fn a_login_two_saved_accounts_hold_is_saved_into_neither_and_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let accounts = accounts(tmp.path());
    // Already overwritten: a and b hold one login (an old incident).
    accounts
        .write_slot(&claude_slot("a", "at-x", "rt-x"))
        .unwrap();
    accounts
        .write_slot(&claude_slot("b", "at-x", "rt-x"))
        .unwrap();
    // The live login is that same one, refreshed: a newer pair for it can't
    // say which of the two it belongs to, so it goes to neither.
    sign_in(tmp.path(), "a", "at-x", "rt-x");
    let snapshot = accounts.list(false).await.unwrap();

    let warning = snapshot
        .warnings
        .iter()
        .find(|w| w.message.contains("hold the same login"))
        .expect("the shared login is reported");
    assert!(warning.message.contains("a@example.com") && warning.message.contains("b@example.com"));

    // And a live login that another account holds is never put into this one.
    let c = accounts.claude_live_may_be_saved(
        &slot_id_for(HarnessId::ClaudeCode, "c"),
        &claude_slot("c", "at-x", "rt-x").credentials,
    );
    assert!(!c);
    assert!(accounts.claude_live_may_be_saved(
        &slot_id_for(HarnessId::ClaudeCode, "c"),
        &claude_slot("c", "at-c", "rt-c").credentials,
    ));
}

#[tokio::test]
async fn the_live_logins_folder_goes_at_once_when_the_live_token_is_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let accounts = accounts(tmp.path());
    let b = claude_slot("b", "at-b", "rt-b");
    accounts.write_slot(&b).unwrap();
    accounts.ensure_login_dir(&b).unwrap();
    // b becomes live holding the very token its folder holds.
    sign_in(tmp.path(), "b", "at-b", "rt-b");
    accounts.list(false).await.unwrap();
    assert!(!accounts.login_dir(&b.id).exists());
}

#[tokio::test]
async fn a_new_login_for_a_saved_account_goes_into_its_slot_and_its_folder_goes() {
    let tmp = tempfile::tempdir().unwrap();
    let accounts = accounts(tmp.path());
    let b = claude_slot("b", "at-b", "rt-b");
    accounts.write_slot(&b).unwrap();
    accounts.ensure_login_dir(&b).unwrap();
    // A terminal signs in as b: a new login (new tokens) for the account.
    sign_in(tmp.path(), "b", "at-b-new", "rt-b-new");
    accounts.list(false).await.unwrap();
    assert_eq!(refresh_of(&accounts, "b").as_deref(), Some("rt-b-new"));
    assert!(!accounts.login_dir(&b.id).exists());
}
