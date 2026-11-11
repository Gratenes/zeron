//! Fires scheduled messages ([`zeron_proto::Schedule`]).
//!
//! Only a chat's host engine fires its schedules: the rows are synced, so
//! every device's engine sees them, and each acts on the ones for chats whose
//! row names it host. The timer is the engine's own, so it doesn't depend on
//! the chat's agent process: a chat the idle reaper ended, or that never ran
//! on this boot, still gets its message, and the message starts a fresh turn
//! the way any queued send does ([`crate::doc_host::DocHost::queue_message`]).
//!
//! One task sleeps until the next pending schedule is due and is woken by any
//! registry change, so a new or cancelled schedule re-arms it, and a restart
//! rebuilds it from the synced rows. A schedule is stamped fired *before* its
//! message is queued: a crash in between loses the message instead of
//! sending it twice. One found past its grace window (the engine was down
//! when it was due) is stamped missed, not sent.
//!
//! A repeating schedule fires the same way, and its one write both stamps the
//! fire and moves its due time to the next occurrence after now
//! ([`crate::schedule_repeat`]): it ends (`firedAt` stamped) on its last run,
//! at `max_runs` or when nothing is left before `until`. One found past its
//! grace skips to the next occurrence with nothing sent, so a long outage
//! never ends in a burst of catch-up.

use std::time::Duration;

use chrono::{DateTime, Utc};
use zeron_proto::Schedule;

use crate::doc_host::DocHost;
use crate::workspace_host::WorkspaceHost;

/// How late a schedule may still be sent after its time (an engine that was
/// down when it was due sends it on start within this). `ZERON_SCHEDULE_GRACE_SECS`
/// overrides it.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(60 * 60);

/// The longest the task sleeps without looking again, so a clock change or a
/// suspended laptop can't leave a due schedule waiting.
const MAX_NAP: Duration = Duration::from_secs(60);

/// How long a first sweep waits for the registry to sync, so a cancel made
/// elsewhere while this engine was down is seen before anything fires.
const SYNC_WAIT: Duration = Duration::from_secs(20);

pub fn grace_from_env() -> Duration {
    std::env::var("ZERON_SCHEDULE_GRACE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(DEFAULT_GRACE, Duration::from_secs)
}

/// Start the scheduler for this engine; it ends with the returned task.
pub fn spawn(
    workspace: WorkspaceHost,
    doc_host: DocHost,
    grace: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        wait_for_sync(&workspace).await;
        let mut changes = workspace.watch_schedules();
        loop {
            let next = sweep(&workspace, &doc_host, Utc::now(), grace);
            let nap = next
                .map(|at| (at - Utc::now()).to_std().unwrap_or(Duration::ZERO))
                .map_or(MAX_NAP, |wait| wait.min(MAX_NAP));
            tokio::select! {
                changed = changes.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                () = tokio::time::sleep(nap) => {}
            }
        }
    })
}

async fn wait_for_sync(workspace: &WorkspaceHost) {
    if !workspace.edge_expected() {
        return;
    }
    let deadline = tokio::time::Instant::now() + SYNC_WAIT;
    while tokio::time::Instant::now() < deadline {
        if workspace.sync_status().is_some_and(|status| status.synced) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// One pass at `now`: send what's due, stamp what's too late, and say when
/// the next pending schedule hosted here comes due.
pub fn sweep(
    workspace: &WorkspaceHost,
    doc_host: &DocHost,
    now: DateTime<Utc>,
    grace: Duration,
) -> Option<DateTime<Utc>> {
    let grace = chrono::Duration::from_std(grace).unwrap_or(chrono::Duration::hours(1));
    let mut next: Option<DateTime<Utc>> = None;
    for schedule in workspace.read_schedules() {
        if !schedule.is_pending() || !workspace.hosts_chat_row(&schedule.chat_id) {
            continue;
        }
        if schedule.fire_at > now {
            next = Some(next.map_or(schedule.fire_at, |n| n.min(schedule.fire_at)));
            continue;
        }
        if schedule.repeat.is_some() {
            let following = sweep_repeating(workspace, doc_host, &schedule, now, grace);
            if let Some(at) = following {
                next = Some(next.map_or(at, |n| n.min(at)));
            }
            continue;
        }
        if now - schedule.fire_at > grace {
            match workspace.miss_schedule(&schedule.id, now) {
                Ok(true) => tracing::warn!(schedule = %schedule.id, chat = %schedule.chat_id,
                    "scheduled message missed: its time passed while this engine was down"),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(schedule = %schedule.id, %error, "couldn't stamp a missed schedule")
                }
            }
            continue;
        }
        match workspace.fire_schedule(&schedule.id, now) {
            Ok(true) => {
                if let Err(error) = deliver(workspace, doc_host, &schedule) {
                    tracing::warn!(schedule = %schedule.id, chat = %schedule.chat_id, %error,
                        "a scheduled message was taken but couldn't be queued");
                }
            }
            Ok(false) => {}
            Err(error) => {
                // Not stamped, so not sent: the next pass tries again.
                tracing::warn!(schedule = %schedule.id, %error, "couldn't take a due schedule");
                next = Some(now + chrono::Duration::seconds(5));
            }
        }
    }
    next
}

/// One due repeating schedule: send it, or skip it when it's too late. The
/// time it is next due (to wake for), if any.
fn sweep_repeating(
    workspace: &WorkspaceHost,
    doc_host: &DocHost,
    schedule: &Schedule,
    now: DateTime<Utc>,
    grace: chrono::Duration,
) -> Option<DateTime<Utc>> {
    if now - schedule.fire_at > grace {
        let following = crate::schedule_repeat::next_skipping(schedule, now);
        match workspace.skip_schedule(&schedule.id, now, following) {
            Ok(true) => {
                tracing::warn!(schedule = %schedule.id, chat = %schedule.chat_id,
                    "a repeating scheduled message skipped an occurrence: its time passed while this engine was down");
                return following;
            }
            Ok(false) => return None,
            Err(error) => {
                tracing::warn!(schedule = %schedule.id, %error, "couldn't skip a late occurrence");
                return Some(now + chrono::Duration::seconds(5));
            }
        }
    }
    let following = crate::schedule_repeat::next_run(schedule, now);
    match workspace.advance_schedule(&schedule.id, now, following) {
        Ok(true) => {
            if let Err(error) = deliver(workspace, doc_host, schedule) {
                tracing::warn!(schedule = %schedule.id, chat = %schedule.chat_id, %error,
                    "a scheduled message was taken but couldn't be queued");
            }
            following
        }
        Ok(false) => None,
        Err(error) => {
            // Not stamped, so not sent: the next pass tries again.
            tracing::warn!(schedule = %schedule.id, %error, "couldn't take a due schedule");
            Some(now + chrono::Duration::seconds(5))
        }
    }
}

fn deliver(
    workspace: &WorkspaceHost,
    doc_host: &DocHost,
    schedule: &Schedule,
) -> Result<(), crate::EngineError> {
    let set_by = schedule.created_by.as_deref().map(|id| {
        let short: String = id.chars().take(8).collect();
        match workspace.chat(id).ok().flatten().and_then(|c| c.title) {
            Some(title) if !title.trim().is_empty() => format!("{} ({short})", title.trim()),
            _ => short,
        }
    });
    let text = zeron_proto::scheduled_message(set_by.as_deref(), schedule.fire_at, &schedule.text);
    doc_host.queue_message(&schedule.chat_id, &text, Vec::new())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::HarnessRegistry;
    use serde_json::json;
    use std::path::Path;
    use zeron_proto::{HarnessId, ScheduleState};
    use zeron_rpc::{RpcService, methods};

    const GRACE: Duration = Duration::from_secs(3600);

    fn core_at(dir: &Path) -> crate::EngineCore {
        crate::EngineCore::assemble_with_profile(
            crate::EngineProfile::local(dir).unwrap(),
            std::sync::Arc::new(HarnessRegistry::new()),
            HarnessId::Codex,
            None,
        )
        .unwrap()
    }

    async fn chat(core: &crate::EngineCore, id: &str, device: &str) {
        core.rpc_service()
            .handle(
                methods::MUTATE,
                json!({"op":"createChat", "chatId":id, "deviceId":device,
                    "config":{"harness":"codex","sandbox":"workspace-write","access":"supervised"}}),
            )
            .await
            .unwrap();
    }

    fn at(ms: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(ms).unwrap()
    }

    /// What a chat's doc holds queued (its queue is paused, so nothing is
    /// taken: a send reaching the queue is the hand-off to the host's drain,
    /// the path every routed send takes).
    fn queued(core: &crate::EngineCore, chat: &str) -> Vec<String> {
        core.doc_host
            .open(chat)
            .unwrap()
            .watch_queue()
            .borrow()
            .iter()
            .map(|q| q.text.clone())
            .collect()
    }

    fn hold(core: &crate::EngineCore, chat: &str) {
        core.doc_host.open(chat).unwrap();
        core.doc_host.pause_all_queues();
    }

    #[tokio::test]
    async fn a_due_schedule_is_sent_once_with_its_header_and_never_again() {
        let temp = tempfile::tempdir().unwrap();
        let core = core_at(temp.path());
        chat(&core, "c", &core.device_id).await;
        chat(&core, "setter", &core.device_id).await;
        core.workspace.chat("setter").unwrap();
        hold(&core, "c");
        let s = core
            .workspace
            .create_schedule("c", "check the deploy", at(10_000), Some("setter"))
            .unwrap();

        // Not due yet: nothing goes, and the pass says when to wake.
        assert_eq!(
            sweep(&core.workspace, &core.doc_host, at(9_000), GRACE),
            Some(at(10_000))
        );
        assert!(queued(&core, "c").is_empty());

        assert_eq!(
            sweep(&core.workspace, &core.doc_host, at(10_500), GRACE),
            None
        );
        let sent = queued(&core, "c");
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(
            sent[0].starts_with(
                "[Scheduled message, set by Zeron chat setter, due 1970-01-01 00:00 UTC.]\n\n"
            ),
            "{}",
            sent[0]
        );
        assert_eq!(
            zeron_proto::scheduled_message_body(&sent[0]),
            Some("check the deploy")
        );
        let row = core.workspace.read_schedules().remove(0);
        assert_eq!(
            (row.id.as_str(), row.state(), row.fired_at),
            (s.id.as_str(), ScheduleState::Fired, Some(at(10_500)))
        );

        // Exactly once, however many passes follow.
        for now in [10_600, 20_000, 3_000_000] {
            sweep(&core.workspace, &core.doc_host, at(now), GRACE);
        }
        assert_eq!(queued(&core, "c").len(), 1);
        core.sessions.shutdown().await;
    }

    #[tokio::test]
    async fn a_cancelled_schedule_never_fires() {
        let temp = tempfile::tempdir().unwrap();
        let core = core_at(temp.path());
        chat(&core, "c", &core.device_id).await;
        hold(&core, "c");
        let s = core
            .workspace
            .create_schedule("c", "nope", at(10_000), None)
            .unwrap();
        assert!(core.workspace.cancel_schedule(&s.id).unwrap());
        assert!(!core.workspace.cancel_schedule(&s.id).unwrap());
        assert_eq!(
            sweep(&core.workspace, &core.doc_host, at(11_000), GRACE),
            None
        );
        assert!(queued(&core, "c").is_empty());
        assert_eq!(
            core.workspace.read_schedules()[0].state(),
            ScheduleState::Cancelled
        );
        core.sessions.shutdown().await;
    }

    #[tokio::test]
    async fn one_found_past_its_grace_is_missed_and_one_within_it_is_sent() {
        let temp = tempfile::tempdir().unwrap();
        let core = core_at(temp.path());
        chat(&core, "c", &core.device_id).await;
        hold(&core, "c");
        let late = core
            .workspace
            .create_schedule("c", "too late", at(1_000), None)
            .unwrap();
        let ok = core
            .workspace
            .create_schedule("c", "still fine", at(3_000_000), None)
            .unwrap();
        // The engine was down from before 1s to 3,700s: the first is over an
        // hour old, the second 700s.
        sweep(&core.workspace, &core.doc_host, at(3_700_000), GRACE);
        let state = |id: &str| {
            core.workspace
                .read_schedules()
                .into_iter()
                .find(|s| s.id == id)
                .unwrap()
                .state()
        };
        assert_eq!(state(&late.id), ScheduleState::Missed);
        assert_eq!(state(&ok.id), ScheduleState::Fired);
        let sent = queued(&core, "c");
        assert_eq!(sent.len(), 1);
        assert_eq!(
            zeron_proto::scheduled_message_body(&sent[0]),
            Some("still fine")
        );
        // The grace is the caller's to set.
        let later = core
            .workspace
            .create_schedule("c", "x", at(10_000_000), None)
            .unwrap();
        sweep(
            &core.workspace,
            &core.doc_host,
            at(10_000_000 + 10_000),
            Duration::from_secs(5),
        );
        assert_eq!(state(&later.id), ScheduleState::Missed);
        core.sessions.shutdown().await;
    }

    #[tokio::test]
    async fn only_the_chats_host_fires_it() {
        let temp = tempfile::tempdir().unwrap();
        let core = core_at(temp.path());
        chat(&core, "elsewhere", "another-device").await;
        hold(&core, "elsewhere");
        core.workspace
            .create_schedule("elsewhere", "not mine to send", at(1_000), None)
            .unwrap();
        assert_eq!(
            sweep(&core.workspace, &core.doc_host, at(2_000), GRACE),
            None
        );
        assert!(queued(&core, "elsewhere").is_empty());
        assert!(
            core.workspace.read_schedules()[0].is_pending(),
            "left for its host"
        );
        // A chat it has no row for isn't its either, and can't be scheduled.
        assert!(
            core.workspace
                .create_schedule("ghost", "x", at(1), None)
                .is_err()
        );
        core.sessions.shutdown().await;
    }

    /// The timer is the engine's: the schedule is read back from disk by an
    /// engine started later, with no agent process anywhere, and fires there
    /// once; an engine after that finds it fired.
    #[tokio::test]
    async fn a_schedule_survives_an_engine_restart_and_fires_once_across_restarts() {
        let temp = tempfile::tempdir().unwrap();
        let (device, id) = {
            let core = core_at(temp.path());
            chat(&core, "c", &core.device_id).await;
            let s = core
                .workspace
                .create_schedule("c", "after the restart", at(10_000), None)
                .unwrap();
            let device = core.device_id.clone();
            core.sessions.shutdown().await;
            (device, s.id)
        };
        {
            let core = core_at(temp.path());
            assert_eq!(core.device_id, device);
            assert_eq!(
                core.workspace.read_schedules().len(),
                1,
                "read back from disk"
            );
            hold(&core, "c");
            sweep(&core.workspace, &core.doc_host, at(12_000), GRACE);
            let sent = queued(&core, "c");
            assert_eq!(sent.len(), 1, "{sent:?}");
            assert_eq!(
                zeron_proto::scheduled_message_body(&sent[0]),
                Some("after the restart")
            );
            core.sessions.shutdown().await;
        }
        let core = core_at(temp.path());
        assert_eq!(core.workspace.read_schedules()[0].id, id);
        assert_eq!(
            core.workspace.read_schedules()[0].state(),
            ScheduleState::Fired,
            "the stamp is on disk"
        );
        hold(&core, "c");
        sweep(&core.workspace, &core.doc_host, at(13_000), GRACE);
        // Never queued a second time, whatever the second engine's queue kept.
        let copies = queued(&core, "c")
            .iter()
            .filter(|t| t.contains("after the restart"))
            .count();
        assert!(copies <= 1, "{copies} copies");
        assert_eq!(
            core.workspace.read_schedules()[0].state(),
            ScheduleState::Fired
        );
        core.sessions.shutdown().await;
    }

    /// The running task: a schedule made while it sleeps re-arms it, one
    /// cancelled before its time doesn't fire.
    #[tokio::test]
    async fn the_task_wakes_for_a_new_schedule_and_skips_a_cancelled_one() {
        let temp = tempfile::tempdir().unwrap();
        let core = core_at(temp.path());
        chat(&core, "c", &core.device_id).await;
        hold(&core, "c");
        let task = spawn(core.workspace.clone(), core.doc_host.clone(), GRACE);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let soon = Utc::now() + chrono::Duration::milliseconds(300);
        let dropped = core
            .workspace
            .create_schedule("c", "cancelled", soon, None)
            .unwrap();
        core.workspace
            .create_schedule("c", "kept", soon, None)
            .unwrap();
        core.workspace.cancel_schedule(&dropped.id).unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let sent = queued(&core, "c");
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(zeron_proto::scheduled_message_body(&sent[0]), Some("kept"));
        task.abort();
        core.sessions.shutdown().await;
    }

    // ── repeating schedules ──

    use zeron_proto::Repeat;

    fn repeating(
        core: &crate::EngineCore,
        chat: &str,
        first_ms: i64,
        secs: u64,
        until: Option<i64>,
        max: Option<u32>,
    ) -> zeron_proto::Schedule {
        core.workspace
            .create_schedule_with(crate::workspace_host::NewSchedule {
                chat_id: chat.into(),
                text: "standup".into(),
                fire_at: at(first_ms),
                created_by: None,
                repeat: Some(Repeat::Every { seconds: secs }),
                until: until.map(at),
                max_runs: max,
            })
            .unwrap()
    }

    fn row(core: &crate::EngineCore, id: &str) -> zeron_proto::Schedule {
        core.workspace
            .read_schedules()
            .into_iter()
            .find(|s| s.id == id)
            .unwrap()
    }

    const HOUR: i64 = 3_600_000;

    #[tokio::test]
    async fn a_repeating_schedule_fires_once_per_occurrence_and_moves_on() {
        let temp = tempfile::tempdir().unwrap();
        let core = core_at(temp.path());
        chat(&core, "c", &core.device_id).await;
        hold(&core, "c");
        let s = repeating(&core, "c", 10_000, 3_600, None, None);
        assert_eq!(
            sweep(&core.workspace, &core.doc_host, at(10_500), GRACE),
            Some(at(10_000 + HOUR))
        );
        assert_eq!(queued(&core, "c").len(), 1);
        let r = row(&core, &s.id);
        assert_eq!(
            (r.fire_at, r.runs, r.last_fired_at, r.fired_at),
            (at(10_000 + HOUR), 1, Some(at(10_500)), None)
        );
        assert!(r.is_pending());
        // The same occurrence is never sent twice.
        for now in [10_600, 20_000, HOUR] {
            sweep(&core.workspace, &core.doc_host, at(now), GRACE);
        }
        assert_eq!(queued(&core, "c").len(), 1);
        // The next one, and the one after.
        sweep(
            &core.workspace,
            &core.doc_host,
            at(10_000 + HOUR + 1),
            GRACE,
        );
        sweep(
            &core.workspace,
            &core.doc_host,
            at(10_000 + 2 * HOUR + 1),
            GRACE,
        );
        let sent = queued(&core, "c");
        assert_eq!(sent.len(), 3);
        assert!(
            sent.iter()
                .all(|t| zeron_proto::scheduled_message_body(t) == Some("standup"))
        );
        assert_eq!(row(&core, &s.id).runs, 3);
        core.sessions.shutdown().await;
    }

    #[tokio::test]
    async fn max_runs_and_until_end_it_on_its_last_run() {
        let temp = tempfile::tempdir().unwrap();
        let core = core_at(temp.path());
        chat(&core, "c", &core.device_id).await;
        hold(&core, "c");
        let by_count = repeating(&core, "c", 10_000, 3_600, None, Some(2));
        let by_time = repeating(&core, "c", 10_000, 3_600, Some(10_000 + HOUR), None);
        for k in 0..5 {
            sweep(
                &core.workspace,
                &core.doc_host,
                at(10_000 + k * HOUR + 1),
                GRACE,
            );
        }
        let a = row(&core, &by_count.id);
        assert_eq!(
            (a.runs, a.state(), a.fired_at.is_some()),
            (2, ScheduleState::Fired, true)
        );
        let b = row(&core, &by_time.id);
        assert_eq!(
            (b.runs, b.state()),
            (2, ScheduleState::Fired),
            "the fire at until is the last"
        );
        assert_eq!(queued(&core, "c").len(), 4);
        core.sessions.shutdown().await;
    }

    #[tokio::test]
    async fn a_repeating_one_found_past_grace_skips_ahead_with_no_catch_up_burst() {
        let temp = tempfile::tempdir().unwrap();
        let core = core_at(temp.path());
        chat(&core, "c", &core.device_id).await;
        hold(&core, "c");
        // Down for 5 hours with an hourly schedule and a one-hour grace.
        let late = repeating(&core, "c", 10_000, 3_600, None, None);
        let now = 10_000 + 5 * HOUR;
        assert_eq!(
            sweep(&core.workspace, &core.doc_host, at(now), GRACE),
            Some(at(10_000 + 6 * HOUR))
        );
        assert!(
            queued(&core, "c").is_empty(),
            "nothing sent for the hours it missed"
        );
        let r = row(&core, &late.id);
        assert_eq!(
            (r.runs, r.last_missed_at, r.fire_at, r.state()),
            (
                0,
                Some(at(now)),
                at(10_000 + 6 * HOUR),
                ScheduleState::Pending
            )
        );
        sweep(&core.workspace, &core.doc_host, at(now + 1), GRACE);
        assert!(queued(&core, "c").is_empty());

        core.workspace.cancel_schedule(&late.id).unwrap();

        // Within the grace it is sent once, then moves to the next future one.
        let soon = repeating(&core, "c", 20_000_000, 3_600, None, None);
        sweep(
            &core.workspace,
            &core.doc_host,
            at(20_000_000 + HOUR / 2),
            GRACE,
        );
        assert_eq!(queued(&core, "c").len(), 1);
        assert_eq!(row(&core, &soon.id).fire_at, at(20_000_000 + HOUR));

        core.workspace.cancel_schedule(&soon.id).unwrap();

        // Past its grace and past its `until`: it ends as missed.
        let over = repeating(&core, "c", 30_000_000, 3_600, Some(30_000_000 + HOUR), None);
        sweep(
            &core.workspace,
            &core.doc_host,
            at(30_000_000 + 5 * HOUR),
            GRACE,
        );
        assert_eq!(row(&core, &over.id).state(), ScheduleState::Missed);
        core.sessions.shutdown().await;
    }

    #[tokio::test]
    async fn a_repeating_schedule_survives_restarts_and_never_repeats_an_occurrence() {
        let temp = tempfile::tempdir().unwrap();
        let id = {
            let core = core_at(temp.path());
            chat(&core, "c", &core.device_id).await;
            let s = repeating(&core, "c", 10_000, 3_600, None, Some(3));
            core.sessions.shutdown().await;
            s.id
        };
        {
            let core = core_at(temp.path());
            hold(&core, "c");
            assert_eq!(
                row(&core, &id).repeat,
                Some(Repeat::Every { seconds: 3_600 }),
                "read back from disk"
            );
            sweep(&core.workspace, &core.doc_host, at(10_500), GRACE);
            assert_eq!(queued(&core, "c").len(), 1);
            core.sessions.shutdown().await;
        }
        {
            let core = core_at(temp.path());
            hold(&core, "c");
            let r = row(&core, &id);
            assert_eq!(
                (r.runs, r.fire_at),
                (1, at(10_000 + HOUR)),
                "the fire and the move are on disk"
            );
            // The same instant again: nothing (the occurrence was taken).
            sweep(&core.workspace, &core.doc_host, at(10_600), GRACE);
            assert!(
                queued(&core, "c")
                    .iter()
                    .filter(|t| t.contains("standup"))
                    .count()
                    <= 1
            );
            sweep(
                &core.workspace,
                &core.doc_host,
                at(10_000 + HOUR + 10),
                GRACE,
            );
            assert_eq!(row(&core, &id).runs, 2);
            core.sessions.shutdown().await;
        }
        let core = core_at(temp.path());
        hold(&core, "c");
        sweep(
            &core.workspace,
            &core.doc_host,
            at(10_000 + 2 * HOUR + 10),
            GRACE,
        );
        let r = row(&core, &id);
        assert_eq!(
            (r.runs, r.state()),
            (3, ScheduleState::Fired),
            "max_runs honoured across three engines"
        );
        sweep(
            &core.workspace,
            &core.doc_host,
            at(10_000 + 9 * HOUR),
            GRACE,
        );
        assert_eq!(row(&core, &id).runs, 3);
        core.sessions.shutdown().await;
    }

    /// A cron schedule end to end: planned in New York across the March DST
    /// change, fired, and moved to the next 02:30-ish occurrence.
    #[tokio::test]
    async fn a_cron_schedule_moves_to_its_next_occurrence_across_dst() {
        let temp = tempfile::tempdir().unwrap();
        let core = core_at(temp.path());
        chat(&core, "c", &core.device_id).await;
        hold(&core, "c");
        let spec = crate::schedule_repeat::RepeatSpec {
            cron: Some("30 2 * * *".into()),
            timezone: Some("America/New_York".into()),
            ..Default::default()
        };
        let now = DateTime::parse_from_rfc3339("2026-03-07T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let plan = crate::schedule_repeat::plan(Some(&spec), None, None, None, now).unwrap();
        assert_eq!(
            plan.first.to_rfc3339(),
            "2026-03-08T07:00:00+00:00",
            "02:30 doesn't exist on the 8th: just after the gap"
        );
        let s = core
            .workspace
            .create_schedule_with(crate::workspace_host::NewSchedule {
                chat_id: "c".into(),
                text: "nightly".into(),
                fire_at: plan.first,
                created_by: None,
                repeat: plan.repeat,
                until: None,
                max_runs: None,
            })
            .unwrap();
        let next = sweep(
            &core.workspace,
            &core.doc_host,
            plan.first + chrono::Duration::seconds(5),
            GRACE,
        );
        assert_eq!(next.unwrap().to_rfc3339(), "2026-03-09T06:30:00+00:00");
        assert_eq!(queued(&core, "c").len(), 1);
        assert_eq!(
            row(&core, &s.id).fire_at.to_rfc3339(),
            "2026-03-09T06:30:00+00:00"
        );
        core.sessions.shutdown().await;
    }
}
