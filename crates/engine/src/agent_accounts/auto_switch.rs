//! Automatic account switching: once the live login of an agent has used the
//! device's threshold (90% by default) of any usage window, the engine
//! switches that agent to the saved account with the most left — the same
//! swap Settings' Switch makes ([`AgentAccounts::activate`]), so running
//! chats pick the new login up from their next turn.
//!
//! - **When.** A background check every [`CHECK_INTERVAL`] while the setting
//!   is on, and soon after a turn starts ([`AutoSwitch::nudge`], at most once
//!   per [`MIN_CHECK_GAP`]). A check probes usage only when some agent has an
//!   account to switch to, through the same throttled, backed-off probes the
//!   accounts page uses.
//! - **Which.** Per agent — per model provider for OpenCode and Pi — the
//!   live login switches once its most-used window not yet reset reaches the
//!   threshold. Usage counts only while fresh ([`FRESH_USAGE`]): a stale
//!   reading never triggers a switch. The target is the switchable account
//!   furthest below the threshold; a reading whose every used window has
//!   reset since counts as unused. Muse's saved accounts are never probed (a
//!   run reports usage only for the account it runs on), so one with no
//!   reading at all is a last resort.
//! - **When not.** Every other account is at the threshold too (the live one
//!   keeps running until a window resets), the live login can't be saved
//!   first (`switchable` false), or the agent keeps its own pool (Hermes) or
//!   one login (Antigravity). A switch the engine refuses (a saved Claude
//!   login whose refresh token is spent) rests that account for
//!   [`REFUSED_COOLDOWN`] and the next one is tried.
//! - **Where.** Off by default, per device (`{data_dir}/account-auto-switch.json`).
//!   Never on a Cloud machine: its logins are the vault's.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;
use zeron_proto::{
    AccountAutoSwitch, AccountAutoSwitchStatus, AccountSwitchEvent, AgentAccount, HarnessId,
};

use super::{AgentAccounts, lock};
use crate::{EngineError, now_ms};

/// Usage moves as turns run; the composer's ring polls at the same pace.
const CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// A turn start asks for a check no more often than this.
const MIN_CHECK_GAP: Duration = Duration::from_secs(60);
/// A reading older than this says nothing about the account now.
const FRESH_USAGE: Duration = Duration::from_secs(15 * 60);
/// How long a refused switch target is passed over.
const REFUSED_COOLDOWN: Duration = Duration::from_secs(30 * 60);

type OnSwitch = Arc<dyn Fn(&AccountSwitchEvent) + Send + Sync>;

#[derive(Clone)]
pub struct AutoSwitch {
    inner: Arc<Inner>,
}

struct Inner {
    accounts: AgentAccounts,
    settings_path: PathBuf,
    cloud: bool,
    settings: Mutex<AccountAutoSwitch>,
    last_switch: tokio::sync::watch::Sender<Option<AccountSwitchEvent>>,
    /// Wakes the worker: a turn started, or the setting changed.
    wake: tokio::sync::Notify,
    /// The setting changed: check at once, whenever the last check ran.
    recheck: std::sync::atomic::AtomicBool,
    /// Switch targets the engine refused, by account id.
    refused: Mutex<HashMap<String, Instant>>,
    on_switch: Mutex<Option<OnSwitch>>,
    shutdown: CancellationToken,
    worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl AutoSwitch {
    pub fn open(accounts: AgentAccounts, data_dir: &Path, cloud: bool) -> Self {
        let settings_path = data_dir.join("account-auto-switch.json");
        let settings = std::fs::read(&settings_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            inner: Arc::new(Inner {
                accounts,
                settings_path,
                cloud,
                settings: Mutex::new(settings),
                last_switch: tokio::sync::watch::Sender::new(None),
                wake: tokio::sync::Notify::new(),
                recheck: std::sync::atomic::AtomicBool::new(false),
                refused: Mutex::new(HashMap::new()),
                on_switch: Mutex::new(None),
                shutdown: CancellationToken::new(),
                worker: Mutex::new(None),
            }),
        }
    }

    fn settings(&self) -> AccountAutoSwitch {
        let mut settings = *lock(&self.inner.settings);
        settings.enabled &= !self.inner.cloud;
        settings
    }

    pub fn status(&self) -> AccountAutoSwitchStatus {
        AccountAutoSwitchStatus {
            settings: self.settings(),
            last_switch: self.inner.last_switch.borrow().clone(),
        }
    }

    pub fn set(&self, settings: AccountAutoSwitch) -> Result<AccountAutoSwitchStatus, EngineError> {
        if self.inner.cloud {
            return Err(EngineError::Other(
                "A Cloud machine uses the account chosen for Cloud — switch it there.".into(),
            ));
        }
        let next = AccountAutoSwitch {
            enabled: settings.enabled,
            threshold_percent: settings.threshold_percent.clamp(
                AccountAutoSwitch::MIN_THRESHOLD,
                AccountAutoSwitch::MAX_THRESHOLD,
            ),
        };
        let parent = self
            .inner
            .settings_path
            .parent()
            .ok_or_else(|| EngineError::Other("settings have no folder".into()))?;
        std::fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(&mut temp, &next)
            .map_err(|e| EngineError::Other(e.to_string()))?;
        temp.write_all(b"\n")?;
        temp.persist(&self.inner.settings_path)
            .map_err(|e| EngineError::Io(e.error))?;
        *lock(&self.inner.settings) = next;
        self.inner
            .recheck
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.inner.wake.notify_one();
        Ok(self.status())
    }

    /// The latest automatic switch, as it happens (desktop banners).
    pub fn watch(&self) -> tokio::sync::watch::Receiver<Option<AccountSwitchEvent>> {
        self.inner.last_switch.subscribe()
    }

    /// Called after every switch this makes (the engine retires voice, as a
    /// manual switch does).
    pub fn set_on_switch(&self, on_switch: OnSwitch) {
        *lock(&self.inner.on_switch) = Some(on_switch);
    }

    /// A turn started: usage is moving, check soon.
    pub fn nudge(&self) {
        if self.settings().enabled {
            self.inner.wake.notify_one();
        }
    }

    /// Run the background check until [`Self::shutdown`]. A no-op outside a
    /// Tokio runtime (bare synchronous assemblies) and on a Cloud machine.
    pub fn start(&self) {
        if self.inner.cloud || tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let mut worker = lock(&self.inner.worker);
        if worker.is_some() {
            return;
        }
        let this = self.clone();
        *worker = Some(tokio::spawn(async move { this.run().await }));
    }

    pub async fn shutdown(&self) {
        self.inner.shutdown.cancel();
        let worker = lock(&self.inner.worker).take();
        if let Some(worker) = worker {
            let _ = worker.await;
        }
    }

    async fn run(&self) {
        let mut last_check: Option<tokio::time::Instant> = None;
        loop {
            let due = match (self.settings().enabled, last_check) {
                (false, _) => None,
                (true, None) => Some(tokio::time::Instant::now()),
                (true, Some(at)) => Some(at + CHECK_INTERVAL),
            };
            tokio::select! {
                _ = self.inner.shutdown.cancelled() => return,
                _ = self.inner.wake.notified() => {}
                _ = async {
                    match due {
                        Some(due) => tokio::time::sleep_until(due).await,
                        None => std::future::pending().await,
                    }
                } => {}
            }
            if !self.settings().enabled {
                continue;
            }
            let recheck = self
                .inner
                .recheck
                .swap(false, std::sync::atomic::Ordering::SeqCst);
            if !recheck && last_check.is_some_and(|at| at.elapsed() < MIN_CHECK_GAP) {
                continue;
            }
            last_check = Some(tokio::time::Instant::now());
            tokio::select! {
                _ = self.inner.shutdown.cancelled() => return,
                result = self.check() => if let Err(err) = result {
                    tracing::warn!(error = %err, "account auto-switch: check failed");
                }
            }
        }
    }

    /// One pass: probe usage when any agent has an account to switch to,
    /// then switch every live login at the threshold. The switches made.
    pub(crate) async fn check(&self) -> Result<Vec<AccountSwitchEvent>, EngineError> {
        let settings = self.settings();
        if !settings.enabled {
            return Ok(Vec::new());
        }
        let threshold = settings.threshold_fraction();
        // A plain list is offline-fast; only an agent with somewhere to go
        // is worth a round of provider calls.
        if !has_alternatives(&self.inner.accounts.list(false).await?.accounts) {
            return Ok(Vec::new());
        }
        let snapshot = self.inner.accounts.list(true).await?;
        let plans = plan_switches(&snapshot.accounts, threshold, Utc::now(), now_ms());
        let mut made = Vec::new();
        for plan in plans {
            if !self.settings().enabled {
                break;
            }
            for (target, _) in &plan.candidates {
                if self.resting(&target.id) {
                    continue;
                }
                match self.inner.accounts.activate(plan.harness, &target.id).await {
                    Ok(_) => {
                        let event = AccountSwitchEvent {
                            harness: plan.harness,
                            provider: plan.from.provider.clone(),
                            from_id: plan.from.id.clone(),
                            from_email: plan.from.email.clone(),
                            to_id: target.id.clone(),
                            to_email: target.email.clone(),
                            used_fraction: plan.used,
                            at: now_ms(),
                        };
                        tracing::info!(
                            harness = ?event.harness,
                            provider = ?event.provider,
                            used = event.used_fraction,
                            "account auto-switch: switched to the account with the most left"
                        );
                        let on_switch = lock(&self.inner.on_switch).clone();
                        if let Some(on_switch) = on_switch {
                            on_switch(&event);
                        }
                        self.inner.last_switch.send_replace(Some(event.clone()));
                        made.push(event);
                        break;
                    }
                    Err(err) => {
                        tracing::warn!(harness = ?plan.harness, error = %err,
                            "account auto-switch: switch refused; trying the next account");
                        lock(&self.inner.refused).insert(target.id.clone(), Instant::now());
                    }
                }
            }
        }
        Ok(made)
    }

    fn resting(&self, account_id: &str) -> bool {
        let mut refused = lock(&self.inner.refused);
        refused.retain(|_, at| at.elapsed() < REFUSED_COOLDOWN);
        refused.contains_key(account_id)
    }
}

/// One live login at the threshold and where it could go, best first.
#[derive(Debug)]
pub(crate) struct SwitchPlan<'a> {
    pub harness: HarnessId,
    pub from: &'a AgentAccount,
    /// The live login's binding usage.
    pub used: f32,
    /// Accounts below the threshold, most left first (`None`: never read).
    pub candidates: Vec<(&'a AgentAccount, Option<f32>)>,
}

/// The single-choice groups auto-switch works on: per agent, per model
/// provider for agents that keep one login per provider. Engine order.
fn groups(accounts: &[AgentAccount]) -> Vec<Vec<&AgentAccount>> {
    let mut groups: Vec<((HarnessId, Option<&str>), Vec<&AgentAccount>)> = Vec::new();
    for account in accounts {
        if !zeron_proto::providers::switches_accounts(account.harness) {
            continue;
        }
        let key = (account.harness, account.provider.as_deref());
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, group)) => group.push(account),
            None => groups.push((key, vec![account])),
        }
    }
    groups.into_iter().map(|(_, group)| group).collect()
}

/// Whether any live login has a saved account to switch to. Pure.
fn has_alternatives(accounts: &[AgentAccount]) -> bool {
    groups(accounts).iter().any(|group| {
        group.iter().any(|a| a.active && a.switchable)
            && group.iter().any(|a| !a.active && a.switchable)
    })
}

/// What an account's usage says now: its most-used window not yet reset.
/// `None` when unknown — never read, or read too long ago to count (unless
/// every window it had used has reset since: nothing of that is left). Pure.
fn usage_now(account: &AgentAccount, now: DateTime<Utc>, now_ms: i64) -> Option<f32> {
    if account.usage_windows.is_empty() {
        return None;
    }
    let open: Vec<f32> = account
        .usage_windows
        .iter()
        .filter(|w| w.resets_at.is_none_or(|at| at > now))
        .map(|w| w.used_fraction.clamp(0.0, 1.0))
        .collect();
    let used = open.iter().copied().fold(0.0, f32::max);
    let fresh = account
        .usage_fetched_at
        .is_some_and(|at| now_ms.saturating_sub(at) <= FRESH_USAGE.as_millis() as i64);
    (fresh || used <= 0.0).then_some(used)
}

/// Every live login at `threshold` with a switchable account below it. Pure.
pub(crate) fn plan_switches(
    accounts: &[AgentAccount],
    threshold: f32,
    now: DateTime<Utc>,
    now_ms: i64,
) -> Vec<SwitchPlan<'_>> {
    let mut plans = Vec::new();
    for group in groups(accounts) {
        let Some(from) = group.iter().copied().find(|a| a.active && a.switchable) else {
            continue;
        };
        // A stale reading never triggers a switch: it may predate a reset.
        let Some(used) = usage_now(from, now, now_ms).filter(|used| *used >= threshold) else {
            continue;
        };
        let mut candidates: Vec<(&AgentAccount, Option<f32>)> = group
            .iter()
            .copied()
            .filter(|a| !a.active && a.switchable)
            .filter_map(|a| match usage_now(a, now, now_ms) {
                Some(left) if left < threshold => Some((a, Some(left))),
                Some(_) => None,
                // Muse's saved accounts are never probed: one never run is
                // worth trying once every account with a reading is spent.
                None if a.harness == HarnessId::Muse && a.usage_fetched_at.is_none() => {
                    Some((a, None))
                }
                None => None,
            })
            .collect();
        // Stable: equal usage keeps engine (creation) order.
        candidates.sort_by(|(_, a), (_, b)| match (a, b) {
            (Some(a), Some(b)) => a.total_cmp(b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        if !candidates.is_empty() {
            plans.push(SwitchPlan {
                harness: from.harness,
                from,
                used,
                candidates,
            });
        }
    }
    plans
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeron_proto::AgentUsageWindow;

    const NOW_MS: i64 = 1_800_000_000_000;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp_millis(NOW_MS).unwrap()
    }

    fn window(used: f32, resets_in_mins: Option<i64>) -> AgentUsageWindow {
        AgentUsageWindow {
            label: "Session".into(),
            used_fraction: used,
            resets_at: resets_in_mins.map(|mins| now() + chrono::Duration::minutes(mins)),
        }
    }

    fn account(
        harness: HarnessId,
        id: &str,
        active: bool,
        windows: Vec<AgentUsageWindow>,
    ) -> AgentAccount {
        AgentAccount {
            id: id.into(),
            harness,
            email: Some(format!("{id}@example.com")),
            plan_label: None,
            active,
            usage_fetched_at: (!windows.is_empty()).then_some(NOW_MS - 60_000),
            usage_windows: windows,
            usage_error: None,
            display_name: None,
            organization: None,
            auth_kind: None,
            switchable: true,
            saved_at: None,
            provider: None,
        }
    }

    fn plan(accounts: &[AgentAccount], threshold: f32) -> Vec<(HarnessId, String, Vec<String>)> {
        plan_switches(accounts, threshold, now(), NOW_MS)
            .into_iter()
            .map(|plan| {
                (
                    plan.harness,
                    plan.from.id.clone(),
                    plan.candidates.iter().map(|(a, _)| a.id.clone()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn a_live_login_at_the_threshold_moves_to_the_account_with_the_most_left() {
        let accounts = [
            account(
                HarnessId::ClaudeCode,
                "a",
                true,
                vec![window(0.4, Some(60)), window(0.91, Some(600))],
            ),
            account(
                HarnessId::ClaudeCode,
                "b",
                false,
                vec![window(0.7, Some(60))],
            ),
            account(
                HarnessId::ClaudeCode,
                "c",
                false,
                vec![window(0.2, Some(60))],
            ),
            account(
                HarnessId::ClaudeCode,
                "d",
                false,
                vec![window(0.95, Some(60))],
            ),
        ];
        assert_eq!(
            plan(&accounts, 0.9),
            vec![(
                HarnessId::ClaudeCode,
                "a".into(),
                vec!["c".into(), "b".into()]
            )],
            "the most-used window binds; a spent account is never a target"
        );
        assert!(
            plan(&accounts, 0.95).is_empty(),
            "below the threshold: stay"
        );
    }

    #[test]
    fn nothing_switches_when_every_other_account_is_spent_too() {
        let accounts = [
            account(HarnessId::Codex, "a", true, vec![window(0.97, Some(60))]),
            account(HarnessId::Codex, "b", false, vec![window(0.92, Some(60))]),
        ];
        assert!(plan(&accounts, 0.9).is_empty());
    }

    #[test]
    fn stale_readings_never_trigger_and_reset_windows_count_as_unused() {
        let mut stale_active = account(HarnessId::Codex, "a", true, vec![window(0.99, Some(60))]);
        stale_active.usage_fetched_at = Some(NOW_MS - 60 * 60_000);
        let spare = account(HarnessId::Codex, "b", false, vec![window(0.1, Some(60))]);
        assert!(
            plan(&[stale_active, spare.clone()], 0.9).is_empty(),
            "an hour-old 99% may predate a reset"
        );

        // A live login whose spent window has reset is no longer spent.
        let reset_active = account(HarnessId::Codex, "a", true, vec![window(0.99, Some(-5))]);
        assert!(plan(&[reset_active, spare], 0.9).is_empty());

        // A saved account read at 100% long ago, its window reset since:
        // nothing of that is left — a target.
        let active = account(HarnessId::Codex, "a", true, vec![window(0.93, Some(60))]);
        let mut rested = account(HarnessId::Codex, "b", false, vec![window(1.0, Some(-30))]);
        rested.usage_fetched_at = Some(NOW_MS - 6 * 60 * 60_000);
        let mut stale = account(HarnessId::Codex, "c", false, vec![window(0.1, Some(30))]);
        stale.usage_fetched_at = Some(NOW_MS - 6 * 60 * 60_000);
        let unread = account(HarnessId::Codex, "d", false, vec![]);
        assert_eq!(
            plan(&[active, rested, stale, unread], 0.9),
            vec![(HarnessId::Codex, "a".into(), vec!["b".into()])],
            "an unreset stale reading, or none, is unknown"
        );
    }

    #[test]
    fn per_provider_agents_switch_within_each_provider() {
        let provider = |mut a: AgentAccount, p: &str| {
            a.provider = Some(p.into());
            a
        };
        let accounts = [
            provider(
                account(HarnessId::Pi, "oa", true, vec![window(0.95, Some(60))]),
                "openai",
            ),
            provider(
                account(HarnessId::Pi, "an", true, vec![window(0.95, Some(60))]),
                "anthropic",
            ),
            provider(
                account(HarnessId::Pi, "ob", false, vec![window(0.1, Some(60))]),
                "openai",
            ),
        ];
        assert_eq!(
            plan(&accounts, 0.9),
            vec![(HarnessId::Pi, "oa".into(), vec!["ob".into()])],
            "an OpenAI login is never a target for Anthropic"
        );
    }

    #[test]
    fn unswitchable_logins_and_pool_agents_are_left_alone() {
        let mut unreadable = account(HarnessId::ClaudeCode, "a", true, vec![window(0.95, None)]);
        unreadable.switchable = false;
        let spare = account(HarnessId::ClaudeCode, "b", false, vec![window(0.1, None)]);
        assert!(
            plan(&[unreadable, spare], 0.9).is_empty(),
            "the live login couldn't be saved first"
        );

        let mut hermes = account(HarnessId::Hermes, "h1", true, vec![window(0.95, None)]);
        hermes.switchable = false;
        let mut hermes_spare = account(HarnessId::Hermes, "h2", false, vec![window(0.1, None)]);
        hermes_spare.switchable = false;
        let accounts = [hermes, hermes_spare];
        assert!(plan(&accounts, 0.9).is_empty());
        assert!(!has_alternatives(&accounts));
    }

    #[test]
    fn a_muse_account_never_run_is_the_last_resort() {
        let accounts = [
            account(HarnessId::Muse, "a", true, vec![window(0.95, Some(60))]),
            account(HarnessId::Muse, "fresh", false, vec![]),
            account(HarnessId::Muse, "b", false, vec![window(0.5, Some(60))]),
        ];
        assert_eq!(
            plan(&accounts, 0.9),
            vec![(
                HarnessId::Muse,
                "a".into(),
                vec!["b".into(), "fresh".into()]
            )]
        );
    }

    #[test]
    fn settings_persist_clamped_and_never_turn_on_for_cloud() {
        let data = tempfile::tempdir().unwrap();
        let accounts = AgentAccounts::new(super::super::AgentAccountsConfig::isolated(data.path()));
        let auto = AutoSwitch::open(accounts.clone(), data.path(), false);
        assert_eq!(auto.status().settings, AccountAutoSwitch::default());
        assert!(!auto.status().settings.enabled, "off by default");

        let status = auto
            .set(AccountAutoSwitch {
                enabled: true,
                threshold_percent: 100,
            })
            .unwrap();
        assert_eq!(
            status.settings.threshold_percent,
            AccountAutoSwitch::MAX_THRESHOLD
        );
        let reopened = AutoSwitch::open(accounts.clone(), data.path(), false);
        assert_eq!(
            reopened.status().settings,
            AccountAutoSwitch {
                enabled: true,
                threshold_percent: AccountAutoSwitch::MAX_THRESHOLD,
            },
            "persisted"
        );

        let cloud = AutoSwitch::open(accounts, data.path(), true);
        assert!(!cloud.status().settings.enabled, "never on for Cloud");
        assert!(cloud.set(AccountAutoSwitch::default()).is_err());
    }
}
