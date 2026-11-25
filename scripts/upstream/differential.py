"""Versioned, deterministic Redis/Valkey differential corpus (execution is hosted-only)."""
import argparse
import json
import random
import sys
import time
from pathlib import Path

import common

MANIFEST = common.ROOT / "ci/upstream/differential-v1.json"


def decode(value):
    if isinstance(value, dict) and set(value) == {"bytes_hex"}:
        return bytes.fromhex(value["bytes_hex"])
    return value


def encode(value):
    if isinstance(value, bytes):
        return {"bytes_hex": value.hex()}
    if isinstance(value, list):
        return [encode(item) for item in value]
    return value


def case_commands(case, seed):
    # Seeded names isolate each run and make every artifact replayable.
    rng = random.Random(seed)
    suffix = f"{rng.getrandbits(32):08x}"
    commands = []
    for command in case["commands"]:
        args = [decode(arg) for arg in command]
        if len(args) > 1 and isinstance(args[1], str):
            args[1] = args[1] + ":" + suffix
        commands.append(args)
    return commands


def run_case(client, case, seed):
    replies = []
    for command in case_commands(case, seed):
        try:
            if case.get("semantic") == "expiry" and len(replies) == 2:
                time.sleep(0.55)
            replies.append({"reply": encode(client.call(*command))})
        except RuntimeError as error:
            replies.append({"error": str(error)})
    if case.get("semantic") == "expiry":
        # PTTL is timing-dependent; compare its type/range and eventual expiry, not milliseconds.
        ttl = replies[1].get("reply")
        replies[1] = {"reply": "valid-positive-ttl" if isinstance(ttl, int) and 0 < ttl <= 500 else "invalid-ttl"}
    return replies


def run(output, candidate):
    manifest = json.loads(MANIFEST.read_text())
    seed = int(manifest["seed"])
    results = {"schema": 1, "manifest": str(MANIFEST.relative_to(common.ROOT)), "seed": seed,
               "candidate": {"path": str(candidate), "sha256": common.digest(candidate)},
               "references": {}, "divergences": []}
    # Existing launchers use pinned sources. Override only the candidate executable.
    original_binary = common.binary
    try:
        common.binary = lambda name: candidate if name == "rillcache" else original_binary(name)
        for reference in manifest["references"]:
            results["references"][reference] = {"version": common.PINS["upstreams"][reference]["version"], "cases": {}}
            for name in ("rillcache", reference):
                with common.server(name, output / reference / name, error_dialect=reference if name == "rillcache" else None) as (_, port):
                    with common.Client(port) as client:
                        for index, case in enumerate(manifest["cases"]):
                            results["references"][reference]["cases"].setdefault(case["id"], {})[name] = run_case(client, case, seed + index)
                            common.save(output / "result.json", results)
            for index, case in enumerate(manifest["cases"]):
                pair = results["references"][reference]["cases"][case["id"]]
                if pair["rillcache"] != pair[reference]:
                    results["divergences"].append({"reference": reference, "case": case["id"],
                                                   "seed": seed + index,
                                                   "candidate": pair["rillcache"], "reference_result": pair[reference]})
                    common.save(output / "result.json", results)
    finally:
        common.binary = original_binary
    common.save(output / "result.json", results)
    return bool(results["divergences"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, default=common.binary("rillcache"))
    args = parser.parse_args()
    try:
        divergent = run(args.output.resolve(), args.candidate.resolve())
    except Exception as error:
        print(f"differential corpus failed: {error}; inspect retained output", file=sys.stderr)
        raise
    return 1 if divergent else 0


if __name__ == "__main__":
    raise SystemExit(main())
