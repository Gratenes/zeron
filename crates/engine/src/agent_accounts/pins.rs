//! Pins: one chat runs on a saved account of its own while the live login
//! stays as it is, so other chats and the user's own terminals keep running
//! on it. Plugins decide *which* account (`zeron.pin_account`, and the
//! `run.preparing` hook that answers a chat's first run); the engine only
//! holds the table and delivers the credential.
//!
//! A pin lasts the chat's life: CLI sessions, resume and prompt caching all
//! assume stable credentials. A chat pinned to the live login (or to an
//! account that is gone) runs exactly as without a pin. Each pin is kept in
//! `{data_dir}/agent-accounts/pins.json`.
//!
//! | harness     | mechanism | how a pinned chat reaches its account |
//! |-------------|-----------|---------------------------------------|
//! | Claude Code | `login-dir` | `CLAUDE_SECURESTORAGE_CONFIG_DIR` = a folder holding only the account's `.credentials.json` (`env` on macOS: `CLAUDE_CODE_OAUTH_TOKEN`) |
//! | Muse        | `root`    | the account's own config root (`XDG_CONFIG_HOME`) |
//!
//! Every other harness reports `pinnable = false` until it has a mechanism.
//! A chat's run credential is a fingerprint of what it was started with, so a
//! session parked on another account (or a token Zeron has since refreshed)
//! is replaced, and resumed, before its next turn.
//!
//! **Claude Code.** The CLI neither refreshes nor writes a token it got from
//! the environment, so the live login can't be touched. Zeron owns the saved
//! login's refresh: a run starting with less than [`CLAUDE_REFRESH_MARGIN`]
//! left on the token refreshes the saved login first (the slot is the only
//! holder of its refresh token). Claude Code treats an environment token as
//! inference-only, so features that need the account's profile scope
//! (Remote Control) are unavailable in a pinned chat.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeron_proto::{AgentAccount, AgentAuthKind, HarnessId};

use super::muse::OWN_LOGIN;
use super::logins::{claude_mechanism, login_dir_usable};
use super::{AccountsRunEnvironment, AgentAccounts, lock, now_ms, str_field, write_file_atomic};

/// The run credential of a chat that runs on the live login.
const LIVE: &str = "live";
/// A pinned Claude run starts with at least this much left on its token
/// (access tokens live ~8h; a turn running past expiry fails).
const CLAUDE_REFRESH_MARGIN: Duration = Duration::from_secs(10 * 60);
/// A chat's cleared pin is remembered this long, so a session parked on the
/// pinned account moves back to the live login.
const CLEARED_KEPT_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// `{data_dir}/agent-accounts/pins.json`.
pub(super) const PINS_FILE: &str = "pins.json";

/// What decides a chat's account for its first run (a plugin's
/// `run.preparing` answer): the account it names, if any.
pub type PinHook = Arc<dyn Fn(HarnessId, String) -> BoxFuture<'static, Option<String>> + Send + Sync>;

/// A chat running on a saved account.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatPin {
    pub chat: String,
    pub harness: HarnessId,
    pub account: String,
    /// Epoch millis it was pinned.
    pub since: i64,
    /// The person chose it in the chat's account picker (see
    /// [`AgentAccounts::pin_chat_account`]); plugins leave it be.
    pub by_user: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct PinEntry {
    /// `None`: the pin was taken back, and the chat runs on the live login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    since: i64,
    /// The person chose this account for the chat.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    by_user: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct PinState {
    /// chat id → harness → pin (a chat may change provider).
    #[serde(default)]
    chats: HashMap<String, HashMap<HarnessId, PinEntry>>,
}

impl PinState {
    pub(super) fn load(file: &std::path::Path) -> Self {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn entry(&self, harness: HarnessId, chat: &str) -> Option<&PinEntry> {
        self.chats.get(chat)?.get(&harness)
    }
}

/// What a pinned chat's run gets.
pub(crate) struct PinRun {
    pub env: BTreeMap<String, String>,
    pub credential: String,
}

fn fingerprint(prefix: &str, secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("{prefix}:{hex}")
}

/// Whether `harness` has a per-chat mechanism at all.
fn pinnable(harness: HarnessId) -> bool {
    matches!(harness, HarnessId::ClaudeCode | HarnessId::Muse)
}

impl AgentAccounts {
    fn persist_pins(&self, state: &PinState) {
        let Ok(root) = self.inner.config.private_root() else { return };
        let Ok(bytes) = serde_json::to_vec_pretty(state) else { return };
        if let Err(err) = write_file_atomic(&root.join(PINS_FILE), &bytes, false) {
            tracing::warn!(error = %err, "could not save the chat pins");
        }
    }

    /// Where a plugin's `run.preparing` answer is asked (a chat's first run).
    pub fn set_pin_hook(&self, hook: PinHook) {
        *lock(&self.inner.pin_hook) = Some(hook);
    }

    /// How a chat can run on `account` beside the live login: `"env"` (a
    /// credential in its process), `"root"` (a config root of its own), or
    /// `None` (not on this device, or not this kind of login: an API key
    /// bills per token, so spreading over it saves nothing).
    pub fn pin_mechanism(&self, account: &AgentAccount) -> Option<&'static str> {
        if account.auth_kind == Some(AgentAuthKind::ApiKey) {
            return None;
        }
        match account.harness {
            HarnessId::ClaudeCode => self
                .read_slot(account.harness, &account.id)
                .is_some_and(|slot| slot.credentials.get("claudeAiOauth").is_some())
                .then_some(claude_mechanism()),
            HarnessId::Muse if account.id != OWN_LOGIN => {
                self.read_slot(account.harness, &account.id).is_some().then_some("root")
            }
            _ => None,
        }
    }

    /// Every chat pinned to an account.
    pub fn pins(&self) -> Vec<ChatPin> {
        let mut pins: Vec<ChatPin> = lock(&self.inner.pins)
            .chats
            .iter()
            .flat_map(|(chat, by_harness)| {
                by_harness.iter().filter_map(move |(harness, entry)| {
                    Some(ChatPin {
                        chat: chat.clone(),
                        harness: *harness,
                        account: entry.account.clone()?,
                        since: entry.since,
                        by_user: entry.by_user,
                    })
                })
            })
            .collect();
        pins.sort_by(|a, b| (a.since, &a.chat).cmp(&(b.since, &b.chat)));
        pins
    }

    /// Pin `chat` to `account` (or, with `None`, take its pin back). What the
    /// account is, and whether it can be pinned, is the caller's to have
    /// checked.
    pub fn set_pin(&self, harness: HarnessId, chat: &str, account: Option<&str>) {
        let now = now_ms();
        let mut state = lock(&self.inner.pins);
        let by_harness = state.chats.entry(chat.to_owned()).or_default();
        match account {
            Some(account) => {
                by_harness.insert(harness, PinEntry { account: Some(account.to_owned()), since: now, by_user: false });
            }
            None => {
                // Remembered, so a session parked on the account moves back.
                if by_harness.get(&harness).is_some_and(|e| e.account.is_some()) {
                    by_harness.insert(harness, PinEntry { account: None, since: now, by_user: false });
                }
            }
        }
        state.chats.retain(|_, by| {
            by.retain(|_, e| e.account.is_some() || now - e.since < CLEARED_KEPT_MS);
            !by.is_empty()
        });
        let snapshot = state.clone();
        drop(state);
        self.persist_pins(&snapshot);
    }

    /// The account `chat` is pinned to, and whether the person chose it.
    pub fn chat_account_pin(&self, harness: HarnessId, chat: &str) -> zeron_proto::ChatAccountPin {
        match lock(&self.inner.pins).entry(harness, chat) {
            Some(entry) => zeron_proto::ChatAccountPin { account: entry.account.clone(), by_user: entry.by_user && entry.account.is_some() },
            None => zeron_proto::ChatAccountPin::default(),
        }
    }

    /// The person's choice of account for one chat. `Some(account)`: the chat
    /// runs on it from its next turn, and plugins don't move it. `None`
    /// (automatic): the pin is taken back and the plugins that pin place the
    /// chat again, as for a new one; with none, rotation or the live login
    /// carry it. An account that can't run a chat is refused (not saved, no
    /// mechanism on this device, or it needs signing in again).
    pub async fn pin_chat_account(
        &self,
        harness: HarnessId,
        chat: &str,
        account: Option<&str>,
    ) -> Result<zeron_proto::ChatAccountPin, crate::EngineError> {
        if !pinnable(harness) {
            return Err(crate::EngineError::Other("A chat can't pick its own account for that agent".into()));
        }
        match account {
            Some(account) => {
                let snapshot = self.list(false).await?;
                let row = snapshot
                    .accounts
                    .iter()
                    .find(|a| a.harness == harness && a.id == account)
                    .ok_or_else(|| crate::EngineError::Other("That saved account no longer exists — refresh and try again.".into()))?;
                if self.pin_mechanism(row).is_none() {
                    return Err(crate::EngineError::Other("A chat can't run on that account on this device".into()));
                }
                if row.needs_signin {
                    let who = row.email.clone().unwrap_or_else(|| "That account".into());
                    return Err(crate::EngineError::Other(format!("{who} needs signing in again (Settings → Accounts) before a chat can use it")));
                }
                let now = now_ms();
                let mut state = lock(&self.inner.pins);
                state
                    .chats
                    .entry(chat.to_owned())
                    .or_default()
                    .insert(harness, PinEntry { account: Some(account.to_owned()), since: now, by_user: true });
                let snapshot = state.clone();
                drop(state);
                self.persist_pins(&snapshot);
            }
            None => {
                self.set_pin(harness, chat, None);
                // Placed as a new chat is: the plugins that pin are asked,
                // and pin it themselves when they answer.
                let hook = lock(&self.inner.pin_hook).clone();
                if let Some(hook) = hook {
                    let _ = hook(harness, chat.to_owned()).await;
                }
            }
        }
        Ok(self.chat_account_pin(harness, chat))
    }

    /// The chat's pin, asking the plugins' `run.preparing` hook first when
    /// it has none yet.
    async fn pin_for_run(&self, harness: HarnessId, chat: &str) -> Option<String> {
        let existing = lock(&self.inner.pins).entry(harness, chat).cloned();
        if let Some(entry) = existing {
            return entry.account;
        }
        let hook = lock(&self.inner.pin_hook).clone()?;
        hook(harness, chat.to_owned()).await
    }

    /// Chats pinned to a login that needs signing in again can't run: when the
    /// account in use changes, their pins are taken back, so they follow the
    /// login now in use instead of staying on one that has to be signed in
    /// first. `accounts` is the listing the change produced.
    pub(super) fn unpin_signed_out(&self, harness: HarnessId, accounts: &[AgentAccount]) {
        let dead: std::collections::HashSet<&str> = accounts
            .iter()
            .filter(|a| a.harness == harness && a.needs_signin)
            .map(|a| a.id.as_str())
            .collect();
        if dead.is_empty() {
            return;
        }
        let chats: Vec<String> = lock(&self.inner.pins)
            .chats
            .iter()
            .filter(|(_, by)| {
                by.get(&harness)
                    .and_then(|entry| entry.account.as_deref())
                    .is_some_and(|account| dead.contains(account))
            })
            .map(|(chat, _)| chat.clone())
            .collect();
        for chat in chats {
            self.set_pin(harness, &chat, None);
        }
    }

    /// A chat pinned to a login that can't run it (its refresh was refused: it
    /// needs signing in again): the pin is taken back, the plugins that pin
    /// are asked again (a chat with no pin is theirs to place, and they skip a
    /// login that needs signing in), and the chat runs on the account they
    /// name that works. With no such account it runs on the live login, as
    /// without a pin: a run is never refused over a pin.
    async fn fall_back_from_pin(&self, harness: HarnessId, chat: &str, dead: &str) -> Option<PinRun> {
        tracing::warn!(%chat, account = %dead, "a chat's pinned account can't run it; moving it to a working one");
        // The hook only answers a chat that has no pin.
        self.set_pin(harness, chat, None);
        let hook = lock(&self.inner.pin_hook).clone()?;
        let account = hook(harness, chat.to_owned()).await?;
        if account != dead
            && let Ok(Some(run)) = self.pin_run(harness, &account).await
        {
            // The hook pinned the chat itself when it is a plugin's; this
            // makes it so for any other.
            self.set_pin(harness, chat, Some(&account));
            return Some(run);
        }
        // Named nothing that works (or the same dead login): no pin.
        self.set_pin(harness, chat, None);
        None
    }

    /// What a run of `chat` needs to reach its account (`None`: the live
    /// login, as without a pin). `Err`: the account can't be used and the
    /// run can't start (the words say what to do).
    pub(crate) async fn pin_run(&self, harness: HarnessId, account: &str) -> Result<Option<PinRun>, String> {
        match harness {
            HarnessId::ClaudeCode => {
                let Some(slot) = self.read_slot(harness, account) else { return Ok(None) };
                let who = slot.profile.email.clone();
                let signed_out = || format!("{who} needs signing in again (Settings → Accounts), or pin this chat to another account");
                if login_dir_usable() {
                    // Refused before (a probe or a refresh, or a run on this
                    // login) and the login is unchanged since: starting a CLI
                    // on it only fails again. The flag clears itself when the
                    // login changes (a sign-in again, or the CLI's refresh).
                    if self.slot_needs_signin(harness, account) {
                        return Err(signed_out());
                    }
                    // The account's own folder: the CLI keeps and refreshes
                    // its login (see `logins`). `None`: the live login.
                    return match self.claude_login_run(&slot).await {
                        Ok(run) => Ok(run.map(|run| PinRun { env: run.env, credential: run.credential })),
                        Err(error) => {
                            if matches!(error, super::ProbeError::Unauthorized { .. }) {
                                self.note_signin_failure(harness, &slot, error);
                            }
                            Err(signed_out())
                        }
                    };
                }
                // The live login runs as it always did, and is never
                // refreshed here: the CLI holds its refresh token.
                if !self.claude_slot_is_not_live(&slot).await {
                    return Ok(None);
                }
                let oauth = slot.credentials.get("claudeAiOauth").cloned().unwrap_or_default();
                let mut token = str_field(&oauth, "accessToken").ok_or_else(signed_out)?;
                let expires = oauth.get("expiresAt").and_then(|v| v.as_i64()).unwrap_or(0);
                if expires - now_ms() < CLAUDE_REFRESH_MARGIN.as_millis() as i64 {
                    // Its refresh was already refused and nothing has changed
                    // since: asking again only fails again.
                    if self.slot_needs_signin(harness, account) {
                        return Err(signed_out());
                    }
                    match self.refresh_claude_slot(&slot).await {
                        Ok(Some(fresh)) => token = fresh,
                        // Another refresh of this slot is in flight: its
                        // result is on disk by the next run.
                        Ok(None) => {}
                        Err(error) => {
                            if matches!(error, super::ProbeError::Unauthorized { .. }) {
                                self.note_signin_failure(harness, &slot, error);
                            }
                            return Err(signed_out());
                        }
                    }
                }
                Ok(Some(PinRun {
                    credential: fingerprint("claude", &token),
                    env: BTreeMap::from([("CLAUDE_CODE_OAUTH_TOKEN".to_owned(), token)]),
                }))
            }
            HarnessId::Muse => {
                if self.read_slot(harness, account).is_none() {
                    return Ok(None);
                }
                let env = self.muse_env(account).map_err(|e| e.to_string())?;
                Ok(Some(PinRun { env, credential: account.to_owned() }))
            }
            _ => Ok(None),
        }
    }
}

/// The agent accounts' spawn-time environment: a pinned chat's account, and
/// for the chats that aren't pinned, built-in rotation and Muse's zeron-kept
/// account ([`AccountsRunEnvironment`]).
pub struct PinRunEnvironment(pub AgentAccounts);

#[async_trait::async_trait]
impl crate::registry::RunEnvironment for PinRunEnvironment {
    async fn prepare(
        &self,
        harness: HarnessId,
        chat_id: Option<&str>,
        request: &mut zeron_proto::RunRequest,
    ) -> Result<(), String> {
        if pinnable(harness)
            && let Some(chat) = chat_id
            && let Some(account) = self.0.pin_for_run(harness, chat).await
        {
            let run = match self.0.pin_run(harness, &account).await {
                Ok(run) => run,
                // A Claude login that needs signing in again (its refresh was
                // refused) doesn't stop the chat: it moves to a working
                // account (see `fall_back_from_pin`). A failure that says
                // nothing of the login (an outage) still stops this run, and
                // the pin stays.
                Err(_) if harness == HarnessId::ClaudeCode && self.0.slot_needs_signin(harness, &account) => {
                    self.0.fall_back_from_pin(harness, chat, &account).await
                }
                Err(why) => return Err(why),
            };
            if let Some(run) = run {
                // Never over what another provider (the Cloud broker) set.
                for (key, value) in run.env {
                    request.env.entry(key).or_insert(value);
                }
                return Ok(());
            }
        }
        AccountsRunEnvironment(self.0.clone()).prepare(harness, chat_id, request).await
    }

    async fn credential(&self, harness: HarnessId, chat_id: Option<&str>) -> Option<String> {
        let pinned = chat_id
            .filter(|_| pinnable(harness))
            .and_then(|chat| lock(&self.0.inner.pins).entry(harness, chat).cloned());
        if let Some(entry) = pinned {
            // A pin taken back reads as a change too, so a parked session
            // moves to the live login before its next turn.
            let run = match entry.account {
                Some(account) => self.0.pin_run(harness, &account).await.ok().flatten(),
                None => None,
            };
            if let Some(run) = run {
                return Some(run.credential);
            }
            if harness != HarnessId::Muse {
                return Some(LIVE.to_owned());
            }
        }
        AccountsRunEnvironment(self.0.clone()).credential(harness, chat_id).await
    }

    async fn run_refused(&self, harness: HarnessId, credential: &str) {
        self.0.note_run_refused(harness, credential);
    }

    async fn record_usage(
        &self,
        harness: HarnessId,
        credential: &str,
        windows: Vec<zeron_proto::AgentUsageWindow>,
        plan_label: Option<String>,
    ) {
        AccountsRunEnvironment(self.0.clone())
            .record_usage(harness, credential, windows, plan_label)
            .await;
    }
}

#[cfg(test)]
mod tests;
