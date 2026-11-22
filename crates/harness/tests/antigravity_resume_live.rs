//! Live check behind `AcpHarness::stops_when_idle` for Antigravity: a parked
//! runtime stopped the way the idle reaper stops it (its interrupt token
//! cancelled between turns), then a new process resumed from the session id,
//! still remembers the first turn. Needs a signed-in Antigravity install.
//! ANTIGRAVITY_ACP_EXECUTABLE can point at an installed `agy_acp_server.par`.
//! cargo test -p zeron-harness --test antigravity_resume_live -- --ignored --nocapture

use futures::StreamExt;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use zeron_harness::{AcpHarness, CancellationToken, Harness, RunControls};
use zeron_proto::{AgentEvent, DoneStatus, RunRequest, SandboxLevel};

fn controls() -> (RunControls, CancellationToken) {
    let (_steer, steering) = mpsc::channel(8);
    let interrupt = CancellationToken::new();
    let controls = RunControls {
        realtime: None,
        execution_lease: None,
        steering,
        interrupt: interrupt.clone(),
        request_input: Box::new(|_| {
            let (tx, rx) = oneshot::channel();
            let _ = tx.send(Vec::new());
            rx
        }),
        turn: Default::default(),
    };
    (controls, interrupt)
}

fn request(cwd: &str, prompt: &str, resume: Option<String>) -> RunRequest {
    RunRequest {
        mcp: None,
        prompt: prompt.into(),
        harness: None,
        model: None,
        reasoning: None,
        model_options: serde_json::Map::new(),
        cwd: cwd.into(),
        sandbox: SandboxLevel::ReadOnly,
        auto_approve: false,
        access: Default::default(),
        plan: false,
        code_mode: false,
        attachments: Vec::new(),
        worktree: None,
        resume,
        env: Default::default(),
        extra_mcp: Vec::new(),
    }
}

/// One turn; after its Done the runtime is stopped like the idle reaper
/// does it. Returns (session id, reply text).
async fn turn(cwd: &str, prompt: &str, resume: Option<String>) -> (String, String) {
    let (controls, interrupt) = controls();
    let mut stream = AcpHarness::antigravity()
        .run(request(cwd, prompt, resume), controls)
        .await
        .expect("antigravity must start");
    let mut session = String::new();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(240), async {
        while let Some(event) = stream.next().await {
            match event.expect("event") {
                AgentEvent::SessionStarted { session_id, .. } => session = session_id,
                AgentEvent::TextDelta { text: delta } => text.push_str(&delta),
                AgentEvent::Done { status, error, .. } => {
                    assert_eq!(status, DoneStatus::Completed, "{error:?} {text:?}");
                    interrupt.cancel();
                    break;
                }
                _ => {}
            }
        }
        // Drain the teardown so the agent process is gone before the resume.
        while tokio::time::timeout(Duration::from_secs(30), stream.next())
            .await
            .ok()
            .flatten()
            .is_some()
        {}
    })
    .await
    .expect("turn timed out");
    (session, text)
}

#[tokio::test]
#[ignore = "live: needs a signed-in Antigravity"]
async fn antigravity_resume_after_idle_stop_keeps_context() {
    let cwd = tempfile::tempdir().unwrap();
    let cwd = cwd.path().display().to_string();
    let (session, first) = turn(
        &cwd,
        "Memory test, do not use any tools. Remember: the codeword is ZEPHYR-4471 and my favorite number is 88. Reply only: OK, noted.",
        None,
    )
    .await;
    eprintln!("turn 1 session={session} reply={first:?}");
    assert!(!session.is_empty(), "no session id");
    let (resumed, second) = turn(
        &cwd,
        "Without using any tools or files: what is the codeword and my favorite number? Reply as CODEWORD/NUMBER.",
        Some(session.clone()),
    )
    .await;
    eprintln!("turn 2 session={resumed} reply={second:?}");
    assert_eq!(resumed, session, "resume opened a different session");
    assert!(
        second.contains("ZEPHYR-4471") && second.contains("88"),
        "resumed session lost the first turn: {second:?}"
    );
}
