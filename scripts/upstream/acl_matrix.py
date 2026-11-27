#!/usr/bin/env python3
"""Versioned ACL allow/deny matrix (gap S2) against the candidate and pinned references.

For every declared scenario the harness loads a policy into an ACL file, seeds
data as the administrator, replays the scenario commands as the restricted user,
records whether each command was allowed or denied, and finally re-reads state as
the administrator to prove a denied operation changed nothing it was not allowed
to change.

The candidate must match the declared expectation exactly. Scenarios marked
`differential` must also reach the same allow/deny decision on pinned Redis and
Valkey; scenarios listed in a declared divergence record the reference decision
and require the reviewed candidate decision instead. Any other difference fails
the run. Evidence is written to result.json/summary.md.

Authoring note: the matrix is a contract for the supported subset, not a claim
that the candidate implements every Redis ACL semantic, and not a security
sign-off.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import sys
from pathlib import Path

import common

MANIFEST = common.ROOT / "ci/upstream/acl-v1.json"
DECISIONS = {"allow", "deny", "error"}
SCENARIO_FIELDS = {"id", "acl", "seed", "commands", "expect", "post", "post_expect",
                   "isolated", "differential", "admin_commands", "admin_setup",
                   "auth_expected", "reference_only"}
FIELDS = ("schema", "matrix", "admin", "user", "declared_divergences", "scenarios")


def fail(message):
    raise ValueError(message)


def check_manifest(manifest, root=common.ROOT):
    if manifest.get("schema") != 1:
        fail("unsupported ACL matrix schema")
    matrix = manifest["matrix"]
    for field in ("id", "version", "profile", "gap", "document"):
        if not matrix.get(field):
            fail(f"matrix.{field} is required")
    document = Path(root, matrix["document"])
    if not document.is_file():
        fail(f"matrix document is missing: {matrix['document']}")
    text = document.read_text(encoding="utf-8")
    for required in (matrix["id"], "ci/upstream/acl-v1.json"):
        if required not in text:
            fail(f"{matrix['document']} must reference {required}")
    for section in ("admin", "user"):
        if not manifest[section].get("user") or not manifest[section].get("password"):
            fail(f"{section} needs user and password")

    scenario_ids = set()
    for scenario in manifest["scenarios"]:
        unknown = set(scenario) - SCENARIO_FIELDS
        if unknown:
            fail(f"{scenario.get('id')}: unknown fields {sorted(unknown)}")
        if not scenario.get("id") or scenario["id"] in scenario_ids:
            fail(f"invalid or duplicate scenario {scenario.get('id')}")
        scenario_ids.add(scenario["id"])
        commands = scenario.get("commands", [])
        expect = scenario.get("expect", [])
        if len(commands) != len(expect):
            fail(f"{scenario['id']}: commands and expect length differ")
        for decision in expect:
            if decision not in DECISIONS:
                fail(f"{scenario['id']}: unknown expectation {decision!r}")
        if len(scenario.get("post", [])) != len(scenario.get("post_expect", [])):
            fail(f"{scenario['id']}: post and post_expect length differ")
        if scenario.get("auth_expected") not in (None, "allow", "deny"):
            fail(f"{scenario['id']}: auth_expected must be allow or deny")
        if scenario.get("reference_only"):
            unknown_refs = set(scenario["reference_only"]) - set(common.PINS["upstreams"])
            if unknown_refs:
                fail(f"{scenario['id']}: unknown reference {sorted(unknown_refs)}")

    for divergence in manifest.get("declared_divergences", []):
        if not divergence.get("id") or not divergence.get("reason"):
            fail("each declared divergence needs an id and a reviewable reason")
        unknown = set(divergence.get("scenarios", [])) - scenario_ids
        if unknown:
            fail(f"{divergence['id']}: unknown scenario {sorted(unknown)}")
        if not divergence.get("scenarios"):
            fail(f"{divergence['id']}: no scenarios listed")
    return manifest


def acl_file(path: Path, manifest: dict, rules: list[str], user: dict, admin: dict) -> Path:
    """Write a bounded ACL file: admin user plus the scenario user."""
    rules = list(rules)
    if f">{user['password']}" not in rules:
        rules.append(f">{user['password']}")
    lines = [
        # Both implementations require an enabled default user in an ACL file;
        # giving it a password keeps unauthenticated connections out.
        f"user default on >{admin['password']} ~* &* +@all",
        f"user {admin['user']} on >{admin['password']} ~* &* +@all",
        " ".join([f"user {user['user']}"] + rules),
    ]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="ascii")
    if path.stat().st_size > 64 * 1024:
        fail("ACL file exceeds the bounded size")
    return path


def decision(reply) -> str:
    """Classify one reply: NOPERM is a policy denial, any other error is 'error'."""
    if isinstance(reply, dict) and "error" in reply:
        return "deny" if reply["error"].startswith("NOPERM") else "error"
    return "allow"


def normalize(reply):
    """Compact evidence tagged by RESP class, without losing bytes."""
    if isinstance(reply, dict):
        return reply
    if isinstance(reply, bytes):
        # RESP simple strings arrive as bytes too; classify by a marker-free copy.
        text = reply.decode(errors="replace")[:200]
        return {"bulk": text, "status": text}
    if isinstance(reply, int):
        return {"int": reply}
    if reply is None:
        return {"null": True}
    if isinstance(reply, list):
        return {"array": [normalize(item) for item in reply][:16]}
    return {"other": str(reply)[:200]}


def run_scenario(server_name: str, scenario: dict, manifest: dict, output: Path) -> dict:
    admin, user = manifest["admin"], manifest["user"]
    directory = output / server_name / scenario["id"]
    directory.mkdir(parents=True, exist_ok=True)
    rules = list(scenario["acl"])
    if f">{user['password']}" not in rules:
        rules.append(f">{user['password']}")
    policy = acl_file(directory / "users.acl", manifest, rules, user, admin)
    record = {"id": scenario["id"], "server": server_name, "decisions": [], "problems": []}
    with common.server(server_name, directory / "server", aclfile=policy,
                       readiness="tcp") as (_, port):
        with common.Client(port) as admin_client:
            admin_client.call("AUTH", admin["user"], admin["password"])
            if scenario.get("admin_commands"):
                for command in scenario["commands"]:
                    record["decisions"].append(decision(_call(admin_client, command)))
            else:
                for command in scenario.get("seed", []):
                    _call(admin_client, command)
                for command in scenario.get("admin_setup", []):
                    _call(admin_client, command)
        if scenario.get("commands") and not scenario.get("admin_commands"):
            with common.Client(port) as user_client:
                record["auth"] = _authenticate(user_client, user)
                if scenario.get("auth_expected"):
                    if scenario["auth_expected"] == "nopass":
                        pass
                record["replies"] = []
                for command in scenario["commands"]:
                    reply = _call(user_client, command)
                    record["decisions"].append(decision(reply))
                    record["replies"].append(normalize(reply))
        elif scenario.get("auth_expected"):
            with common.Client(port) as user_client:
                record["auth"] = _authenticate(user_client, user)
        with common.Client(port) as admin_client:
            admin_client.call("AUTH", admin["user"], admin["password"])
            post = []
            for command in scenario.get("post", []):
                post.append(normalize(_call(admin_client, command)))
            record["post"] = post
    return record


def _authenticate(client, user: dict):
    try:
        reply = client.call("AUTH", user["user"], user["password"])
    except RuntimeError as error:
        return {"outcome": "deny", "detail": str(error)[:160]}
    return {"outcome": "allow" if reply == b"OK" else "other", "detail": normalize(reply)}


def _call(client, command):
    try:
        return client.call(*command)
    except RuntimeError as error:
        return {"error": str(error)[:200]}


def verify(manifest: dict, records: dict) -> list[dict]:
    """Check the candidate expectation, divergence declarations and differential agreement."""
    problems = []
    divergence = {}
    for entry in manifest.get("declared_divergences", []):
        for scenario_id in entry["scenarios"]:
            divergence[scenario_id] = entry["id"]
    for scenario in manifest["scenarios"]:
        candidate = records["rillcache"][scenario["id"]]
        expected = scenario["expect"]
        if candidate["decisions"] != expected:
            problems.append({"kind": "candidate-decision", "scenario": scenario["id"],
                             "expected": expected, "observed": candidate["decisions"]})
        for observed, wanted in zip(candidate["post"], scenario.get("post_expect", [])):
            kind = wanted[0]
            value = wanted[1]
            actual = observed.get(kind)
            if kind == "bulk" and actual == value:
                continue
            if kind == "int" and actual == value:
                continue
            if kind == "status" and actual == value:
                continue
            problems.append({"kind": "candidate-state", "scenario": scenario["id"],
                             "expected": wanted, "observed": observed})
        if scenario.get("auth_expected"):
            actual = candidate.get("auth", {}).get("outcome")
            if actual != scenario["auth_expected"]:
                problems.append({"kind": "candidate-auth", "scenario": scenario["id"],
                                 "expected": scenario["auth_expected"], "observed": actual})
        if not scenario.get("differential", True):
            continue
        for reference in common.PINS["upstreams"]:
            if reference in scenario.get("reference_only", []) and reference != "rillcache":
                continue
            observed = records[reference][scenario["id"]]["decisions"]
            if observed == expected:
                continue
            if scenario["id"] in divergence:
                candidate_expected = expected
                if records["rillcache"][scenario["id"]]["decisions"] != candidate_expected:
                    problems.append({"kind": "divergence-candidate", "scenario": scenario["id"],
                                     "declared": divergence[scenario["id"]]})
                continue
            problems.append({"kind": "reference-difference", "scenario": scenario["id"],
                             "reference": reference, "expected": expected, "observed": observed})
    return problems


def run(output: Path, manifest: dict) -> dict:
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    records = {}
    result = {"schema": 1,
              "matrix": {"id": manifest["matrix"]["id"], "version": manifest["matrix"]["version"],
                         "gap": manifest["matrix"]["gap"], "document": manifest["matrix"]["document"]},
              "references": {name: {"version": pin["version"], "sha256": pin["sha256"]}
                             for name, pin in common.PINS["upstreams"].items()},
              "candidate": {"sha256": common.digest(common.binary("rillcache")),
                            "head": os.environ.get("SOURCE_HEAD_SHA") or head()},
              "environment": {"platform": platform.platform(), "python": platform.python_version()},
              "scenarios": [], "failures": []}
    for server_name in ("rillcache", *common.PINS["upstreams"]):
        records[server_name] = {}
        for scenario in manifest["scenarios"]:
            records[server_name][scenario["id"]] = run_scenario(server_name, scenario, manifest, output)
    for scenario in manifest["scenarios"]:
        entry = {"id": scenario["id"], "candidate": records["rillcache"][scenario["id"]],
                 "expect": scenario["expect"],
                 "references": {name: records[name][scenario["id"]]["decisions"]
                                for name in common.PINS["upstreams"]}}
        result["scenarios"].append(entry)
    result["failures"] = verify(manifest, records)
    result["status"] = "passed" if not result["failures"] else "failed"
    common.save(output / "result.json", result)
    return result


def head():
    return common.probe(["git", "rev-parse", "HEAD"], timeout=10, cwd=common.ROOT,
                        capture_output=True, text=True, check=True).stdout.strip()


def summary_text(result: dict) -> str:
    lines = [f"## ACL allow/deny matrix: {result['status']}", "",
             f"Matrix `{result['matrix']['id']}` v{result['matrix']['version']} "
             f"(gap {result['matrix']['gap']}) for candidate `{result['candidate']['head']}`.", "",
             "| Scenario | Candidate | Redis | Valkey | Declared |",
             "| --- | --- | --- | --- | --- |"]
    for entry in result["scenarios"]:
        declared = "yes" if entry["candidate"]["decisions"] != entry["expect"] else ""
        lines.append(f"| {entry['id']} | {', '.join(entry['candidate']['decisions']) or '-'} | "
                     f"{', '.join(entry['references'].get('redis', [])) or '-'} | "
                     f"{', '.join(entry['references'].get('valkey', [])) or '-'} | {declared} |")
    lines += ["", f"- failures: {len(result['failures'])}", "",
              "Every scenario seeds state as the administrator, replays the commands as the "
              "restricted user, and re-reads state as the administrator so a denied operation "
              "must leave data it was not allowed to change untouched. This is the supported "
              "subset contract, not full Redis ACL equivalence and not a security sign-off.", ""]
    return "\n".join(lines)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=common.ROOT / "target/upstream/evidence/acl-matrix")
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    parser.add_argument("--check", action="store_true", help="validate the matrix offline; no servers run")
    args = parser.parse_args(argv)
    try:
        manifest = check_manifest(json.loads(args.manifest.read_text(encoding="utf-8")))
    except (OSError, KeyError, TypeError, ValueError) as error:
        print(f"ACL matrix invalid: {error}", file=sys.stderr)
        return 2
    if args.check:
        print(f"ACL matrix valid: {manifest['matrix']['id']} v{manifest['matrix']['version']}, "
              f"{len(manifest['scenarios'])} scenarios, "
              f"{len(manifest.get('declared_divergences', []))} declared divergences.")
        return 0
    for name in ("rillcache", *common.PINS["upstreams"]):
        binary = common.binary(name)
        if not Path(binary).is_file():
            print(f"missing server binary for {name}: {binary}", file=sys.stderr)
            return 2
    result = run(args.output, manifest)
    common.summary(summary_text(result), args.output / "summary.md")
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
