#!/usr/bin/env python3
"""Explicit bounded live Go/actual-WASM/owned-PTY witness. Never an ordinary test gate.

Requires RAIV_LIVE_SMOKE=opencode-go/deepseek-v4.1-flash/natural-recall,
RAIV_GUEST_WASM and selected OPENCODE_API_KEY from the environment. Does not log
raw terminal/provider data or credentials; preserves native test namespaces.
"""
import fcntl
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


def safe_reply_summary(line):
    """Accept only the exact native finite diagnostic grammar; never raw provider text."""
    match = re.fullmatch(
        rb"native_normalized_reply request_id=([123]) "
        rb"control=(patch_memory|finish|trade|set_wake|veto|pass|other|invalid_or_multiple) "
        rb"argument_bytes_and_empty_map=(None|Some\(\(([0-9]{1,5}), (true|false)\)\)) "
        rb"text_bytes=([0-9]{1,5}) posts=([0-6])", line,
    )
    if not match:
        return None
    count = "none" if match[4] is None else match[4].decode()
    empty = "unknown" if match[5] is None else match[5].decode()
    return (f"native_reply id={match[1].decode()} control={match[2].decode()} "
            f"argument_bytes={count} empty_map={empty} text_bytes={match[6].decode()} posts={match[7].decode()}")


def selection(with_correction, high_reasoning):
    if high_reasoning and not with_correction:
        raise ValueError("high reasoning requires the explicit six-call envelope")
    suffix = "-high" if high_reasoning else ("-correction" if with_correction else "")
    test = "_with_high_reasoning" if high_reasoning else ("_with_correction" if with_correction else "")
    return "opencode-go/deepseek-v4.1-flash/natural-recall" + suffix, test


def safe_provider_class(line):
    if re.fullmatch(rb"native_provider_class=(Configuration|Credential|Request|Transport|Timeout|Status\([0-9]{3}\)|ContentType|Limit|Response|Stream\((Frame|Limit|Incomplete|Truncated|Refused|Failed|Allocation|Normalize\((Json|Schema|Model|Incomplete|Limit|Allocation|Wire)\))\)|CredentialEcho|Allocation)", line):
        return line.decode()
    return None


def safe_reasoning_summary(line):
    match = re.fullmatch(rb"native_reasoning request_id=([123]) bytes=(None|Some\(([0-9]{1,5})\)) posts=([0-6])", line)
    if not match:
        return None
    size = "none" if match[3] is None else match[3].decode()
    return f"native_reasoning id={match[1].decode()} bytes={size} posts={match[4].decode()}"


def main(with_correction=False, high_reasoning=False):
    approval, test_suffix = selection(with_correction, high_reasoning)
    if os.environ.get("RAIV_LIVE_SMOKE") != approval:
        raise SystemExit("explicit bounded live smoke approval selector required")
    if not os.environ.get("OPENCODE_API_KEY"):
        raise SystemExit("selected credential unavailable; value withheld")
    module = Path(os.environ.get("RAIV_GUEST_WASM", ""))
    if not module.is_file():
        raise SystemExit("selected actual guest artifact required")
    env = os.environ.copy()
    env["RAIV_GUEST_WASM"] = str(module.resolve())
    command = [
        "cargo", "test", "--locked", "-p", "raiv-agent-host", "--bin", "raiv-agent",
        "app::advance::live_tests::live_smoke::interactive_smoke::live_provider_smoke_interactive_memory_recall",
        "--", "--exact", "--ignored", "--test-threads=1", "--nocapture",
    ]
    command[7] += test_suffix
    root = Path(__file__).resolve().parents[2]
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 120, 0, 0))
    child = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                             env=env, cwd=root, start_new_session=True)
    os.close(slave)
    pending = b""
    lines = b""
    total = 0
    stage = 0
    submitted = 0
    exits = 0
    ready = False
    waiting = False
    proofs = 0
    passed = False
    prompts = [
        b"Remember my nickname ORION-RAIV-7 in intents.nickname using patch_memory, then finish with a normal response without tool calls.",
        b"RECALL_COPY: read the saved intents.nickname from accepted Memory; copy exactly that value to tactics.recalled_nickname using patch_memory, then finish with a normal response without tool calls.",
    ]
    deadline = time.monotonic() + (170 if with_correction else 130)  # Explicit4/6-call cap plus native/PTY margin.
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
            if total > 1048576:
                raise RuntimeError("owned PTY output cap; raw output withheld")
            pending += chunk
            lines += chunk
            while b"\n" in lines:
                line, lines = lines.split(b"\n", 1)
                line = line.strip(b"\r")
                if re.fullmatch(rb"SMOKE_HOME=[A-Za-z0-9_./-]+", line):
                    print(line.decode(), flush=True)
                elif re.fullmatch(rb"SMOKE_STAGE=[12]", line):
                    stage = int(line[-1:])
                    ready = True
                    pending = pending.split(b"SMOKE_STAGE=" + str(stage).encode(), 1)[-1]
                    print("stage_start=" + str(stage), flush=True)
                elif re.fullmatch(rb"SMOKE_PROOF stage=[12] accepted=true semantic_memory=true calls=[23] world_unchanged=true", line):
                    proofs += 1
                    print(line.decode(), flush=True)
                elif re.fullmatch(rb"native_reply_failure=[a-z_]+ posts=[0-6]", line):
                    print(line.decode(), flush=True)
                elif line.startswith(b"native_reasoning "):
                    summary = safe_reasoning_summary(line)
                    if summary is not None:
                        print(summary, flush=True)
                elif line.startswith(b"native_normalized_reply "):
                    summary = safe_reply_summary(line)
                    if summary is not None:
                        print(summary, flush=True)
                elif re.fullmatch(rb"native_corrective_feedback=invalid_arguments request_id=[123]", line):
                    print(line.decode(), flush=True)
                elif line.startswith(b"native_provider_class="):
                    summary = safe_provider_class(line)
                    if summary is not None:
                        print(summary, flush=True)
                elif line.startswith(b"native_provider_failure="):
                    print("native_provider_failure=true; details withheld", flush=True)
                elif re.fullmatch(rb"SMOKE_RESULT=passed stages=2 calls=[4-6] independent_pinned_restart=true behavioral_recall=true no_effects=true", line):
                    passed = with_correction or b"calls=4 " in line
                    print(line.decode(), flush=True)
            entered = pending.find(b"\x1b[?1049h")
            if ready and entered >= 0 and b"\x1b[?25h" in pending[entered:] and submitted < stage:
                # First editable frame after fresh alternate-screen entry, not pre-raw stdin.
                os.write(master, b"\x1b[200~" + prompts[stage - 1] + b"\x1b[201~\r")
                submitted += 1
                ready = False
                waiting = True
                pending = b""
            elif waiting and b"blocked" in pending:
                # Finite max_events=1 blocks after known acceptance OR refusal; Rust checks outcome.
                os.write(master, b"\x1b")
                exits += 1
                waiting = False
                pending = b""
            if len(pending) > 131072:
                pending = pending[-131072:]
        if child.poll() is None:
            child.wait(timeout=2)
        print(f"native_process_exit={child.returncode} submitted_events={submitted} fenced_exits={exits} checked_stages={proofs}")
        if child.returncode != 0 or not passed or proofs != 2 or submitted != 2 or exits != 2:
            raise RuntimeError("interactive live witness incomplete/failed; raw output withheld; inspect retained native checkpoint, never retry automatically")
    finally:
        if child.poll() is None:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
        os.close(master)


def check_filter():
    line = (b"native_normalized_reply request_id=2 control=finish "
            b"argument_bytes_and_empty_map=Some((1, true)) text_bytes=0 posts=2")
    assert safe_reply_summary(line) == ("native_reply id=2 control=finish argument_bytes=1 "
                                        "empty_map=true text_bytes=0 posts=2")
    assert "empty_map=false" in safe_reply_summary(line.replace(b"(1, true)", b"(20, false)"))
    assert "empty_map=unknown" in safe_reply_summary(line.replace(b"Some((1, true))", b"None"))
    for bad in [line + b" private-body", line.replace(b"finish", b"SECRET"),
                line.replace(b"true", b"SECRET"), line.replace(b"posts=2", b"posts=9"),
                line.replace(b"(1,", b"(123456,"), line.replace(b"text_bytes=0", b"text_bytes=raw-text")]:
        assert safe_reply_summary(bad) is None
    assert safe_provider_class(b"native_provider_class=Response") == "native_provider_class=Response"
    assert safe_provider_class(b"native_provider_class=Status(400)") == "native_provider_class=Status(400)"
    for kind in [b"Frame", b"Limit", b"Incomplete", b"Truncated", b"Refused", b"Failed", b"Allocation",
                 b"Normalize(Json)", b"Normalize(Schema)", b"Normalize(Model)", b"Normalize(Incomplete)",
                 b"Normalize(Limit)", b"Normalize(Allocation)", b"Normalize(Wire)"]:
        line = b"native_provider_class=Stream(" + kind + b")"
        assert safe_provider_class(line) == line.decode()
    for bad in [b"native_provider_class=SECRET", b"native_provider_class=Status(1234)", b"native_provider_class=Response private body",
                b"native_provider_class=Stream(SECRET)", b"native_provider_class=Stream(Normalize(SECRET))",
                b"native_provider_class=Stream(Truncated) private body"]:
        assert safe_provider_class(bad) is None
    reason = b"native_reasoning request_id=1 bytes=Some(0) posts=1"
    assert safe_reasoning_summary(reason) == "native_reasoning id=1 bytes=0 posts=1"
    assert "bytes=none" in safe_reasoning_summary(reason.replace(b"Some(0)", b"None"))
    assert "bytes=32768" in safe_reasoning_summary(reason.replace(b"Some(0)", b"Some(32768)"))
    for bad in [reason + b" private thought", reason.replace(b"Some(0)", b"Some(SECRET)"),
                reason.replace(b"Some(0)", b"Some(123456)"), reason.replace(b"posts=1", b"posts=7")]:
        assert safe_reasoning_summary(bad) is None
    prefix = "opencode-go/deepseek-v4.1-flash/natural-recall"
    assert selection(False, False) == (prefix, "")
    assert selection(True, False) == (prefix + "-correction", "_with_correction")
    assert selection(True, True) == (prefix + "-high", "_with_high_reasoning")
    try:
        selection(False, True)
    except ValueError:
        pass
    else:
        raise AssertionError("high selector must not use the two-call envelope")
    print("diagnostic filter/selectors passed; no credential/TTY/network access")


if __name__ == "__main__":
    if sys.argv[1:] == ["--check-filter"]:
        check_filter()
    elif sys.argv[1:] == ["--with-correction"]:
        main(with_correction=True)
    elif sys.argv[1:] == ["--with-high-reasoning"]:
        main(with_correction=True, high_reasoning=True)
    elif sys.argv[1:]:
        raise SystemExit("usage: test-live-interactive.py [--check-filter|--with-correction|--with-high-reasoning]")
    else:
        main()
