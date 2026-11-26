#!/usr/bin/env python3
"""Generate the pinned upstream command inventory used by the C5 matrix.

The matrix compares the command names reported by the tested Rillcache head with
the names reported by the repository-pinned Redis and Valkey servers. Those two
references are only available after building the pinned source archives
(`make competitors`), so this generator runs on demand and its output is
committed as `tests/fixtures/upstream-command-inventory.tsv`. The ordinary
offline check reads that committed file; it never needs the reference servers.

Standard library only. Run it after changing `benchmarks/targets.mk` pins:

    make competitors
    python3 scripts/upstream_command_inventory.py \\
        --redis-server target/competitors/redis-8.10.2/src/redis-server \\
        --valkey-server target/competitors/valkey-9.1.2/src/valkey-server \\
        --redis-archive target/competitors/redis-8.10.2.tar.gz \\
        --valkey-archive target/competitors/valkey-9.1.2.tar.gz
"""

from __future__ import annotations

import argparse
import hashlib
import os
import re
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGETS = Path("benchmarks/targets.mk")
FIXTURE = Path("tests/fixtures/upstream-command-inventory.tsv")
REFERENCE_COUNT = 2

PIN = re.compile(r"^(\w+_VERSION|REDIS_SHA256|VALKEY_SHA256)\s*:=\s*(\S+)$", re.M)


def pins(root: Path = ROOT) -> dict[str, str]:
    """Return the pinned reference versions and archive checksums."""
    text = (root / TARGETS).read_text(encoding="utf-8")
    found = dict(PIN.findall(text))
    required = {"REDIS_VERSION", "REDIS_SHA256", "VALKEY_VERSION", "VALKEY_SHA256"}
    missing = required - found.keys()
    if missing:
        raise ValueError(f"{TARGETS} lacks pins: {sorted(missing)}")
    return found


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


class Connection:
    """Minimal RESP2 client used only for `COMMAND` discovery."""

    def __init__(self, address: tuple[str, int]) -> None:
        self.socket = socket.create_connection(address, timeout=30)
        self.buffer = b""

    def close(self) -> None:
        self.socket.close()

    def _fill(self) -> None:
        block = self.socket.recv(1 << 16)
        if not block:
            raise EOFError("reference server closed the connection")
        self.buffer += block

    def _line(self) -> bytes:
        while b"\r\n" not in self.buffer:
            self._fill()
        line, self.buffer = self.buffer.split(b"\r\n", 1)
        return line

    def _read(self, count: int) -> bytes:
        while len(self.buffer) < count:
            self._fill()
        body, self.buffer = self.buffer[:count], self.buffer[count:]
        return body

    def reply(self) -> tuple[str, object]:
        line = self._line()
        kind, rest = line[:1], line[1:]
        if kind == b"+":
            return ("status", rest.decode("utf-8", "replace"))
        if kind == b"-":
            return ("error", rest.decode("utf-8", "replace"))
        if kind == b":":
            return ("integer", int(rest))
        if kind == b"$":
            length = int(rest)
            if length < 0:
                return ("null", None)
            body = self._read(length)
            self._read(2)
            return ("bulk", body.decode("utf-8", "replace"))
        if kind == b"*":
            count = int(rest)
            if count < 0:
                return ("null", None)
            return ("array", [self.reply() for _ in range(count)])
        raise ValueError(f"unexpected RESP prefix {kind!r}")

    def request(self, *args: str) -> tuple[str, object]:
        payload = [b"*%d\r\n" % len(args)]
        for argument in args:
            encoded = argument.encode()
            payload.append(b"$%d\r\n%s\r\n" % (len(encoded), encoded))
        self.socket.sendall(b"".join(payload))
        return self.reply()


def flatten(reply: tuple[str, object]) -> list[tuple[str, int]]:
    """Flatten `COMMAND` output, including one nesting level of subcommands."""
    kind, entries = reply
    if kind != "array" or entries is None:
        raise ValueError("COMMAND must return an array")
    rows: list[tuple[str, int]] = []
    for entry in entries:
        info_kind, info = entry
        if info_kind != "array" or info is None or len(info) < 2:
            raise ValueError("COMMAND entries must be arrays with name and arity")
        name = info[0][1]
        arity = int(info[1][1])
        if not isinstance(name, str):
            raise ValueError("COMMAND entry name must be a bulk string")
        rows.append((name, arity))
        subcommands = info[9] if len(info) > 9 else ("null", None)
        if subcommands[0] == "array" and subcommands[1]:
            for subcommand in subcommands[1]:
                sub_info = subcommand[1]
                if sub_info is None or len(sub_info) < 2:
                    raise ValueError("subcommand entries must carry name and arity")
                rows.append((sub_info[0][1], int(sub_info[1][1])))
    return rows


def collect(server: Path) -> list[tuple[str, int]]:
    """Start one pinned reference on loopback and read its `COMMAND` table."""
    probe = socket.socket()
    probe.bind(("127.0.0.1", 0))
    port = probe.getsockname()[1]
    probe.close()
    directory = tempfile.mkdtemp(prefix="rillcache-inventory-")
    process = subprocess.Popen(
        [
            str(server),
            "--port",
            str(port),
            "--bind",
            "127.0.0.1",
            "--dir",
            directory,
            "--save",
            "",
            "--appendonly",
            "no",
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    connection = None
    deadline = time.monotonic() + 30
    try:
        while connection is None:
            if process.poll() is not None:
                raise SystemExit(f"{server} exited before accepting connections")
            if time.monotonic() > deadline:
                raise SystemExit(f"{server} did not accept connections within 30s")
            try:
                connection = Connection(("127.0.0.1", port))
            except OSError:
                time.sleep(0.05)
        return flatten(connection.request("COMMAND"))
    finally:
        if connection is not None:
            connection.close()
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def render(
    redis: list[tuple[str, int]],
    valkey: list[tuple[str, int]],
    pinned: dict[str, str],
) -> str:
    """Render the committed fixture from both reference tables."""
    redis_rows = dict(redis)
    valkey_rows = dict(valkey)
    if len(redis_rows) != len(redis) or len(valkey_rows) != len(valkey):
        raise SystemExit("a reference reported a duplicate command name")
    union = sorted(redis_rows.keys() | valkey_rows.keys())
    lines = [
        "# Upstream command inventory for the C5 differential command matrix.",
        "#",
        "# Generated by scripts/upstream_command_inventory.py from the servers built",
        "# from the repository-pinned source archives. Do not edit by hand; regenerate",
        "# after changing benchmarks/targets.mk and re-run the offline check.",
        "#",
        f"# redis-version: {pinned['REDIS_VERSION']}",
        "# redis-source: https://github.com/redis/redis/archive/refs/tags/"
        f"{pinned['REDIS_VERSION']}.tar.gz",
        f"# redis-tarball-sha256: {pinned['REDIS_SHA256']}",
        f"# redis-command-rows: {len(redis_rows)}",
        f"# valkey-version: {pinned['VALKEY_VERSION']}",
        "# valkey-source: https://github.com/valkey-io/valkey/archive/refs/tags/"
        f"{pinned['VALKEY_VERSION']}.tar.gz",
        f"# valkey-tarball-sha256: {pinned['VALKEY_SHA256']}",
        f"# valkey-command-rows: {len(valkey_rows)}",
        f"# union-rows: {len(union)}",
        "#",
        "# Columns, tab separated: name, reference arity (positive = fixed token",
        "# count, negative = minimum token count). A zero arity means that reference",
        "# does not expose the name.",
        "#",
        "# name\tredis-arity\tvalkey-arity",
    ]
    for name in union:
        lines.append(
            f"{name}\t{redis_rows.get(name, 0)}\t{valkey_rows.get(name, 0)}"
        )
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None, root: Path = ROOT) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--redis-server", type=Path, required=True)
    parser.add_argument("--valkey-server", type=Path, required=True)
    parser.add_argument("--redis-archive", type=Path)
    parser.add_argument("--valkey-archive", type=Path)
    parser.add_argument("--out", type=Path, default=root / FIXTURE)
    arguments = parser.parse_args(argv)

    pinned = pins(root)
    for archive, expected, label in (
        (arguments.redis_archive, pinned["REDIS_SHA256"], "Redis"),
        (arguments.valkey_archive, pinned["VALKEY_SHA256"], "Valkey"),
    ):
        if archive is None:
            continue
        actual = sha256(archive)
        if actual != expected:
            raise SystemExit(
                f"{label} archive {archive} has sha256 {actual}, expected {expected}"
            )
    redis = collect(arguments.redis_server)
    valkey = collect(arguments.valkey_server)
    text = render(redis, valkey, pinned)
    arguments.out.parent.mkdir(parents=True, exist_ok=True)
    arguments.out.write_text(text, encoding="utf-8")
    union = len({name for name, _ in redis} | {name for name, _ in valkey})
    print(
        f"wrote {arguments.out}: redis={len(redis)} valkey={len(valkey)}"
        f" union={union}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
