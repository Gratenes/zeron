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

fn view() -> RunView {
    RunView {
        voice_live: false,
        silent: std::time::Duration::from_secs(600),
        age: std::time::Duration::from_secs(600),
        started: false,
        agent_pid_gone: None,
        is_claude: true,
        unconfirmed_steers: false,
    }
}

#[test]
fn a_never_started_local_run_is_dead_whatever_credential_rotation_gave_it() {
    // (A run's credential is not an input: the engine is a Cloud machine or not.)
    assert!(is_dead(&view(), SILENT_AFTER, false));
    assert!(
        !is_dead(&view(), SILENT_AFTER, true),
        "a Cloud machine's setup is allowed its time"
    );
    let young = RunView {
        age: std::time::Duration::from_secs(60),
        silent: std::time::Duration::from_secs(60),
        ..view()
    };
    assert!(!is_dead(&young, SILENT_AFTER, false));
}

#[test]
fn a_started_run_is_judged_by_its_agent_not_its_silence() {
    let started = RunView {
        started: true,
        ..view()
    };
    // Claude, no pid on an open stream: the process is gone.
    assert!(is_dead(&started, SILENT_AFTER, false));
    // Any harness: a pid that is gone.
    let pi = RunView {
        is_claude: false,
        ..started
    };
    assert!(
        !is_dead(&pi, SILENT_AFTER, false),
        "no pid reported: quiet is allowed"
    );
    let gone = RunView {
        agent_pid_gone: Some(true),
        ..pi
    };
    assert!(is_dead(&gone, SILENT_AFTER, false));
    let alive = RunView {
        agent_pid_gone: Some(false),
        is_claude: true,
        ..view()
    };
    let alive = RunView {
        started: true,
        ..alive
    };
    assert!(
        !is_dead(&alive, SILENT_AFTER, false),
        "a live agent is never judged"
    );
}

#[test]
fn a_harness_without_a_pid_is_judged_only_with_a_message_it_never_took_up() {
    let silent_long = RunView {
        started: true,
        is_claude: false,
        silent: UNSEEN_AFTER,
        ..view()
    };
    assert!(
        !is_dead(&silent_long, SILENT_AFTER, false),
        "a long quiet tool call"
    );
    let with_steer = RunView {
        unconfirmed_steers: true,
        ..silent_long
    };
    assert!(is_dead(&with_steer, SILENT_AFTER, false));
    let not_yet = RunView {
        silent: UNSEEN_AFTER - std::time::Duration::from_secs(1),
        ..with_steer
    };
    assert!(!is_dead(&not_yet, SILENT_AFTER, false));
}

#[test]
fn a_run_on_a_voice_call_is_never_judged() {
    assert!(!is_dead(
        &RunView {
            voice_live: true,
            ..view()
        },
        SILENT_AFTER,
        false
    ));
}
