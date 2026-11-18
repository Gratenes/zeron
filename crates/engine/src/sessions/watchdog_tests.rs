//! The watchdog's own logic, with no runs to drive: a working row with
//! nothing behind it. The runs it must recover (a wedged parked run, a dead
//! agent, a full mailbox) are in `tests/stall_watchdog.rs`.

use super::*;

fn engine() -> (SessionsEngine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let sessions = SessionsEngine::new(
        "host".into(),
        Arc::new(RunJournal::open(dir.path().join("journals")).unwrap()),
        Arc::new(HarnessRegistry::new()),
    );
    (sessions, dir)
}

#[tokio::test]
async fn a_working_row_with_no_run_is_reported_and_then_cleared() {
    let (sessions, _dir) = engine();
    sessions.seed_status("ghost", SessionStatus::Working);
    sessions.seed_status("quiet", SessionStatus::Idle);

    let report = sessions.run_report();
    assert!(report.runs.is_empty());
    assert_eq!(report.working_without_run, ["ghost"]);
    assert_eq!(report.statuses.len(), 1, "idle rows are not listed");

    // Too fresh to judge: a run being made has not stamped its row yet.
    let acted = sessions
        .watchdog_tick(std::time::Duration::from_secs(3600), SILENT_AFTER)
        .await;
    assert_eq!(acted, 0);
    assert_eq!(
        sessions.session_status("ghost").unwrap().status,
        SessionStatus::Working
    );

    let acted = sessions
        .watchdog_tick(std::time::Duration::ZERO, SILENT_AFTER)
        .await;
    assert_eq!(acted, 1);
    let row = sessions.session_status("ghost").unwrap();
    assert_eq!(row.status, SessionStatus::Errored);
    assert_eq!(
        row.last_turn_outcome.map(|o| o.status),
        Some(DoneStatus::Errored)
    );
    assert_eq!(
        sessions.session_status("quiet").unwrap().status,
        SessionStatus::Idle,
        "an idle chat is left alone"
    );
    assert!(sessions.run_report().working_without_run.is_empty());
}

#[tokio::test]
async fn a_chat_awaiting_an_answer_with_no_run_is_not_cleared() {
    // Only working rows are claimed: a parked question has its own states.
    let (sessions, _dir) = engine();
    sessions.seed_status("asking", SessionStatus::AwaitingInput);
    let acted = sessions
        .watchdog_tick(std::time::Duration::ZERO, SILENT_AFTER)
        .await;
    assert_eq!(acted, 0);
}

#[tokio::test]
async fn a_chat_with_a_revival_on_its_way_is_not_cleared() {
    let (sessions, _dir) = engine();
    sessions.seed_status("reviving", SessionStatus::Working);
    lock(&sessions.inner.revivals_starting).insert("reviving".into());
    let acted = sessions
        .watchdog_tick(std::time::Duration::ZERO, SILENT_AFTER)
        .await;
    assert_eq!(acted, 0);
}
