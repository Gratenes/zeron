//! Whose login `~/.claude/.credentials.json` holds, when `~/.claude.json`
//! can't be trusted to say.
//!
//! The live Claude login is two files read together: the identity
//! (`~/.claude.json` → `oauthAccount`) and the secret
//! (`~/.claude/.credentials.json`). Nothing ties the two. The Claude CLI
//! rewrites `oauthAccount` from a profile call made with *its own* token
//! whenever its stored profile is stale, and every chat on a saved account
//! (see [`super::logins`]) is a CLI holding a token other than the live one.
//! So a chat on account B stamps B into the shared file while the secret
//! beside it is still the live account A's.
//!
//! Read naively that is a login switch to B: A's token is saved into B's slot
//! (B's own refresh token, single-use and unrecoverable, is gone), B's login
//! folder is removed as "the live login" out from under the chats running on
//! it, and the next list does it again for C.
//!
//! What tells the two cases apart is the secret. A real sign-in (a terminal
//! `claude /login`, a switch) changes the tokens along with the identity; a
//! foreign write changes the identity under tokens that did not change. So:
//!
//! - an identity that changes while the token pair is the same as at the last
//!   look is a foreign write: the identity seen before stands;
//! - a token pair first seen that a saved account already holds (and only
//!   one does) belongs to that account, whatever the file says;
//! - a token pair held by more than one saved account (already overwritten)
//!   is saved into none of them, and the accounts are reported;
//! - the live login's folder goes only when the live token is that
//!   account's: the folder's own token, or a newer one seen twice running.

use std::collections::HashMap;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeron_proto::{AgentAccountWarning, HarnessId};

use super::logins::{expires_at, oauth, refresh_token};
use super::{AgentAccounts, Slot, lock, slot_id_for, str_field};

/// What the last look at the live Claude login found.
#[derive(Default)]
pub(super) struct LiveClaude {
    seen: Option<Seen>,
}

struct Seen {
    /// Of the token pair (never the tokens themselves).
    fingerprint: String,
    /// Who the live login was taken to be.
    key: Option<String>,
    oauth: Value,
    refresh: String,
    expires: Option<i64>,
    /// The same pair and the same account as the look before this one.
    stable: bool,
}

fn token_pair(credentials: &Value) -> Option<(String, String)> {
    let o = oauth(credentials)?;
    let refresh = str_field(o, "refreshToken").filter(|t| !t.is_empty())?;
    let access = str_field(o, "accessToken").filter(|t| !t.is_empty())?;
    Some((refresh, access))
}

fn fingerprint(refresh: &str, access: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(refresh.as_bytes());
    hash.update([0]);
    hash.update(access.as_bytes());
    format!("{:x}", hash.finalize())
}

/// The account key `detect_claude` derives from an `oauthAccount`.
fn identity_key(oauth: &Value) -> Option<String> {
    str_field(oauth, "accountUuid").or_else(|| str_field(oauth, "emailAddress"))
}

/// The `oauthAccount` a saved account stands for.
fn slot_identity(slot: &Slot) -> Value {
    slot.claude_config
        .as_ref()
        .and_then(|c| c.get("oauthAccount"))
        .cloned()
        .unwrap_or_else(
            || json!({ "accountUuid": slot.account_key, "emailAddress": slot.profile.email }),
        )
}

impl AgentAccounts {
    /// The saved Claude accounts (but `except`) whose login holds `refresh`.
    pub(super) fn claude_slots_holding(&self, refresh: &str, except: &str) -> Vec<Slot> {
        self.read_slots(HarnessId::ClaudeCode)
            .into_iter()
            .filter(|s| s.id != except && refresh_token(&s.credentials).as_deref() == Some(refresh))
            .collect()
    }

    /// The identity to take the live Claude login for: what `oauthAccount`
    /// says, unless the secret says otherwise (see the module docs).
    /// `credentials` is `None` when the secret couldn't be read: nothing to
    /// pair the identity with, so it stands and nothing is remembered.
    pub(super) fn settle_claude_identity(
        &self,
        oauth_account: Value,
        credentials: Option<&Value>,
    ) -> Value {
        let Some((refresh, access)) = credentials.and_then(token_pair) else {
            return oauth_account;
        };
        let print = fingerprint(&refresh, &access);
        let claimed = identity_key(&oauth_account);
        let mut chosen = oauth_account;
        let mut live = lock(&self.inner.claude_live);
        let previous = live.seen.as_ref().filter(|s| s.fingerprint == print);
        if let Some(previous) = previous {
            if previous.key != claimed {
                tracing::warn!(was = ?previous.key, claimed = ?claimed,
                    "the Claude identity changed under an unchanged login: a chat's CLI wrote it, not a sign-in; keeping the account the login was seen as");
                chosen = previous.oauth.clone();
            }
        } else if let [owner] = self.claude_slots_holding(&refresh, "").as_slice()
            && Some(&owner.account_key) != claimed.as_ref()
        {
            tracing::warn!(claimed = ?claimed, owner = %owner.id,
                "the live Claude login belongs to another saved account than the identity file names; taking it for its owner");
            chosen = slot_identity(owner);
        }
        let key = identity_key(&chosen);
        let stable = previous.is_some_and(|p| p.key == key);
        live.seen = Some(Seen {
            fingerprint: print,
            key,
            oauth: chosen.clone(),
            refresh,
            expires: credentials.and_then(expires_at),
            stable,
        });
        chosen
    }

    /// The account key of the live Claude login, given the identity file's
    /// claim (`claimed`) and the secret read beside it: the account the last
    /// look took this very login for, when there was one; else the claim.
    /// Read-only: decisions that must not misjudge who is live (a removal, a
    /// refresh that would spend the live refresh token) use it.
    pub(super) fn live_claude_key_for(
        &self,
        credentials: &Value,
        claimed: Option<String>,
    ) -> Option<String> {
        let Some((refresh, access)) = token_pair(credentials) else {
            return claimed;
        };
        let print = fingerprint(&refresh, &access);
        let live = lock(&self.inner.claude_live);
        match live.seen.as_ref() {
            Some(seen) if seen.fingerprint == print => seen.key.clone().or(claimed),
            _ => claimed,
        }
    }

    /// Whether the live Claude login may be saved into `slot_id`: not while
    /// another saved account already holds that very login.
    pub(super) fn claude_live_may_be_saved(&self, slot_id: &str, credentials: &Value) -> bool {
        let Some(refresh) = refresh_token(credentials) else {
            return true;
        };
        let holders = self.claude_slots_holding(&refresh, slot_id);
        if holders.is_empty() {
            return true;
        }
        let ids: Vec<&str> = holders.iter().map(|s| s.id.as_str()).collect();
        tracing::warn!(slot = %slot_id, held_by = ?ids,
            "the live Claude login is already held by another saved account; not saving it here");
        false
    }

    /// The live login's account has no folder of its own: it goes, folded
    /// back into the slot, but only when the live token really is that
    /// account's (see the module docs). Otherwise chats running on the folder
    /// keep it.
    pub(super) async fn retire_live_login_dir(&self, key: &str) {
        let slot_id = slot_id_for(HarnessId::ClaudeCode, key);
        if !self.login_dir(&slot_id).exists() {
            return;
        }
        let verdict = {
            let live = lock(&self.inner.claude_live);
            live.seen
                .as_ref()
                .map(|s| (s.refresh.clone(), s.expires, s.stable))
        };
        let Some((live_refresh, live_expires, stable)) = verdict else {
            // Nothing known of the live secret: nothing to fold back.
            return;
        };
        let retire = match self.read_login(&slot_id) {
            // A wiped or unreadable folder holds nothing worth keeping.
            None => true,
            Some(folder) => {
                refresh_token(&folder).as_deref() == Some(live_refresh.as_str())
                    || (stable
                        && live_expires >= expires_at(&folder)
                        && self
                            .claude_slots_holding(&live_refresh, &slot_id)
                            .is_empty())
            }
        };
        if retire {
            self.retire_login_dir(&slot_id, true, "is the live login")
                .await;
        } else {
            tracing::debug!(slot = %slot_id,
                "keeping a Claude login folder: the live login isn't (yet) known to be this account's");
        }
    }

    /// Saved Claude accounts that hold one and the same login, as their
    /// emails: all but one of them were overwritten with it.
    pub(super) fn duplicate_claude_logins(&self) -> Vec<Vec<String>> {
        let mut by_login: HashMap<String, Vec<String>> = HashMap::new();
        for slot in self.read_slots(HarnessId::ClaudeCode) {
            if let Some(refresh) = refresh_token(&slot.credentials) {
                by_login
                    .entry(refresh)
                    .or_default()
                    .push(slot.profile.email);
            }
        }
        let mut groups: Vec<Vec<String>> = by_login
            .into_values()
            .filter(|emails| emails.len() > 1)
            .collect();
        for group in &mut groups {
            group.sort();
        }
        groups.sort();
        groups
    }

    /// One warning per set of saved Claude accounts holding the same login.
    pub(super) fn duplicate_login_warnings(&self) -> Vec<AgentAccountWarning> {
        self.duplicate_claude_logins()
            .into_iter()
            .map(|emails| AgentAccountWarning {
                harness: HarnessId::ClaudeCode,
                message: format!(
                    "These saved Claude accounts hold the same login: {}. All but one lost \
                     their own — sign each of the others in again.",
                    emails.join(", ")
                ),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
