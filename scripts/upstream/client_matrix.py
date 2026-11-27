#!/usr/bin/env python3
"""Run the versioned language-client matrix against the built candidate.

Every declared client executes the same case set against fresh candidate
servers: a password/ACL plaintext listener, a TLS listener with the test CA
identity and a TLS listener with a certificate that must be rejected. Each
client harness reports one result per declared case and the runner fails closed
on a missing, failed or undeclared exclusion. This is C7 evidence for the exact
tested head only; it never authorizes a release claim.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import subprocess
import sys
from pathlib import Path

import common

MANIFEST = common.ROOT / "ci/upstream/clients-v1.json"
TLS_FIXTURES = common.ROOT / "tests/fixtures/tls"
CASE_ID = re.compile(r"^[a-z0-9][a-z0-9-]{2,63}$")
STATUSES = ("passed", "failed", "excluded")
CLIENT_FIELDS = {"id", "version", "language", "pin", "harness", "command", "exclusions", "expectations"}


def fail(message):
    raise ValueError(message)


def read_pin(client):
    """Return the pinned version and integrity text of one client's lock file."""
    pin = Path(common.ROOT, client["pin"])
    if not pin.is_file():
        fail(f"{client['id']}: pin file is missing: {client['pin']}")
    text = pin.read_text(encoding="utf-8")
    if client["id"] == "redis-py":
        match = re.search(r"^redis==([^\s]+)\s+--hash=sha256:([0-9a-f]{64})$", text, re.M)
        if match is None:
            fail("redis-py requirements must pin one version and one sha256 hash")
        return {"version": match[1], "integrity": "sha256:" + match[2]}
    if client["id"] == "node-redis":
        lock = json.loads(text)
        package = lock.get("packages", {}).get("node_modules/redis")
        if not package or not package.get("integrity"):
            fail("node-redis package-lock must pin the redis package integrity")
        return {"version": package["version"], "integrity": package["integrity"]}
    if client["id"] == "go-redis":
        match = re.search(r"^\s*(?:require\s+)?github\.com/redis/go-redis/v9\s+(v\S+)", text, re.M)
        if match is None:
            fail("go-redis go.mod must pin the redis module version")
        sums = Path(common.ROOT, client["pin"].replace("go.mod", "go.sum"))
        if not sums.is_file() or "github.com/redis/go-redis/v9" not in sums.read_text(encoding="utf-8"):
            fail("go-redis go.sum must pin the redis module checksums")
        return {"version": match[1], "integrity": "go.sum"}
    fail(f"{client['id']}: unsupported client for pin validation")


def check_manifest(manifest, root=common.ROOT):
    if manifest.get("schema") != 1:
        fail("unsupported client matrix schema")
    matrix = manifest["matrix"]
    for field in ("id", "version", "profile", "document", "gap"):
        if not matrix.get(field):
            fail(f"matrix.{field} is required")
    document = Path(root, matrix["document"])
    if not document.is_file():
        fail(f"matrix document is missing: {matrix['document']}")
    text = document.read_text(encoding="utf-8")
    relative = Path(root, "ci/upstream/clients-v1.json").relative_to(root).as_posix()
    for required in (matrix["id"], relative, manifest["clients"][0]["id"]):
        if required not in text:
            fail(f"{matrix['document']} must reference {required}")
    gaps = set(re.findall(r"\*\*([A-Z][1-9][0-9]*)\. ",
                          Path(root, "docs/production-readiness-gaps.md").read_text(encoding="utf-8")))
    if matrix["gap"] not in gaps:
        fail(f"matrix gap {matrix['gap']} is not in the canonical checklist")

    cases = manifest["cases"]
    if not cases:
        fail("the matrix declares no cases")
    ids = set()
    for case in cases:
        if not CASE_ID.match(case.get("id", "")) or case["id"] in ids:
            fail(f"invalid or duplicate case id: {case.get('id')}")
        ids.add(case["id"])
        if not case.get("description"):
            fail(f"{case['id']}: a description is required")

    clients = manifest["clients"]
    if not clients:
        fail("the matrix declares no clients")
    seen = set()
    for client in clients:
        unknown = set(client) - CLIENT_FIELDS
        if unknown:
            fail(f"{client.get('id')}: unknown client fields {sorted(unknown)}")
        if client["id"] in seen:
            fail(f"duplicate client {client['id']}")
        seen.add(client["id"])
        if client["version"] != read_pin(client)["version"]:
            fail(f"{client['id']}: manifest version differs from its pinned lock file")
        if not Path(root, client["harness"]).is_file():
            fail(f"{client['id']}: harness is missing: {client['harness']}")
        if not client["command"]:
            fail(f"{client['id']}: harness command is required")
        for exclusion in client["exclusions"]:
            if exclusion["case"] not in ids:
                fail(f"{client['id']}: exclusion references unknown case {exclusion['case']}")
            if not exclusion.get("reason"):
                fail(f"{client['id']}: exclusion needs a reviewable reason")
        for case_id, expectation in client.get("expectations", {}).items():
            if case_id not in ids:
                fail(f"{client['id']}: expectation references unknown case {case_id}")
            if not expectation.get("reason") or not expectation.get("detail_regex"):
                fail(f"{client['id']}: {case_id} expectation needs a reviewable reason and detail regex")
            try:
                re.compile(expectation["detail_regex"])
            except re.error as error:
                fail(f"{client['id']}: {case_id} detail regex is invalid: {error}")

    credentials = manifest["credentials"]
    for field in ("named_user", "named_password", "limited_user", "limited_password", "default_password"):
        if not credentials.get(field):
            fail(f"credentials.{field} is required")
    for name in ("server.crt", "server.key", "alternate.crt", "alternate.key", "ca.crt"):
        if not (Path(root) / TLS_FIXTURES / name).is_file():
            fail(f"TLS fixture is missing: {name}")
    return manifest


def harness_arguments(client, manifest, ports):
    credentials = manifest["credentials"]
    return [
        *client["command"][1:],
        "--plain-port", str(ports["plain"]),
        "--tls-port", str(ports["tls"]),
        "--alt-tls-port", str(ports["alt"]),
        "--named-user", credentials["named_user"],
        "--named-password", credentials["named_password"],
        "--limited-user", credentials["limited_user"],
        "--limited-password", credentials["limited_password"],
        "--default-password", credentials["default_password"],
        "--ca", str(TLS_FIXTURES / "ca.crt"),
    ]


def run_harness(client, manifest, ports, output):
    directory = Path(output) / client["id"]
    directory.mkdir(parents=True, exist_ok=True)
    result_path = directory / "cases.json"
    argv = [*client["command"][:1], *harness_arguments(client, manifest, ports), "--json-out", str(result_path)]
    environment = {**os.environ, "SOURCE_HEAD_SHA": os.environ.get("SOURCE_HEAD_SHA", head())}
    with (directory / "harness.log").open("wb") as log:
        completed = subprocess.run(argv, cwd=common.ROOT, stdout=log, stderr=subprocess.STDOUT,
                                   timeout=300, check=False, env=environment)
    record = {"exit_code": completed.returncode, "argv": argv,
              "log": str((directory / "harness.log").relative_to(output))}
    if not result_path.is_file():
        record.update(status="failed", failure="harness produced no case results")
        return record
    try:
        payload = json.loads(result_path.read_text(encoding="utf-8"))
    except ValueError as error:
        record.update(status="failed", failure=f"invalid case results: {error}")
        return record
    declared = {case["id"] for case in manifest["cases"]}
    reported = {}
    for case in payload.get("cases", []):
        if case.get("id") in reported:
            record.update(status="failed", failure=f"duplicate case result {case.get('id')}")
            return record
        reported[case.get("id")] = case
    exclusions = {item["case"]: item for item in client["exclusions"]}
    problems = []
    for case_id in sorted(declared):
        case = reported.get(case_id)
        if case is None:
            problems.append({"case": case_id, "problem": "case was not reported"})
            continue
        if case.get("status") not in STATUSES:
            problems.append({"case": case_id, "problem": f"unknown status {case.get('status')}"})
        elif case_id in exclusions:
            if case["status"] != "excluded":
                problems.append({"case": case_id, "problem": "declared exclusion ran instead of being excluded"})
        elif case["status"] != "passed":
            problems.append({"case": case_id, "problem": case.get("status"),
                             "detail": case.get("detail", "")})
        else:
            expectation = client.get("expectations", {}).get(case_id)
            if expectation is not None and not re.fullmatch(expectation["detail_regex"],
                                                            case.get("detail", "")):
                problems.append({"case": case_id, "problem": "declared detail drifted",
                                 "detail": case.get("detail", ""),
                                 "expected": expectation["detail_regex"]})
    extra = sorted(set(reported) - declared)
    if extra:
        problems.append({"case": None, "problem": f"undeclared case results: {extra}"})
    record["version"] = payload.get("version")
    record["protocols"] = payload.get("protocols")
    record["cases"] = [{key: case[key] for key in ("id", "status", "detail") if key in case}
                       for case in payload.get("cases", [])]
    if record["version"] != client["version"]:
        problems.append({"case": None, "problem": f"harness version {record['version']} differs from the pin"})
    record["status"] = "passed" if not problems else "failed"
    if problems:
        record["problems"] = problems
    return record


def head():
    if os.environ.get("SOURCE_HEAD_SHA"):
        return os.environ["SOURCE_HEAD_SHA"]
    return common.probe(["git", "rev-parse", "HEAD"], timeout=10, cwd=common.ROOT,
                        capture_output=True, text=True, check=True).stdout.strip()


def write_acl(path, credentials):
    path.write_text(
        f"user default on >{credentials['default_password']} ~* &* +@all\n"
        f"user {credentials['named_user']} on >{credentials['named_password']} ~* &* +@all\n"
        f"user {credentials['limited_user']} on >{credentials['limited_password']} ~allowed:* &* +get +ping\n",
        encoding="ascii")


def distinct_ports(count):
    """Reserve distinct loopback ports; a repeated bind would collide at startup."""
    names = ("plain", "tls", "alt")
    chosen = []
    while len(chosen) < count:
        candidate = common.available_port()
        if candidate not in chosen:
            chosen.append(candidate)
    return dict(zip(names, chosen))


def run(output, candidate, manifest):
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    credentials = manifest["credentials"]
    aclfile = output / "servers" / "users.acl"
    aclfile.parent.mkdir(parents=True, exist_ok=True)
    write_acl(aclfile, credentials)
    original_binary = common.binary
    result = {
        "schema": 1,
        "matrix": {"id": manifest["matrix"]["id"], "version": manifest["matrix"]["version"],
                   "profile": manifest["matrix"]["profile"], "document": manifest["matrix"]["document"],
                   "gap": manifest["matrix"]["gap"]},
        "candidate": {"path": str(candidate), "sha256": common.digest(candidate), "head": head()},
        "environment": {"platform": platform.platform(), "python": platform.python_version(),
                        "node": node_version(), "go": go_version(),
                        "runner_image": os.environ.get("ImageOS")},
        "clients": [], "failures": [],
    }
    ports = distinct_ports(3)
    try:
        common.binary = lambda name: candidate if name == "rillcache" else original_binary(name)
        listeners = {
            "plain": {"aclfile": aclfile},
            "tls": {"aclfile": aclfile, "tls": (TLS_FIXTURES / "server.crt", TLS_FIXTURES / "server.key")},
            "alt": {"aclfile": aclfile, "tls": (TLS_FIXTURES / "alternate.crt", TLS_FIXTURES / "alternate.key")},
        }
        active = {}
        from contextlib import ExitStack
        with ExitStack() as stack:
            for name, options in listeners.items():
                tls = options.get("tls")
                stack.enter_context(common.server(
                    "rillcache", output / "servers" / name, port=ports[name],
                    aclfile=options["aclfile"], tls=tls, readiness="tcp"))
            for client in manifest["clients"]:
                record = {"id": client["id"], "language": client["language"],
                          "declared_version": client["version"], "pin": client["pin"],
                          "integrity": read_pin(client)["integrity"]}
                record.update(run_harness(client, manifest, ports, output))
                if record["status"] != "passed":
                    result["failures"].append({"kind": "client", "id": client["id"],
                                               "problems": record.get("problems", record.get("failure"))})
                result["clients"].append(record)
    finally:
        common.binary = original_binary
    result["status"] = "passed" if not result["failures"] else "failed"
    common.save(output / "result.json", result)
    return result


def node_version():
    try:
        return common.probe(["node", "--version"], timeout=30, capture_output=True, text=True,
                            check=False).stdout.strip() or None
    except OSError:
        return None


def go_version():
    try:
        return common.probe(["go", "version"], timeout=30, capture_output=True, text=True,
                            check=False).stdout.strip() or None
    except OSError:
        return None


def summary_text(result):
    lines = [
        f"## Language-client matrix: {result['status']}", "",
        f"Matrix `{result['matrix']['id']}` v{result['matrix']['version']} for candidate "
        f"`{result['candidate']['head']}` (`{result['candidate']['sha256']}`).", "",
        "| Client | Pinned version | Cases | Status |", "| --- | --- | --- | --- |",
    ]
    for client in result["clients"]:
        cases = client.get("cases", [])
        passed = sum(1 for case in cases if case.get("status") == "passed")
        lines.append(f"| {client['id']} | {client['declared_version']} | {passed}/{len(cases)} passed | "
                     f"{client['status']} |")
    lines += ["", f"- failures: {len(result['failures'])}", "",
              "Every client executes the same declared case set against fresh password/ACL, TLS and "
              "wrong-identity candidate servers. A missing, failed or undeclared case fails this job. "
              "A green matrix is per-client evidence for this exact head; it is not a support window, "
              "release acceptance or a claim that other client versions behave identically.", ""]
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="evidence directory")
    parser.add_argument("--candidate", type=Path, default=None)
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    parser.add_argument("--check", action="store_true", help="validate the matrix only; no servers or clients run")
    args = parser.parse_args(argv)
    try:
        manifest = check_manifest(json.loads(args.manifest.read_text(encoding="utf-8")))
    except (OSError, KeyError, TypeError, ValueError) as error:
        print(f"client matrix invalid: {error}", file=sys.stderr)
        return 2
    if args.check:
        print(f"Client matrix valid: {manifest['matrix']['id']} v{manifest['matrix']['version']}, "
              f"{len(manifest['clients'])} clients, {len(manifest['cases'])} cases.")
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
