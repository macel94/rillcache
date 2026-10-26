#!/usr/bin/env python3
"""Build pinned core servers and the existing Cargo target, only in Actions."""

import argparse
import os
from pathlib import Path
import shutil
import tarfile

from common import EVIDENCE, PINS, ROOT, WORK, checked, digest, environment, save, source, summary


def validate_pins():
    import re
    import tomllib

    toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    if PINS["rust"] != toolchain:
        raise ValueError("Rust pin differs from rust-toolchain.toml")
    targets = (ROOT / "benchmarks/targets.mk").read_text()
    for name, pin in PINS["upstreams"].items():
        for key in ("version", "sha256"):
            match = re.search(rf"^{name.upper()}_{key.upper()}\s*:=\s*(\S+)$", targets, re.M)
            if match is None or match[1] != pin[key]:
                raise ValueError(f"{name} {key} differs from benchmarks/targets.mk")


def tcl_hashes(tree):
    paths = [tree / "runtest", *sorted((tree / "tests").rglob("*.tcl"))]
    return {str(path.relative_to(tree)): digest(path) for path in paths if "tests/tmp/" not in str(path)}


def build(bundle=False):
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("Expensive builds run in GitHub Actions, not on the agent host")
    validate_pins()
    WORK.mkdir(parents=True, exist_ok=True)
    build_evidence = EVIDENCE / "build"
    checked(["rustup", "show", "active-toolchain"], build_evidence / "toolchain", timeout=300)
    save(build_evidence / "environment.json", environment())
    checked(PINS["candidate_build"], build_evidence / "candidate", timeout=600,
            env={**os.environ, "CARGO_BUILD_JOBS": "2"})
    shutil.copy2(ROOT / "target/release/rillcache", WORK / "rillcache")
    cache = ROOT / "target/upstream-downloads"
    cache.mkdir(parents=True, exist_ok=True)
    originals = {}
    for name, pin in PINS["upstreams"].items():
        archive = cache / f"{name}-{pin['version']}-{pin['sha256']}.tar.gz"
        if not archive.exists():
            temporary = archive.with_suffix(".partial")
            checked(["curl", "--proto", "=https", "--tlsv1.2", "--fail", "--location", "--show-error",
                     "--retry", "3", "--connect-timeout", "10", "--max-time", "180", pin["url"],
                     "--output", str(temporary)], build_evidence / f"download-{name}", timeout=600)
            if digest(temporary) != pin["sha256"]:
                raise ValueError(f"{name} download checksum mismatch")
            temporary.rename(archive)
        if digest(archive) != pin["sha256"]:
            raise ValueError(f"{name} cached archive checksum mismatch; refusing reuse")
        tree = source(name)
        if tree.exists():
            raise ValueError(f"refusing to reuse compiled source tree: {tree}")
        with tarfile.open(archive) as tar:
            # Extraction filter blocks traversal, unsafe links and device nodes.
            tar.extractall(WORK, filter="data")
        originals[name] = tcl_hashes(tree)
        checked(["make", "-C", str(tree / "src"), "-j2", *PINS["native_make_args"],
                 f"{name}-server", f"{name}-cli", f"{name}-benchmark"],
                build_evidence / name, timeout=900)
        if originals[name] != tcl_hashes(tree):
            raise ValueError(f"{name} build modified upstream Tcl files")
        checked([str(tree / "src" / f"{name}-server"), "--version"], build_evidence / f"version-{name}")
    checked([str(WORK / "rillcache"), "--version"], build_evidence / "version-candidate")
    checked([str(source("redis") / "src/redis-benchmark"), "--version"], build_evidence / "version-generator")
    save(WORK / "tcl-hashes.json", originals)
    shutil.copy2(WORK / "tcl-hashes.json", build_evidence / "tcl-hashes.json")
    metadata = {
        "pins": PINS, "built_git_head": environment()["git_head"]["stdout"].strip(),
        "binary_sha256": {name: digest(WORK / "rillcache" if name == "rillcache" else source(name) / "src" / f"{name}-server")
                          for name in ("rillcache", "redis", "valkey")},
        "generator_sha256": digest(source("redis") / "src/redis-benchmark"),
        "candidate_features": "existing defaults; no feature flags; no external crates",
        "native_scope": "core standalone; no bundled modules, TLS or implementation unit tests",
    }
    save(WORK / "build.json", metadata)
    save(build_evidence / "build.json", metadata)
    if bundle:
        def omit_objects(info):
            return None if Path(info.name).suffix in (".o", ".a", ".d") else info
        with tarfile.open(WORK / "bundle.tar.gz", "w:gz") as tar:
            for item in ("rillcache", "build.json", "tcl-hashes.json", source("redis").name, source("valkey").name):
                tar.add(WORK / item, arcname=item, filter=omit_objects)
    return metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", action="store_true")
    args = parser.parse_args()
    result = {"status": "infrastructure_error"}
    code = 2
    try:
        result = {"status": "built", **build(args.bundle)}
        code = 0
    except (Exception, KeyboardInterrupt) as error:
        result["error"] = str(error)
    finally:
        save(EVIDENCE / "build/result.json", result)
        summary(f"## Upstream build: {result['status']}\n\nRust {PINS['rust']}; Redis {PINS['upstreams']['redis']['version']}; "
                f"Valkey {PINS['upstreams']['valkey']['version']}. See build logs and source/binary hashes.\n",
                EVIDENCE / "build/summary.md")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
