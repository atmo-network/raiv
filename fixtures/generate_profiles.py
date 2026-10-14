"""Reproduce synthetic first-standard profile metadata and dependent witnesses.

Fixed fixture directory only; no accepted checkpoints, recovery or migration input.
Independent preferred CBOR/hashlib/base32 from generate_lifecycle, never Raiv code.
Emit a JSON filename -> document bundle on stdout; caller compares/installs changes.
"""
import base64
import copy
import hashlib
import json
import sys
from pathlib import Path
from generate_lifecycle import cbor, cid

ROOT = Path(__file__).resolve().parent / "format"
DESCRIPTION = ("Patch the object itself using case-sensitive intents, strategy and tactics. "
               "Unsupported roots are skipped with unsupported_root feedback, never remapped. "
               "Selected roots must be object/null: objects merge, null deletes, arrays replace wholly "
               "and forbid nested null; empty object fields normalize away. "
               "Result contains full memory, changed, processed_planes and skipped_planes reasons; "
               "processed does not imply changed. Partial/all-skipped/no-effect remains Continue "
               "without forced retry. Absent supported planes may be created. "
               "Persisted Memory is null-free; no checkpoint or authority change.")
POLICY = ("Fixture local-trusted policy. Editable Memory has optional intents (chosen goals, "
          "commitments, unfinished tasks and completion conditions), strategy (approaches, reasons, "
          "uncertainties and revision conditions), and tactics (situational implementation ideas "
          "and conditional continuations). These exact case-sensitive names provide low-cost "
          "semantic discipline, not a guarantee of good reasoning. Patch arguments are the object "
          "itself, not paths, operations or a result envelope. Under selected supported roots, "
          "objects merge, null deletes object fields, arrays replace wholly and contain no null "
          "at any depth. Empty object fields normalize away; array positions and false/0/empty "
          "text/empty arrays remain. Unsupported top-level names are skipped with explicit "
          "unsupported_root reasons, never translated or materialized; raw skipped data still "
          "obeys all model-data and resource bounds. Invalid selected content rejects the selected "
          "patch without mutation; no nested salvage. The advertised closed root schema describes "
          "intended vocabulary, not reject-all-unknown handling. Read the full returned memory, "
          "changed, processed_planes and skipped_planes; processed does not imply changed. "
          "Partial/all-skipped/no-effect stays Continue without forced correction, replay or Finish. "
          "Absent supported planes may be created. No references or cross-plane cascades. Tactics "
          "do not cancel accepted Commands or establish success. Memory is separate from mandate, "
          "observations, Receipts, phase, budgets and authority. Patching stays in RAM; a complete "
          "response without tool calls finishes an Episode without clearing Memory.")


def decode(payload):
    # Only the trusted fixture subset; exact preferred re-encoding is checked below.
    pos = 0

    def item():
        nonlocal pos
        head = payload[pos]
        pos += 1
        major, additional = head >> 5, head & 31
        if additional < 24:
            size = additional
        else:
            width = {24: 1, 25: 2, 26: 4, 27: 8}[additional]
            size = int.from_bytes(payload[pos:pos + width], "big")
            pos += width
        if major == 0:
            return size
        if major == 1:
            return -1 - size
        if major in (2, 3):
            value = payload[pos:pos + size]
            pos += size
            return {"$bytes": value.hex()} if major == 2 else value.decode("utf-8")
        if major == 4:
            return [item() for _ in range(size)]
        if major == 5:
            fields = {}
            for _ in range(size):
                key = item()
                assert isinstance(key, str) and key not in fields
                fields[key] = item()
            return fields
        if major == 6:
            assert size == 42
            value = bytes.fromhex(item()["$bytes"])
            assert len(value) == 37 and value[:5] == b"\x00\x01\x71\x12\x20"
            return {"$link": "b" + base64.b32encode(value[1:]).decode().lower().rstrip("=")}
        assert major == 7
        return {20: False, 21: True, 22: None}[size]

    record = item()
    assert pos == len(payload) and cbor(record) == payload
    return record


def documents():
    source = {p.name: json.loads(p.read_text()) for p in sorted(ROOT.glob("*.json"))}
    blocks = {}

    def collect(value):
        if isinstance(value, list):
            for child in value:
                collect(child)
        elif isinstance(value, dict):
            if "hex" in value and ("cid" in value or "record" in value):
                record = decode(bytes.fromhex(value["hex"]))
                if "record" in value:
                    assert value["record"] == record
                if "cid" in value:
                    assert cid(record) == value["cid"]
                    assert value["cid"] not in blocks or blocks[value["cid"]] == record
                    blocks[value["cid"]] = record
            for child in value.values():
                collect(child)

    for document in source.values():
        collect(document)
    mapping, rebuilt, active = {}, {}, set()

    def links(value):
        if isinstance(value, list):
            return [links(child) for child in value]
        if isinstance(value, dict):
            if set(value) == {"$link"}:
                key = value["$link"]
                return {"$link": resolve(key) if key in blocks else key}
            return {key: links(child) for key, child in value.items()}
        return value

    def metadata(record):
        result = copy.deepcopy(record)
        if result.get("kind") == "definition":
            result["tools"] = [tool for tool in result["tools"] if tool["id"] != "no_op"]
        if result.get("kind") == "definition" and result.get("memory_profile") == "intents-strategy-tactics/1":
            result["policy"] = POLICY
            tool = next(tool for tool in result["tools"] if tool["id"] == "patch_memory")
            tool["description"] = DESCRIPTION
        return result

    def resolve(old):
        if old in mapping:
            return mapping[old]
        assert old not in active
        active.add(old)
        rebuilt[old] = links(metadata(blocks[old]))
        mapping[old] = cid(rebuilt[old])
        active.remove(old)
        return mapping[old]

    for old in blocks:
        resolve(old)
    sizes = {mapping[old]: len(cbor(record)) for old, record in rebuilt.items()}

    def update(value):
        if isinstance(value, list):
            return [update(child) for child in value]
        if isinstance(value, str):
            return mapping.get(value, value)
        if not isinstance(value, dict):
            return value
        if "hex" in value and ("record" in value or value.get("cid") in rebuilt):
            record = rebuilt[value["cid"]] if value.get("cid") in rebuilt else links(metadata(decode(bytes.fromhex(value["hex"]))))
            result = copy.deepcopy(value)
            encoded = cbor(record)
            result["hex"] = encoded.hex()
            if "record" in result:
                result["record"] = record
            if "cid" in result:
                result["cid"] = cid(record)
            if "sha256" in result:
                result["sha256"] = hashlib.sha256(encoded).hexdigest()
            return result
        result = {key: update(child) for key, child in value.items()}
        if "blocks" in result:
            result["closure_bytes"] = sum(len(block["hex"]) // 2 for block in result["blocks"])
        if "required_cids" in result:
            result["required_cids"].sort()
            result["closure_bytes"] = sum(sizes[key] for key in result["required_cids"])
        return result

    return {name: update(document) for name, document in source.items()}


if __name__ == "__main__":
    bundle = documents()
    if sys.argv[1:] == ["--check"]:
        assert all(document == json.loads((ROOT / name).read_text()) for name, document in bundle.items())
        print(f"Synthetic profile metadata/link/byte totals reproduce across {len(bundle)} fixture documents")
    elif not sys.argv[1:]:
        print(json.dumps(bundle, indent=2))
    else:
        raise SystemExit("usage: python fixtures/generate_profiles.py [--check]")
