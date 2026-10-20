//! Agent accounts over FFI: a device's CLI logins (Claude Code, Codex, …),
//! their plan usage, switching between them, and signing in to new ones.
//! See `zeron_client::agent_accounts`.

use chrono::Utc;
use zeron_client::agent_accounts as aa;

use super::{CoreClient, CoreResult, on_runtime};

/// One agent zeron can sign into.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentProvider {
    /// Wire harness id (`claude-code`).
    pub harness: String,
    pub name: String,
    /// The CLI's own login command.
    pub command: String,
    pub reports_usage: bool,
    pub switches_accounts: bool,
    pub keeps_one_login: bool,
    pub note: Option<String>,
    /// Ways to add an account, in button order.
    pub login_options: Vec<AgentLoginOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentLoginOption {
    /// `StartAgentLogin`'s provider (`None` = the agent's only login).
    pub provider: Option<String>,
    pub label: String,
}

/// A CLI plan rate-limit window ("Session", "Week").
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentUsageWindow {
    pub label: String,
    /// 0.0..=1.0
    pub used_fraction: f32,
    pub resets_at_ms: Option<i64>,
    /// "resets 3:45 PM" / "resets Thu" / "resets Sep 14".
    pub reset_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentAccount {
    pub id: String,
    /// Wire harness id.
    pub harness: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub organization: Option<String>,
    pub plan_label: Option<String>,
    /// The agent's live login (per `provider` group).
    pub active: bool,
    /// Signed in with a raw API key rather than an account.
    pub api_key: bool,
    /// Can be switched to / forgotten.
    pub switchable: bool,
    /// The model-provider group for agents that keep one login per provider.
    pub provider: Option<String>,
    pub usage_windows: Vec<AgentUsageWindow>,
    /// The most-used window (the binding limit).
    pub used_fraction: Option<f32>,
    pub usage_fetched_at_ms: Option<i64>,
    pub usage_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentAccountWarning {
    pub harness: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AgentAccountsSnapshot {
    pub accounts: Vec<AgentAccount>,
    pub warnings: Vec<AgentAccountWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentLoginMode {
    /// Paste the code the browser shows back (`complete_agent_login`).
    PasteCode,
    /// The browser finishes on its own; poll until done.
    Browser,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentLoginStart {
    pub login_id: String,
    /// Empty when the page is only known later (a poll carries it).
    pub url: String,
    pub mode: AgentLoginMode,
    /// The OAuth redirect lands on this loopback port of the HOST device, so
    /// a phone browser cannot finish it.
    pub callback_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AgentLoginStatus {
    Pending,
    Done,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentLoginPoll {
    pub status: AgentLoginStatus,
    pub message: Option<String>,
    /// A sign-in page that only became known now; open it once.
    pub url: Option<String>,
    pub callback_port: Option<u16>,
}

impl From<aa::AgentProvider> for AgentProvider {
    fn from(p: aa::AgentProvider) -> Self {
        Self {
            harness: p.harness.into(),
            name: p.name.into(),
            command: p.command.into(),
            reports_usage: p.reports_usage,
            switches_accounts: p.switches_accounts,
            keeps_one_login: p.keeps_one_login,
            note: p.note.map(Into::into),
            login_options: p
                .login_options
                .into_iter()
                .map(|o| AgentLoginOption {
                    provider: o.provider.map(Into::into),
                    label: o.label.into(),
                })
                .collect(),
        }
    }
}

impl From<zeron_proto::AgentAccountsSnapshot> for AgentAccountsSnapshot {
    fn from(s: zeron_proto::AgentAccountsSnapshot) -> Self {
        let now = Utc::now();
        Self {
            accounts: s
                .accounts
                .into_iter()
                .map(|a| AgentAccount {
                    used_fraction: aa::used_fraction(&a),
                    harness: aa::wire(&a.harness),
                    usage_windows: a
                        .usage_windows
                        .into_iter()
                        .map(|w| AgentUsageWindow {
                            reset_label: aa::format_reset(w.resets_at, now),
                            resets_at_ms: w.resets_at.map(|t| t.timestamp_millis()),
                            label: w.label,
                            used_fraction: w.used_fraction.clamp(0.0, 1.0),
                        })
                        .collect(),
                    api_key: a.auth_kind == Some(zeron_proto::AgentAuthKind::ApiKey),
                    id: a.id,
                    email: a.email,
                    display_name: a.display_name,
                    organization: a.organization,
                    plan_label: a.plan_label,
                    active: a.active,
                    switchable: a.switchable,
                    provider: a.provider,
                    usage_fetched_at_ms: a.usage_fetched_at,
                    usage_error: a.usage_error,
                })
                .collect(),
            warnings: s
                .warnings
                .into_iter()
                .map(|w| AgentAccountWarning {
                    harness: aa::wire(&w.harness),
                    message: w.message,
                })
                .collect(),
        }
    }
}

impl From<zeron_proto::AgentLoginStart> for AgentLoginStart {
    fn from(s: zeron_proto::AgentLoginStart) -> Self {
        Self {
            login_id: s.login_id,
            url: s.url,
            mode: match s.mode {
                zeron_proto::AgentLoginMode::PasteCode => AgentLoginMode::PasteCode,
                zeron_proto::AgentLoginMode::Browser => AgentLoginMode::Browser,
            },
            callback_port: s.callback_port,
        }
    }
}

impl From<zeron_proto::AgentLoginPoll> for AgentLoginPoll {
    fn from(p: zeron_proto::AgentLoginPoll) -> Self {
        Self {
            status: match p.status {
                zeron_proto::AgentLoginStatus::Pending => AgentLoginStatus::Pending,
                zeron_proto::AgentLoginStatus::Done => AgentLoginStatus::Done,
                zeron_proto::AgentLoginStatus::Error => AgentLoginStatus::Error,
            },
            message: p.message,
            url: p.url,
            callback_port: p.callback_port,
        }
    }
}

/// Every agent with a login of its own, in display order.
#[uniffi::export]
pub fn agent_providers() -> Vec<AgentProvider> {
    aa::agent_providers().into_iter().map(Into::into).collect()
}

/// Whether a sign-in URL relayed from a device may be opened (web pages and
/// loopback only).
#[uniffi::export]
pub fn is_openable_login_url(url: String) -> bool {
    aa::is_openable_login_url(&url)
}

#[uniffi::export]
impl CoreClient {
    /// The agent logins on `device_id`. `force_usage` probes each provider's
    /// plan usage now (slower); otherwise the device serves its cache.
    pub async fn list_agent_accounts(
        &self,
        device_id: String,
        force_usage: bool,
    ) -> CoreResult<AgentAccountsSnapshot> {
        let client = self.client.clone();
        on_runtime(async move { client.list_agent_accounts(&device_id, force_usage).await })
            .await
            .map(Into::into)
    }

    /// Make `account_id` the live login of `harness` on the device (every
    /// session of that agent on the device switches with it).
    pub async fn activate_agent_account(
        &self,
        device_id: String,
        harness: String,
        account_id: String,
    ) -> CoreResult<AgentAccountsSnapshot> {
        let client = self.client.clone();
        on_runtime(async move {
            client
                .activate_agent_account(&device_id, &harness, &account_id)
                .await
        })
        .await
        .map(Into::into)
    }

    pub async fn forget_agent_account(
        &self,
        device_id: String,
        harness: String,
        account_id: String,
    ) -> CoreResult<AgentAccountsSnapshot> {
        let client = self.client.clone();
        on_runtime(async move {
            client
                .forget_agent_account(&device_id, &harness, &account_id)
                .await
        })
        .await
        .map(Into::into)
    }

    pub async fn start_agent_login(
        &self,
        device_id: String,
        harness: String,
        provider: Option<String>,
    ) -> CoreResult<AgentLoginStart> {
        let client = self.client.clone();
        on_runtime(async move {
            client
                .start_agent_login(&device_id, &harness, provider.as_deref())
                .await
        })
        .await
        .map(Into::into)
    }

    pub async fn poll_agent_login(
        &self,
        device_id: String,
        login_id: String,
    ) -> CoreResult<AgentLoginPoll> {
        let client = self.client.clone();
        on_runtime(async move { client.poll_agent_login(&device_id, &login_id).await })
            .await
            .map(Into::into)
    }

    pub async fn complete_agent_login(
        &self,
        device_id: String,
        login_id: String,
        code: String,
    ) -> CoreResult<AgentAccountsSnapshot> {
        let client = self.client.clone();
        on_runtime(async move {
            client
                .complete_agent_login(&device_id, &login_id, &code)
                .await
        })
        .await
        .map(Into::into)
    }

    pub async fn cancel_agent_login(&self, device_id: String, login_id: String) -> CoreResult<()> {
        let client = self.client.clone();
        on_runtime(async move { client.cancel_agent_login(&device_id, &login_id).await }).await
    }
}
