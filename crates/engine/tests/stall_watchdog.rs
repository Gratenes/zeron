//! A chat never goes silent because a run stopped answering: a message a
//! parked run took and never started on is sent again after 30 s; a run
//! whose agent process is gone is dropped with a note; a mailbox nothing
//! drains is replaced; and every run's start and drop can be read back
//! (`run_report`).
//!
//! The harness models one agent per `run()`, each spawn following the next
//! step of a script: answering normally, going deaf after its first answer
//! (the stuck run: its mailbox takes messages and nothing reads them), or
//! never saying anything at all.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio::sync::mpsc;

use zeron_doc::{MessagePart, MessageRole};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::{Harness, HarnessError, RunControls};
use zeron_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SessionStatus, SteeringMode,
};

const CHAT: &str = "chat-stall";

#[derive(Clone, Copy, Debug)]
enum Behavior {
    /// Answers every prompt, steers included.
    Normal,
    /// Answers its first prompt, then reads nothing more and says nothing.
    DeafAfterFirst,
    /// Like [`Self::DeafAfterFirst`], and ignores its teardown too: the run
    /// entry would stay listed for ever.
    UnkillableAfterFirst,
    /// Starts a turn and works on it until it is stopped.
    Busy,
    /// Never gets as far as opening its stream: the spawn itself hangs.
    HangSpawn,
    /// Opens its stream and never says a word.
    Mute,
    /// Starts a turn, then goes silent with an agent pid that is gone.
    DeadMidTurn(u32),
}

struct Agent {
    script: Mutex<Vec<Behavior>>,
    spawns: AtomicUsize,
    /// What each spawn was asked, in order.
    prompts: Mutex<Vec<(usize, String)>>,
}

fn done() -> AgentEvent {
    AgentEvent::Done {
        status: DoneStatus::Completed,
        result: None,
        error: None,
        session_id: Some("provider-session".into()),
    }
}

#[async_trait]
impl Harness for Agent {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Agent"
    }
    fn supports_steering(&self) -> bool {
        true
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::StepBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[ReasoningLevel::Medium]
    }
    fn deterministic_turn_end(&self) -> bool {
        true
    }
    fn stops_turn_in_place(&self) -> bool {
        true
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        mut controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        let spawn = self.spawns.fetch_add(1, Ordering::SeqCst) + 1;
        let behavior = self
            .script
            .lock()
            .unwrap()
            .get(spawn - 1)
            .copied()
            .unwrap_or(Behavior::Normal);
        self.prompts
            .lock()
            .unwrap()
            .push((spawn, request.prompt.clone()));
        if matches!(behavior, Behavior::HangSpawn) {
            controls.interrupt.cancelled().await;
            return Err(HarnessError::Protocol("spawn cancelled".into()));
        }
        if let Behavior::DeadMidTurn(pid) = behavior {
            controls.turn.set_agent_pid(Some(pid));
        }
        let (tx, rx) = mpsc::channel::<Result<AgentEvent, HarnessError>>(64);
        tokio::spawn(async move {
            let send = |event: AgentEvent| {
                let tx = tx.clone();
                async move { tx.send(Ok(event)).await.is_ok() }
            };
            let answer = |prompt: String| {
                let send = send.clone();
                async move {
                    send(AgentEvent::TextDelta {
                        text: format!("answered: {prompt}"),
                    })
                    .await
                        && send(done()).await
                }
            };
            let interrupted = |tx: mpsc::Sender<Result<AgentEvent, HarnessError>>| async move {
                let _ = tx
                    .send(Ok(AgentEvent::Done {
                        status: DoneStatus::Interrupted,
                        result: None,
                        error: None,
                        session_id: None,
                    }))
                    .await;
            };
            if !matches!(behavior, Behavior::Mute) {
                let _ = send(AgentEvent::SessionStarted {
                    harness: HarnessId::Mock,
                    model: "agent".into(),
                    tools: vec![],
                    cwd: request.cwd.clone(),
                    session_id: "provider-session".into(),
                    assistant_message_id: "a0".into(),
                })
                .await;
            }
            match behavior {
                Behavior::HangSpawn => unreachable!("returned before streaming"),
                Behavior::Mute | Behavior::DeadMidTurn(_) | Behavior::Busy => {
                    controls.interrupt.cancelled().await;
                    interrupted(tx).await;
                }
                Behavior::UnkillableAfterFirst => {
                    if !answer(request.prompt.clone()).await {
                        return;
                    }
                    let _hold = (tx, controls);
                    std::future::pending::<()>().await;
                }
                Behavior::DeafAfterFirst => {
                    if !answer(request.prompt.clone()).await {
                        return;
                    }
                    // Holds the mailbox open and never reads it.
                    controls.interrupt.cancelled().await;
                    interrupted(tx).await;
                }
                Behavior::Normal => {
                    if !answer(request.prompt.clone()).await {
                        return;
                    }
                    loop {
                        tokio::select! {
                            biased;
                            _ = controls.interrupt.cancelled() => {
                                interrupted(tx).await;
                                return;
                            }
                            steer = controls.steering.recv() => {
                                let Some(steer) = steer else { return };
                                let boundary = AgentEvent::Steered {
                                    assistant_message_id: None,
                                    next_assistant_message_id: None,
                                };
                                if !send(boundary).await || !answer(steer.prompt.clone()).await {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        });
        Ok(
            futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (e, rx)) })
                .boxed(),
        )
    }
}

struct Rig {
    core: EngineCore,
    agent: Arc<Agent>,
    _dir: tempfile::TempDir,
}

/// What account rotation and pins give every local Claude run: a credential.
struct LocalRotation;

#[async_trait]
impl zeron_engine::registry::RunEnvironment for LocalRotation {
    async fn prepare(
        &self,
        _harness: HarnessId,
        _chat: Option<&str>,
        _request: &mut RunRequest,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn credential(&self, _harness: HarnessId, _chat: Option<&str>) -> Option<String> {
        Some("login:local-rotation".into())
    }
}

fn rig(script: Vec<Behavior>) -> Rig {
    rig_with(script, false)
}

fn rig_with(script: Vec<Behavior>, local_credential: bool) -> Rig {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let agent = Arc::new(Agent {
        script: Mutex::new(script),
        spawns: AtomicUsize::new(0),
        prompts: Mutex::new(Vec::new()),
    });
    let registry = HarnessRegistry::new();
    registry.register(agent.clone());
    if local_credential {
        registry.set_run_environment(Arc::new(LocalRotation));
    }
    let dir = tempfile::tempdir().unwrap();
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    Rig {
        core,
        agent,
        _dir: dir,
    }
}

fn request(prompt: &str) -> RunRequest {
    RunRequest {
        mcp: None,
        env: Default::default(),
        prompt: prompt.into(),
        harness: Some(HarnessId::Mock),
        model: Some("agent-1".into()),
        reasoning: Some(ReasoningLevel::Medium),
        model_options: Default::default(),
        cwd: "/tmp".into(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: false,
        access: Default::default(),
        plan: false,
        attachments: Vec::new(),
        worktree: None,
        resume: None,
        extra_mcp: Vec::new(),
    }
}

async fn wait_for(mut predicate: impl FnMut() -> bool, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(900);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

impl Rig {
    async fn send(&self, prompt: &str) {
        self.core
            .sessions
            .dispatch(CHAT, HarnessId::Mock, request(prompt), None)
            .await
            .unwrap();
    }

    fn status(&self) -> Option<SessionStatus> {
        self.core.sessions.session_status(CHAT).map(|s| s.status)
    }

    fn spawns(&self) -> usize {
        self.agent.spawns.load(Ordering::SeqCst)
    }

    /// Whether the transcript carries `text` (an answer or an error note).
    fn shows(&self, text: &str) -> bool {
        self.core
            .doc_host
            .open(CHAT)
            .ok()
            .and_then(|h| h.doc().read_entries().ok())
            .unwrap_or_default()
            .iter()
            .any(|entry| {
                entry.role == MessageRole::Assistant
                    && entry.parts.iter().any(|part| match part {
                        MessagePart::Text { text: t, .. } => t.contains(text),
                        MessagePart::Error { message, .. } => message.contains(text),
                        _ => false,
                    })
            })
    }

    async fn answered_first_prompt(&self) {
        self.send("first").await;
        wait_for(
            || self.status() == Some(SessionStatus::Idle) && self.shows("answered: first"),
            "the first turn to park",
        )
        .await;
    }
}

/// The 2026-11-18 stall: a parked run took a message and never started on it.
#[tokio::test(start_paused = true)]
async fn a_message_a_wedged_parked_run_took_is_sent_again_after_30_s() {
    let rig = rig(vec![Behavior::DeafAfterFirst, Behavior::Normal]);
    rig.answered_first_prompt().await;
    assert_eq!(rig.spawns(), 1);

    rig.send("second").await;
    assert_eq!(rig.status(), Some(SessionStatus::Working));
    assert_eq!(rig.spawns(), 1, "the parked run took it into its mailbox");
    let report = rig.core.sessions.run_report();
    assert_eq!(report.runs.len(), 1);
    assert_eq!(
        report.runs[0].pending_steers, 1,
        "accepted, never confirmed"
    );

    tokio::time::sleep(Duration::from_secs(31)).await;
    wait_for(
        || rig.shows("answered: second") && rig.status() == Some(SessionStatus::Idle),
        "the message to be answered by a fresh run",
    )
    .await;
    assert_eq!(rig.spawns(), 2, "the stuck run was replaced, once");
    assert!(
        rig.shows("Agent unresponsive, restarting"),
        "the chat says what happened"
    );
    let asked: Vec<_> = rig
        .agent
        .prompts
        .lock()
        .unwrap()
        .iter()
        .filter(|(spawn, _)| *spawn == 2)
        .map(|(_, prompt)| prompt.clone())
        .collect();
    assert_eq!(asked, ["second"], "sent to the new run exactly once");
    rig.core.sessions.shutdown().await;
}

/// A turn already running is judged on its own time: a long tool call is
/// quiet for minutes and a message sent into it must not kill it.
#[tokio::test(start_paused = true)]
async fn a_message_sent_into_a_turn_in_flight_is_never_second_guessed() {
    let rig = rig(vec![Behavior::Mute]);
    rig.send("work").await;
    wait_for(|| rig.status() == Some(SessionStatus::Working), "working").await;
    rig.send("also this").await;
    tokio::time::sleep(Duration::from_secs(100)).await;
    assert_eq!(rig.spawns(), 1, "the quiet run was left alone");
    assert!(!rig.shows("Agent unresponsive"));
    rig.core.sessions.shutdown().await;
}

/// Sending it again is tried once: a second run that is just as silent ends
/// the turn with a note instead of looping.
#[tokio::test(start_paused = true)]
async fn a_message_that_gets_no_answer_twice_ends_the_turn_with_a_note() {
    let rig = rig(vec![
        Behavior::DeafAfterFirst,
        Behavior::Mute,
        Behavior::Normal,
    ]);
    rig.answered_first_prompt().await;
    rig.send("second").await;
    tokio::time::sleep(Duration::from_secs(31)).await;
    wait_for(|| rig.spawns() == 2, "the second spawn").await;
    // The second run (a fresh start) says nothing either.
    tokio::time::sleep(Duration::from_secs(130)).await;
    wait_for(
        || rig.status() == Some(SessionStatus::Errored),
        "the turn to end errored",
    )
    .await;
    assert!(rig.shows("Agent unresponsive — send again"));
    assert_eq!(rig.spawns(), 2, "no third attempt for the same message");
    rig.core.sessions.shutdown().await;
}

/// A run whose agent process is gone, and silent, is dropped with a note
/// the next time the watchdog looks.
#[tokio::test(start_paused = true)]
async fn a_run_whose_agent_process_is_gone_is_dropped_with_a_note() {
    // A real pid that has exited.
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let gone = child.id();
    child.wait().unwrap();

    let rig = rig(vec![Behavior::DeadMidTurn(gone)]);
    rig.send("work").await;
    wait_for(
        || {
            rig.core
                .sessions
                .run_report()
                .runs
                .first()
                .is_some_and(|run| run.agent_alive == Some(false))
        },
        "the run to show its agent gone",
    )
    .await;
    assert_eq!(rig.status(), Some(SessionStatus::Working));

    // Three silent minutes, then the next 30 s look.
    tokio::time::sleep(Duration::from_secs(3 * 60 + 40)).await;
    wait_for(
        || {
            rig.status() == Some(SessionStatus::Errored)
                && rig.core.sessions.run_report().runs.is_empty()
        },
        "the watchdog to drop the run",
    )
    .await;
    assert!(rig.shows("Agent stopped unexpectedly"));
    rig.core.sessions.shutdown().await;
}

/// A mailbox nothing drains fills up: the send that finds it full replaces
/// the runtime, and the messages stuck in it are sent again, not lost.
#[tokio::test(start_paused = true)]
async fn a_full_mailbox_nothing_drains_is_replaced_without_losing_what_it_held() {
    let rig = rig(vec![Behavior::DeafAfterFirst, Behavior::Normal]);
    rig.answered_first_prompt().await;
    // 32 messages fit; the 33rd finds no room.
    for n in 0..33 {
        rig.send(&format!("m{n}")).await;
    }
    wait_for(|| rig.spawns() >= 2, "the runtime to be replaced").await;
    wait_for(
        || (0..33).all(|n| rig.shows(&format!("answered: m{n}"))),
        "every message to be answered",
    )
    .await;
    assert_eq!(rig.spawns(), 2);
    rig.core.sessions.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn the_run_report_lists_a_live_run_with_its_age() {
    let rig = rig(vec![Behavior::Normal]);
    rig.answered_first_prompt().await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    let report = rig.core.sessions.run_report();
    assert_eq!(report.runs.len(), 1);
    let run = &report.runs[0];
    assert_eq!(run.chat_id, CHAT);
    assert!(run.started);
    assert!(run.age_s >= 10, "age {}", run.age_s);
    assert!(run.silent_s >= 10, "silent {}", run.silent_s);
    assert_eq!(run.pending_steers, 0);
    assert!(report.working_without_run.is_empty());
    rig.core.sessions.shutdown().await;
}

/// A run that ignores its teardown must not stay listed: the entry is
/// removed and the message goes to a fresh run.
#[tokio::test(start_paused = true)]
async fn a_run_that_ignores_its_teardown_is_dropped_and_its_message_sent_again() {
    let rig = rig(vec![Behavior::UnkillableAfterFirst, Behavior::Normal]);
    rig.answered_first_prompt().await;
    rig.send("second").await;
    tokio::time::sleep(Duration::from_secs(31)).await;
    wait_for(
        || rig.shows("answered: second") && rig.status() == Some(SessionStatus::Idle),
        "the message to be answered by a fresh run",
    )
    .await;
    assert_eq!(rig.spawns(), 2);
    let report = rig.core.sessions.run_report();
    assert_eq!(report.runs.len(), 1, "only the new run is listed");
    rig.core.sessions.shutdown().await;
}

/// A stop and an immediate resend: the stopped run's teardown and the new
/// run's start overlap. An agent must come up and answer.
#[tokio::test(start_paused = true)]
async fn a_message_sent_right_after_a_stop_starts_an_agent_that_answers() {
    for concurrent in [false, true] {
        let rig = rig(vec![Behavior::Busy, Behavior::Normal]);
        rig.send("work").await;
        wait_for(|| rig.spawns() == 1, "the agent to start").await;
        if concurrent {
            let (stopped, sent) = tokio::join!(
                rig.core.sessions.interrupt(CHAT),
                rig.core
                    .sessions
                    .dispatch(CHAT, HarnessId::Mock, request("again"), None)
            );
            stopped.unwrap();
            sent.unwrap();
        } else {
            rig.core.sessions.interrupt(CHAT).await.unwrap();
            rig.send("again").await;
        }
        wait_for(
            || rig.shows("answered: again") && rig.status() == Some(SessionStatus::Idle),
            "the resent message to be answered",
        )
        .await;
        assert_eq!(rig.spawns(), 2, "concurrent={concurrent}: one new agent");
        rig.core.sessions.shutdown().await;
    }
}

/// Two sends at once after a stop (a queue flush racing a resend) start one
/// agent between them, never two beside each other.
#[tokio::test(start_paused = true)]
async fn two_sends_at_once_after_a_stop_share_one_agent() {
    let rig = rig(vec![Behavior::Busy, Behavior::Normal, Behavior::Normal]);
    rig.send("work").await;
    wait_for(|| rig.spawns() == 1, "the agent to start").await;
    rig.core.sessions.interrupt(CHAT).await.unwrap();
    let (a, b) = tokio::join!(
        rig.core
            .sessions
            .dispatch(CHAT, HarnessId::Mock, request("one"), None),
        rig.core
            .sessions
            .dispatch(CHAT, HarnessId::Mock, request("two"), None)
    );
    a.unwrap();
    b.unwrap();
    wait_for(
        || rig.shows("answered: one") && rig.shows("answered: two"),
        "both to be answered",
    )
    .await;
    assert_eq!(rig.spawns(), 2, "one agent for both, not one each");
    rig.core.sessions.shutdown().await;
}

/// Local account rotation and pins give EVERY Claude run a credential, which
/// is not what a Cloud machine's broker is: a run that never gets as far as
/// opening its stream is still judged, and still ends with a note.
#[tokio::test(start_paused = true)]
async fn a_run_that_never_spawns_is_judged_though_rotation_gave_it_a_credential() {
    let rig = rig_with(
        vec![Behavior::HangSpawn, Behavior::HangSpawn, Behavior::Normal],
        true,
    );
    rig.send("hello").await;
    // 120 s: sent again once. 240 s: the second run is just as stuck.
    tokio::time::sleep(Duration::from_secs(250)).await;
    wait_for(
        || rig.status() == Some(SessionStatus::Errored),
        "the turn to end with a note",
    )
    .await;
    assert!(
        rig.shows("Agent unresponsive"),
        "the chat says what happened"
    );
    assert_eq!(rig.spawns(), 2, "sent again once, not for ever");
    assert!(rig.core.sessions.run_report().runs.is_empty());
    rig.core.sessions.shutdown().await;
}

/// A harness that reports no agent pid can be quiet for a long tool call and
/// is left alone. Not with a message it took and never confirmed: after half
/// an hour of silence the run is dropped and the message sent again.
#[tokio::test(start_paused = true)]
async fn a_quiet_run_without_a_pid_is_left_alone_but_not_with_a_message_it_never_took_up() {
    let quiet = rig(vec![Behavior::Busy]);
    quiet.send("long build").await;
    wait_for(|| quiet.spawns() == 1, "the agent to start").await;
    tokio::time::sleep(Duration::from_secs(40 * 60)).await;
    assert_eq!(quiet.spawns(), 1, "a long quiet turn is not a stuck run");
    assert_eq!(quiet.core.sessions.run_report().runs.len(), 1);
    quiet.core.sessions.shutdown().await;

    let stuck = rig(vec![Behavior::Busy, Behavior::Normal]);
    stuck.send("long build").await;
    wait_for(|| stuck.spawns() == 1, "the agent to start").await;
    // Sent while the turn runs: taken into the mailbox, never confirmed.
    stuck.send("also this").await;
    assert_eq!(stuck.core.sessions.run_report().runs[0].pending_steers, 1);
    tokio::time::sleep(Duration::from_secs(31 * 60)).await;
    wait_for(
        || stuck.shows("answered: also this") && stuck.status() == Some(SessionStatus::Idle),
        "the message to be answered by a fresh run",
    )
    .await;
    assert_eq!(stuck.spawns(), 2);
    stuck.core.sessions.shutdown().await;
}
