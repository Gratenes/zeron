//! Login folders against real directories: the first-start migration, what
//! the folder's credentials override, and a refresh that never spends a
//! refresh token twice (the CLI's lock, and a refresh the CLI already made).

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::super::*;
use super::*;

fn config(root: &Path) -> AgentAccountsConfig {
    AgentAccountsConfig {
        data_dir: root.join("data"),
        claude_config_dir: root.join("claude"),
        claude_config_file: root.join("claude.json"),
        codex_home: root.join("codex"),
        cursor_sdk_auth_file: root.join("cursor-sdk").join("auth.json"),
        claude_keychain_service: None,
        antigravity_home: Some(root.join("gemini")),
        antigravity_keychain: false,
        ..AgentAccountsConfig::isolated(root)
    }
}

fn write_live_claude(root: &Path, uuid: &str, email: &str) {
    let config = config(root);
    std::fs::create_dir_all(&config.claude_config_dir).unwrap();
    std::fs::write(
        &config.claude_config_file,
        serde_json::json!({ "oauthAccount": { "accountUuid": uuid, "emailAddress": email } })
            .to_string(),
    )
    .unwrap();
    std::fs::write(
        config.claude_creds_file(),
        serde_json::json!({ "claudeAiOauth": { "accessToken": "live-token", "refreshToken": "live-rt", "expiresAt": now_ms() + 3_600_000 } })
            .to_string(),
    )
    .unwrap();
}

fn claude_slot(uuid: &str, access: &str, refresh: &str, expires_at: i64) -> Slot {
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
            "claudeAiOauth": { "accessToken": access, "refreshToken": refresh, "expiresAt": expires_at },
            "mcpOAuth": { "srv": { "accessToken": "mcp" } },
        }),
        claude_config: None,
        saved_at: now_ms(),
        created_at: None,
        store_key: None,
    }
}

/// A token endpoint answering every refresh with a new pair, counting them.
async fn token_server() -> (String, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = vec![0u8; 16 * 1024];
            let read = socket.read(&mut request).await.unwrap_or(0);
            // Only refreshes count: stray probes of a recycled port (a HEAD
            // from another test's helper) are not.
            if !request[..read].starts_with(b"POST") {
                continue;
            }
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            let body = format!(
                r#"{{"access_token":"at-new-{n}","refresh_token":"rt-new-{n}","expires_in":28800}}"#
            );
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(reply.as_bytes()).await;
        }
    });
    (url, hits)
}

fn accounts(root: &Path, token_url: &str) -> AgentAccounts {
    let endpoints = ProbeEndpoints {
        claude_token: token_url.to_owned(),
        ..Default::default()
    };
    AgentAccounts::with_endpoints(config(root), endpoints, Default::default())
}

fn oauth_in(file: &Path) -> serde_json::Value {
    let creds: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    creds["claudeAiOauth"].clone()
}

/// The CLI refreshing inside a chat: it saves the new pair into the folder.
fn cli_refreshes(accounts: &AgentAccounts, slot: &Slot, access: &str, refresh: &str) {
    let file = accounts.login_file(&slot.id);
    let mut creds: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    creds["claudeAiOauth"] = serde_json::json!({
        "accessToken": access, "refreshToken": refresh, "expiresAt": now_ms() + 28_800_000,
    });
    std::fs::write(&file, creds.to_string()).unwrap();
}

#[test]
fn the_first_start_moves_saved_logins_into_folders_without_touching_a_refresh_token() {
    let tmp = tempfile::tempdir().unwrap();
    let accounts = accounts(tmp.path(), "http://127.0.0.1:9/never");
    let live = claude_slot("live", "live-token", "live-rt", now_ms() + 3_600_000);
    let b = claude_slot("b", "at-b", "rt-b", now_ms() + 3_600_000);
    let spent = claude_slot("c", "at-c", "rt-c", now_ms() - 60_000);
    for slot in [&live, &b, &spent] {
        accounts.write_slot(slot).unwrap();
    }
    accounts.migrate_login_dirs(Some("live"));
    assert!(
        !accounts.login_dir(&live.id).exists(),
        "the live login never has a folder"
    );
    for (slot, refresh) in [(&b, "rt-b"), (&spent, "rt-c")] {
        let held = oauth_in(&accounts.login_file(&slot.id));
        assert_eq!(
            held["refreshToken"], refresh,
            "the folder holds the slot's own pair"
        );
    }
    // Nothing was spent: no request went anywhere, and the slots read as saved.
    assert_eq!(
        accounts
            .read_slot(HarnessId::ClaudeCode, &b.id)
            .unwrap()
            .credentials["claudeAiOauth"]["refreshToken"],
        "rt-b"
    );
    // Done once: a folder removed later isn't made again by the migration,
    // only by the account's next run.
    std::fs::remove_dir_all(accounts.login_dir(&b.id)).unwrap();
    accounts.migrate_login_dirs(Some("live"));
    assert!(!accounts.login_dir(&b.id).exists());
    // And the folder is private.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&accounts.login_dir(&spent.id)), 0o700);
        assert_eq!(mode(&accounts.login_file(&spent.id)), 0o600);
    }
}

#[test]
fn what_the_folder_holds_is_what_every_read_of_the_slot_sees() {
    let tmp = tempfile::tempdir().unwrap();
    let accounts = accounts(tmp.path(), "http://127.0.0.1:9/never");
    let b = claude_slot("b", "at-b", "rt-b", now_ms() + 60_000);
    accounts.write_slot(&b).unwrap();
    accounts.ensure_login_dir(&b).unwrap();
    cli_refreshes(&accounts, &b, "at-b2", "rt-b2");
    for slot in [
        accounts.read_slot(HarnessId::ClaudeCode, &b.id).unwrap(),
        accounts.read_slots(HarnessId::ClaudeCode).remove(0),
    ] {
        assert_eq!(slot.credentials["claudeAiOauth"]["refreshToken"], "rt-b2");
        assert_eq!(
            slot.credentials["mcpOAuth"]["srv"]["accessToken"], "mcp",
            "siblings stay"
        );
    }
    // The slot file itself still has what it was saved with.
    let raw: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            accounts
                .slots_dir(HarnessId::ClaudeCode)
                .unwrap()
                .join(format!("{}.json", b.id)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(raw["credentials"]["claudeAiOauth"]["refreshToken"], "rt-b");
    // A folder the CLI wiped (invalid_grant) overrides nothing.
    std::fs::write(accounts.login_file(&b.id), "{}").unwrap();
    assert_eq!(
        accounts
            .read_slot(HarnessId::ClaudeCode, &b.id)
            .unwrap()
            .credentials["claudeAiOauth"]["refreshToken"],
        "rt-b"
    );
}

#[test]
fn a_folder_is_made_once_and_never_over_what_the_cli_saved_in_it() {
    let tmp = tempfile::tempdir().unwrap();
    let accounts = accounts(tmp.path(), "http://127.0.0.1:9/never");
    let b = claude_slot("b", "at-b", "rt-b", now_ms() + 60_000);
    accounts.write_slot(&b).unwrap();
    accounts.ensure_login_dir(&b).unwrap();
    cli_refreshes(&accounts, &b, "at-b2", "rt-b2");
    accounts.ensure_login_dir(&b).unwrap();
    assert_eq!(
        oauth_in(&accounts.login_file(&b.id))["refreshToken"],
        "rt-b2"
    );
    // A slot written from an older read never replaces the CLI's newer pair…
    accounts.write_slot(&b).unwrap();
    assert_eq!(
        oauth_in(&accounts.login_file(&b.id))["refreshToken"],
        "rt-b2"
    );
    // …but a new sign-in of the account (a later expiry) does.
    let mut again = claude_slot("b", "at-b3", "rt-b3", now_ms() + 90_000_000);
    again.credentials["claudeAiOauth"]["expiresAt"] = serde_json::json!(now_ms() + 90_000_000);
    accounts.write_slot(&again).unwrap();
    assert_eq!(
        oauth_in(&accounts.login_file(&b.id))["refreshToken"],
        "rt-b3"
    );
}

#[tokio::test]
async fn a_refresh_the_cli_already_made_is_adopted_not_spent_again() {
    let tmp = tempfile::tempdir().unwrap();
    let (url, hits) = token_server().await;
    let accounts = accounts(tmp.path(), &url);
    write_live_claude(tmp.path(), "live", "live@example.com");
    let b = claude_slot("b", "at-b", "rt-b", now_ms() - 1000);
    accounts.write_slot(&b).unwrap();
    accounts.ensure_login_dir(&b).unwrap();
    // The caller read `b` before the chat's CLI refreshed.
    cli_refreshes(&accounts, &b, "at-from-cli", "rt-from-cli");
    let got = accounts.refresh_claude_slot(&b).await.unwrap();
    assert_eq!(got.as_deref(), Some("at-from-cli"));
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "the refresh token wasn't posted"
    );
}

#[tokio::test]
async fn zeron_refreshes_only_holding_the_clis_lock_and_writes_the_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let (url, hits) = token_server().await;
    let accounts = accounts(tmp.path(), &url);
    write_live_claude(tmp.path(), "live", "live@example.com");
    let b = claude_slot("b", "at-b", "rt-b", now_ms() - 1000);
    accounts.write_slot(&b).unwrap();
    let dir = accounts.ensure_login_dir(&b).unwrap();
    // A CLI mid-refresh holds the lock: zeron doesn't refresh over it.
    let held =
        zeron_harness::claude::refresh_lock::RefreshLock::acquire(&dir, Duration::from_secs(2))
            .await
            .unwrap();
    assert_eq!(accounts.refresh_claude_slot(&b).await.unwrap(), None);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    drop(held);
    // Free, it refreshes: the new pair is in the folder (and so in every read).
    assert_eq!(
        accounts.refresh_claude_slot(&b).await.unwrap().as_deref(),
        Some("at-new-1")
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        oauth_in(&accounts.login_file(&b.id))["refreshToken"],
        "rt-new-1"
    );
    assert_eq!(
        accounts
            .read_slot(HarnessId::ClaudeCode, &b.id)
            .unwrap()
            .credentials["claudeAiOauth"]["accessToken"],
        "at-new-1"
    );
    assert_eq!(
        oauth_in(&accounts.login_file(&b.id))["expiresAt"]
            .as_i64()
            .unwrap()
            > now_ms() + 3_600_000,
        true
    );
    // The lock is released afterwards.
    assert!(!zeron_harness::claude::refresh_lock::refresh_in_flight(
        &dir
    ));
}

#[tokio::test]
async fn two_refreshes_of_one_login_post_its_refresh_token_once() {
    let tmp = tempfile::tempdir().unwrap();
    let (url, hits) = token_server().await;
    let accounts = accounts(tmp.path(), &url);
    write_live_claude(tmp.path(), "live", "live@example.com");
    let b = claude_slot("b", "at-b", "rt-b", now_ms() - 1000);
    accounts.write_slot(&b).unwrap();
    let (x, y) = tokio::join!(
        accounts.refresh_claude_slot(&b),
        accounts.refresh_claude_slot(&b)
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(x.unwrap().is_some() ^ y.unwrap().is_some());
}

#[tokio::test]
async fn a_chat_on_a_saved_account_gets_its_folder_and_a_credential_a_refresh_never_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let (url, hits) = token_server().await;
    let accounts = accounts(tmp.path(), &url);
    write_live_claude(tmp.path(), "live", "live@example.com");
    let live = claude_slot("live", "live-token", "live-rt", now_ms() + 3_600_000);
    let b = claude_slot("b", "at-b", "rt-b", now_ms() + 3_600_000);
    accounts.write_slot(&live).unwrap();
    accounts.write_slot(&b).unwrap();
    let run = accounts
        .claude_login_run(&b)
        .await
        .unwrap()
        .expect("a run of its own");
    assert_eq!(
        run.env[SECURE_STORAGE_ENV],
        accounts.login_dir(&b.id).to_string_lossy()
    );
    assert_eq!(run.env.len(), 1, "only the credential store moves");
    assert_eq!(run.credential, format!("login:{}", b.id));
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "a token with time left is left to the CLI"
    );
    // The CLI refreshes in its chat: the credential is the same, so nothing
    // reads as a changed account.
    cli_refreshes(&accounts, &b, "at-b2", "rt-b2");
    let again = accounts.claude_login_run(&b).await.unwrap().unwrap();
    assert_eq!(again.credential, run.credential);
    // The live login runs as it always did.
    assert!(accounts.claude_login_run(&live).await.unwrap().is_none());
    assert!(!accounts.login_dir(&live.id).exists());
}

#[tokio::test]
async fn a_run_on_an_expired_login_checks_it_and_a_dead_one_reads_as_signed_out() {
    let tmp = tempfile::tempdir().unwrap();
    let (url, hits) = token_server().await;
    let accounts = accounts(tmp.path(), &url);
    write_live_claude(tmp.path(), "live", "live@example.com");
    let b = claude_slot("b", "at-b", "rt-b", now_ms() - 1000);
    accounts.write_slot(&b).unwrap();
    accounts.claude_login_run(&b).await.unwrap().unwrap();
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "an expired token is refreshed first, under the lock"
    );
    assert_eq!(
        oauth_in(&accounts.login_file(&b.id))["refreshToken"],
        "rt-new-1"
    );

    // A refresh the provider refuses: signed out, and rotation skips it.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let dead = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = vec![0u8; 8192];
            let _ = socket.read(&mut buf).await;
            let body = r#"{"error":"invalid_grant"}"#;
            let _ = socket
                .write_all(format!("HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())
                .await;
        }
    });
    let spent_root = tempfile::tempdir().unwrap();
    let accounts = self::accounts(spent_root.path(), &dead);
    write_live_claude(spent_root.path(), "live", "live@example.com");
    let c = claude_slot("c", "at-c", "rt-c", now_ms() - 1000);
    accounts.write_slot(&c).unwrap();
    assert!(matches!(
        accounts.claude_login_run(&c).await,
        Err(ProbeError::Unauthorized { .. })
    ));
}

#[tokio::test]
async fn a_login_that_becomes_live_is_folded_back_into_its_slot_and_its_folder_goes() {
    let tmp = tempfile::tempdir().unwrap();
    let accounts = accounts(tmp.path(), "http://127.0.0.1:9/never");
    let b = claude_slot("b", "at-b", "rt-b", now_ms() + 60_000);
    accounts.write_slot(&b).unwrap();
    accounts.ensure_login_dir(&b).unwrap();
    cli_refreshes(&accounts, &b, "at-b2", "rt-b2");
    accounts.retire_login_dir(&b.id, true, "test").await;
    assert!(!accounts.login_dir(&b.id).exists());
    // The slot keeps the newest pair the folder held: no refresh token lost.
    assert_eq!(
        accounts
            .read_slot(HarnessId::ClaudeCode, &b.id)
            .unwrap()
            .credentials["claudeAiOauth"]["refreshToken"],
        "rt-b2"
    );
    // A removed account's folder goes without being folded back.
    let c = claude_slot("c", "at-c", "rt-c", now_ms() + 60_000);
    accounts.write_slot(&c).unwrap();
    accounts.ensure_login_dir(&c).unwrap();
    accounts.retire_login_dir(&c.id, false, "test").await;
    assert!(!accounts.login_dir(&c.id).exists());
}
