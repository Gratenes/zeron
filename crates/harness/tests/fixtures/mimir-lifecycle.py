#!/usr/bin/env python3
"""Strict ACP lifecycle peer for public harness tests; never calls a provider."""
import json
import os
import sys

assert sys.argv[1:] == ["acp"], sys.argv
SESSION = "mimir-lifecycle-session"
prompt_id = None
scenario = None
opened_with = None
pending = {}
load_only = "load-only" in os.path.basename(os.getcwd())


def emit(frame):
    print(json.dumps({"jsonrpc": "2.0", **frame}), flush=True)


def reply(request_id, result):
    emit({"id": request_id, "result": result})


def update(value, session=SESSION):
    emit({"method": "session/update", "params": {"sessionId": session, "update": value}})


def text(value, session=SESSION):
    update({"sessionUpdate": "agent_message_chunk", "messageId": value,
            "content": {"type": "text", "text": value}}, session)


def done(value="VERIFIED"):
    text(value)
    reply(prompt_id, {"stopReason": "end_turn"})


def form(request_id, *, full=False, session=SESSION, unsupported=False):
    properties = {"q0": {"type": "string", "title": "Example name?", "minLength": 1, "maxLength": 4096}}
    required = ["q0"]
    if full:
        properties.update({
            "q1": {"type": "string", "title": "Which implementation?", "oneOf": [
                {"const": "o0", "title": "Same", "description": "First"},
                {"const": "o1", "title": "Same", "description": "Second"},
                {"const": "none", "title": "None of the above"}]},
            "q1_note": {"type": "string", "title": "Optional note", "maxLength": 4096},
            "q2": {"type": "array", "title": "Which checks?", "items": {"anyOf": [
                {"const": "o0", "title": "Unit", "description": "Fast"},
                {"const": "o1", "title": "Process", "description": "Wire"},
                {"const": "none", "title": "None of the above"}]},
                "minItems": 1, "maxItems": 2, "uniqueItems": True},
            "q2_note": {"type": "string", "title": "Optional note", "maxLength": 4096}})
        required += ["q1", "q2"]
    if unsupported:
        properties["q0"]["pattern"] = "unsupported-pattern"
    pending[request_id] = True
    emit({"id": request_id, "method": "elicitation/create", "params": {
        "sessionId": session, "toolCallId": "tool-" + request_id, "mode": "form",
        "message": "Ordinary answers are public. Do not enter credentials.",
        "requestedSchema": {"type": "object", "properties": properties, "required": required}}})


def cancel(request_id):
    emit({"method": "$/cancel_request", "params": {"requestId": request_id}})


def answer(frame):
    request_id = frame["id"]
    assert request_id in pending, ("unexpected/stale response", frame)
    del pending[request_id]
    result = frame.get("result")
    if request_id == "full":
        assert result == {"action": "accept", "content": {
            "q0": "  Casey  ", "q1": "o1", "q1_note": "Keep this note",
            "q2": ["o0", "o1"]}}, frame
        done("FORM_ACCEPTED_EXACT")
    elif request_id == "old":
        assert result == {"action": "cancel"}, frame
        form("successor")
        cancel("old")  # A late cancellation must not affect the successor.
        cancel(999)    # Nor may an unrelated ID affect it.
    elif request_id == "successor":
        assert result == {"action": "accept", "content": {"q0": "Fresh answer"}}, frame
        done("SUCCESSOR_ACCEPTED_WITHOUT_STALE_ANSWER")
    elif request_id in ("foreign", "unsupported"):
        assert frame.get("error", {}).get("code") == -32602, frame
        assert result is None, frame
        text("WRONG_SESSION_TEXT", session="foreign-session")
        done("REJECTED_WITHOUT_ASKING")
    elif request_id == "decline":
        assert result == {"action": "decline"}, frame
        done("DECLINED_WITHOUT_ANSWER")
    elif request_id in ("dismiss", "interrupt", "prompt-ended"):
        assert result == {"action": "cancel"}, frame
        if request_id == "dismiss":
            done("DISMISSED_WITHOUT_ANSWER")
    else:
        raise AssertionError(("unhandled response", frame))


for line in sys.stdin:
    frame = json.loads(line)
    method, params = frame.get("method"), frame.get("params", {})
    if method is None:
        answer(frame)
    elif method == "initialize":
        assert params["protocolVersion"] == 1, params
        assert params["clientCapabilities"]["elicitation"]["form"] == {}, params
        assert not params["clientCapabilities"].get("terminal", False), params
        capabilities = {"loadSession": True, "sessionCapabilities": {"close": {}}}
        if not load_only:
            capabilities["sessionCapabilities"]["resume"] = {}
        reply(frame["id"], {"protocolVersion": 1, "agentInfo": {"name": "mimir", "version": "fixture"},
                            "agentCapabilities": capabilities})
    elif method in ("session/new", "session/resume", "session/load"):
        assert os.path.isabs(params["cwd"]), params
        if method != "session/new":
            assert params["sessionId"] == SESSION, params
            assert method == ("session/load" if load_only else "session/resume"), method
        opened_with = method
        if method == "session/load":
            text("REPLAYED_HISTORY_MUST_NOT_DUPLICATE")
        reply(frame["id"], {"sessionId": SESSION, "configOptions": []})
    elif method == "session/prompt":
        assert params["sessionId"] == SESSION, params
        prompt_id = frame["id"]
        assert len(params["prompt"]) == 1 and params["prompt"][0]["type"] == "text", params
        scenario = params["prompt"][0]["text"]
        update({"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "plan", "description": "Plan a task", "input": {"hint": "task"}},
            {"name": "goal", "description": "Manage the autonomous goal", "input": {"hint": "objective|resume|pause|clear"}}]})
        if scenario == "forms":
            form("full", full=True)
        elif scenario == "cancel-successor":
            form("old")
            cancel("old")
        elif scenario in ("foreign", "unsupported", "decline", "dismiss", "interrupt", "prompt-ended"):
            form(scenario, session="foreign-session" if scenario == "foreign" else SESSION,
                 unsupported=scenario == "unsupported")
            if scenario == "prompt-ended":
                reply(prompt_id, {"stopReason": "end_turn"})
        elif scenario in ("goal-paused", "goal-blocked"):
            reply(prompt_id, {"stopReason": "end_turn", "_meta": {"mimir": {
                "outcome": "interaction-required", "interaction": {
                    "kind": scenario, "question": "Which implementation should I use?",
                    "goalId": "PRIVATE_GOAL_ID", "phase": "PRIVATE_PHASE", "cause": "PRIVATE_CAUSE",
                    "reason": "PRIVATE_REASON", "workTurns": 987,
                    "timing": {"kind": "bounded", "requestedSeconds": 1800}, "command": "/goal resume"}}}})
        elif scenario in ("/plan", "/goal", "/goal resume"):
            done("COMMAND_UNCHANGED:" + scenario)
        elif scenario == "resume-check":
            done("OPENED_WITH:" + opened_with)
        else:
            raise AssertionError(("unexpected prompt text", scenario))
    elif method == "session/cancel":
        assert params["sessionId"] == SESSION, params
        reply(prompt_id, {"stopReason": "cancelled"})
    elif method == "session/close":
        reply(frame["id"], {})
    else:
        raise AssertionError(("unexpected method", frame))
