//! Agent accounts: the CLI logins (Claude Code, Codex, …) an execution device
//! runs its agents as, their plan usage, and signing in to new ones. Logins
//! are per device, so every call targets the device that hosts the agent —
//! a session's host for its usage ring, any device from Settings.
//!
//! The provider table and the pure presentation rules mirror the desktop's
//! (`crates/ui/src/settings/accounts.rs`); per-provider behaviour is
//! documented engine-side (`crates/engine/src/agent_accounts.rs`).

use chrono::{DateTime, Local, Utc};
use serde_json::{Value, json};
use zeron_proto::{
    AgentAccount, AgentAccountWarning, AgentAccountsSnapshot, AgentLoginPoll, AgentLoginStart,
};
use zeron_rpc::methods;

use crate::client::{Backend, Client};
use crate::error::{ClientError, Result};

/// One agent zeron can sign into, in display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProvider {
    /// Wire harness id (`claude-code`).
    pub harness: &'static str,
    pub name: &'static str,
    /// The CLI's own login command, named in empty-state copy.
    pub command: &'static str,
    /// Reports plan usage (no meters and no "unavailable" note otherwise).
    pub reports_usage: bool,
    /// zeron switches between its saved logins (Hermes rotates its own
    /// credential pool; Antigravity keeps one login).
    pub switches_accounts: bool,
    /// The agent holds exactly one login: signing in again only re-confirms.
    pub keeps_one_login: bool,
    /// Standing note for an agent whose accounts work differently.
    pub note: Option<&'static str>,
    /// Ways to add an account, in button order.
    pub login_options: Vec<LoginOption>,
}

/// One sign-in: agents that keep a login per model provider (OpenCode, Pi,
/// Hermes) name it; the rest have one (`provider: None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginOption {
    pub provider: Option<&'static str>,
    pub label: &'static str,
}

const PROVIDERS: [(&str, &str, &str); 9] = [
    ("claude-code", "Claude Code", "claude"),
    ("codex", "Codex", "codex"),
    ("cursor", "Cursor", "cursor-agent"),
    ("antigravity", "Antigravity", "Antigravity"),
    ("grok", "Grok", "grok login"),
    ("devin", "Devin", "devin auth login"),
    ("opencode", "OpenCode", "opencode auth login"),
    ("pi", "Pi", "pi"),
    ("hermes", "Hermes", "hermes auth add"),
];

/// Every agent with a login of its own, in display order.
pub fn agent_providers() -> Vec<AgentProvider> {
    PROVIDERS
        .iter()
        .map(|&(harness, name, command)| {
            let option = |provider, label| LoginOption {
                provider: Some(provider),
                label,
            };
            let login_options = match harness {
                "opencode" => vec![
                    option("openai", "ChatGPT"),
                    option("github-copilot", "GitHub Copilot"),
                ],
                "pi" => vec![option("openai-codex", "ChatGPT")],
                "hermes" => vec![
                    option("openai-codex", "ChatGPT"),
                    option("nous", "Nous Portal"),
                ],
                _ => vec![LoginOption {
                    provider: None,
                    label: name,
                }],
            };
            AgentProvider {
                harness,
                name,
                command,
                reports_usage: harness != "antigravity",
                switches_accounts: !matches!(harness, "hermes" | "antigravity"),
                keeps_one_login: harness == "antigravity",
                note: (harness == "hermes").then_some(
                    "Hermes manages its own credential pool and rotates through it. Accounts \
                     added here go through `hermes auth add`; remove one with \
                     `hermes auth remove`.",
                ),
                login_options,
            }
        })
        .collect()
}

/// The provider entry for a wire harness id, if that agent signs in.
pub fn agent_provider(harness: &str) -> Option<AgentProvider> {
    agent_providers().into_iter().find(|p| p.harness == harness)
}

/// The binding limit: the account's most-used window (0..=1).
pub fn used_fraction(account: &AgentAccount) -> Option<f32> {
    account
        .usage_windows
        .iter()
        .map(|w| w.used_fraction.clamp(0.0, 1.0))
        .reduce(f32::max)
}

/// The optimistic half of a switch: `account` becomes the live login of its
/// group (its agent, or that agent's model provider).
pub fn mark_switched(snapshot: &mut AgentAccountsSnapshot, harness: &str, account_id: &str) {
    let Some(provider) = snapshot
        .accounts
        .iter()
        .find(|a| wire(&a.harness) == harness && a.id == account_id)
        .map(|a| a.provider.clone())
    else {
        return;
    };
    for row in &mut snapshot.accounts {
        if wire(&row.harness) == harness && row.provider == provider {
            row.active = row.id == account_id;
        }
    }
}

/// Compact reset moment: a local clock time within ~22h, a weekday within a
/// week, else month + day. Pure given `now`.
pub fn format_reset(resets_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<String> {
    let at = resets_at?;
    let local = at.with_timezone(&Local);
    let hours = at.signed_duration_since(now).num_hours();
    Some(if hours < 22 {
        format!("resets {}", local.format("%-I:%M %p"))
    } else if hours < 24 * 7 {
        format!("resets {}", local.format("%a"))
    } else {
        format!("resets {}", local.format("%b %-d"))
    })
}

/// Whether a login URL may be opened: web pages and loopback callbacks only,
/// never `file://` or custom schemes (the URL comes from another device).
pub fn is_openable_login_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    match scheme.to_ascii_lowercase().as_str() {
        "https" => true,
        "http" => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            let authority = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            let host = match authority.strip_prefix('[') {
                Some(v6) => v6.split(']').next().unwrap_or(""),
                None => authority.split(':').next().unwrap_or(""),
            };
            matches!(host, "localhost" | "127.0.0.1" | "::1")
        }
        _ => false,
    }
}

/// Wire id of a proto harness (`claude-code`).
pub fn wire(harness: &zeron_proto::HarnessId) -> String {
    serde_json::to_value(harness)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Decode a snapshot, skipping rows a newer engine added for an agent this
/// build doesn't know rather than failing the whole list.
fn decode_snapshot(value: Value) -> Result<AgentAccountsSnapshot> {
    fn rows<T: serde::de::DeserializeOwned>(value: Option<&Value>) -> Vec<T> {
        value
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| serde_json::from_value(item.clone()).ok())
                    .collect()
            })
            .unwrap_or_default()
    }
    if !value.is_object() {
        return Err(ClientError::HostError(
            "ListAgentAccounts: bad reply".into(),
        ));
    }
    Ok(AgentAccountsSnapshot {
        accounts: rows::<AgentAccount>(value.get("accounts")),
        warnings: rows::<AgentAccountWarning>(value.get("warnings")),
    })
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|e| ClientError::HostError(e.to_string()))
}

impl Client {
    async fn call_accounts(&self, device_id: &str, method: &str, params: Value) -> Result<Value> {
        match self.inner.backend() {
            Backend::Demo(demo) => demo.agent_accounts_call(method, params).await,
            Backend::Live(live) => live.relay.call(device_id, method, params).await,
        }
    }

    /// The device's agent logins. `force_usage` makes the engine probe each
    /// provider's plan usage now; otherwise it serves its cache.
    pub async fn list_agent_accounts(
        &self,
        device_id: &str,
        force_usage: bool,
    ) -> Result<AgentAccountsSnapshot> {
        let reply = self
            .call_accounts(
                device_id,
                methods::LIST_AGENT_ACCOUNTS,
                json!({ "forceUsage": force_usage }),
            )
            .await?;
        decode_snapshot(reply)
    }

    /// Make `account_id` the live login of its agent on the device.
    pub async fn activate_agent_account(
        &self,
        device_id: &str,
        harness: &str,
        account_id: &str,
    ) -> Result<AgentAccountsSnapshot> {
        let reply = self
            .call_accounts(
                device_id,
                methods::ACTIVATE_AGENT_ACCOUNT,
                json!({ "harness": harness, "accountId": account_id, "id": account_id }),
            )
            .await?;
        decode_snapshot(reply)
    }

    /// Delete a saved login from the device.
    pub async fn forget_agent_account(
        &self,
        device_id: &str,
        harness: &str,
        account_id: &str,
    ) -> Result<AgentAccountsSnapshot> {
        let reply = self
            .call_accounts(
                device_id,
                methods::FORGET_AGENT_ACCOUNT,
                json!({ "harness": harness, "accountId": account_id, "id": account_id }),
            )
            .await?;
        decode_snapshot(reply)
    }

    /// Start a sign-in on the device: open the returned URL, then poll
    /// (browser mode) or complete with a pasted code.
    pub async fn start_agent_login(
        &self,
        device_id: &str,
        harness: &str,
        provider: Option<&str>,
    ) -> Result<AgentLoginStart> {
        let mut params = json!({ "harness": harness });
        if let Some(provider) = provider.filter(|p| !p.is_empty()) {
            params["provider"] = json!(provider);
        }
        let reply = self
            .call_accounts(device_id, methods::START_AGENT_LOGIN, params)
            .await?;
        decode(reply)
    }

    pub async fn poll_agent_login(
        &self,
        device_id: &str,
        login_id: &str,
    ) -> Result<AgentLoginPoll> {
        let reply = self
            .call_accounts(
                device_id,
                methods::POLL_AGENT_LOGIN,
                json!({ "loginId": login_id }),
            )
            .await?;
        decode(reply)
    }

    pub async fn complete_agent_login(
        &self,
        device_id: &str,
        login_id: &str,
        code: &str,
    ) -> Result<AgentAccountsSnapshot> {
        let reply = self
            .call_accounts(
                device_id,
                methods::COMPLETE_AGENT_LOGIN,
                json!({ "loginId": login_id, "code": code.trim() }),
            )
            .await?;
        decode_snapshot(reply)
    }

    pub async fn cancel_agent_login(&self, device_id: &str, login_id: &str) -> Result<()> {
        self.call_accounts(
            device_id,
            methods::CANCEL_AGENT_LOGIN,
            json!({ "loginId": login_id }),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_skips_unknown_harness_rows() {
        let snapshot = decode_snapshot(json!({
            "accounts": [
                { "id": "a", "harness": "claude-code", "email": null, "planLabel": "Max", "active": true },
                { "id": "b", "harness": "from-the-future", "email": null, "planLabel": null, "active": false }
            ],
            "warnings": []
        }))
        .unwrap();
        assert_eq!(snapshot.accounts.len(), 1);
        assert_eq!(snapshot.accounts[0].id, "a");
    }

    #[test]
    fn switch_moves_active_within_provider_group() {
        let mut snapshot = decode_snapshot(json!({
            "accounts": [
                { "id": "a", "harness": "opencode", "email": null, "planLabel": null, "active": true, "provider": "openai" },
                { "id": "b", "harness": "opencode", "email": null, "planLabel": null, "active": false, "provider": "openai" },
                { "id": "c", "harness": "opencode", "email": null, "planLabel": null, "active": true, "provider": "github-copilot" }
            ],
            "warnings": []
        }))
        .unwrap();
        mark_switched(&mut snapshot, "opencode", "b");
        let active: Vec<_> = snapshot.accounts.iter().map(|a| a.active).collect();
        assert_eq!(active, [false, true, true]);
    }

    #[test]
    fn login_urls_are_web_or_loopback_only() {
        assert!(is_openable_login_url("https://claude.ai/oauth/authorize"));
        assert!(is_openable_login_url("http://localhost:1455/auth"));
        assert!(!is_openable_login_url("http://example.com/"));
        assert!(!is_openable_login_url("file:///etc/passwd"));
        assert!(!is_openable_login_url("intent://x"));
    }
}
