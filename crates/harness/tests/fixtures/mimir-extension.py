#!/usr/bin/env python3
"""Strict public-extension peer: a pause RPC must not create another model lease."""
import json
import os
import sys
import threading
output_lock = threading.Lock()

assert sys.argv[1:] == ["acp"]
S = "native-parent"
phase = None
prompt_id = None
prompts = 0
restarted = False
paused = False

def emit(value):
    with output_lock:
        print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)

def reply(i, value):
    emit({"id": i, "result": value})

def update(value):
    emit({"method": "session/update", "params": {"sessionId": S, "update": value}})

def text(value):
    return {"sessionUpdate": "agent_message_chunk", "messageId": "child-text", "content": {"type": "text", "text": value}}

def state():
    return {"sessionId": S,
            "goal": None if phase is None else {"id": "goal-1", "objective": "Inspect public files", "phase": phase, "reason": None, "completion": None},
            "children": [] if phase is None else [{"childId": "child-1", "toolCallId": "launch-1", "title": "Inspect files", "agent": "explore", "status": "completed" if paused else "running", "transcript": True}],
            "childrenTruncated": False}

def late_link():
    global phase
    phase = "active"
    emit({"method": "_mimir/session/state", "params": state()})

for line in sys.stdin:
    frame = json.loads(line)
    method = frame.get("method")
    p = frame.get("params", {})
    i = frame.get("id")
    with open("extension-requests.jsonl", "a") as log:
        log.write(json.dumps({"method": method, "params": p, "paused": paused}) + "\n")
    if method == "initialize":
        assert p["clientCapabilities"]["_meta"]["mimir.dev/session"] == {"version": 1}
        reply(i, {"protocolVersion": 1, "agentCapabilities": {"_meta": {"mimir.dev/session": {"version": 1, "goalControl": True, "childTranscript": True}}}})
    elif method in ("session/new", "session/load"):
        if method == "session/load":
            phase = "active"
            paused = True
            restarted = True
        reply(i, {"sessionId": S, "configOptions": []})
    elif method == "_mimir/session/state":
        assert p["sessionId"] == S
        reply(i, state())
    elif method == "session/prompt":
        prompts += 1
        request_text = p["prompt"][0]["text"]
        if prompts == 1 and request_text == "late child":
            update({"sessionUpdate": "tool_call", "toolCallId": "launch-1", "title": "Unknown until trusted state", "kind": "other", "status": "in_progress"})
            reply(i, {"stopReason": "end_turn"})
            threading.Timer(1.0, late_link).start()
        elif prompts == 1 and request_text == "reattach child":
            assert paused, "reattach must use session/load"
            emit({"method": "_mimir/session/state", "params": state()})
            reply(i, {"stopReason": "end_turn"})
        elif prompts == 1:
            assert request_text == "start native goal"
            prompt_id = i
            phase = "active"
            update({"sessionUpdate": "tool_call", "toolCallId": "launch-1", "title": "A descriptive title is not agent identity", "kind": "other", "status": "in_progress"})
            emit({"method": "_mimir/session/state", "params": state()})
            foreign = state()
            foreign["sessionId"] = "foreign"
            foreign["goal"]["objective"] = "WRONG_SESSION"
            emit({"method": "_mimir/session/state", "params": foreign})
        else:
            assert prompts == 2 and paused and request_text == "/goal resume", (prompts, request_text)
            phase = "complete"
            emit({"method": "_mimir/session/state", "params": state()})
            update(text("RESUMED_ON_SECOND_LEASE"))
            reply(i, {"stopReason": "end_turn"})
    elif method == "_mimir/session/goal":
        assert p["sessionId"] == S and prompts == 1, "control acquired another model lease"
        command = p["command"]
        if command == "pause":
            assert prompt_id is not None
            phase = "paused"
            paused = True
            update({"sessionUpdate": "tool_call_update", "toolCallId": "launch-1", "status": "completed", "content": [{"type": "content", "content": {"type": "text", "text": "Public child finished"}}]})
            # Deliberately make the parent response ready before the control.
            # Its acknowledgement must still precede the parent's Done.
            reply(prompt_id, {"stopReason": "end_turn", "_meta": {"mimir": {"outcome": "interaction-required"}}})
            prompt_id = None
        else:
            assert command == "show"
        threading.Timer(0.2, reply, args=(i, state())).start()
    elif method == "_mimir/session/child":
        assert p["sessionId"] == S and p["childId"] == "child-1"
        cursor = p.get("cursor", 0)
        revision = "final" if paused else ("r2" if restarted else "r1")
        if cursor and not restarted:
            restarted = True
            emit({"id": i, "error": {"code": -32600, "message": "child revision changed; restart at cursor zero"}})
            continue
        if cursor == 0:
            updates = [text("FINAL_PUBLIC_CHILD" if paused else ("PUBLIC_CHILD" if restarted else "STALE_REVISION")),
                       {"sessionUpdate": "tool_call", "toolCallId": "child-tool", "title": "printf public", "kind": "execute"}]
            next_cursor = 2
        else:
            assert cursor == 2 and p["revision"] == revision
            updates = [{"sessionUpdate": "tool_call_update", "toolCallId": "child-tool", "status": "completed", "content": [{"type": "content", "content": {"type": "text", "text": "FINAL_OUTPUT" if paused else "PUBLIC_OUTPUT"}}]}]
            next_cursor = None
        reply(i, {"sessionId": S, "childId": "child-1", "revision": revision, "updates": updates, "nextCursor": next_cursor, "omittedUpdates": 2 if cursor == 0 else 0})
    elif method == "session/cancel":
        if prompt_id is not None:
            reply(prompt_id, {"stopReason": "cancelled"})
        break
    else:
        raise AssertionError(frame)
