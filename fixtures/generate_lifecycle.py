"""Print independent first-standard lifecycle witnesses; no Raiv code or file writes.

Run from any directory: python fixtures/generate_lifecycle.py
Preferred CBOR is limited to this synthetic fixture subset. Hashing/base32 use the
Python standard library. This is fixture generation, not an accepted-data migration.
"""
import base64
import copy
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent / "format"


def records(name):
    return json.loads((ROOT / (name + ".json")).read_text())


def head(major, number):
    if number < 24:
        return bytes([major * 32 + number])
    for width, additional in [(1, 24), (2, 25), (4, 26), (8, 27)]:
        if number < 1 << (width * 8):
            return bytes([major * 32 + additional]) + number.to_bytes(width, "big")
    raise ValueError("fixture integer/length exceeds u64")


def cbor(value):
    if value is None:
        return b"\xf6"
    if isinstance(value, bool):
        return b"\xf5" if value else b"\xf4"
    if isinstance(value, int):
        return head(0, value) if value >= 0 else head(1, -1 - value)
    if isinstance(value, str):
        payload = value.encode("utf-8")
        return head(3, len(payload)) + payload
    if isinstance(value, list):
        return head(4, len(value)) + b"".join(cbor(child) for child in value)
    if isinstance(value, dict):
        if set(value) == {"$bytes"}:
            payload = bytes.fromhex(value["$bytes"])
            return head(2, len(payload)) + payload
        if set(value) == {"$link"}:
            text = value["$link"][1:].upper()
            payload = b"\x00" + base64.b32decode(text + "=" * (-len(text) % 8))
            assert len(payload) == 37 and payload[:5] == b"\x00\x01\x71\x12\x20"
            return head(6, 42) + head(2, len(payload)) + payload
        entries = sorted((cbor(key), cbor(child)) for key, child in value.items())
        return head(5, len(entries)) + b"".join(key + child for key, child in entries)
    raise TypeError("unsupported fixture value")


def digest(value):
    return hashlib.sha256(cbor(value)).hexdigest()


def cid(value):
    binary = b"\x01\x71\x12\x20" + bytes.fromhex(digest(value))
    return "b" + base64.b32encode(binary).decode().lower().rstrip("=")


def raw(number):
    return {"$bytes": (bytes([number]) * 32).hex()}


def link(key):
    return {"$link": key} if key else None


def witness(value):
    return {"cid": cid(value), "hex": cbor(value).hex()}


def merge(base, patch):
    result = copy.deepcopy(base) if isinstance(base, dict) else {}
    for key, value in patch.items():
        if value is None:
            result.pop(key, None)
        elif isinstance(value, dict):
            result[key] = merge(result.get(key), value)
        else:
            result[key] = copy.deepcopy(value)
    return result


def normalize(value):
    if isinstance(value, dict):
        result = {key: normalize(child) for key, child in value.items()}
        return {key: child for key, child in result.items() if child != {}}
    if isinstance(value, list):
        assert all(child is not None for child in value)
        return [normalize(child) for child in value]
    assert value is not None
    return value


def generate(world_id=None):
    definition = records("definition")[1]["record"]
    state = copy.deepcopy(records("state")[1]["record"])
    genesis = records("genesis")[1]
    # Start from explicitly authored genesis fields, never transform accepted bytes.
    state.update(agent_id={"$bytes": genesis["agent_id"]}, revision=0,
                 definition=link(cid(definition)), memory=link(cid({"kind": "memory", "version": 1, "value": {}})),
                 current_episode=None, history=[], continuation={"tag": "await_actor"})
    assert cid(state) == genesis["root"]
    templates = [
        (None, {"intents": {"trade": "complete_trade"}, "strategy": {"retry": {"rule": "Inspect receipt before retry."}}, "tactics": {"trade": {"attempt": 1}}, "goals": [None], "Tactics": {"wrong": True}}, "trade", {"amount": "3", "attempt": 1}),
        ("failed", {"tactics": {"trade": {"attempt": 2}}}, "trade", {"amount": "3", "attempt": 2}),
        ("succeeded", {"tactics": {"trade": None, "wake": {"after_ticks": 1}}}, "set_wake", {"after_ticks": 1}),
        ("succeeded", {"intents": {"trade": None}, "tactics": None, "strategy": {"retry": {"observed": "succeeded_after_retry"}}}, None, {}),
        (None, {"intent": None, "operations": [None]}, None, {}),
        (None, {"strategy": {}}, None, {}),
    ]
    known = {cid(definition): definition}
    value, episode, previous = {}, None, None
    empty = {"kind": "memory", "version": 1, "value": {}}
    known[cid(empty)] = empty
    steps = []
    for index, (outcome, patch, tool_id, arguments) in enumerate(templates):
        event = raw(170 + index)
        input_record = copy.deepcopy(records("input")[0]["record"])
        input_record["event_id"] = event
        input_record["budget_admission"].update(activation_units=100, remaining_episode_units=100, remaining_model_calls=16)
        if world_id is not None:
            input_record["budget_admission"]["admission_id"] = raw(0x90 + index)
            input_record["observed_at"] = {"domain": raw(0x30), "number": index, "reference": None}
            input_record["observation"] = {}
        if previous is not None:
            receipt = copy.deepcopy(records("terminal")["receipts"][0]["record"])
            receipt.update(action_id=previous["action_id"], command_digest={"$bytes": digest(["raiv/command/1", definition["execution_domain"], previous])}, outcome=outcome)
            if world_id is not None:
                receipt.update(result={}, execution_reference=None, finalized_at={"domain": {"$bytes": world_id}, "number": index, "reference": None}, costs={"virtual_units": 0 if index == 3 else 1})
            input_record.update(cause="action_result", receipt=receipt)
        else:
            assert outcome is None
        entry = state["memory"]
        if episode is None:
            eid = {"$bytes": digest(["raiv/episode/1", definition["execution_domain"], state["agent_id"], event])}
            episode = {"kind": "episode", "version": 1, "id": eid, "trigger_event_id": event, "entry_memory": entry, "boundaries": [], "closed": False}
        before = value
        selected = {key: child for key, child in patch.items() if key in ("intents", "strategy", "tactics")}
        value = normalize(merge(value, selected))
        ordered = sorted(patch, key=cbor)
        report = {"memory": copy.deepcopy(value), "changed": before != value,
                  "processed_planes": [key for key in ordered if key in selected],
                  "skipped_planes": [{"plane": key, "reason": "unsupported_root"} for key in ordered if key not in selected]}
        memory = {"kind": "memory", "version": 1, "value": value}
        ordinal = len(episode["boundaries"])
        if tool_id is None:
            exit_value = {"tag": "await_actor"}
            previous = None
            continuation = {"tag": "await_actor"}
            episode["closed"] = True
        else:
            tool = next(tool for tool in definition["tools"] if tool["id"] == tool_id)
            aid = {"$bytes": digest(["raiv/action/1", definition["execution_domain"], state["agent_id"], episode["id"], ordinal])}
            previous = {"action_id": aid, "tool_id": tool_id, "tool_version": tool["version"], "authority_binding": tool["authority_binding"], "arguments": arguments}
            exit_value = {"tag": "await_action", "command": previous}
            continuation = {"tag": "await_action", "action_id": aid}
        boundary = {"kind": "boundary", "version": 1, "ordinal": ordinal, "input": link(cid(input_record)), "memory": link(cid(memory)), "exit": exit_value,
                    "usage": {"model_calls": 2, "local_tool_calls": 1, "input_tokens": None, "output_tokens": None, "billed_units": None}}
        episode["boundaries"].append(link(cid(boundary)))
        history = state["history"] + ([link(cid(episode))] if episode["closed"] else [])
        state = dict(state, revision=index + 1, memory=link(cid(memory)), history=history[-max(1, definition["limits"]["history_episodes"]):],
                     current_episode=None if episode["closed"] else link(cid(episode)), continuation=continuation)
        for record in [input_record, memory, boundary, episode, state]:
            known[cid(record)] = copy.deepcopy(record)
        required = set()
        def visit(key):
            if key in required:
                return
            required.add(key)
            def links(value):
                if isinstance(value, dict):
                    if set(value) == {"$link"}:
                        visit(value["$link"])
                    else:
                        for child in value.values():
                            links(child)
                elif isinstance(value, list):
                    for child in value:
                        links(child)
            links(known[key])
        visit(cid(state))
        steps.append({"event": 170 + index, "outcome": outcome, "patch": patch, "tool_id": tool_id, "arguments": arguments, "memory_value": copy.deepcopy(value),
                      "patch_result": {"value": report, "hex": cbor(report).hex(), "sha256": digest(report)},
                      "input": witness(input_record), "memory": witness(memory), "boundary": witness(boundary), "episode": witness(episode), "state": witness(state),
                      "required_cids": sorted(required), "closure_bytes": sum(len(cbor(known[key])) for key in required)})
        if episode["closed"]:
            episode = None
    return {"evidence": "Independent preferred-CBOR/hashlib/base32 scripted checkpoint DATA, not host acceptance, actual effects or WASM execution.",
            "genesis_root": genesis["root"], "definition_cid": cid(definition), "steps": steps}


if __name__ == "__main__":
    print(json.dumps(generate(), indent=2))
