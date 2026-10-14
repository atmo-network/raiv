#!/usr/bin/env python3
"""Opt-in actual run.sh/owned-PTY Memory continuity. Raw terminal data stays in RAM.

Python is test-only PTY/independent-fixture-codec plumbing; the user launcher is Bash.
No inference retries, changed test mandate, old-state repair or accepted-source migration.
"""
import codecs
import fcntl
import hashlib
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time
import tomllib
import unicodedata

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "fixtures"))
from generate_profiles import decode
from generate_lifecycle import cid

APPROVAL = "opencode-go/deepseek-v4.1-flash/launcher-memory-1024"
MANDATE = "Assist the local operator. Preserve useful behavioral continuity with patch_memory when needed, then finish with a complete normal response without tool calls. External actions are not approved."
PROMPTS = [
    b"Remember my nickname ORION-RAIV-7 in intents.nickname using patch_memory, then answer briefly without further tool calls.",
    b"Read the saved intents.nickname from accepted Memory. Use patch_memory to copy exactly that existing value to tactics.recalled_nickname, preserving intents. Then answer briefly without further tool calls.",
]


def environment():
    env = os.environ.copy()
    env.pop("OPENCODE_API_KEY", None)
    return env


def status(session, world=False):
    build = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve()
    command = [str(build / "debug/raiv-agent"), "world-status" if world else "status",
               "--config", str(session / "config.toml")]
    result = subprocess.run(command, env=environment(), stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=5, cwd=ROOT)
    if result.returncode:
        return None  # Native held-directory lock may be busy during this owned Run.
    assert len(result.stdout) <= 4096
    values = {}
    for line in result.stdout.decode().splitlines():
        key, value = line.split("=", 1)
        assert key not in values
        values[key] = value
    return values


def configuration(session):
    raw = (session / "config.toml").read_bytes()
    config = tomllib.loads(raw.decode())
    assert len(raw) <= 16384
    assert config["agent"]["mandate"] == MANDATE
    assert config["agent"]["profile"] == "survivor"
    inference = config["inference"]
    assert inference["model"] == "deepseek-v4.1-flash"
    assert inference["base_url"] == "https://opencode.ai/zen/go/v1"
    assert inference["reasoning_effort"] == "high"
    assert inference["max_tokens"] == 1024 and inference["timeout_ms"] == 20000
    assert inference["api_key_env"] == "OPENCODE_API_KEY" and inference["stream"] is True
    assert hashlib.sha256(raw).hexdigest() == (session / "config.sha256").read_text().strip()
    plan = tomllib.loads((session / "grants.toml").read_text())
    assert plan == dict(activation_units="100", remaining_episode_units="100",
                        remaining_model_calls="3", remaining_actions="0", max_events="2")
    assert config["executor"]["virtual_units"] == "10"
    module = (session / "guest.wasm").read_bytes()
    assert len(module) <= 8388608
    assert hashlib.sha256(module).hexdigest() == config["runtime"]["module_digest"]
    return config


def block(session, link):
    name = link["$link"] if isinstance(link, dict) else link
    assert re.fullmatch(r"bafyre[a-z2-7]{53}", name)
    raw = (session / "blocks" / name).read_bytes()
    assert len(raw) <= 8388608
    record = decode(raw)  # Oracle verifies exact preferred re-encoding, not bytes-as-CBOR-Bytes.
    assert cid(record) == name
    return record


def checked(session, revision):
    info = status(session)
    world = status(session, world=True)
    assert info is not None and world is not None
    assert info["revision"] == str(revision) and info["phase"] == "AwaitActor"
    assert info["unresolved"] == "0" and info["consumed_admissions"] == str(revision)
    assert info["completed_episodes"] == str(revision) and info["pending_action_id"] == "none"
    assert world["virtual_units_available"] == "10" and world["ledger_rows"] == "0"
    state = block(session, info["root"])
    assert state["kind"] == "agent_state" and state["revision"] == revision
    assert state["continuation"] == {"tag": "await_actor"} and state["current_episode"] is None
    memory = block(session, state["memory"])
    expected = {"intents": {"nickname": "ORION-RAIV-7"}}
    if revision == 2:
        expected["tactics"] = {"recalled_nickname": "ORION-RAIV-7"}
    assert memory == {"kind": "memory", "version": 1, "value": expected}
    calls = 0
    assert len(state["history"]) == revision
    for link in state["history"]:
        episode = block(session, link)
        assert episode["closed"] is True and len(episode["boundaries"]) == 1
        boundary = block(session, episode["boundaries"][0])
        assert boundary["exit"] == {"tag": "await_actor"}
        usage = boundary["usage"]
        assert 2 <= usage["model_calls"] <= 3 and usage["local_tool_calls"] == 1
        calls += usage["model_calls"]
    manifest_raw = (session / "blocks/raiv-manifest.cbor").read_bytes()
    assert len(manifest_raw) <= 8388608
    manifest = decode(manifest_raw)
    assert manifest[:2] == ["raiv-host/local-checkpoint", 1] and manifest[4] == revision
    rows = manifest[9]
    assert len(rows) == revision and all(row[2] == 3 and row[3] == 1 for row in rows)
    assert len({row[0][4]["$bytes"] for row in rows}) == revision
    assert len({decode(bytes.fromhex(row[1]["$bytes"]))["admission_id"]["$bytes"] for row in rows}) == revision
    return calls, (session / "world/raiv-world.cbor").read_bytes(), manifest_raw


class Screen:
    """Only the CSI/cursor/erase subset emitted by Crossterm, bounded and RAM-only."""
    def __init__(self):
        self.cells = [[" "] * 120 for _ in range(30)]
        self.row = self.col = 0
        self.escape = ""
        self.active = self.cursor = False
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def feed(self, chunk):
        for char in self.decoder.decode(chunk):
            if self.escape:
                self.escape += char
                if self.escape == "\x1b[":
                    continue
                if len(self.escape) == 2 and char != "[":
                    self.escape = ""
                elif len(self.escape) > 2 and "@" <= char <= "~":
                    self.csi(self.escape[2:-1], char)
                    self.escape = ""
                elif len(self.escape) > 64:
                    self.escape = ""
                continue
            if char == "\x1b":
                self.escape = char
            elif char == "\r":
                self.col = 0
            elif char == "\n":
                self.row = min(29, self.row + 1)
            elif char >= " " and not unicodedata.combining(char):
                self.cells[self.row][self.col] = char
                width = 2 if unicodedata.east_asian_width(char) in ("W", "F") else 1
                self.col = min(119, self.col + width)

    def csi(self, params, command):
        if params == "?1049":
            self.active = command == "h"
            if self.active:
                self.cells = [[" "] * 120 for _ in range(30)]
                self.row = self.col = 0
        elif params == "?25":
            self.cursor = command == "h"
        elif not params.startswith("?"):
            values = [int(value or "0") for value in params.split(";")]
            if command in ("H", "f"):
                self.row = min(29, max(0, (values[0] or 1) - 1))
                self.col = min(119, max(0, (values[1] if len(values) > 1 else 1) - 1))
            elif command == "J" and values[0] in (2, 3):
                self.cells = [[" "] * 120 for _ in range(30)]
            elif command == "K":
                start, end = (0, 120) if values[0] == 2 else ((0, self.col + 1) if values[0] == 1 else (self.col, 120))
                self.cells[self.row][start:end] = [" "] * (end - start)
            elif command in "ABCD":
                amount = values[0] or 1
                if command == "A": self.row = max(0, self.row - amount)
                if command == "B": self.row = min(29, self.row + amount)
                if command == "C": self.col = min(119, self.col + amount)
                if command == "D": self.col = max(0, self.col - amount)

    def text(self):
        return "\n".join("".join(line) for line in self.cells)


def stage(session, number, previous=None):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
    env = os.environ.copy()
    env["TERM"] = "xterm"
    child = subprocess.Popen([str(ROOT / "run.sh"), str(session)], stdin=slave,
                             stdout=slave, stderr=slave, cwd=ROOT, env=env, start_new_session=True)
    os.close(slave)
    pending = b""
    total = 0
    approved = submitted = exited = False
    textual = False
    screen = Screen()
    deadline = time.monotonic() + 90  # <=3 HTTP calls at20s plus bounded build/native/PTY margin.
    try:
        while time.monotonic() < deadline:
            if not select.select([master], [], [], 0.1)[0]:
                if child.poll() is not None:
                    break
                continue
            try:
                chunk = os.read(master, 65536)
            except OSError:
                break
            total += len(chunk)
            assert total <= 1048576
            pending = (pending + chunk)[-131072:]
            screen.feed(chunk)
            if not approved and b"Type run (anything else stops):" in pending:
                configuration(session)
                initial = status(session)
                assert initial is not None and initial["revision"] == str(number - 1)
                assert initial["unresolved"] == "0" and initial["phase"] == "AwaitActor"
                if previous is not None:
                    assert (session / "blocks/raiv-manifest.cbor").read_bytes() == previous
                os.write(master, b"run\n")
                approved = True
                print(f"LAUNCHER_CONTROL stage={number} source_selected=true", flush=True)
                pending = b""
            elif approved and not submitted and screen.active and screen.cursor:
                # The first editable frame is after native raw-mode/alternate-screen entry.
                os.write(master, b"\x1b[200~" + PROMPTS[number - 1] + b"\x1b[201~\r")
                submitted = True
                print(f"LAUNCHER_CONTROL stage={number} editable_frame=true submitted=1", flush=True)
                pending = b""
            elif submitted and not exited:
                view = screen.text()
                textual = textual or any("Agent: " in line and line.split("Agent: ", 1)[1].strip(" │") for line in view.splitlines())
                done = "Boundary ACCEPTED" in view or "blocked" in view
                if done:
                    info = status(session)
                    if info is not None:
                        if info["unresolved"] != "0":
                            category = "native_unknown"
                            for marker, label in [(b"finish_reason=length", "length"),
                                                  (b"terminal/DONE/EOF incomplete", "incomplete"),
                                                  (b"framing/delta schema invalid", "frame"),
                                                  (b"model differs", "model"), (b"transport timeout", "timeout")]:
                                if marker.decode() in view:
                                    category = label
                                    break
                            print(f"LAUNCHER_REFUSAL stage={number} category={category} unresolved=1; raw_details_withheld", flush=True)
                            raise RuntimeError("unresolved native attempt")
                        assert info["revision"] == str(number)
                        assert textual
                        os.write(master, b"\x1b")
                        exited = True
                        pending = b""
        if child.poll() is None:
            child.wait(timeout=3)
        assert child.returncode == 0 and approved and submitted and exited and textual
        print(f"LAUNCHER_UI stage={number} approved=true submitted=1 textual=true fenced_exit=true process=0", flush=True)
    finally:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
        os.close(master)


def check_offline():
    # Independent fixture-codec and exact prompt/cap checks; never launches a process/reads a key.
    assert b"ORION-RAIV-7" not in PROMPTS[1]
    example = decode(bytes.fromhex("a3646b696e64666d656d6f72796576616c7565a06776657273696f6e01"))
    assert example == {"kind": "memory", "value": {}, "version": 1}
    assert cid(example) == "bafyreifg4eef2j33kx4epxkh72suj2lyamprn67hgksobxzvpxrqma5mtm"
    assert b"patch_memory" in PROMPTS[0] and b"patch_memory" in PROMPTS[1]
    screen = Screen()
    screen.feed(b"\x1b[?1049h\x1b[1;1HStatuX: Boundary ACCEPXED\x1b[1;6Hs\x1b[1;23HT")
    assert "Status: Boundary ACCEPTED" in screen.text()
    screen.feed(b"\x1b[2;1HAgent: ok\x1b[?25h")
    assert screen.active and screen.cursor and "Agent: ok" in screen.text()
    print("launcher PTY/oracle offline checks passed; no credential/process/network")


def main(resume=None):
    if os.environ.get("RAIV_LIVE_SMOKE") != APPROVAL:
        raise SystemExit("explicit bounded launcher live selector required")
    if not os.environ.get("OPENCODE_API_KEY"):
        raise SystemExit("selected credential unavailable; value withheld")
    parent = ROOT / "var/raiv"
    parent.mkdir(parents=True, exist_ok=True)
    session = Path(resume).resolve() if resume else parent / ("launcher-witness-" + os.urandom(12).hex())
    if resume:
        assert session.parent == parent and session.name.startswith("launcher-witness-")
        configuration(session)
    else:
        assert not session.exists()  # run.sh alone creates this owned, separately approved namespace.
    print(f"LAUNCHER_HOME={session}", flush=True)
    assert b"ORION-RAIV-7" not in PROMPTS[1]
    if not resume:
        stage(session, 1)
    first_calls, world, previous = checked(session, 1)
    print(f"LAUNCHER_PROOF stage=1 revision=1 semantic_memory=true calls={first_calls} world_unchanged=true", flush=True)
    stage(session, 2, previous)
    total_calls, final_world, _ = checked(session, 2)
    assert final_world == world and 4 <= total_calls <= 6
    print(f"LAUNCHER_PROOF stage=2 revision=2 semantic_memory=true calls={total_calls-first_calls} world_unchanged=true", flush=True)
    print(f"LAUNCHER_RESULT=passed stages=2 calls={total_calls} ordinary_bash=true fresh_ui_vm=true behavioral_recall=true no_effects=true", flush=True)


if __name__ == "__main__":
    if sys.argv[1:] == ["--check-offline"]:
        check_offline()
    elif sys.argv[1:] and not (len(sys.argv) == 3 and sys.argv[1] == "--continue-accepted"):
        raise SystemExit("usage: test-live-launcher.py [--check-offline|--continue-accepted <owned-session>]")
    else:
        try:
            main(sys.argv[2] if len(sys.argv) == 3 else None)
        except Exception:
            raise SystemExit("ordinary launcher witness refused/incomplete; raw output withheld; inspect retained source, never retry automatically")
