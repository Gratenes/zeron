//! A batch the room refuses is refused on every push, for ever, unless the
//! client does something about it. The room refuses a batch whole for one op
//! over `MAX_OP_BYTES`; a `sessions` row with a long background command was
//! one (2026-11: 1,798 refusals across four old chats, replayed on every
//! connection). Rows are now bounded where written, queued ops that exceed
//! the cap are clipped or dropped before they are pushed, and a batch the
//! room refuses is split, then its lone op dropped.

use super::*;
use zeron_proto::{BackgroundTask, SessionStatus, SessionWaiting, WaitingKind, WaitingOn};

fn ts(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap_or(DateTime::UNIX_EPOCH)
}

fn session(chat: &str, tasks: Vec<BackgroundTask>) -> zeron_proto::Session {
    zeron_proto::Session {
        chat_id: chat.into(),
        device_id: "dev-a".into(),
        status: SessionStatus::Working,
        started_at: None,
        updated_at: ts(1_000),
        running_subagents: 0,
        background_tasks: tasks,
        last_completed_turn: None,
        last_turn_outcome: None,
        terminal_activity: None,
        waiting: None,
    }
}

fn task(n: usize, command_bytes: usize) -> BackgroundTask {
    BackgroundTask {
        id: format!("task-{n}"),
        command: "x".repeat(command_bytes),
        description: Some("d".repeat(command_bytes)),
        started_at: ts(2_000),
    }
}

fn op_bytes(op: &RowOp) -> usize {
    serde_json::to_vec(op).unwrap().len()
}

fn ops(doc: &RegistryDoc) -> Vec<&RowOp> {
    doc.pending.iter().flat_map(|b| &b.ops).collect()
}

fn chat_op(id: &str, title: &str) -> RowOp {
    RowOp {
        kind: KIND_CHATS.into(),
        id: id.into(),
        op: OpKind::Upsert,
        set: Some(fields([("title", json!(title))])),
        hlc: encode_hlc(1_000, 0, "dev-a"),
        clocks: None,
    }
}

#[test]
fn a_session_row_with_huge_background_commands_is_bounded_where_it_is_written() {
    let mut doc = RegistryDoc::new("dev-a");
    // 40 background shells, each with a 5 KB script as its command line.
    let tasks: Vec<_> = (0..40).map(|n| task(n, 5_000)).collect();
    doc.upsert_session(&session("chat-1", tasks)).unwrap();

    let queued = ops(&doc);
    assert_eq!(queued.len(), 1);
    assert!(
        op_bytes(queued[0]) <= MAX_OP_BYTES,
        "{} bytes: the room would refuse it",
        op_bytes(queued[0])
    );
    let set = queued[0].set.as_ref().unwrap();
    let kept = set["backgroundTasks"].as_array().unwrap();
    assert_eq!(kept.len(), SYNCED_LIST_MAX);
    assert_eq!(
        kept[0]["id"], "task-0",
        "ids stay whole: tasks are addressed by them"
    );
    assert_eq!(
        kept[0]["command"].as_str().unwrap().len(),
        SYNCED_COMMAND_MAX
    );
    assert_eq!(
        kept[0]["description"].as_str().unwrap().len(),
        SYNCED_DESCRIPTION_MAX
    );
}

#[test]
fn clipping_never_splits_a_character() {
    let mut doc = RegistryDoc::new("dev-a");
    let mut one = task(0, 0);
    // Three-byte characters straddling the byte cap.
    one.command = "€".repeat(400);
    doc.upsert_session(&session("chat-1", vec![one])).unwrap();
    let set = ops(&doc)[0].set.clone().unwrap();
    let command = set["backgroundTasks"][0]["command"].as_str().unwrap();
    assert!(command.len() <= SYNCED_COMMAND_MAX);
    assert!(command.chars().all(|c| c == '€'), "cut on a boundary");
}

#[test]
fn a_waiting_row_with_long_labels_is_bounded_too() {
    let mut doc = RegistryDoc::new("dev-a");
    let mut row = session("chat-1", vec![]);
    row.waiting = Some(SessionWaiting {
        since: ts(1_000),
        on: (0..30)
            .map(|n| WaitingOn {
                id: format!("w-{n}"),
                kind: WaitingKind::Process,
                label: "y".repeat(5_000),
            })
            .collect(),
    });
    doc.upsert_session(&row).unwrap();
    let queued = ops(&doc);
    assert!(op_bytes(queued[0]) <= MAX_OP_BYTES);
    let on = queued[0].set.as_ref().unwrap()["waiting"]["on"]
        .as_array()
        .unwrap();
    assert_eq!(on.len(), SYNCED_LIST_MAX);
}

#[test]
fn a_session_row_that_still_cannot_fit_loses_its_lists_not_its_status() {
    let mut op = RowOp {
        kind: KIND_SESSIONS.into(),
        id: "chat-1".into(),
        op: OpKind::Upsert,
        set: Some(fields([
            ("status", json!("working")),
            // Ids are kept whole (up to their own cap), so 16 of them overflow
            // only with an absurd count of fields; model that with a big blob.
            (
                "backgroundTasks",
                json!([{ "id": "t", "command": "c", "pad": "p".repeat(20_000) }]),
            ),
        ])),
        hlc: encode_hlc(1_000, 0, "dev-a"),
        clocks: None,
    };
    assert!(fit_op(&mut op));
    let set = op.set.unwrap();
    assert_eq!(set["status"], "working");
    assert_eq!(set["backgroundTasks"], Value::Null);
}

#[test]
fn an_oversized_op_queued_before_the_cap_is_clipped_before_it_is_pushed() {
    let mut doc = RegistryDoc::new("dev-a");
    // What an older build left in the outbox: a refused session row, and its
    // neighbour in the same batch.
    let tasks: Vec<_> = (0..40).map(|n| task(n, 5_000)).collect();
    let poison = RowOp {
        kind: KIND_SESSIONS.into(),
        id: "chat-1".into(),
        op: OpKind::Upsert,
        set: Some(fields([
            ("chatId", json!("chat-1")),
            ("status", json!("working")),
            ("backgroundTasks", serde_json::to_value(&tasks).unwrap()),
        ])),
        hlc: encode_hlc(1_000, 0, "dev-a"),
        clocks: None,
    };
    assert!(op_bytes(&poison) > MAX_OP_BYTES);
    doc.pending.push(PendingBatch {
        batch: "b-old".into(),
        ops: vec![poison, chat_op("chat-9", "fine")],
        in_flight: false,
    });

    let pushable = doc.take_pushable();
    assert_eq!(pushable.len(), 1);
    assert_eq!(pushable[0].ops.len(), 2, "the neighbour is untouched");
    assert!(
        pushable[0]
            .ops
            .iter()
            .all(|op| op_bytes(op) <= MAX_OP_BYTES)
    );
    assert_eq!(
        pushable[0].ops[0].set.as_ref().unwrap()["status"],
        "working"
    );
}

#[test]
fn a_queued_op_that_cannot_be_made_to_fit_is_dropped_and_its_batch_survives() {
    let mut doc = RegistryDoc::new("dev-a");
    doc.pending.push(PendingBatch {
        batch: "b-old".into(),
        ops: vec![
            chat_op("chat-1", &"t".repeat(20_000)),
            chat_op("chat-2", "ok"),
        ],
        in_flight: false,
    });
    doc.pending.push(PendingBatch {
        batch: "b-only".into(),
        ops: vec![chat_op("chat-3", &"t".repeat(20_000))],
        in_flight: false,
    });
    let pushable = doc.take_pushable();
    assert_eq!(pushable.len(), 1, "a batch of nothing is not pushed");
    assert_eq!(pushable[0].batch, "b-old");
    assert_eq!(pushable[0].ops.len(), 1);
    assert_eq!(pushable[0].ops[0].id, "chat-2");
}

#[test]
fn a_new_oversized_write_is_never_queued() {
    let mut doc = RegistryDoc::new("dev-a");
    doc.enqueue_ops(vec![
        chat_op("chat-1", &"t".repeat(20_000)),
        chat_op("chat-2", "ok"),
    ]);
    assert_eq!(ops(&doc).len(), 1);
    assert_eq!(ops(&doc)[0].id, "chat-2");
}

fn batch_of(doc: &mut RegistryDoc, name: &str, ids: &[&str]) {
    doc.pending.push(PendingBatch {
        batch: name.into(),
        ops: ids.iter().map(|id| chat_op(id, "t")).collect(),
        in_flight: false,
    });
}

#[test]
fn a_batch_the_room_refuses_is_split_once_and_then_its_lone_op_dropped() {
    let mut doc = RegistryDoc::new("dev-a");
    batch_of(&mut doc, "b-1", &["a", "b", "c"]);
    batch_of(&mut doc, "b-2", &["z"]);
    let sent = doc.take_pushable();
    assert_eq!(sent.len(), 2);

    // HTTPS: the answer names the batch.
    let outcome = doc.reject_batch("b-1");
    assert_eq!(outcome.split, 3);
    assert!(outcome.dropped.is_empty());
    assert!(outcome.repush());
    assert_eq!(doc.pending_len(), 4, "three singles and the untouched b-2");
    let order: Vec<_> = doc.pending.iter().map(|b| b.ops[0].id.clone()).collect();
    assert_eq!(order, ["a", "b", "c", "z"], "order is kept");
    assert!(
        doc.pending[..3]
            .iter()
            .all(|b| !b.in_flight && b.ops.len() == 1)
    );
    assert!(doc.pending[3].in_flight, "the other batch is not touched");

    // One of the singles is the culprit: refused alone, it is dropped.
    let singles = doc.take_pushable();
    assert_eq!(singles.len(), 3);
    let culprit = singles[1].batch.clone();
    let outcome = doc.reject_batch(&culprit);
    assert_eq!(outcome.dropped, [(KIND_CHATS.to_owned(), "b".to_owned())]);
    assert!(!outcome.repush());
    let left: Vec<_> = doc.pending.iter().map(|b| b.ops[0].id.clone()).collect();
    assert_eq!(left, ["a", "c", "z"]);

    // An unknown batch (already acked) is nothing.
    assert_eq!(doc.reject_batch("b-gone"), Rejection::default());
}

#[test]
fn a_refusal_that_names_a_row_isolates_it_without_dropping_an_innocent_op() {
    let mut doc = RegistryDoc::new("dev-a");
    batch_of(&mut doc, "b-1", &["a", "b"]);
    batch_of(&mut doc, "b-2", &["c"]);
    doc.take_pushable();

    // The socket's error names the row. `b` is in the two-op batch: it is
    // split, never dropped on a guess.
    let outcome = doc.reject_row(KIND_CHATS, "b");
    assert_eq!(outcome.split, 2);
    assert!(outcome.dropped.is_empty());
    assert!(outcome.repush());

    // It goes out again as singles, and is refused again, alone this time.
    doc.take_pushable();
    let outcome = doc.reject_row(KIND_CHATS, "b");
    assert_eq!(outcome.dropped, [(KIND_CHATS.to_owned(), "b".to_owned())]);
    let left: Vec<_> = doc.pending.iter().map(|b| b.ops[0].id.clone()).collect();
    assert_eq!(left, ["a", "c"]);
}

#[test]
fn two_ops_on_one_row_in_flight_are_requeued_not_dropped_on_a_refusal() {
    let mut doc = RegistryDoc::new("dev-a");
    batch_of(&mut doc, "b-1", &["a"]);
    batch_of(&mut doc, "b-2", &["a"]);
    doc.take_pushable();
    let outcome = doc.reject_row(KIND_CHATS, "a");
    assert!(
        outcome.dropped.is_empty(),
        "which of the two was refused is unknown"
    );
    assert_eq!(outcome.requeued, 2);
    assert!(doc.pending.iter().all(|b| !b.in_flight));
    assert_eq!(doc.pending_len(), 2);
}
