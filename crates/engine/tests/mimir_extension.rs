//! Real ACP decoding, native controls, and replacement child docs through public engine APIs.
#![cfg(unix)]
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use zeron_doc::{MessagePart, MessageRole, SessionCommandPayload};
use zeron_engine::{EngineCore, HarnessRegistry};
use zeron_harness::AcpHarness;
use zeron_proto::{ChatConfig, GoalPhase, HarnessId, RunRequest, SandboxLevel, SessionStatus};
const CHAT: &str = "native-goal";

fn core(path: &Path) -> EngineCore {
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(
        AcpHarness::mimir().with_executable(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../harness/tests/fixtures/mimir-extension.py"),
        ),
    ));
    EngineCore::assemble(path, Arc::new(registry), HarnessId::Mimir, None).unwrap()
}
fn request(path: &Path, prompt: &str) -> RunRequest {
    RunRequest {
        prompt: prompt.into(),
        harness: Some(HarnessId::Mimir),
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: path.display().to_string(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: true,
        resume: None,
        attachments: vec![],
        worktree: None,
    }
}
fn create(core: &EngineCore, path: &Path) {
    core.workspace
        .create_chat(
            CHAT,
            None,
            Some(&core.device_id),
            Some(ChatConfig {
                harness: HarnessId::Mimir,
                model: None,
                reasoning: None,
                model_options: Default::default(),
                sandbox: SandboxLevel::WorkspaceWrite,
            }),
            Some(path.display().to_string()),
        )
        .unwrap();
}
async fn wait(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(15), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("public engine state advances");
}
fn child_ref(core: &EngineCore) -> Option<String> {
    core.doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .read_entries()
        .unwrap()
        .into_iter()
        .flat_map(|e| e.parts)
        .find_map(|p| match p {
            MessagePart::Tool {
                id,
                subagent_ref,
                call,
                ..
            } if id == "launch-1" && call.is_subagent_spawn() => subagent_ref,
            _ => None,
        })
}
fn text(core: &EngineCore, doc: &str) -> String {
    core.doc_host
        .open(doc)
        .unwrap()
        .doc()
        .read_entries()
        .unwrap()
        .into_iter()
        .flat_map(|e| e.parts)
        .filter_map(|p| match p {
            MessagePart::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn active_goal_controls_bypass_queue_without_success_or_extra_model_lease() {
    let dir = tempfile::tempdir().unwrap();
    let core = core(dir.path());
    create(&core, dir.path());
    core.sessions
        .dispatch(
            CHAT,
            HarnessId::Mimir,
            request(dir.path(), "start native goal"),
            None,
        )
        .await
        .unwrap();
    wait(|| {
        core.sessions
            .session_status(CHAT)
            .is_some_and(|s| s.goal.is_some_and(|g| g.phase == GoalPhase::Active))
            && child_ref(&core).is_some()
    })
    .await;
    let child = child_ref(&core).unwrap();
    assert_eq!(text(&core, &child), "PUBLIC_CHILD");
    assert_eq!(
        core.doc_host
            .fetch_tool_blob(&format!("{child}/child-tool"))
            .await
            .unwrap(),
        "PUBLIC_OUTPUT"
    );
    let child_handle = core.doc_host.open(&child).unwrap();
    let first = child_handle.doc().doc().oplog_vv();
    tokio::time::sleep(Duration::from_millis(2100)).await;
    assert_eq!(
        child_handle.doc().doc().oplog_vv(),
        first,
        "same snapshot must not append history"
    );
    core.doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Steer {
                prompt: "/goal pause".into(),
                message_id: Some("pause-user".into()),
            },
        )
        .unwrap();
    // Establish which independently delivered durable command is in flight
    // before submitting an identical follower (delivery tasks may race).
    wait(|| {
        std::fs::read_to_string(dir.path().join("extension-requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .any(|line| {
                serde_json::from_str::<serde_json::Value>(line)
                    .is_ok_and(|frame| frame["method"] == "_mimir/session/goal")
            })
    })
    .await;
    core.doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Steer {
                prompt: "/goal pause".into(),
                message_id: Some("duplicate-user".into()),
            },
        )
        .unwrap();
    wait(|| {
        core.sessions.session_status(CHAT).is_some_and(|s| {
            s.status == SessionStatus::Idle && s.goal.is_some_and(|g| g.phase == GoalPhase::Paused)
        }) && text(&core, &child) == "FINAL_PUBLIC_CHILD"
    })
    .await;
    assert!(
        core.sessions
            .session_status(CHAT)
            .unwrap()
            .last_completed_turn
            .is_none(),
        "goal pause is not successful completion"
    );
    let users = core
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .read_entries()
        .unwrap();
    assert_eq!(
        users
            .iter()
            .find(|e| e.id == "duplicate-user")
            .unwrap()
            .status,
        Some(zeron_doc::MessageStatus::Aborted)
    );
    assert_eq!(
        users.iter().find(|e| e.id == "pause-user").unwrap().status,
        Some(zeron_doc::MessageStatus::Complete)
    );
    assert_eq!(
        core.doc_host
            .fetch_tool_blob(&format!("{child}/child-tool"))
            .await
            .unwrap(),
        "FINAL_OUTPUT"
    );
    let child_entries = child_handle.doc().read_entries().unwrap();
    assert_eq!(child_entries.len(), 1);
    assert!(
        child_entries
            .iter()
            .all(|e| e.role == MessageRole::Assistant)
    );
    assert!(child_entries[0].parts.iter().any(|p| matches!(p, MessagePart::Error { message, .. } if message.contains("2 public transcript updates omitted"))));
    core.doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Steer {
                prompt: "/goal resume".into(),
                message_id: Some("resume-user".into()),
            },
        )
        .unwrap();
    wait(|| {
        core.sessions.session_status(CHAT).is_some_and(|s| {
            s.status == SessionStatus::Idle
                && s.goal.is_some_and(|g| g.phase == GoalPhase::Complete)
        })
    })
    .await;
    let lines = std::fs::read_to_string(dir.path().join("extension-requests.jsonl")).unwrap();
    let prompts = lines
        .lines()
        .filter(|s| {
            serde_json::from_str::<serde_json::Value>(s).unwrap()["method"] == "session/prompt"
        })
        .count();
    assert_eq!(
        prompts, 2,
        "start and resume only; pause never became a queued model prompt"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn trusted_link_after_done_and_reattach_updates_original_card_and_real_child_doc() {
    let dir = tempfile::tempdir().unwrap();
    let first = core(dir.path());
    create(&first, dir.path());
    first
        .sessions
        .dispatch(
            CHAT,
            HarnessId::Mimir,
            request(dir.path(), "late child"),
            None,
        )
        .await
        .unwrap();
    wait(|| {
        first
            .sessions
            .session_status(CHAT)
            .is_some_and(|s| s.status == SessionStatus::Idle)
    })
    .await;
    let root = first.doc_host.open(CHAT).unwrap();
    let before = root.doc().read_entries().unwrap();
    assert!(
        child_ref(&first).is_none(),
        "the strict peer delays semantic linkage until after Done"
    );
    wait(|| child_ref(&first).is_some()).await;
    let child = child_ref(&first).unwrap();
    assert_eq!(text(&first, &child), "PUBLIC_CHILD");
    assert_eq!(
        root.doc().read_entries().unwrap().len(),
        before.len(),
        "late semantics must not mint a parent turn"
    );
    let marker = first
        .sessions
        .session_status(CHAT)
        .unwrap()
        .last_completed_turn;
    first.shutdown().await;
    drop(root);
    drop(first);
    let restored = core(dir.path());
    assert_eq!(text(&restored, &child), "PUBLIC_CHILD");
    restored
        .sessions
        .dispatch(
            CHAT,
            HarnessId::Mimir,
            request(dir.path(), "reattach child"),
            None,
        )
        .await
        .unwrap();
    wait(|| {
        text(&restored, &child) == "FINAL_PUBLIC_CHILD"
            && restored
                .sessions
                .session_status(CHAT)
                .is_some_and(|s| s.status == SessionStatus::Idle)
    })
    .await;
    assert_eq!(child_ref(&restored).as_deref(), Some(child.as_str()));
    assert_eq!(
        restored
            .doc_host
            .open(&child)
            .unwrap()
            .doc()
            .read_entries()
            .unwrap()
            .len(),
        1
    );
    let entries = restored
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .read_entries()
        .unwrap();
    assert_eq!(
        entries
            .iter()
            .flat_map(|e| &e.parts)
            .filter(|p| p.id() == "launch-1")
            .count(),
        1
    );
    assert!(marker.is_some());
    let lines = std::fs::read_to_string(dir.path().join("extension-requests.jsonl")).unwrap();
    assert!(lines.lines().any(
        |s| serde_json::from_str::<serde_json::Value>(s).unwrap()["method"] == "session/load"
    ));
    restored.shutdown().await;
}
