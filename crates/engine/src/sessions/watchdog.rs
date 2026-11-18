//! The stall watchdog: what keeps "the agent stopped answering" from being
//! silent.
//!
//! Three things can leave a chat looking busy with nothing behind it, and a
//! message sent to it then goes nowhere (2026-11-18: six chats showed working
//! with one agent process alive, and a message produced no turn until the
//! engine was restarted):
//!
//! * a status row that says working with no run entry behind it;
//! * a run entry whose agent process is gone and has been silent a while;
//! * a message a parked run accepted that no turn ever started on.
//!
//! Each is cleared with a visible note in the chat instead of silence, and
//! the last two re-dispatch what the dead run held. Every run's start and
//! drop is logged (`run started` / `run dropped`), the run table can be read
//! with [`SessionsEngine::run_report`] (the `ListRuns` RPC), and the
//! watchdog logs that table whenever it fires, so the next occurrence
//! leaves its own evidence.
//!
//! This is not a stall timeout on quiet runs: a run is only judged when its
//! agent process is *known* to be gone, or when a parked run took a message
//! and showed no sign of life at all.

use super::*;

/// How often the watchdog looks.
const TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// A working row with no run behind it must be this stale (a live run
/// restamps its row every 30 s) before it is cleared: a run being made
/// meanwhile has not stamped yet.
pub(super) const ORPHAN_GRACE: std::time::Duration = std::time::Duration::from_secs(60);

/// A run whose agent process is gone must also have shown no sign of life
/// for this long.
pub(super) const SILENT_AFTER: std::time::Duration = std::time::Duration::from_secs(3 * 60);

/// A message sent to a parked run starts a turn within this long, or the run
/// is presumed stuck. Its agent process still being alive stretches it to
/// [`TURN_START_ALIVE`]: a slow provider answers late, a wedged runtime
/// never does.
pub(super) const TURN_START: std::time::Duration = std::time::Duration::from_secs(30);
pub(super) const TURN_START_ALIVE: std::time::Duration = std::time::Duration::from_secs(120);

/// A new run's first sign of life: a cold start resumes a whole session.
pub(super) const FRESH_START: std::time::Duration = std::time::Duration::from_secs(120);
pub(super) const FRESH_START_ALIVE: std::time::Duration = std::time::Duration::from_secs(240);

/// The note a turn the watchdog gave up on ends with.
pub const STALL_NOTE: &str = "Agent stopped unexpectedly — send again";

/// The note when its message is being sent again.
const RESTART_NOTE: &str = "Agent unresponsive, restarting";

/// The note when sending it again did not help either.
const UNRESPONSIVE_NOTE: &str = "Agent unresponsive — send again";

/// One live run, as the run report lists it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub chat_id: String,
    pub run_id: String,
    pub harness: String,
    /// Seconds since the run's entry was made.
    pub age_s: u64,
    /// Seconds since its last sign of life.
    pub silent_s: u64,
    /// The harness stream is open.
    pub started: bool,
    pub agent_pid: Option<u32>,
    /// `None`: no pid known, or the platform can't tell.
    pub agent_alive: Option<bool>,
    pub open_tools: usize,
    pub background: bool,
    /// Messages accepted into the run's mailbox that no turn has confirmed.
    pub pending_steers: usize,
    pub status: Option<String>,
}

/// A status row that is not idle, as the run report lists it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    pub chat_id: String,
    pub status: String,
    /// Seconds since the row was last stamped.
    pub age_s: i64,
    pub has_run: bool,
}

/// The engine's run table and the chats that look busy.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub runs: Vec<RunInfo>,
    pub statuses: Vec<StatusInfo>,
    /// Chats working with no run behind them (what the watchdog clears).
    pub working_without_run: Vec<String>,
}

impl SessionsEngine {
    /// The run table, with each run's age and how long it has been silent,
    /// and every non-idle status row.
    pub fn run_report(&self) -> RunReport {
        let now = mono_ms();
        let statuses = lock(&self.inner.statuses).clone();
        let runs: Vec<RunInfo> = lock(&self.inner.runs)
            .iter()
            .map(|(chat_id, run)| {
                let pid = run.turn.agent_pid();
                RunInfo {
                    chat_id: chat_id.clone(),
                    run_id: run.run_id.clone(),
                    harness: format!("{:?}", run.runtime_config.harness_id),
                    age_s: run.born.elapsed().as_secs(),
                    silent_s: u64::try_from((now - run.activity.at_ms()) / 1000).unwrap_or(0),
                    started: run.started.load(std::sync::atomic::Ordering::Acquire),
                    agent_pid: pid,
                    agent_alive: pid.and_then(zeron_harness::process::process_alive),
                    open_tools: run.open_tools.load(std::sync::atomic::Ordering::Acquire),
                    background: run.turn.background_live(),
                    pending_steers: lock(&run.routed_steers).len(),
                    status: statuses.get(chat_id).map(|s| format!("{:?}", s.status)),
                }
            })
            .collect();
        let live: std::collections::HashSet<&str> =
            runs.iter().map(|run| run.chat_id.as_str()).collect();
        let now_utc = Utc::now();
        let mut rows: Vec<StatusInfo> = statuses
            .values()
            .filter(|s| s.status != SessionStatus::Idle)
            .map(|s| StatusInfo {
                chat_id: s.chat_id.clone(),
                status: format!("{:?}", s.status),
                age_s: (now_utc - s.updated_at).num_seconds(),
                has_run: live.contains(s.chat_id.as_str()),
            })
            .collect();
        rows.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
        let working_without_run = rows
            .iter()
            .filter(|row| row.status == "Working" && !row.has_run)
            .map(|row| row.chat_id.clone())
            .collect();
        let mut runs = runs;
        runs.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
        RunReport {
            runs,
            statuses: rows,
            working_without_run,
        }
    }

    /// Start the watchdog's loop (once, at engine assembly). It holds the
    /// engine weakly: a retired engine's loop ends with it.
    pub fn start_watchdog(&self) {
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(inner) = inner.upgrade() else { return };
                if inner
                    .shutting_down
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    return;
                }
                let engine = SessionsEngine { inner };
                engine.watchdog_tick(ORPHAN_GRACE, SILENT_AFTER).await;
            }
        });
    }

    /// One look. Returns how many chats it acted on.
    pub(crate) async fn watchdog_tick(
        &self,
        orphan_grace: std::time::Duration,
        silent_after: std::time::Duration,
    ) -> usize {
        let orphans = self.orphaned_statuses(orphan_grace);
        let dead = self.dead_runs(silent_after);
        if orphans.is_empty() && dead.is_empty() {
            return 0;
        }
        // The evidence first: the table as the watchdog found it.
        match serde_json::to_string(&self.run_report()) {
            Ok(report) => {
                tracing::warn!(
                    orphans = orphans.len(),
                    dead_runs = dead.len(),
                    %report,
                    "stall watchdog fired"
                );
            }
            Err(err) => tracing::warn!(error = %err, "stall watchdog fired; report failed"),
        }
        let mut acted = 0;
        for chat_id in orphans {
            self.clear_orphan(&chat_id);
            acted += 1;
        }
        for (chat_id, run_id, harness_id) in dead {
            self.recover_dead_run(&chat_id, &run_id, harness_id).await;
            acted += 1;
        }
        acted
    }

    /// Chats whose row says working, stale past `grace`, with no run, no
    /// revival on its way and no warm start behind them.
    fn orphaned_statuses(&self, grace: std::time::Duration) -> Vec<String> {
        let now = Utc::now();
        let grace = chrono::Duration::from_std(grace).unwrap_or(chrono::Duration::seconds(60));
        let stale: Vec<String> = {
            let statuses = lock(&self.inner.statuses);
            let runs = lock(&self.inner.runs);
            statuses
                .values()
                .filter(|s| {
                    s.status == SessionStatus::Working
                        && !runs.contains_key(&s.chat_id)
                        && now - s.updated_at >= grace
                })
                .map(|s| s.chat_id.clone())
                .collect()
        };
        stale
            .into_iter()
            .filter(|chat| {
                !self.revival_pending(chat) && !lock(&self.inner.revivals_starting).contains(chat)
            })
            .collect()
    }

    /// Runs that have shown no sign of life for `silent_after` and have no
    /// agent behind them: its process is known to be gone; or Claude's, which
    /// reports its pid for as long as the process lives, reports none on an
    /// open stream; or the run never opened its stream at all (wedged on its
    /// way to spawning; a Cloud machine's setup is allowed its time).
    fn dead_runs(&self, silent_after: std::time::Duration) -> Vec<(String, String, HarnessId)> {
        let now = mono_ms();
        let silent_ms = i64::try_from(silent_after.as_millis()).unwrap_or(i64::MAX);
        lock(&self.inner.runs)
            .iter()
            .filter(|(_, run)| {
                if run.voice_active.live() || now - run.activity.at_ms() < silent_ms {
                    return false;
                }
                if !run.started.load(std::sync::atomic::Ordering::Acquire) {
                    return run.credential.is_none() && run.born.elapsed() >= silent_after;
                }
                match run.turn.agent_pid() {
                    Some(pid) => zeron_harness::process::process_alive(pid) == Some(false),
                    None => run.runtime_config.harness_id == HarnessId::ClaudeCode,
                }
            })
            .map(|(chat, run)| {
                (
                    chat.clone(),
                    run.run_id.clone(),
                    run.runtime_config.harness_id,
                )
            })
            .collect()
    }

    /// End a turn nothing is running with a note in the transcript and an
    /// errored status.
    fn clear_orphan(&self, chat_id: &str) {
        tracing::warn!(chat = %chat_id, "working with no run behind it; clearing");
        self.end_turn_with_note(chat_id, STALL_NOTE, "");
    }

    /// Write `note` where the turn's reply would be (the user's last message
    /// keyed by `key` when nothing was streaming), tell the journal's
    /// subscribers, and settle the status as errored.
    fn end_turn_with_note(&self, chat_id: &str, note: &str, key_suffix: &str) {
        if let Ok(handle) = self.doc_handle(chat_id) {
            let stamped = handle
                .mark_abandoned_streams(note)
                .map(|stamped| stamped.len())
                .unwrap_or(0);
            if stamped == 0
                && let Some(last) = handle
                    .doc()
                    .read_entries()
                    .unwrap_or_default()
                    .into_iter()
                    .rev()
                    .find(|entry| entry.role == MessageRole::User)
            {
                let key = format!("{}{key_suffix}", last.id);
                if let Err(err) = handle.note_turn_failure(&key, note) {
                    tracing::warn!(chat = %chat_id, error = %err, "stall note failed");
                }
            }
        }
        self.inner.publish(
            chat_id,
            &AgentEvent::Done {
                status: DoneStatus::Errored,
                result: None,
                error: Some(note.to_owned()),
                session_id: None,
            },
        );
        self.inner.set_status_with_completion(
            chat_id,
            SessionStatus::Errored,
            false,
            None,
            Some(DoneStatus::Errored),
        );
    }

    /// Drop a run whose agent is gone, say so in the chat, and send again
    /// what it had accepted.
    async fn recover_dead_run(&self, chat_id: &str, run_id: &str, harness_id: HarnessId) {
        let in_flight = self.turn_in_flight(chat_id);
        tracing::warn!(chat = %chat_id, run_id = %run_id, in_flight, "run's agent process is gone; dropping it");
        let orphans = self
            .force_drop(chat_id, run_id, "watchdog: agent process gone")
            .await;
        if in_flight {
            self.end_turn_with_note(chat_id, STALL_NOTE, "");
        }
        spawn_orphan_redispatch(&self.inner, chat_id, harness_id, orphans);
    }

    /// Take the run's unconfirmed messages, end the run, and remove its entry
    /// whatever the run task does: one wedged on a lock or an await would
    /// leave it listed forever. Returns the messages to send again.
    pub(super) async fn force_drop(
        &self,
        chat_id: &str,
        run_id: &str,
        reason: &str,
    ) -> Vec<RoutedSteer> {
        // Ours before the run task's: it keeps only what a Stop did not cancel.
        let orphans: Vec<RoutedSteer> = lock(&self.inner.runs)
            .get(chat_id)
            .filter(|h| h.run_id == run_id)
            .map(|h| std::mem::take(&mut *lock(&h.routed_steers)).into())
            .unwrap_or_default();
        if self.is_live(chat_id, run_id) {
            let _ = self.terminate(chat_id).await;
        }
        self.inner.remove_run_because(chat_id, run_id, reason);
        orphans
    }

    /// Whether a message sent to `run_id` has had no sign of life from it
    /// since the run showed `seen` of them: `Some(agent alive)` if so, `None`
    /// if the run is fine (gone, answering, or the message was taken).
    fn unanswered(
        &self,
        chat_id: &str,
        run_id: &str,
        message_id: &str,
        seen: u64,
        in_mailbox: bool,
    ) -> Option<Option<bool>> {
        let runs = lock(&self.inner.runs);
        let run = runs.get(chat_id).filter(|h| h.run_id == run_id)?;
        if run.activity.count() != seen {
            return None;
        }
        if in_mailbox
            && !lock(&run.routed_steers)
                .iter()
                .any(|s| s.message_id == message_id)
        {
            return None; // the turn took it
        }
        drop(runs);
        if !self.turn_in_flight(chat_id) {
            return None; // settled (a Stop, an error) in the meantime
        }
        let pid = lock(&self.inner.runs)
            .get(chat_id)
            .filter(|h| h.run_id == run_id)
            .and_then(|h| h.turn.agent_pid());
        Some(pid.and_then(zeron_harness::process::process_alive))
    }

    /// Watch a message just handed to `run_id` (to a parked run's mailbox
    /// when `in_mailbox`, else to a run just started): if no turn shows any
    /// sign of life in time, drop the run and send the message once more.
    pub(super) fn watch_turn_start(
        &self,
        chat_id: &str,
        run_id: &str,
        message_id: String,
        request: RunRequest,
        harness_id: HarnessId,
        in_mailbox: bool,
    ) {
        let (first, alive) = if in_mailbox {
            (TURN_START, TURN_START_ALIVE)
        } else {
            (FRESH_START, FRESH_START_ALIVE)
        };
        self.watch_turn_start_for(
            chat_id, run_id, message_id, request, harness_id, in_mailbox, first, alive,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn watch_turn_start_for(
        &self,
        chat_id: &str,
        run_id: &str,
        message_id: String,
        request: RunRequest,
        harness_id: HarnessId,
        in_mailbox: bool,
        first: std::time::Duration,
        alive: std::time::Duration,
    ) {
        let weak = Arc::downgrade(&self.inner);
        let chat_id = chat_id.to_owned();
        let run_id = run_id.to_owned();
        // Judged against the signs of life the run had as the message went in.
        let Some(seen) = lock(&self.inner.runs)
            .get(&chat_id)
            .filter(|h| h.run_id == run_id)
            .map(|h| h.activity.count())
        else {
            return;
        };
        tokio::spawn(async move {
            tokio::time::sleep(first).await;
            let mut extended = false;
            let engine = loop {
                let Some(inner) = weak.upgrade() else { return };
                let engine = SessionsEngine { inner };
                let Some(agent_alive) =
                    engine.unanswered(&chat_id, &run_id, &message_id, seen, in_mailbox)
                else {
                    return;
                };
                if agent_alive == Some(true) && !extended {
                    extended = true;
                    drop(engine);
                    tokio::time::sleep(alive.saturating_sub(first)).await;
                    continue;
                }
                break engine;
            };
            engine
                .recover_unresponsive(&chat_id, &run_id, message_id, request, harness_id)
                .await;
        });
    }

    /// A run took a message and never started on it: drop the run and send
    /// the message again once. A second failure for the same message ends
    /// the turn with a note instead.
    async fn recover_unresponsive(
        &self,
        chat_id: &str,
        run_id: &str,
        message_id: String,
        request: RunRequest,
        harness_id: HarnessId,
    ) {
        let first_time = lock(&self.inner.stall_retries).insert(message_id.clone());
        tracing::warn!(
            chat = %chat_id,
            run_id = %run_id,
            message = %message_id,
            retrying = first_time,
            report = %serde_json::to_string(&self.run_report()).unwrap_or_default(),
            "run took a message and never started a turn"
        );
        let mut orphans = self
            .force_drop(chat_id, run_id, "watchdog: no turn started")
            .await;
        if !first_time {
            self.end_turn_with_note(chat_id, UNRESPONSIVE_NOTE, "");
            spawn_orphan_redispatch(&self.inner, chat_id, harness_id, orphans);
            return;
        }
        // A visible note the person can read, while the message goes again.
        if let Ok(handle) = self.doc_handle(chat_id)
            && let Err(err) =
                handle.note_turn_failure(&format!("{message_id}-restart"), RESTART_NOTE)
        {
            tracing::warn!(chat = %chat_id, error = %err, "restart note failed");
        }
        if !orphans.iter().any(|s| s.message_id == message_id) {
            orphans.push(RoutedSteer {
                prompt: request.prompt.clone(),
                message_id,
                fork_history: false,
                after_stop: false,
                request: Some(Box::new(request)),
            });
        }
        spawn_orphan_redispatch(&self.inner, chat_id, harness_id, orphans);
    }
}

#[cfg(test)]
#[path = "watchdog_tests.rs"]
mod tests;
