#!/usr/bin/env python3
"""Execute the versioned compatibility profile against pinned Redis and Valkey.

Every declared row, exclusion and client row is executed on the built candidate
and on each pinned native reference, and the outcome is recorded as pass/fail in
the evidence directory. This is C1 evidence for the exact tested head; it never
authorizes a production, drop-in or support claim.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import subprocess
import sys
from pathlib import Path

import common

MANIFEST = common.ROOT / "ci/upstream/profile-v1.json"
CHECKLIST = common.ROOT / "docs/production-readiness-gaps.md"
TARGETS = common.ROOT / "benchmarks/targets.mk"
GAP_ID = re.compile(r"\*\*([A-Z][1-9][0-9]*)\. ")
PLACEHOLDER = "{p}"
CLIENT_STATUS = ("tested", "partial", "excluded")
CLIENT_BINARIES = {"redis-cli": lambda: common.source("redis") / "src" / "redis-cli"}
ROW_FIELDS = {"id", "family", "commands", "references", "sort_replies", "sort_tail_replies",
              "sort_pair_replies", "ttl_indices", "length_replies", "float_replies", "select_fields"}
MAX_TTL = 86_400_000
FLOAT_DECIMALS = 6


class CommandError(str):
    """A top-level command error reply; kept as a value so rows stay comparable."""


def fail(message):
    raise ValueError(message)


def decode(value):
    if isinstance(value, dict) and set(value) == {"bytes_hex"}:
        return bytes.fromhex(value["bytes_hex"])
    return value


def evidence(value):
    """JSON-safe evidence without losing the difference between error and bulk."""
    if isinstance(value, (CommandError, common.ErrorReply)):
        return {"error": str(value)}
    if isinstance(value, bytes):
        return {"bytes_hex": value.hex()}
    if isinstance(value, list):
        return [evidence(item) for item in value]
    return value


def canonical(replies):
    return json.dumps(evidence(replies), sort_keys=True)


def digest(replies):
    return hashlib.sha256(canonical(replies).encode()).hexdigest()


def check_manifest(manifest, root=common.ROOT):
    """Validate the profile against the pins, the checklist and the target doc."""
    if manifest.get("schema") != 1:
        fail("unsupported profile schema")
    profile = manifest["profile"]
    for field in ("id", "version", "status", "application_version", "document", "readiness_gap"):
        if not profile.get(field):
            fail(f"profile.{field} is required")
    if profile["status"] not in ("draft", "supported", "withdrawn"):
        fail("unknown profile status")
    document = Path(root) / profile["document"]
    if not document.is_file():
        fail(f"profile document is missing: {profile['document']}")
    text = document.read_text(encoding="utf-8")
    relative = Path(root, "ci/upstream/profile-v1.json").relative_to(root).as_posix()
    for required in (profile["id"], relative):
        if required not in text:
            fail(f"{profile['document']} must reference {required}")

    references = manifest["references"]
    if set(references) != set(common.PINS["upstreams"]):
        fail("profile references differ from the pinned upstream set")
    targets = Path(root).joinpath(TARGETS.relative_to(root)).read_text(encoding="utf-8")
    for name, pin in common.PINS["upstreams"].items():
        declared = references[name]
        if declared["version"] != pin["version"] or declared["sha256"] != pin["sha256"]:
            fail(f"{name} profile pin differs from ci/upstream/pins.json")
        for key in ("version", "sha256"):
            match = re.search(rf"^{name.upper()}_{key.upper()}\s*:=\s*(\S+)$", targets, re.M)
            if match is None or match[1] != pin[key]:
                fail(f"{name} {key} differs from benchmarks/targets.mk")

    commands = manifest["commands"]
    for field in ("source", "compared_fields", "minimum_rows"):
        if field not in commands:
            fail(f"commands.{field} is required")
    if commands["source"] != "COMMAND LIST":
        fail("the supported command surface must be the COMMAND LIST result")
    if not isinstance(commands["minimum_rows"], int) or commands["minimum_rows"] < 1:
        fail("commands.minimum_rows must be a positive integer")
    seen = set()
    for divergence in commands["declared_divergences"]:
        if divergence["id"] in seen:
            fail(f"duplicate declared divergence {divergence['id']}")
        seen.add(divergence["id"])
        if divergence["field"] not in ("presence", "arity"):
            fail(f"{divergence['id']}: unsupported compared field")
        if divergence["field"] == "presence" and set(divergence["references"]) - set(references):
            fail(f"{divergence['id']}: unknown reference name")
        if not divergence.get("reason") or not divergence.get("command"):
            fail(f"{divergence['id']}: a command and reviewable reason are required")
    for noop in commands.get("declared_noops", []):
        if not noop.get("reason") or not noop.get("command"):
            fail("declared no-ops need a command and a reviewable reason")

    rows = manifest["rows"]
    if not rows:
        fail("profile declares no rows")
    row_ids = set()
    for row in rows:
        unknown = set(row) - ROW_FIELDS
        if unknown:
            fail(f"{row.get('id')}: unknown row fields {sorted(unknown)}")
        if "id" not in row or "family" not in row or not row.get("commands"):
            fail("each row needs id, family and commands")
        if row["id"] in row_ids:
            fail(f"duplicate row {row['id']}")
        row_ids.add(row["id"])
        for argv in row["commands"]:
            if not isinstance(argv, list) or not argv or not all(isinstance(part, (str, dict)) for part in argv):
                fail(f"{row['id']}: rows use string argv or one bytes_hex object per argument")
        for field in ("sort_replies", "sort_tail_replies", "sort_pair_replies", "length_replies",
                      "ttl_indices"):
            for index in row.get(field, []):
                if not isinstance(index, int) or not 0 <= index < len(row["commands"]):
                    fail(f"{row['id']}: {field} index {index} is out of range")
        for index in row.get("float_replies", []):
            if not isinstance(index, int) or not 0 <= index < len(row["commands"]):
                fail(f"{row['id']}: float_replies index {index} is out of range")
        for entry in row.get("select_fields", []):
            if not 0 <= entry["index"] < len(row["commands"]) or not entry.get("fields"):
                fail(f"{row['id']}: select_fields needs an in-range index and fields")
            entry.setdefault("each", False)
        if set(row.get("references", references)) - set(references):
            fail(f"{row['id']}: unknown reference scope")

    for divergence in manifest.get("declared_row_divergences", []):
        if divergence["row"] not in row_ids:
            fail(f"{divergence['id']}: unknown row {divergence['row']}")
        if not divergence.get("reason"):
            fail(f"{divergence['id']}: a reviewable reason is required")
        if set(divergence["references"]) - set(references):
            fail(f"{divergence['id']}: unknown reference name")
        for field in ("candidate_sha256", "reference_sha256"):
            if not re.fullmatch(r"[0-9a-f]{64}", divergence.get(field, "")):
                fail(f"{divergence['id']}: {field} must be a sha256 digest of the frozen replies")

    gaps = set(GAP_ID.findall(Path(root, CHECKLIST).read_text(encoding="utf-8")))
    for exclusion in manifest["exclusions"]:
        if exclusion["gap"] not in gaps:
            fail(f"{exclusion['id']}: gap {exclusion['gap']} is not in the canonical checklist")
        if not exclusion.get("reason"):
            fail(f"{exclusion['id']}: an exclusion needs a reviewable reason")
        if not exclusion["commands"]:
            fail(f"{exclusion['id']}: an exclusion needs at least one command")
        for command in exclusion["commands"]:
            if command["candidate"] not in ("error", "ok"):
                fail(f"{exclusion['id']}: candidate expectation must be error or ok")
            if command["reference"] not in ("success", "error", "any"):
                fail(f"{exclusion['id']}: unknown reference expectation")
            if not command["argv"]:
                fail(f"{exclusion['id']}: empty exclusion argv")

    clients = manifest["clients"]
    if not clients:
        fail("profile must declare its client matrix")
    client_ids = set()
    for client in clients:
        if client["id"] in client_ids:
            fail(f"duplicate client {client['id']}")
        client_ids.add(client["id"])
        if client["status"] not in CLIENT_STATUS:
            fail(f"{client['id']}: unsupported client status")
        if client["status"] == "tested" and not client.get("evidence"):
            fail(f"{client['id']}: a tested client needs evidence")
        if client["status"] != "tested" and not (client.get("evidence") or client.get("gap")):
            fail(f"{client['id']}: only tested clients may omit an evidence or gap note")

    client_rows = manifest.get("client_rows", [])
    if not client_rows:
        fail("profile must execute at least one client row")
    executed = set()
    for row in client_rows:
        if row["id"] in executed:
            fail(f"duplicate client row {row['id']}")
        executed.add(row["id"])
        if row["client"] not in CLIENT_BINARIES:
            fail(f"{row['id']}: unknown client {row['client']}")
        if row["client"] not in client_ids:
            fail(f"{row['id']}: client is not declared in the client matrix")
        if not row.get("args"):
            fail(f"{row['id']}: client row needs args")
    return manifest


def substitute(value, prefix):
    if isinstance(value, str):
        return value.replace(PLACEHOLDER, prefix)
    return value


def commands_for(commands, prefix):
    return [[substitute(decode(part), prefix) for part in argv] for argv in commands]


def sortable(item):
    return json.dumps(evidence(item), sort_keys=True)


def normalize(row, replies):
    """Apply only the bounded, declared normalizations of this row."""
    replies = list(replies)
    for index in row.get("ttl_indices", []):
        value = replies[index]
        if isinstance(value, int) and value > 0:
            replies[index] = "valid-positive-ttl" if value <= MAX_TTL else "implausible-ttl"
    for index in row.get("length_replies", []):
        value = replies[index]
        if isinstance(value, list):
            replies[index] = {"reply_length": len(value)}
    for index in row.get("sort_replies", []):
        value = replies[index]
        if isinstance(value, list):
            replies[index] = sorted(value, key=sortable)
    for index in row.get("sort_tail_replies", []):
        value = replies[index]
        if isinstance(value, list) and len(value) == 2 and isinstance(value[1], list):
            replies[index] = [value[0], sorted(value[1], key=sortable)]
    for index in row.get("sort_pair_replies", []):
        value = replies[index]
        if isinstance(value, list) and len(value) % 2 == 0:
            pairs = [(value[position], value[position + 1]) for position in range(0, len(value), 2)]
            replies[index] = [item for pair in sorted(pairs, key=lambda pair: sortable(pair[0])) for item in pair]
    for index in row.get("float_replies", []):
        replies[index] = round_floats(replies[index])
    for entry in row.get("select_fields", []):
        replies[entry["index"]] = select_fields(replies[entry["index"]], entry["fields"], entry.get("each", False))
    return replies


def round_floats(value):
    """Compare numeric reply fields by value at 1e-6 instead of by text formatting."""
    if isinstance(value, bytes):
        try:
            text = value.decode("ascii")
            parsed = float(text)
        except (UnicodeDecodeError, ValueError):
            return value
        if "inf" in text or "nan" in text or not any(marker in text for marker in (".", "e", "E")):
            return value
        return {"float": round(parsed, FLOAT_DECIMALS)}
    if isinstance(value, list):
        return [round_floats(item) for item in value]
    return value


def select_fields(value, fields, each):
    """Keep only the named portable fields of a flat field/value reply."""
    wanted = [field.encode() for field in fields]

    def one_map(item):
        if not isinstance(item, list) or len(item) % 2 or any(not isinstance(part, bytes) for part in item[::2]):
            return item
        return [item[position + offset] for position in range(0, len(item), 2) if item[position] in wanted
                for offset in (0, 1)]

    if each and isinstance(value, list):
        return [one_map(item) for item in value]
    return one_map(value)


def probe(client, commands):
    replies = []
    for command in commands:
        try:
            replies.append(client.call(*command))
        except RuntimeError as error:
            replies.append(CommandError(str(error)))
    return replies


def binary_identity(name):
    """Record the exact reference binary; a missing one cannot produce a green run."""
    path = Path(common.binary(name))
    return {"path": str(path), "sha256": common.digest(path) if path.is_file() else None}


def head():
    """Record the tested revision without probing unavailable toolchains."""
    if os.environ.get("SOURCE_HEAD_SHA"):
        return os.environ["SOURCE_HEAD_SHA"]
    result = common.probe(["git", "rev-parse", "HEAD"], timeout=10, cwd=common.ROOT,
                          capture_output=True, text=True, check=True)
    return result.stdout.strip()


def surface(client):
    names = [name.decode() for name in client.call("COMMAND", "LIST")]
    inventory = {}
    for name in names:
        reply = client.call("COMMAND", "INFO", name)
        if not reply or reply[0] is None:
            inventory[name] = {"present": False}
            continue
        inventory[name] = {"present": True, "arity": reply[0][1]}
    return inventory


def surface_comparison(candidate, reference, declared, reference_name):
    mismatches = []
    exemptions = {item["command"].lower(): item for item in declared}
    for command, expected in candidate.items():
        actual = reference.get(command)
        exemption = exemptions.get(command)
        if actual is None:
            if exemption and exemption["field"] == "presence" and reference_name in exemption.get("references", []):
                continue
            mismatches.append({"command": command, "field": "presence", "candidate": expected,
                               "reference": "missing",
                               "declared_divergence": exemption["id"] if exemption else None})
            continue
        if expected.get("arity") != actual.get("arity"):
            if exemption and exemption["field"] == "arity" and exemption.get("candidate") == expected.get("arity"):
                continue
            mismatches.append({"command": command, "field": "arity", "candidate": expected.get("arity"),
                               "reference": actual.get("arity"),
                               "declared_divergence": exemption["id"] if exemption else None})
    return mismatches


def run_client_row(binary, port, row):
    prefix = f"c-{row['id']}-"
    argv = [str(binary), "-h", "127.0.0.1", "-p", str(port), *[substitute(arg, prefix) for arg in row["args"]]]
    result = subprocess.run(argv, capture_output=True, timeout=20, check=False)
    stdout = result.stdout
    if row.get("sort_lines"):
        stdout = b"\n".join(sorted(stdout.rstrip(b"\n").split(b"\n")))
    return {"exit_code": result.returncode, "stdout_hex": stdout.hex(),
            "stderr": result.stderr.decode(errors="replace")[:400]}


def frozen(manifest, row_id, reference_name):
    for divergence in manifest.get("declared_row_divergences", []):
        if divergence["row"] == row_id and reference_name in divergence["references"]:
            return divergence
    return None


def classify_row(manifest, row_id, reference_name, candidate_replies, reference_replies):
    """Pass, freeze a reviewed divergence verbatim, or fail a changed row."""
    if candidate_replies == reference_replies:
        return {"status": "passed"}
    record = {"candidate": evidence(candidate_replies),
              "reference_replies": evidence(reference_replies),
              "candidate_sha256": digest(candidate_replies),
              "reference_sha256": digest(reference_replies)}
    declared = frozen(manifest, row_id, reference_name)
    if declared and declared["candidate_sha256"] == record["candidate_sha256"] \
            and declared["reference_sha256"] == record["reference_sha256"]:
        record.update(status="declared-divergence", declared_divergence=declared["id"])
        return record
    record["status"] = "failed"
    if declared:
        record["failure"] = "declared divergence drifted; review the frozen replies"
    return record


def run(output, candidate, manifest):
    output = Path(output).resolve()
    original_binary = common.binary
    client_binaries = {name: factory() for name, factory in CLIENT_BINARIES.items()}
    for name, path in client_binaries.items():
        if not Path(path).is_file():
            fail(f"pinned {name} is missing: {path}")
    result = {
        "schema": 1,
        "profile": {"id": manifest["profile"]["id"], "version": manifest["profile"]["version"],
                    "status": manifest["profile"]["status"], "document": manifest["profile"]["document"]},
        "candidate": {"path": str(candidate), "sha256": common.digest(candidate), "head": head()},
        "references": {name: {"version": pin["version"], "sha256": pin["sha256"], **binary_identity(name)}
                       for name, pin in common.PINS["upstreams"].items()},
        "surface": {}, "rows": [], "exclusions": [], "clients": [], "failures": [],
        "environment": {"platform": platform.platform(), "python": platform.python_version(),
                        "runner_image": os.environ.get("ImageOS"), "pins": common.PINS},
    }
    try:
        common.binary = lambda name: candidate if name == "rillcache" else original_binary(name)
        for reference_name in manifest["references"]:
            scoped = [row for row in manifest["rows"]
                      if reference_name in row.get("references", manifest["references"])]
            responses = {}
            surfaces = {}
            for name in ("rillcache", reference_name):
                with common.server(name, output / reference_name / name,
                                   error_dialect=reference_name if name == "rillcache" else None) as (_, port):
                    with common.Client(port) as client:
                        surfaces[name] = surface(client)
                        responses[name] = {}
                        for row in manifest["rows"]:
                            if row not in scoped:
                                continue
                            prefix = f"{row['id']}-"
                            responses[name][row["id"]] = normalize(row, probe(client, commands_for(row["commands"], prefix)))
            for name in ("rillcache", reference_name):
                with common.server(name, output / reference_name / f"{name}-clients",
                                   error_dialect=reference_name if name == "rillcache" else None) as (_, port):
                    responses[f"{name}-clients"] = {
                        row["id"]: run_client_row(client_binaries[row["client"]], port, row)
                        for row in manifest["client_rows"]}
            for exclusion in manifest["exclusions"]:
                for name in ("rillcache", reference_name):
                    with common.server(name, output / reference_name / f"{name}-exclusion-{exclusion['id']}",
                                       error_dialect=reference_name if name == "rillcache" else None) as (_, port):
                        with common.Client(port) as client:
                            responses[f"{name}-exclusion-{exclusion['id']}"] = [
                                probe(client, commands_for([command["argv"]], f"x-{exclusion['id']}-"))[0]
                                for command in exclusion["commands"]]

            for row in manifest["rows"]:
                scoped_to = [name for name in manifest["references"]
                             if name in row.get("references", manifest["references"])]
                record = {"id": row["id"], "family": row["family"], "reference": reference_name,
                          "scoped_to": scoped_to}
                if row not in scoped:
                    record["status"] = "not-in-scope"
                    result["rows"].append(record)
                    continue
                record.update(classify_row(manifest, row["id"], reference_name,
                                           responses["rillcache"][row["id"]],
                                           responses[reference_name][row["id"]]))
                result["rows"].append(record)
                if record["status"] == "failed":
                    result["failures"].append({"kind": "row", "id": row["id"], "reference": reference_name})

            for exclusion in manifest["exclusions"]:
                candidate_outcomes = responses[f"rillcache-exclusion-{exclusion['id']}"]
                reference_outcomes = responses[f"{reference_name}-exclusion-{exclusion['id']}"]
                problems = []
                for position, command in enumerate(exclusion["commands"]):
                    candidate_outcome = candidate_outcomes[position]
                    reference_outcome = reference_outcomes[position]
                    rejected = isinstance(candidate_outcome, (CommandError, common.ErrorReply))
                    if command["candidate"] == "error" and not rejected:
                        problems.append({"argv": command["argv"], "reason": "candidate accepted an excluded command",
                                         "candidate": evidence(candidate_outcome)})
                    if command["candidate"] == "ok" and rejected:
                        problems.append({"argv": command["argv"], "reason": "candidate rejected a declared supported form",
                                         "candidate": evidence(candidate_outcome)})
                    reference_rejected = isinstance(reference_outcome, (CommandError, common.ErrorReply))
                    if command["reference"] == "success" and reference_rejected:
                        problems.append({"argv": command["argv"], "reason": "pinned reference did not accept the command",
                                         "reference": evidence(reference_outcome)})
                    if command["reference"] == "error" and not reference_rejected:
                        problems.append({"argv": command["argv"], "reason": "pinned reference did not reject the command",
                                         "reference": evidence(reference_outcome)})
                record = {"id": exclusion["id"], "gap": exclusion["gap"], "reference": reference_name,
                          "status": "enforced" if not problems else "violated",
                          "candidate": [evidence(item) for item in candidate_outcomes],
                          "reference_replies": [evidence(item) for item in reference_outcomes]}
                if problems:
                    record["problems"] = problems
                    result["failures"].append({"kind": "exclusion", "id": exclusion["id"], "reference": reference_name})
                result["exclusions"].append(record)

            for row in manifest["client_rows"]:
                candidate_result = responses["rillcache-clients"][row["id"]]
                reference_result = responses[f"{reference_name}-clients"][row["id"]]
                passed = candidate_result == reference_result
                record = {"id": row["id"], "client": row["client"], "reference": reference_name,
                          "status": "passed" if passed else "failed"}
                if not passed:
                    record["candidate"] = candidate_result
                    record["reference_result"] = reference_result
                    result["failures"].append({"kind": "client-row", "id": row["id"], "reference": reference_name})
                result["clients"].append(record)

            mismatches = surface_comparison(surfaces["rillcache"], surfaces[reference_name],
                                            manifest["commands"]["declared_divergences"], reference_name)
            result["surface"][reference_name] = {
                "candidate_commands": len(surfaces["rillcache"]),
                "reference_commands": len(surfaces[reference_name]),
                "reference_only_commands": len(set(surfaces[reference_name]) - set(surfaces["rillcache"])),
                "declared_divergences": len(manifest["commands"]["declared_divergences"]),
                "mismatches": mismatches,
                "status": "passed" if not mismatches else "failed",
            }
            if mismatches:
                result["failures"].append({"kind": "surface", "reference": reference_name, "count": len(mismatches)})
            if len(surfaces["rillcache"]) < manifest["commands"].get("minimum_rows", 1):
                result["failures"].append({"kind": "surface-size", "candidate": len(surfaces["rillcache"]),
                                           "minimum": manifest["commands"].get("minimum_rows", 0)})
    finally:
        common.binary = original_binary
    for name, identity in result["references"].items():
        if identity["sha256"] is None:
            result["failures"].append({"kind": "reference-binary", "id": name,
                                       "reason": "pinned reference binary identity is missing"})
    result["status"] = "passed" if not result["failures"] else "failed"
    common.save(output / "result.json", result)
    return result


def summary_text(result):
    surfaces = "".join(
        f"- {name}: {item['candidate_commands']} candidate commands, "
        f"{item['reference_commands']} reference commands, {item['reference_only_commands']} reference-only, "
        f"{len(item['mismatches'])} undeclared mismatches\n"
        for name, item in result["surface"].items())
    def count(items, status):
        return sum(1 for item in items if item["status"] == status)
    rows = [item for item in result["rows"] if item["status"] != "not-in-scope"]
    return (
        f"## Compatibility profile matrix: {result['status']}\n\n"
        f"Profile `{result['profile']['id']}` v{result['profile']['version']} "
        f"({result['profile']['status']}) for candidate `{result['candidate']['head']}`.\n\n"
        f"{surfaces}"
        f"- rows: {len(rows)} executed, {count(rows, 'passed')} passed, "
        f"{count(rows, 'declared-divergence')} declared divergences, {count(rows, 'failed')} failed\n"
        f"- exclusions: {len(result['exclusions'])} checked, "
        f"{count(result['exclusions'], 'enforced')} enforced, {count(result['exclusions'], 'violated')} violated\n"
        f"- client rows: {len(result['clients'])} executed, {count(result['clients'], 'passed')} passed, "
        f"{count(result['clients'], 'failed')} failed\n"
        f"- failures: {len(result['failures'])}\n\n"
        "Rows marked as declared divergences keep their exact candidate and reference replies in "
        "`result.json`, frozen by sha256 in the manifest; any change fails the job. A green run is "
        "per-row compatibility evidence for this exact head only. It is not production readiness, "
        "a support window or a language-client commitment.\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="evidence directory")
    parser.add_argument("--candidate", type=Path, default=None)
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    parser.add_argument("--check", action="store_true", help="validate the profile only; no servers run")
    args = parser.parse_args(argv)
    try:
        manifest = check_manifest(json.loads(args.manifest.read_text(encoding="utf-8")))
    except (OSError, KeyError, TypeError, ValueError) as error:
        print(f"compatibility profile invalid: {error}", file=sys.stderr)
        return 2
    if args.check:
        print(f"Compatibility profile valid: {manifest['profile']['id']} v{manifest['profile']['version']}, "
              f"{len(manifest['rows'])} rows, {len(manifest['exclusions'])} exclusions, "
              f"{len(manifest['client_rows'])} client rows.")
        return 0
    if args.output is None:
        print("--output is required unless --check is used", file=sys.stderr)
        return 2
    candidate = (args.candidate or common.binary("rillcache")).resolve()
    if not candidate.is_file():
        print(f"candidate executable missing: {candidate}", file=sys.stderr)
        return 2
    result = run(args.output, candidate, manifest)
    common.summary(summary_text(result), Path(args.output) / "summary.md")
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
