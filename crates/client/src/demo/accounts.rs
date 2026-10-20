//! Demo agent accounts: two Claude Code logins and a Codex one with plan
//! usage, switchable and forgettable; sign-ins land after a few polls.

use std::time::Duration;

use chrono::Utc;
use serde_json::{Value, json};
use zeron_proto::{
    AgentAccount, AgentAccountsSnapshot, AgentAuthKind, AgentLoginMode, AgentLoginPoll,
    AgentLoginStart, AgentLoginStatus, AgentUsageWindow, HarnessId,
};
use zeron_rpc::methods;

use super::DemoHost;
use crate::agent_accounts::{mark_switched, wire};
use crate::error::{ClientError, Result};
use crate::lock;

#[derive(Default)]
pub(super) struct DemoAccounts {
    snapshot: Option<AgentAccountsSnapshot>,
    /// Pending sign-in: (login id, harness, polls so far).
    login: Option<(String, HarnessId, u32)>,
    seq: u32,
}

fn window(label: &str, used: f32, hours: i64) -> AgentUsageWindow {
    AgentUsageWindow {
        label: label.into(),
        used_fraction: used,
        resets_at: Some(Utc::now() + chrono::Duration::hours(hours)),
    }
}

fn account(id: &str, harness: HarnessId, email: &str, plan: &str, active: bool) -> AgentAccount {
    AgentAccount {
        id: id.into(),
        harness,
        email: Some(email.into()),
        plan_label: Some(plan.into()),
        active,
        usage_windows: Vec::new(),
        usage_fetched_at: Some(Utc::now().timestamp_millis()),
        usage_error: None,
        display_name: None,
        organization: None,
        auth_kind: Some(AgentAuthKind::Oauth),
        switchable: true,
        saved_at: Some(Utc::now().timestamp_millis()),
        provider: None,
    }
}

fn seeded() -> AgentAccountsSnapshot {
    let mut work = account(
        "demo-claude-work",
        HarnessId::ClaudeCode,
        "ada@work.dev",
        "Max",
        true,
    );
    work.usage_windows = vec![window("Session", 0.42, 3), window("Week", 0.78, 80)];
    let mut home = account(
        "demo-claude-home",
        HarnessId::ClaudeCode,
        "ada@home.dev",
        "Pro",
        false,
    );
    home.usage_windows = vec![window("Session", 0.05, 4), window("Week", 0.12, 120)];
    let mut codex = account("demo-codex", HarnessId::Codex, "ada@work.dev", "Plus", true);
    codex.usage_windows = vec![window("5h", 0.93, 1), window("Week", 0.31, 50)];
    AgentAccountsSnapshot {
        accounts: vec![work, home, codex],
        warnings: Vec::new(),
    }
}

fn param<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ClientError::HostError(format!("missing {key}")))
}

fn harness(params: &Value) -> Result<HarnessId> {
    serde_json::from_value(params.get("harness").cloned().unwrap_or_default())
        .map_err(|e| ClientError::HostError(e.to_string()))
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| ClientError::Internal(e.to_string()))
}

impl DemoHost {
    pub(crate) async fn agent_accounts_call(&self, method: &str, params: Value) -> Result<Value> {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let mut state = lock(&self.accounts);
        let state = &mut *state;
        let snapshot = state.snapshot.get_or_insert_with(seeded);
        match method {
            methods::LIST_AGENT_ACCOUNTS => to_value(snapshot),
            methods::ACTIVATE_AGENT_ACCOUNT => {
                let harness = wire(&harness(&params)?);
                mark_switched(snapshot, &harness, param(&params, "accountId")?);
                to_value(snapshot)
            }
            methods::FORGET_AGENT_ACCOUNT => {
                let id = param(&params, "accountId")?;
                snapshot.accounts.retain(|a| a.id != id || a.active);
                to_value(snapshot)
            }
            methods::START_AGENT_LOGIN => {
                let harness = harness(&params)?;
                state.seq += 1;
                let login_id = format!("demo-login-{}", state.seq);
                state.login = Some((login_id.clone(), harness, 0));
                to_value(&AgentLoginStart {
                    login_id,
                    url: "https://zeron.sh/".into(),
                    mode: if harness == HarnessId::ClaudeCode {
                        AgentLoginMode::PasteCode
                    } else {
                        AgentLoginMode::Browser
                    },
                    callback_port: None,
                })
            }
            methods::POLL_AGENT_LOGIN => {
                let id = param(&params, "loginId")?;
                let Some((login_id, harness, polls)) = state.login.as_mut() else {
                    return Err(ClientError::HostError("no sign-in in progress".into()));
                };
                if login_id != id {
                    return Err(ClientError::HostError("unknown sign-in".into()));
                }
                *polls += 1;
                let done = *polls >= 3;
                if done {
                    let n = state.seq;
                    let harness = *harness;
                    snapshot.accounts.push(account(
                        &format!("demo-new-{n}"),
                        harness,
                        &format!("new{n}@demo.dev"),
                        "Pro",
                        false,
                    ));
                    state.login = None;
                }
                to_value(&AgentLoginPoll {
                    status: if done {
                        AgentLoginStatus::Done
                    } else {
                        AgentLoginStatus::Pending
                    },
                    message: None,
                    url: None,
                    callback_port: None,
                })
            }
            methods::COMPLETE_AGENT_LOGIN => {
                let code = param(&params, "code")?;
                if code.trim().is_empty() {
                    return Err(ClientError::HostError(
                        "Paste the code from the browser".into(),
                    ));
                }
                let Some((_, harness, _)) = state.login.take() else {
                    return Err(ClientError::HostError("no sign-in in progress".into()));
                };
                let n = state.seq;
                snapshot.accounts.push(account(
                    &format!("demo-new-{n}"),
                    harness,
                    &format!("new{n}@demo.dev"),
                    "Pro",
                    false,
                ));
                to_value(snapshot)
            }
            methods::CANCEL_AGENT_LOGIN => {
                state.login = None;
                Ok(json!({ "ok": true }))
            }
            other => Err(ClientError::Unsupported(other.to_owned())),
        }
    }
}
