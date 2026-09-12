#!/usr/bin/env python3
"""Deterministic native Mimir ACP peer; no provider or private journal access."""
import json
import sys

assert sys.argv[1:] == ["acp"], sys.argv
provider, model, effort, mode = "other", "other-model", "off", "mode-build"


def option(id, category, current, values):
    return {"id": id, "name": id, "type": "select", "category": category,
            "currentValue": current, "options": [{"value": v, "name": v} for v in values]}


def configs():
    models = ["gpt-5.6-luna", "small"] if provider == "chatgpt" else ["other-model"]
    efforts = ["off", "low", "medium", "high"] if model == "gpt-5.6-luna" else ["off"]
    return [option("mimir.mode", "mode", mode, ["mode-build", "mode-plan"]),
            option("mimir.provider", "model_config", provider, ["locked", "other", "chatgpt"]),
            option("model", "model", model, models),
            option("mimir.reasoning", "thought_level", effort, efforts)]


def emit(frame):
    print(json.dumps({"jsonrpc": "2.0", **frame}), flush=True)


def update(value):
    emit({"method": "session/update", "params": {"sessionId": "mimir-session", "update": value}})


for line in sys.stdin:
    req = json.loads(line)
    method, params = req.get("method"), req.get("params", {})
    result = {}
    if method == "initialize":
        result = {"protocolVersion": 1, "agentInfo": {"name": "mimir", "version": "fixture"},
                  "agentCapabilities": {"loadSession": True, "sessionCapabilities": {"resume": {}}}}
    elif method in ("session/new", "session/load", "session/resume"):
        update({"sessionUpdate": "available_commands_update", "availableCommands": [
            {"name": "plan", "description": "Plan before implementing"},
            {"name": "goal", "description": "Manage the persistent goal", "input": {"hint": "objective"}}]})
        result = {"sessionId": "mimir-session", "configOptions": configs()}
    elif method == "session/set_config_option":
        id, value = params["configId"], params["value"]
        if (id, value) in [("mimir.mode", "mode-plan"), ("mimir.reasoning", "high")]:
            emit({"id": req["id"], "error": {"code": -32000, "message": "requested setting rejected by fixture policy"}})
            continue
        if id == "mimir.provider" and value == "locked":
            emit({"id": req["id"], "error": {"code": -32000, "message": "Provider credentials are missing"}})
            continue
        config = next(c for c in configs() if c["id"] == id)
        if value not in [v["value"] for v in config["options"]]:
            emit({"id": req["id"], "error": {"code": -32602, "message": "unavailable option"}})
            continue
        if id == "mimir.provider":
            provider = value
            model = "small" if provider == "chatgpt" else "other-model"
            effort = "off"
        elif id == "model":
            model, effort = value, "off"
        elif id == "mimir.reasoning":
            # Delayed model-change notification precedes the NEW effort reply.
            update({"sessionUpdate": "config_option_update", "configOptions": configs()})
            effort = value
        elif id == "mimir.mode":
            mode = value
        result = {"configOptions": configs()}
    elif method == "session/prompt":
        update({"sessionUpdate": "agent_message_chunk", "messageId": "answer-1",
                "content": {"type": "text", "text": f"{provider}/{model} {effort}"}})
        result = {"stopReason": "end_turn"}
    elif method == "session/cancel":
        continue
    elif method == "session/close":
        pass
    else:
        if "id" in req:
            emit({"id": req["id"], "error": {"code": -32601, "message": "unsupported"}})
        continue
    if "id" in req:
        emit({"id": req["id"], "result": result})
