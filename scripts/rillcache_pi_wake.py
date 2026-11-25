#!/usr/bin/env python3
"""Safely wake the pinned Rillcache Pi session without repeating blocked audits."""
from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import time
from pathlib import Path
from typing import Callable

REPO = os.environ.get("RILLCACHE_REPO", "macel94/rillcache")
STATE_DIR = Path(os.environ.get("RILLCACHE_WAKE_STATE_DIR", "/root/.local/state/rillcache-frame-lifecycle"))
CHAT = Path(os.environ.get(
    "RILLCACHE_PI_CHAT",
    "/root/.pi/agent/sessions/--root-worktrees-rillcache-acl-sort-followon--/"
    "2026-11-18T20-45-47-334Z_01a1f068-91c5-77c5-8d1a-e3e9f7c7a184.jsonl",
))
TASK = Path(os.environ.get("RILLCACHE_PI_TASK", str(STATE_DIR / "issue-loop-task.md")))
STATE_FILE = Path(os.environ.get("RILLCACHE_WAKE_STATE", str(STATE_DIR / "wake-state.json")))
PANE = os.environ.get("RILLCACHE_PI_PANE", "%0")
PID = int(os.environ.get("RILLCACHE_PI_PID", "3915422"))
PANE_PID = int(os.environ.get("RILLCACHE_TMUX_PANE_PID", str(PID)))
PANE_COMMAND = os.environ.get("RILLCACHE_TMUX_PANE_COMMAND", "pi")
START_TICKS = os.environ.get("RILLCACHE_PI_START_TICKS", "25951090")
TMUX = ["/usr/bin/tmux", "-S", os.environ.get("RILLCACHE_TMUX_SOCKET", "/tmp/tmux-0/default")]
SESSION = os.environ.get("RILLCACHE_PI_SESSION", "rillcache-pr393-owner")
META = "#{session_name}|#{pane_pid}|#{pane_current_command}|#{pane_dead}|#{pane_in_mode}"
EXPECTED = f"{SESSION}|{PANE_PID}|{PANE_COMMAND}|0|0"
MIN_WAIT_SECONDS = 60 * 60
HEARTBEAT_SECONDS = 24 * 60 * 60
TAIL_BYTES = 1024 * 1024
WAITING_MARKER = "<!-- RILLCACHE_WAKE: WAITING -->"
CONTINUE_MARKER = "<!-- RILLCACHE_WAKE: CONTINUE -->"
Runner = Callable[..., str]


def command(*args: str, runner: Runner | None = None) -> str:
    run = runner or (lambda *argv: subprocess.check_output(argv, text=True, timeout=8))
    return run(*args).strip()


def _normalize(value):
    if isinstance(value, dict):
        return {key: _normalize(item) for key, item in value.items()}
    if isinstance(value, list):
        items = [_normalize(item) for item in value]
        return sorted(items, key=lambda item: json.dumps(item, sort_keys=True, separators=(",", ":")))
    return value


def external_fingerprint(runner: Runner | None = None) -> str:
    """Fingerprint external state relevant to readiness without reading PR code."""
    main = command("gh", "api", f"repos/{REPO}/commits/main", "--jq", ".sha", runner=runner)
    prs = json.loads(command(
        "gh", "pr", "list", "--repo", REPO, "--state", "open", "--limit", "100",
        "--json", "number,headRefOid,updatedAt,reviewDecision,mergeStateStatus,statusCheckRollup,isDraft,state",
        runner=runner,
    ))
    issues = json.loads(command(
        "gh", "issue", "list", "--repo", REPO, "--state", "open", "--limit", "100",
        "--json", "number,updatedAt,assignees,labels,state", runner=runner,
    ))
    snapshot = _normalize({
        "main": main,
        "pull_requests": prs,
        "issues": issues,
    })
    encoded = json.dumps(snapshot, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def latest_turn() -> tuple[tuple[int, int] | None, str | None]:
    """Return the completed idle turn stamp and its textual final response."""
    with CHAT.open("rb") as log:
        before = CHAT.stat()
        if time.time() - before.st_mtime < 60:
            return None, None
        log.seek(max(0, before.st_size - TAIL_BYTES))
        lines = log.read(TAIL_BYTES).splitlines()
        if not lines or CHAT.stat().st_mtime_ns != before.st_mtime_ns:
            return None, None
        last = json.loads(lines[-1])
    message = last.get("message") or {}
    if not (
        last.get("type") == "message"
        and message.get("role") == "assistant"
        and message.get("stopReason") == "stop"
    ):
        return None, None
    content = message.get("content", "")
    if isinstance(content, list):
        text = "\n".join(part.get("text", "") for part in content if isinstance(part, dict))
    else:
        text = str(content)
    return (before.st_mtime_ns, before.st_size), text


def waiting_mode(text: str | None) -> str:
    if text and CONTINUE_MARKER in text:
        return "continue"
    if text and WAITING_MARKER in text:
        return "waiting"
    # Fail toward continuing unless the assistant explicitly declared a stable wait.
    return "continue"


def should_wake(
    mode: str,
    previous: dict | None,
    fingerprint: str,
    prompt_hash: str,
    now: float,
    heartbeat_seconds: int = HEARTBEAT_SECONDS,
) -> tuple[bool, str]:
    if mode != "waiting":
        return True, "assistant did not declare an external wait"
    if previous is None:
        return True, "no prior wait state"
    if previous.get("prompt_hash") != prompt_hash:
        return True, "persistent prompt changed"
    elapsed = now - float(previous.get("last_wake", 0))
    if elapsed < MIN_WAIT_SECONDS:
        return False, "coalescing external changes until one-hour minimum interval"
    if previous.get("fingerprint") != fingerprint:
        return True, "GitHub readiness state changed"
    if elapsed >= heartbeat_seconds:
        return True, "periodic readiness heartbeat due"
    return False, "unchanged external wait; suppressing redundant audit"


def read_state(path: Path = STATE_FILE) -> dict | None:
    try:
        value = json.loads(path.read_text())
        return value if isinstance(value, dict) else None
    except FileNotFoundError:
        return None


def write_state(path: Path, fingerprint: str, prompt_hash: str, now: float) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps({
        "version": 1,
        "fingerprint": fingerprint,
        "prompt_hash": prompt_hash,
        "last_wake": now,
    }, sort_keys=True) + "\n")
    os.replace(temporary, path)


def tmux(*args: str) -> str:
    return subprocess.check_output([*TMUX, *args], text=True, timeout=3)


def expected_pane_meta(session: str, pane_pid: int, pane_command: str) -> str:
    return f"{session}|{pane_pid}|{pane_command}|0|0"


def pane_owns_pi(pane_pid: int, pi_pid: int, children: str) -> bool:
    return pane_pid == pi_pid or str(pi_pid) in children.split()


def empty_composer(screen: str) -> bool:
    lines = screen.rstrip("\n").splitlines()
    borders = [i for i, line in enumerate(lines) if re.fullmatch(r"─{10,}", line.rstrip())]
    if len(borders) < 2 or borders[-1] + 5 < len(lines):
        return False
    top, bottom = borders[-2:]
    return bottom == top + 2 and not lines[top + 1].strip()


def ready() -> tuple[tuple[int, int] | None, str]:
    if (STATE_DIR / "PAUSE-PI-WAKE").exists():
        return None, "paused"
    if tmux("display-message", "-p", "-t", PANE, META).strip() != EXPECTED:
        return None, "pinned pane is absent, changed, or in copy mode"
    if PID != PANE_PID:
        try:
            children = Path(f"/proc/{PANE_PID}/task/{PANE_PID}/children").read_text()
        except OSError:
            return None, "pinned pane process tree is unavailable"
        if not pane_owns_pi(PANE_PID, PID, children):
            return None, "pinned Pi is not a child of the expected tmux pane"
    stat = Path(f"/proc/{PID}/stat").read_text().rsplit(")", 1)[1].split()
    if stat[19] != START_TICKS:
        return None, "pinned Pi process identity changed"
    stamp, _ = latest_turn()
    if stamp is None:
        return None, "Pi is active or transcript does not show a completed idle turn"
    if not empty_composer(tmux("capture-pane", "-p", "-t", PANE)):
        return None, "no empty Pi composer visible; leaving input alone"
    return stamp, "idle"


def main() -> None:
    try:
        first, reason = ready()
        if first is None:
            print(f"SKIP: {reason}")
            return
        time.sleep(1)
        second, reason = ready()
        if first != second:
            print(f"SKIP: state changed during idle check ({reason})")
            return

        prompt = TASK.read_text().strip()
        if not prompt:
            print("SKIP: persistent issue-loop prompt is empty")
            return
        prompt_hash = hashlib.sha256(prompt.encode()).hexdigest()
        _, final_text = latest_turn()
        mode = waiting_mode(final_text)
        previous = read_state()
        now = time.time()
        fingerprint = ""
        if mode == "waiting" and previous is not None and previous.get("prompt_hash") == prompt_hash:
            try:
                fingerprint = external_fingerprint()
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                print(f"SKIP: unable to verify external readiness state ({type(error).__name__})")
                return
        wake, reason = should_wake(mode, previous, fingerprint, prompt_hash, now)
        if not wake:
            print(f"SKIP: {reason}")
            return

        tmux("send-keys", "-t", PANE, "-l", "--", prompt, ";", "send-keys", "-t", PANE, "Enter")
        # Persist only after the input was accepted by tmux. An interrupted
        # state write can cause a duplicate wake, never a permanently lost one.
        if not fingerprint:
            fingerprint = external_fingerprint()
        write_state(STATE_FILE, fingerprint, prompt_hash, now)
        print(f"WAKE: prompted pinned Pi pane {PANE} (pid {PID}): {reason}")
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"SKIP: unable to verify or wake pinned pane ({type(error).__name__})")


if __name__ == "__main__":
    main()
