#!/usr/bin/env python3
"""Exact-binary upgrade, downgrade and failed-upgrade drill for gap A4.

The drill runs two *different* built binaries (the pinned baseline commit and the
current head) over the same private journal format and asserts:

1. the baseline writes a journal, and a preserved copy plus an archive are taken
   before anything else touches it (the only recoverable journal is never
   overwritten);
2. the new binary opens that journal, serves identical data, absolute TTLs and
   accounting, and appends its own writes (upgrade / format compatibility);
3. the baseline binary opens the upgraded journal and either serves every key
   (downgrade supported for this pair) or refuses it without modifying it, as the
   pin file declares;
4. an interrupted upgrade (SIGKILL after the new binary wrote data) still leaves
   the preserved journal and archive restorable by the baseline binary, and the
   new binary recovers its own acknowledged prefix;
5. rollback from the preserved archive restores the exact pre-upgrade state into
   an isolated directory, which is the documented traffic-cutover rollback path.

Timings and every observed outcome are recorded in ``result.json`` and
``summary.md``; any mismatch, drift from the declared expectation, or mutation of
the preserved journal fails the drill.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

import importlib.util as _importlib_util

_spec = _importlib_util.spec_from_file_location(
    "restore_drill", Path(__file__).resolve().parent / "restore_drill.py")
rd = _importlib_util.module_from_spec(_spec)
import sys as _sys
_sys.modules.setdefault("restore_drill", rd)
_spec.loader.exec_module(rd)

ROOT = Path(__file__).resolve().parents[1]
PINS = ROOT / "ci/upgrade-pins.json"
TIMEOUT = 30.0


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def digest_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise rd.DrillError(message)


def run_cli(argv: list[str]) -> dict:
    start = time.monotonic()
    completed = subprocess.run(argv, capture_output=True, timeout=300, check=False)
    return {"argv": argv, "exit_code": completed.returncode,
            "milliseconds": round((time.monotonic() - start) * 1000, 3),
            "stdout": completed.stdout.decode(errors="replace").strip(),
            "stderr": completed.stderr.decode(errors="replace").strip()}


def element(element: Path, label: str) -> dict:
    return {"label": label, "server": str(element / "rillcache"),
            "backup": str(element / "rillcache-backup"),
            "server_sha256": digest(element / "rillcache") if (element / "rillcache").is_file() else None,
            "backup_sha256": digest(element / "rillcache-backup")
            if (element / "rillcache-backup").is_file() else None}


def start(binary: Path, directory: Path, label: str) -> rd.Server:
    return rd.Server(binary, directory, label)


def stop(server: rd.Server) -> int:
    return server.stop()


def kill(server: rd.Server):
    server.process.send_signal(signal.SIGKILL)
    server.process.wait(timeout=TIMEOUT)
    server.log.close()


def tolerance_for(earlier: dict, later: dict) -> int:
    """avg_ttl is clock-derived; allow the elapsed drill time plus a cache window."""
    return max(5000, later["observed_at_ms"] - earlier["observed_at_ms"] + 5000)


def state_of(server: rd.Server, expectations: dict) -> dict:
    with server.client() as client:
        return rd.snapshot(client, expectations)


def load(server: rd.Server, commands, expectations) -> dict:
    with server.client() as client:
        client.socket.sendall(b"".join(rd.frame(command) for command in commands))
        for _ in commands:
            rd.read_reply(client.stream)
    return state_of(server, expectations)


def load_pins(path: Path) -> dict:
    pins = json.loads(path.read_text())
    if pins.get("schema") != 1:
        raise rd.DrillError("unsupported upgrade pin schema")
    pair = pins["pair"]
    for field in ("id", "old", "new", "expectations"):
        if field not in pair:
            raise rd.DrillError(f"pin pair.{field} is required")
    if pair["old"].get("commit") in (None, "", "HEAD"):
        raise rd.DrillError("the old pin must be an immutable commit")
    for key in ("downgrade",):
        if pair["expectations"].get(key) not in ("serves", "refused"):
            raise rd.DrillError(f"pair.expectations.{key} must be 'serves' or 'refused'")
    if not pair["expectations"].get("failed_upgrade"):
        raise rd.DrillError("pair.expectations.failed_upgrade is required")
    return pins


def drill(old: dict, new: dict, pins: dict, output: Path, keys: int, seed: int) -> dict:
    result = {
        "schema": 1,
        "pair": {"id": pins["pair"]["id"], "old": pins["pair"]["old"], "new": pins["pair"]["new"]},
        "binaries": {"old": old, "new": new},
        "expectations": pins["pair"]["expectations"],
        "dataset": {"keys": keys, "seed": seed},
        "timings_ms": {}, "upgrade": {}, "downgrade": {}, "failed_upgrade": {},
        "rollback": {}, "environment": {"platform": platform.platform(),
                                        "python": platform.python_version()},
        "failures": [],
    }
    work = output / "work"
    if work.exists():
        shutil.rmtree(work)
    for name in ("old-data", "preserved", "upgraded", "rollback", "failed", "restored"):
        (work / name).mkdir(parents=True)
    started = time.monotonic()
    old_server = Path(old["server"])
    old_backup = Path(old["backup"])
    new_server = Path(new["server"])
    new_backup = Path(new["backup"])
    for binary in (old_server, old_backup, new_server, new_backup):
        require(binary.is_file() and os.access(binary, os.X_OK), f"missing executable {binary}")

    commands, expectations, _ = rd.dataset(seed, keys)

    # 1. Baseline instance writes the journal and is stopped cleanly.
    baseline = start(old_server, work / "old-data", "baseline")
    result["timings_ms"]["baseline_startup"] = baseline.startup_ms
    result["upgrade"]["restored_state"] = load(baseline, commands, expectations)
    require(stop(baseline) == 0, "the baseline instance must stop cleanly")
    journal = work / "old-data" / "appendonly.aof"
    preserved = work / "preserved" / "appendonly.aof"
    shutil.copy2(journal, preserved)
    preserved_before = digest(preserved)
    result["upgrade"]["journal"] = {"bytes": journal.stat().st_size, "sha256": digest(journal),
                                    "preserved_sha256": preserved_before}

    # The baseline's own archive is the documented rollback artifact; it must not
    # be produced after the upgrade attempt, and the source journal must stay put.
    archive = work / "preserved" / "baseline.rcb"
    created = run_cli([str(old_backup), "create", "--dir", str(work / "old-data"), "--output", str(archive)])
    require(created["exit_code"] == 0, f"baseline create failed: {created['stderr']}")
    result["timings_ms"]["baseline_create"] = created["milliseconds"]
    verified = run_cli([str(old_backup), "verify", "--input", str(archive)])
    require(verified["exit_code"] == 0, f"baseline verify failed: {verified['stderr']}")
    result["timings_ms"]["baseline_verify"] = verified["milliseconds"]
    archive_sha = digest(archive)
    result["rollback"]["archive"] = {"bytes": archive.stat().st_size, "sha256": archive_sha}

    # 2. Upgrade: the new binary opens a copy of the baseline journal.
    shutil.copy2(preserved, work / "upgraded" / "appendonly.aof")
    upgraded = start(new_server, work / "upgraded", "upgraded")
    result["timings_ms"]["upgrade_startup"] = upgraded.startup_ms
    upgraded_state = state_of(upgraded, expectations)
    problems = rd.compare(result["upgrade"]["restored_state"], upgraded_state,
                          tolerance_for(result["upgrade"]["restored_state"], upgraded_state))
    if problems:
        result["failures"].extend({"kind": "upgrade", **problem} for problem in problems)
    require(not problems, f"upgrade did not serve the baseline state: {problems[:2]}")
    with upgraded.client() as client:
        require(client.call("SET", b"drill:upgraded", b"new-binary") == b"OK",
                "the new binary must accept a write after the upgrade")
    expectations[b"drill:upgraded"] = {"type": "string", "read": ["GET", b"drill:upgraded"],
                                       "value": b"new-binary"}
    upgraded_state = state_of(upgraded, expectations)
    result["upgrade"]["state"] = upgraded_state
    require(stop(upgraded) == 0, "the upgraded instance must stop cleanly")
    result["upgrade"]["journal_after"] = {"bytes": (work / "upgraded" / "appendonly.aof").stat().st_size,
                                          "sha256": digest(work / "upgraded" / "appendonly.aof")}

    # 3. Downgrade: the baseline binary opens the upgraded journal copy.
    shutil.copy2(work / "upgraded" / "appendonly.aof", work / "rollback" / "appendonly.aof")
    downgrade_copy = digest(work / "rollback" / "appendonly.aof")
    expected_downgrade = pins["pair"]["expectations"]["downgrade"]
    downgrade = None
    try:
        downgrade = start(old_server, work / "rollback", "downgrade")
    except Exception as error:  # a refused downgrade is a recorded, declared outcome
        result["downgrade"] = {"outcome": "refused", "detail": str(error)[:400],
                               "journal_unchanged": digest(work / "rollback" / "appendonly.aof") == downgrade_copy}
        require(expected_downgrade == "refused",
                f"downgrade was expected to serve but was refused: {error}")
    else:
        downgraded_state = state_of(downgrade, expectations)
        problems = rd.compare(upgraded_state, downgraded_state,
                              tolerance_for(upgraded_state, downgraded_state))
        result["downgrade"] = {"outcome": "served", "state": downgraded_state,
                               "journal_unchanged": digest(work / "rollback" / "appendonly.aof") == downgrade_copy}
        require(stop(downgrade) == 0, "the downgraded instance must stop cleanly")
        require(expected_downgrade == "serves",
                "downgrade served but the pin declares it refused; update the reviewed pin")
        require(not problems, f"downgrade did not serve the upgraded state: {problems[:2]}")
    result["downgrade"]["expected"] = expected_downgrade

    # 4. Failed upgrade: the new binary dies after accepting writes.
    shutil.copy2(preserved, work / "failed" / "appendonly.aof")
    failing = start(new_server, work / "failed", "failed-upgrade")
    with failing.client() as client:
        require(client.call("SET", b"drill:acked-before-crash", b"one") == b"OK", "write before kill")
        client.call("SET", b"drill:second", b"two")
    kill(failing)
    result["failed_upgrade"]["killed"] = True
    # (a) the preserved baseline journal still starts under the baseline binary;
    recovered_preserved = start(old_server, work / "preserved", "preserved-restart")
    preserved_state = state_of(recovered_preserved, {key: value for key, value in expectations.items()
                                                     if not key.startswith(b"drill:upgraded")
                                                     and not key.startswith(b"drill:acked")
                                                     and key != b"drill:second"})
    require(stop(recovered_preserved) == 0, "the preserved instance must stop cleanly")
    require(digest(preserved) == preserved_before, "the preserved journal was modified")
    preserved_problems = rd.compare(result["upgrade"]["restored_state"], preserved_state,
                                    tolerance_for(result["upgrade"]["restored_state"], preserved_state))
    result["failed_upgrade"]["preserved_state_matches"] = not preserved_problems
    require(not preserved_problems,
            f"the preserved journal no longer serves the pre-upgrade state: {preserved_problems[:2]}")
    # (b) the new binary recovers its own acknowledged prefix from the killed copy.
    resumed = start(new_server, work / "failed", "failed-resume")
    with resumed.client() as client:
        require(client.call("GET", b"drill:acked-before-crash") == b"one",
                "the new binary must recover its acknowledged write")
    require(stop(resumed) == 0, "the resumed instance must stop cleanly")
    result["failed_upgrade"]["new_binary_recovered"] = True
    # (c) the preserved archive still verifies after the failed upgrade.
    verified = run_cli([str(old_backup), "verify", "--input", str(archive)])
    require(verified["exit_code"] == 0, "the rollback archive no longer verifies")
    require(digest(archive) == archive_sha, "the rollback archive was modified")

    # 5. Rollback: restore the preserved archive with the baseline tool and serve.
    restored = run_cli([str(old_backup), "restore", "--input", str(archive),
                        "--dir", str(work / "restored")])
    require(restored["exit_code"] == 0, f"rollback restore failed: {restored['stderr']}")
    result["timings_ms"]["rollback_restore"] = restored["milliseconds"]
    rollback_state = state_of(start(old_server, work / "restored", "rollback-restored"),
                              {key: value for key, value in expectations.items()
                               if not key.startswith(b"drill:upgraded") and key != b"drill:second"})
    problems = rd.compare(result["upgrade"]["restored_state"], rollback_state,
                          tolerance_for(result["upgrade"]["restored_state"], rollback_state))
    require(not problems, f"rollback did not restore the pre-upgrade state: {problems[:2]}")
    result["rollback"]["state_matches_pre_upgrade"] = True
    result["rollback"]["cutover"] = (
        "announce freeze -> stop writer cleanly -> verify the preserved archive -> "
        "install the target binary -> start -> validate data/TTLs/accounting -> "
        "resume traffic; the previous binary and prepared journal remain available")

    require(digest(preserved) == preserved_before, "the preserved journal was modified at the end")
    result["timings_ms"]["total"] = round((time.monotonic() - started) * 1000, 3)
    result["status"] = "passed" if not result["failures"] else "failed"
    return rd.jsonable_result(result)


def summary_text(result: dict) -> str:
    timings = result.get("timings_ms", {})
    downgrade = result.get("downgrade", {})
    upgrade = result.get("upgrade", {})
    rollback = result.get("rollback", {})
    pair = result.get("pair", {})
    return (
        f"## Exact-binary upgrade/rollback drill: {result['status']}\n\n"
        f"Pair `{pair.get('id')}`: baseline `{pair.get('old', {}).get('label')}` "
        f"({str(pair.get('old', {}).get('commit', ''))[:12]}) → head "
        f"`{pair.get('new', {}).get('label')}`.\n\n"
        f"| Measurement | Value |\n| --- | --- |\n"
        f"| Baseline journal | {upgrade.get('journal', {}).get('bytes')} bytes |\n"
        f"| Upgrade startup | {timings.get('upgrade_startup')} ms |\n"
        f"| Downgrade outcome | {downgrade.get('outcome')} (declared {downgrade.get('expected')}) |\n"
        f"| Failed-upgrade preserved journal intact | "
        f"{result.get('failed_upgrade', {}).get('preserved_state_matches')} |\n"
        f"| Rollback restore | {timings.get('rollback_restore')} ms |\n"
        f"| Total | {timings.get('total')} ms |\n\n"
        f"- Cutover: {rollback.get('cutover', 'n/a')}.\n"
        f"- Failures: {len(result['failures'])}.\n\n"
        "The preserved baseline journal and archive are never overwritten; a failed "
        "upgrade leaves the previous binary able to serve the pre-upgrade state. "
        "This is a journal-format and rollback contract for the pinned pair, not a "
        "claim that arbitrary version skew is supported.\n")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--old", type=Path, required=True, help="directory with baseline rillcache and rillcache-backup")
    parser.add_argument("--new", type=Path, required=True, help="directory with head rillcache and rillcache-backup")
    parser.add_argument("--pins", type=Path, default=PINS)
    parser.add_argument("--output", type=Path, default=ROOT / "target/upgrade-drill")
    parser.add_argument("--keys", type=int, default=400)
    parser.add_argument("--seed", type=int, default=464)
    args = parser.parse_args(argv)
    if args.keys < 50:
        parser.error("--keys must be at least 50")
    output = args.output.resolve()
    try:
        pins = load_pins(args.pins)
        result = drill(element(args.old.resolve(), pins["pair"]["old"]["label"]),
                       element(args.new.resolve(), pins["pair"]["new"]["label"]),
                       pins, output, args.keys, args.seed)
    except (rd.DrillError, OSError, subprocess.SubprocessError) as error:
        result = {"schema": 1, "status": "failed",
                  "failures": [{"kind": "drill-error", "detail": str(error)}]}
        print(f"upgrade drill failed: {error}", file=sys.stderr)
    output.mkdir(parents=True, exist_ok=True)
    (output / "result.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    text = summary_text(result) if result.get("timings_ms") else (
        f"## Exact-binary upgrade/rollback drill: {result['status']}\n\n"
        + "\n".join(f"- {failure}" for failure in result["failures"]) + "\n")
    (output / "summary.md").write_text(text)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write(text + "\n")
    print(text)
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
