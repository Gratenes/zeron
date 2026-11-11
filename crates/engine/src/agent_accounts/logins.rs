//! Claude login folders: each saved Claude account that isn't the live login
//! gets a folder of its own holding only its `.credentials.json`, and a chat
//! that runs on the account starts with `CLAUDE_SECURESTORAGE_CONFIG_DIR`
//! pointing at it (mechanism `login-dir`).
//!
//! The variable relocates only the CLI's credential store. Settings,
//! transcripts, memory and plugins stay in the user's own config dir, so
//! resume and everything else just work, while the CLI holds the account's
//! full login: it refreshes its own token (and `Remote Control`, which needs
//! the profile scope, works), serialised across processes by the lock it
//! keeps in that folder (`.oauth_refresh.lock`, see
//! [`zeron_harness::claude::refresh_lock`]).
//!
//! This replaces handing a chat the slot's *access token* in
//! `CLAUDE_CODE_OAUTH_TOKEN`. Such a token is inference-only and can't be
//! refreshed by the CLI, so the engine had to: and a refresh kills the access
//! token, so every chat already running (or parked) on the old one failed
//! with a 401 "Authentication failed", at its next request, with nothing to
//! notice it until a user turn relaunched it.
//!
//! **One holder per refresh token.** Refresh tokens are single-use. A saved
//! login's refresh token lives in its folder from then on; the slot file's
//! copy is only what the folder started from (and where it is folded back to
//! when the account becomes the live login), and every read of a slot sees
//! the folder's newer credentials ([`AgentAccounts::overlay_login`]). Zeron
//! refreshes the login (an expired probe, a run that starts on an expired
//! token) only holding the CLI's own [`RefreshLock`] on the folder and after
//! re-reading it, so a refresh the CLI already made is adopted instead of
//! spent twice. The live login never has a folder: when an account becomes
//! live its folder is folded back into the slot and removed.
//!
//! Not on macOS: the CLI keeps its login in the Keychain there, under a name
//! derived from the config dir, and never reads a credentials file the folder
//! holds. Chats there keep the access-token environment mechanism.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use zeron_harness::claude::refresh_lock::RefreshLock;
use zeron_proto::HarnessId;

use super::{
    AgentAccounts, EngineError, NoCredentials, ProbeError, Slot, now_ms, private_dir, read_json,
    stage_file_atomic, str_field, write_file_atomic,
};

/// `{data_dir}/agent-accounts/claude-logins/{slot id}/`.
pub(super) const LOGIN_DIRS: &str = "claude-logins";
/// Claude Code's credential store relocation (see module docs).
pub(crate) const SECURE_STORAGE_ENV: &str = "CLAUDE_SECURESTORAGE_CONFIG_DIR";
const CREDENTIALS_FILE: &str = ".credentials.json";
/// Marks the first-start migration done (see [`AgentAccounts::migrate_login_dirs`]).
const MIGRATED_MARK: &str = ".migrated";
/// How long zeron waits for a CLI that is mid-refresh before it gives up on
/// its own refresh (the CLI's POST times out at 30 s).
const LOCK_WAIT: Duration = Duration::from_millis(if cfg!(test) { 700 } else { 40_000 });
/// A run starting on a token this close to expiry (or past it) checks the
/// login first, so a dead one is skipped instead of failing the turn.
const START_MARGIN_MS: i64 = 60_000;

/// What a chat on a saved Claude account gets.
pub(crate) struct LoginRun {
    pub env: BTreeMap<String, String>,
    pub credential: String,
}

/// Whether Claude chats on this OS reach a saved account by folder.
pub(crate) fn login_dir_usable() -> bool {
    !cfg!(target_os = "macos")
}

/// The mechanism name pins report for Claude Code.
pub(crate) fn claude_mechanism() -> &'static str {
    if login_dir_usable() {
        "login-dir"
    } else {
        "env"
    }
}

pub(super) fn oauth(credentials: &Value) -> Option<&Value> {
    credentials.get("claudeAiOauth")
}

pub(super) fn expires_at(credentials: &Value) -> Option<i64> {
    oauth(credentials)?
        .get("expiresAt")
        .and_then(Value::as_i64)
        .filter(|ms| *ms > 0)
}

pub(super) fn refresh_token(credentials: &Value) -> Option<String> {
    str_field(oauth(credentials)?, "refreshToken")
}

impl AgentAccounts {
    pub(super) fn login_dir(&self, slot_id: &str) -> PathBuf {
        self.inner.config.root_dir().join(LOGIN_DIRS).join(slot_id)
    }

    fn login_file(&self, slot_id: &str) -> PathBuf {
        self.login_dir(slot_id).join(CREDENTIALS_FILE)
    }

    /// What the folder holds now (`None`: no folder, or the CLI wiped it).
    pub(super) fn read_login(&self, slot_id: &str) -> Option<Value> {
        let creds = read_json(&self.login_file(slot_id))?;
        (oauth(&creds).is_some_and(|o| str_field(o, "accessToken").is_some())).then_some(creds)
    }

    /// `slot` as the folder has it now: the folder's keys over the slot's.
    /// Every read of a Claude slot goes through here, so whoever refreshed
    /// last (the CLI in a chat, or zeron) is who everyone sees.
    pub(super) fn overlay_login(&self, slot: &mut Slot) {
        if slot.harness != HarnessId::ClaudeCode {
            return;
        }
        let Some(Value::Object(from)) = self.read_login(&slot.id) else {
            return;
        };
        if let Some(to) = slot.credentials.as_object_mut() {
            to.extend(from);
        }
    }

    /// The folder of `slot`, created from the slot's login when it has none
    /// yet (never over a folder that exists: the CLI may have refreshed in it).
    pub(super) fn ensure_login_dir(&self, slot: &Slot) -> Result<PathBuf, EngineError> {
        let dir = self.login_dir(&slot.id);
        private_dir(&dir)?;
        let file = dir.join(CREDENTIALS_FILE);
        if file.exists() {
            return Ok(dir);
        }
        let bytes = serde_json::to_vec_pretty(&slot.credentials)
            .map_err(|e| EngineError::Other(format!("serialize login: {e}")))?;
        match stage_file_atomic(&file, &bytes, true)?.persist_noclobber(&file) {
            Ok(_) => {}
            // Made by a racing run meanwhile: theirs stands.
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.error.into()),
        }
        Ok(dir)
    }

    /// Keeps the folder in step with a slot written elsewhere (a re-login of
    /// the account), but only forward: a slot older than what the CLI saved in
    /// the folder never replaces it.
    pub(super) fn sync_login_dir(&self, slot: &Slot) {
        if slot.harness != HarnessId::ClaudeCode {
            return;
        }
        let Some(held) = self.read_login(&slot.id) else {
            return;
        };
        if refresh_token(&held) == refresh_token(&slot.credentials)
            || expires_at(&slot.credentials) <= expires_at(&held)
        {
            return;
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&slot.credentials)
            && let Err(err) = write_file_atomic(&self.login_file(&slot.id), &bytes, true)
        {
            tracing::warn!(slot = %slot.id, error = %err, "could not update a Claude login folder");
        }
    }

    /// `slot_id` is the live login (or was removed): its folder goes. With
    /// `fold_back`, what the folder holds is first saved into the slot, so
    /// the newest pair survives. Holds the CLI's refresh lock while it does,
    /// so no refresh is in flight when the folder goes.
    pub(super) async fn retire_login_dir(&self, slot_id: &str, fold_back: bool, why: &str) {
        let dir = self.login_dir(slot_id);
        if !dir.exists() {
            return;
        }
        let lock = match RefreshLock::acquire(&dir, LOCK_WAIT).await {
            Ok(lock) => lock,
            Err(err) => {
                tracing::warn!(slot = %slot_id, error = %err,
                    "a Claude login folder is busy; keeping it for now");
                return;
            }
        };
        if fold_back
            && let Some(slot) = self.read_slot(HarnessId::ClaudeCode, slot_id)
            && let Err(err) = self.write_slot(&slot)
        {
            tracing::warn!(slot = %slot_id, error = %err, "could not fold a Claude login back");
        }
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => tracing::info!(slot = %slot_id, why, "Claude login folder removed"),
            Err(err) => tracing::warn!(slot = %slot_id, error = %err,
                "could not remove a Claude login folder"),
        }
        drop(lock);
    }

    /// First start with login folders: every saved Claude login that isn't
    /// the live one moves into its folder (copied: the slot file keeps the
    /// pair it was saved with, which the folder's newer credentials override).
    /// Nothing is refreshed, so no refresh token is spent. Done once
    /// (`claude-logins/.migrated`); a login that appears later gets its
    /// folder at its first run.
    pub(super) fn migrate_login_dirs(&self, live_key: Option<&str>) {
        if !login_dir_usable() {
            return;
        }
        let root = self.inner.config.root_dir().join(LOGIN_DIRS);
        let mark = root.join(MIGRATED_MARK);
        if mark.exists() || private_dir(&root).is_err() {
            return;
        }
        let mut moved = 0;
        for slot in self.read_slots(HarnessId::ClaudeCode) {
            if live_key == Some(slot.account_key.as_str())
                || oauth(&slot.credentials).is_none_or(|o| str_field(o, "refreshToken").is_none())
            {
                continue;
            }
            match self.ensure_login_dir(&slot) {
                Ok(_) => moved += 1,
                Err(err) => tracing::warn!(slot = %slot.id, error = %err,
                    "could not move a Claude login into its folder"),
            }
        }
        if std::fs::write(&mark, b"").is_ok() {
            tracing::info!(moved, "Claude logins moved into folders of their own");
        }
    }

    /// What a chat on the saved account `slot` gets (`None`: the live login,
    /// as without a pin or rotation). `Err`: the login is dead (signed out).
    pub(super) async fn claude_login_run(
        &self,
        slot: &Slot,
    ) -> Result<Option<LoginRun>, ProbeError> {
        if !self.claude_slot_is_not_live(slot).await {
            return Ok(None);
        }
        let missing = ProbeError::NoCredentials {
            why: NoCredentials::Missing,
        };
        let slot = self
            .read_slot(slot.harness, &slot.id)
            .unwrap_or_else(|| slot.clone());
        oauth(&slot.credentials)
            .and_then(|o| str_field(o, "refreshToken"))
            .ok_or(missing)?;
        let dir = self
            .ensure_login_dir(&slot)
            .map_err(|_| ProbeError::NoCredentials {
                why: NoCredentials::Missing,
            })?;
        // The CLI refreshes its own token; zeron only makes sure a login that
        // has already expired is still alive, so a dead one reads as signed
        // out here instead of as a failed turn.
        if expires_at(&slot.credentials).is_none_or(|at| at - now_ms() < START_MARGIN_MS)
            && self.inner.endpoints.allow_slot_refresh
        {
            match self.refresh_claude_slot(&slot).await {
                Err(error @ ProbeError::Unauthorized { .. }) => return Err(error),
                // Another refresh is in flight, or this one failed for a
                // reason that says nothing about the login: the CLI tries
                // for itself.
                Ok(_) | Err(_) => {}
            }
        }
        Ok(Some(LoginRun {
            env: BTreeMap::from([(
                SECURE_STORAGE_ENV.to_owned(),
                dir.to_string_lossy().into_owned(),
            )]),
            // Stable across the CLI's own refreshes: only a change of
            // account (or of mechanism) replaces a parked session.
            credential: format!("login:{}", slot.id),
        }))
    }

    /// Refresh the saved login `slot` while holding the CLI's refresh lock on
    /// its folder, after reading the folder again: a refresh the CLI made
    /// meanwhile is adopted, never spent twice. `Ok(None)`: the CLI is still
    /// refreshing (the folder holds the result by the next read).
    pub(super) async fn refresh_login_dir(
        &self,
        slot: &Slot,
    ) -> Result<Option<String>, ProbeError> {
        let dir = self
            .ensure_login_dir(slot)
            .map_err(|_| ProbeError::NoCredentials {
                why: NoCredentials::Missing,
            })?;
        let Ok(_lock) = RefreshLock::acquire(&dir, LOCK_WAIT).await else {
            return Ok(None);
        };
        let current = self
            .read_slot(slot.harness, &slot.id)
            .unwrap_or_else(|| slot.clone());
        if refresh_token(&current.credentials) != refresh_token(&slot.credentials)
            && let Some(access) =
                oauth(&current.credentials).and_then(|o| str_field(o, "accessToken"))
        {
            tracing::info!(slot = %slot.id, "a Claude login was already refreshed by its chat; using that");
            return Ok(Some(access));
        }
        self.refresh_claude_slot_once(&current).await.map(Some)
    }
}

#[cfg(test)]
mod tests;
