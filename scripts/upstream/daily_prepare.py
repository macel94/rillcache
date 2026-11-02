#!/usr/bin/env python3
"""Prepare five checksum-pinned benchmark tools; never application dependencies."""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import tarfile

from common import EVIDENCE, ROOT, WORK, binary, checked, digest, save, source
from prepare import build

PINS_PATH = ROOT / "ci/daily-pins.json"
PINS = json.loads(PINS_PATH.read_text())
TOOLS = WORK / "daily-tools"
BUILD = EVIDENCE / "daily-build"


def checksum(path, algorithm):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, algorithm).hexdigest()


def download(pin, name, *, license_file=False):
    algorithm = "sha256" if license_file or "sha256" in pin else "sha512"
    expected = pin["license_sha256" if license_file else algorithm]
    url = pin["license_url" if license_file else "url"]
    if not url.startswith("https://") or not re.fullmatch(r"[0-9a-f]{" + str(64 if algorithm == "sha256" else 128) + "}", expected):
        raise ValueError("invalid HTTPS/checksum pin")
    cache = ROOT / "target/upstream-downloads"
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / f"daily-{name}-{expected}"
    if not path.exists():
        temporary = path.with_suffix(".partial")
        checked(["curl", "--proto", "=https", "--tlsv1.2", "--fail", "--location", "--show-error",
                 "--retry", "2", "--connect-timeout", "10", "--max-time", "180", url, "-o", str(temporary)],
                BUILD / f"download-{name}", timeout=600)
        if checksum(temporary, algorithm) != expected:
            raise ValueError(f"{name} archive checksum differs")
        temporary.rename(path)
    if checksum(path, algorithm) != expected:
        raise ValueError(f"{name} cached checksum differs; refusing reuse")
    return path


def extract(archive, destination):
    if destination.exists():
        raise ValueError(f"refusing stale extracted tools: {destination}")
    destination.mkdir(parents=True)
    with tarfile.open(archive, "r:*") as tar:
        tar.extractall(destination, filter="data")


def tree_hashes():
    return {str(p.relative_to(TOOLS)): digest(p) for p in sorted(TOOLS.rglob("*")) if p.is_file()}


def runtime_env():
    # Never inherit GH_TOKEN, credentials or model-provider variables in servers.
    return {"PATH": os.environ["PATH"], "LANG": "C", "LC_ALL": "C",
            "DOTNET_ROOT": str(TOOLS / "dotnet"), "DOTNET_ROOT_X64": str(TOOLS / "dotnet"),
            "DOTNET_MULTILEVEL_LOOKUP": "0", "DOTNET_BUNDLE_EXTRACT_BASE_DIR": str(WORK / "dotnet-bundle")}


def prepare():
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("Native builds/benchmark preparation run in Actions only")
    if platform.system() != "Linux" or platform.machine() != "x86_64" or len(os.sched_getaffinity(0)) < 4:
        raise RuntimeError("Daily release tools require Linux x64 and at least four allowed logical CPUs")
    native = build()
    for name in ("dragonfly", "garnet", "dotnet"):
        extract(download(PINS[name], name), TOOLS / name)
    dragonflies = [p for p in (TOOLS / "dragonfly").iterdir() if p.is_file() and p.name.startswith("dragonfly")]
    if len(dragonflies) != 1:
        raise ValueError("ambiguous Dragonfly archive executable")
    binaries = {name: str(binary(name).resolve()) for name in ("rillcache", "redis", "valkey")}
    binaries.update(dragonfly=str(dragonflies[0].resolve()),
                    garnet=str((TOOLS / "garnet/net10.0/GarnetServer").resolve()),
                    generator=str((source("valkey") / "src/valkey-benchmark").resolve()))
    for name in ("dragonfly", "garnet"):
        license_path = download(PINS[name], name + "-license", license_file=True)
        (BUILD / "licenses").mkdir(parents=True, exist_ok=True)
        shutil.copy2(license_path, BUILD / "licenses" / f"{name}.txt")
        checked([binaries[name], "--version"], BUILD / f"version-{name}", timeout=30, env=runtime_env())
        version = (BUILD / f"version-{name}/stdout.log").read_text()
        if PINS[name]["version"] not in version:
            raise ValueError(f"{name} version differs from pin")
        if name == "garnet" and PINS[name]["revision"] not in version:
            raise ValueError("Garnet release source revision differs")
    checked([binaries["generator"], "--version"], BUILD / "generator-version", env=runtime_env())
    checked([str(TOOLS / "dotnet/dotnet"), "--list-runtimes"], BUILD / "dotnet-runtime", env=runtime_env())
    if f"Microsoft.NETCore.App {PINS['dotnet']['version']} " not in (BUILD / "dotnet-runtime/stdout.log").read_text():
        raise ValueError("required private .NET runtime missing")
    metadata = {"schema": 1, "source": native["built_git_head"], "native_build": native, "tool_pins": PINS,
                "binaries": binaries, "binary_sha256": {name: digest(path) for name, path in binaries.items()},
                "comparison_generator": {"upstream": "valkey", "source_pin": native["pins"]["upstreams"]["valkey"]},
                "runtime_files_sha256": tree_hashes()}
    save(WORK / "daily-build.json", metadata)
    save(BUILD / "build.json", metadata)
    return metadata


if __name__ == "__main__":
    try:
        prepare()
    except BaseException as error:
        save(BUILD / "failure.json", {"complete": False, "error": str(error)})
        raise
