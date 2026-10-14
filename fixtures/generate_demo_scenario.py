#!/usr/bin/env python3
"""Independent fixed DATA oracle for the native unit-ledger demo, no accepted-storage input."""
import json
import sys
from pathlib import Path
from generate_lifecycle import cbor, generate

def document():
    value = generate("cc" * 32)
    value["evidence"] = "Independent preferred-CBOR/hashlib/base32 five-boundary native unit-world recipe DATA; not execution provenance, host acceptance or real tool semantics."
    value["steps"] = value["steps"][:5]
    for step in value["steps"]:
        step["patch_hex"] = cbor(step["patch"]).hex()
        step["arguments_hex"] = cbor(step["arguments"]).hex()
    return json.dumps(value, indent=2) + "\n"

if __name__ == "__main__":
    content = document()
    if sys.argv[1:] == ["--check"]:
        if (Path(__file__).parent / "format/demo_scenario.json").read_text() != content:
            raise SystemExit("demo_scenario.json differs from independent recipe oracle")
    elif not sys.argv[1:]:
        print(content, end="")
    else:
        raise SystemExit("usage: generate_demo_scenario.py [--check]")
