#!/usr/bin/env python3
"""Run original pinned Tcl unit over TCP: native control, then candidate."""

import argparse
from collections import Counter
import fnmatch
import json
from pathlib import Path
import re
import shutil

from common import (Client, EVIDENCE, PINS, ROOT, SCOPE, WORK, command, environment,
                    save, server, source, summary)
from prepare import tcl_hashes

ANSI = re.compile(r"\x1b\[[0-9;]*m")


def events(text):
    text = ANSI.sub("", text)
    observed = {key: [] for key in ("ok", "err", "skip", "ignore", "exception")}
    for match in re.finditer(r"^\[(ok|err|skip|ignore|exception)\]: (.*)$", text, re.M):
        kind, name = match.groups()
        if kind == "ok":
            name = re.sub(r" \(\d+ ms\)$", "", name)
        elif kind == "err":
            name = re.split(r" in tests/", name, maxsplit=1)[0]
        observed[kind].append(name)
    observed["unit_completed"] = bool(re.search(r"^\[\d+/\d+ done\]:", text, re.M))
    return observed


def classify(result):
    counts = result["events"]
    attempted = len(counts["ok"]) + len(counts["err"])
    if result.get("timed_out"):
        return "timeout"
    if not result.get("server_alive", True):
        return "server_exit"
    if attempted == 0:
        return "prerequisite_blocked" if result.get("missing_prerequisites") else "zero_tests_or_harness_error"
    if counts["err"]:
        return "test_failures"  # Includes assertion failures AND command exceptions under --durable.
    if counts["exception"] or not counts["unit_completed"]:
        return "harness_error_or_unreached_tests"
    if result["exit_code"] != 0 or result.get("missing_prerequisites"):
        return "prerequisite_or_harness_error"
    return "passed_selected_scope"


def inventory(tree, selected):
    config = SCOPE["compatibility"]
    units = []
    # Include all original unit/integration/module files, not just selected files.
    for path in sorted((tree / "tests").rglob("*.tcl")):
        relative = path.relative_to(tree / "tests").with_suffix("").as_posix()
        if not relative.startswith(("unit/", "integration/")):
            continue
        reason = "selected" if relative in config["suites"] else next(
            item["reason"] for item in config["excluded_units"] if fnmatch.fnmatchcase(relative, item["glob"]))
        units.append({"unit": relative, "selected_in_matrix": relative in config["suites"],
                      "selected_in_this_job": relative == selected, "reason": reason})
    return units


def prerequisites(port, reference):
    results = []
    # Diagnostic probes do NOT bypass or replace original Tcl setup. All commands
    # are subsequently issued by the unchanged upstream harness again.
    for text in SCOPE["compatibility"]["harness_prerequisites"][reference]:
        try:
            with Client(port) as client:
                reply = client.call(*text.split())
            results.append({"command": text, "status": "available", "reply": repr(reply)})
        except (OSError, ValueError, RuntimeError) as error:
            results.append({"command": text, "status": "unavailable", "error": str(error)})
    return results


def run(reference, suite, name, directory):
    tree = source(reference)
    config = SCOPE["compatibility"]
    argv = ["tclsh", "tests/test_helper.tcl", "--single", suite, *config["options"],
            "--tags", " ".join("-" + tag for tag in config["denied_tags"]), "--host", "127.0.0.1"]
    with server(name, directory / "instance", compatibility=True) as (proc, port):
        probes = prerequisites(port, reference)
        argv += ["--port", str(port)]
        result = command(argv, directory / "tcl", cwd=tree, timeout=config["timeout_seconds_per_run"])
        text = (directory / "tcl/stdout.log").read_text(errors="replace")
        text += "\n" + (directory / "tcl/stderr.log").read_text(errors="replace")
        result.update({"events": events(text), "server_alive": proc.poll() is None,
                       "prerequisites": probes, "missing_prerequisites": [p for p in probes if p["status"] != "available"]})
        result["status"] = classify(result)
    save(directory / "result.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", choices=PINS["upstreams"], required=True)
    parser.add_argument("--suite", choices=SCOPE["compatibility"]["suites"], required=True)
    args = parser.parse_args()
    directory = EVIDENCE / f"compat-{args.reference}-{args.suite.replace('/', '-')}"
    directory.mkdir(parents=True, exist_ok=True)
    result = {"reference": args.reference, "suite": args.suite, "native": {"status": "not_run"},
              "candidate": {"status": "not_run"}, "status": "infrastructure_error"}
    code = 2
    try:
        save(directory / "environment.json", environment())
        shutil.copy2(WORK / "build.json", directory / "build.json")
        original = json.loads((WORK / "tcl-hashes.json").read_text())[args.reference]
        tree = source(args.reference)
        if original != tcl_hashes(tree):
            raise ValueError("upstream Tcl differs from verified source archive")
        save(directory / "unit-inventory.json", inventory(tree, args.suite))
        save(directory / "scope.json", SCOPE["compatibility"])
        # Preserve native result before attempting candidate, even if either fails.
        for role, name in (("native", args.reference), ("candidate", "rillcache")):
            try:
                result[role] = run(args.reference, args.suite, name, directory / role)
            except Exception as error:
                result[role] = {"status": "infrastructure_error", "error": str(error)}
            save(directory / "result.json", result)
        if original != tcl_hashes(tree):
            raise ValueError("Tcl run modified original harness/tests")
        native = result["native"].get("events", {})
        candidate = result["candidate"].get("events", {})
        native_names = Counter(native.get("ok", []) + native.get("err", []))
        candidate_names = Counter(candidate.get("ok", []) + candidate.get("err", []) + candidate.get("skip", []))
        result["unreached_native_observed_tests"] = list((native_names - candidate_names).elements())
        result["unreached_limit"] = "Native-observed names only; dynamic/conditional tests absent from native output are unknown, not counted as covered. Group ignore events are not test counts."
        if result["native"]["status"] != "passed_selected_scope":
            result["status"] = "invalid_native_control"
            code = 2
        elif result["candidate"]["status"] != "passed_selected_scope":
            result["status"] = "candidate_failed"
            code = 1
        else:
            result["status"] = "passed_selected_scope"
            code = 0
    except (Exception, KeyboardInterrupt) as error:
        result["error"] = str(error)
    finally:
        save(directory / "result.json", result)
        text = f"## Tcl compatibility: {args.reference} / {args.suite}\n\n**{result['status']}**\n\n"
        text += "| Endpoint | Status | Exit | Passed | Test failures¹ | Auto skip events | Group ignore events | Exceptions |\n|---|---|---:|---:|---:|---:|---:|---:|\n"
        for role in ("native", "candidate"):
            item = result[role]
            data = item.get("events", {})
            text += f"| {role} | {item['status']} | {item.get('exit_code', 'n/a')} | " + " | ".join(str(len(data.get(k, []))) for k in ("ok", "err", "skip", "ignore", "exception")) + " |\n"
        text += f"\nNative-observed tests not reached by candidate: **{len(result.get('unreached_native_observed_tests', []))}** (names in JSON).\n\n"
        text += "¹ Upstream test failures include assertions and command exceptions. Harness/setup exceptions and zero-test results are never passes. "
        text += "Skip/ignore events are not an exact skipped-assertion count; conditional un-emitted tests remain unknown. "
        text += "Single DB; encoding/digest checks disabled; DEBUG/replication/slow tags excluded identically. "
        text += "See scope, complete unit inventory, revisions, prerequisites and raw native/candidate logs in artifacts.\n\n"
        text += f"Redis `{PINS['upstreams']['redis']['revision']}`; Valkey `{PINS['upstreams']['valkey']['revision']}`.\n"
        summary(text, directory / "summary.md")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
