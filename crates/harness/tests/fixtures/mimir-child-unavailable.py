#!/usr/bin/env python3
"""Strict peer for bounded child transcript retries and lifecycle resets."""
import json
import sys
import time

assert sys.argv[1:] == ["acp"]
SESSION = "unavailable-child-parent"
transcript_available = True
goal_calls = 0
blocker_visible = False
child_calls = {"recover-child": 0, "unavailable-child": 0, "blocker-child": 0}


def emit(value):
    print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)


def reply(request_id, value):
    emit({"id": request_id, "result": value})


def state():
    return {
        "sessionId": SESSION,
        "goal": {
            "id": "goal-1",
            "objective": "Test unavailable public transcripts",
            "phase": "active",
            "reason": None,
            "completion": None,
        },
        "children": [
            {
                "childId": "recover-child",
                "toolCallId": "recover-tool",
                "title": "Eventually readable",
                "agent": "explore",
                "status": "running",
                "transcript": True,
            },
            {
                "childId": "unavailable-child",
                "toolCallId": "unavailable-tool",
                "title": "Unavailable transcript",
                "agent": "explore",
                "status": "completed",
                "transcript": transcript_available,
            },
            {
                "childId": "blocker-child",
                "toolCallId": "blocker-tool",
                "title": "Coalescing barrier",
                "agent": "explore",
                "status": "completed",
                "transcript": blocker_visible,
            },

        ],
        "childrenTruncated": False,
    }


for line in sys.stdin:
    frame = json.loads(line)
    method = frame.get("method")
    params = frame.get("params", {})
    request_id = frame.get("id")
    with open("child-unavailable-requests.jsonl", "a") as log:
        log.write(json.dumps({
            "method": method,
            "childId": params.get("childId"),
            "transcriptAvailable": transcript_available,
        }) + "\n")

    if method == "initialize":
        assert params["clientCapabilities"]["_meta"]["mimir.dev/session"] == {"version": 1}
        reply(request_id, {
            "protocolVersion": 1,
            "agentCapabilities": {"_meta": {"mimir.dev/session": {
                "version": 1,
                "goalControl": True,
                "childTranscript": True,
            }}},
        })
    elif method == "session/new":
        reply(request_id, {"sessionId": SESSION, "configOptions": []})
    elif method == "_mimir/session/state":
        assert params["sessionId"] == SESSION
        reply(request_id, state())
    elif method == "session/prompt":
        reply(request_id, {"stopReason": "end_turn"})
    elif method == "_mimir/session/child":
        child = params["childId"]
        assert params["sessionId"] == SESSION and child in child_calls
        child_calls[child] += 1
        if child == "blocker-child":
            transcript_available = False
            emit({"method": "_mimir/session/state", "params": state()})
            transcript_available = True
            emit({"method": "_mimir/session/state", "params": state()})
            # Keep the observer inside this RPC until both notifications are applied.
            time.sleep(0.1)
            reply(request_id, {
                "sessionId": SESSION,
                "childId": child,
                "revision": "barrier",
                "updates": [],
                "nextCursor": None,
                "omittedUpdates": 0,
            })
        elif child == "recover-child" and child_calls[child] > 1:
            reply(request_id, {
                "sessionId": SESSION,
                "childId": child,
                "revision": "available",
                "updates": [{
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "RECOVERED_PUBLIC_CHILD"},
                }],
                "nextCursor": None,
                "omittedUpdates": 0,
            })
        else:
            emit({"id": request_id, "error": {
                "code": -32600,
                "message": "public child transcript is unavailable",
            }})
    elif method == "_mimir/session/goal":
        assert params == {"sessionId": SESSION, "command": "show"}
        goal_calls += 1
        assert goal_calls == 1
        blocker_visible = True
        reply(request_id, state())
    elif method == "session/cancel":
        break
    else:
        raise AssertionError(frame)
