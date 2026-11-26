#!/usr/bin/env python3
"""Render or check the C5 command and subcommand differential matrix.

The matrix records, for every command name reported by the repository-pinned
Redis and Valkey servers (see `tests/fixtures/upstream-command-inventory.tsv`),
what the tested Rillcache head advertises through `COMMAND` and, for names it
does not advertise, what a minimal-arity request actually returns. It is
evidence, not a readiness or compatibility claim.

`tests/command_matrix.rs` runs this check against the tested head in the
default, fixed-one-worker and four-worker modes, so a command-surface change
that is not reflected here fails `make ci`. Regenerate after an intentional
surface change:

    python3 scripts/command_matrix.py --spawn target/debug/rillcache --write

Standard library only.
"""

from __future__ import annotations

import argparse
import re
import socket
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = Path("tests/fixtures/upstream-command-inventory.tsv")
MATRIX = Path("docs/production-readiness/command-matrix.md")
TARGETS = Path("benchmarks/targets.mk")
UNSUPPORTED = Path("docs/unsupported.md")
PROFILE = Path("docs/production-profile.md")
NAME = re.compile(r"^[A-Za-z][A-Za-z0-9|_.-]*$")
PIN = re.compile(r"^(\w+_VERSION|REDIS_SHA256|VALKEY_SHA256)\s*:=\s*(\S+)$", re.M)
NAMED_EXCLUSION = re.compile(
    r"(not supported|is unsupported|not implemented|support disabled)",
    re.IGNORECASE,
)
REJECTED_SUBCOMMAND = re.compile(
    r"(unsupported .*subcommand|unknown .*subcommand|unknown command)", re.IGNORECASE
)
UNKNOWN_COMMAND = re.compile(r"^(err )?unknown command$", re.IGNORECASE)
REPLY_TIMEOUT = 5.0


class Connection:
    """Minimal RESP2 client; the matrix only needs commands, statuses and errors."""

    def __init__(self, address: tuple[str, int]) -> None:
        self.socket = socket.create_connection(address, timeout=REPLY_TIMEOUT)
        self.socket.settimeout(REPLY_TIMEOUT)
        self.buffer = b""

    def close(self) -> None:
        self.socket.close()

    def _fill(self) -> None:
        block = self.socket.recv(1 << 16)
        if not block:
            raise EOFError("the tested head closed the connection during the matrix run")
        self.buffer += block

    def _exact(self, count: int) -> bytes:
        while len(self.buffer) < count:
            self._fill()
        body, self.buffer = self.buffer[:count], self.buffer[count:]
        return body

    def _line(self) -> bytes:
        while b"\r\n" not in self.buffer:
            self._fill()
        line, self.buffer = self.buffer.split(b"\r\n", 1)
        return line

    def reply(self) -> tuple[str, object]:
        line = self._line()
        prefix, rest = line[:1], line[1:]
        if prefix == b"+":
            return ("status", rest.decode("utf-8", "replace"))
        if prefix == b"-":
            return ("error", rest.decode("utf-8", "replace"))
        if prefix == b":":
            return ("integer", int(rest))
        if prefix == b"$":
            length = int(rest)
            if length < 0:
                return ("null", None)
            body = self._exact(length)
            self._exact(2)
            return ("bulk", body)
        if prefix == b"*":
            count = int(rest)
            if count < 0:
                return ("null", None)
            return ("array", [self.reply() for _ in range(count)])
        raise ValueError(f"unexpected RESP prefix {prefix!r}")

    def request(self, *args: str) -> tuple[str, object]:
        payload = [b"*%d\r\n" % len(args)]
        for argument in args:
            encoded = argument.encode()
            payload.append(b"$%d\r\n%s\r\n" % (len(encoded), encoded))
        self.socket.sendall(b"".join(payload))
        return self.reply()


def free_port() -> int:
    probe = socket.socket()
    probe.bind(("127.0.0.1", 0))
    port = probe.getsockname()[1]
    probe.close()
    return port


class Target:
    """An already running server (`--addr`) or one spawned for the run."""

    def __init__(self, addr: str | None, spawn: str | None, threads: int | None) -> None:
        self.process: subprocess.Popen[bytes] | None = None
        self.directory: str | None = None
        if addr is not None:
            host, _, port = addr.rpartition(":")
            self.address = (host or "127.0.0.1", int(port))
            return
        assert spawn is not None
        port = free_port()
        self.address = ("127.0.0.1", port)
        self.directory = tempfile.mkdtemp(prefix="rillcache-matrix-")
        command = [spawn, "--bind", f"127.0.0.1:{port}"]
        if threads is not None:
            command.extend(["--threads", str(threads)])
        self.process = subprocess.Popen(
            command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )

    def connect(self) -> Connection:
        deadline = time.monotonic() + 30
        while True:
            try:
                return Connection(self.address)
            except OSError as error:
                if self.process is not None and self.process.poll() is not None:
                    raise SystemExit(
                        f"the tested head exited before accepting connections: {error}"
                    )
                if time.monotonic() > deadline:
                    raise SystemExit("the tested head did not accept a connection")
                time.sleep(0.05)

    def close(self) -> None:
        if self.process is not None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()


@dataclass(frozen=True)
class Row:
    name: str
    redis: int
    valkey: int


def load_fixture(path: Path, root: Path) -> tuple[list[Row], dict[str, str]]:
    """Load and validate the committed inventory; return rows and its header."""
    text = path.read_text(encoding="utf-8")
    header: dict[str, str] = {}
    rows: list[Row] = []
    for number, line in enumerate(text.splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 3:
            raise SystemExit(f"{path}:{number}: expected three tab-separated fields")
        name, redis, valkey = fields
        if name != name.strip() or not NAME.fullmatch(name):
            raise SystemExit(f"{path}:{number}: invalid command name {name!r}")
        rows.append(Row(name, int(redis), int(valkey)))
    for line in text.splitlines():
        match = re.match(r"^#\s*([\w-]+):\s*(\S+)$", line)
        if match:
            header[match.group(1)] = match.group(2)
    names = [row.name for row in rows]
    if names != sorted(names) or len(set(names)) != len(names):
        raise SystemExit(f"{path}: rows must be unique and sorted by name")
    if any(row.redis == 0 and row.valkey == 0 for row in rows):
        raise SystemExit(f"{path}: every row must exist in at least one reference")
    pinned = dict(PIN.findall((root / TARGETS).read_text(encoding="utf-8")))
    expected = {
        "redis-version": pinned["REDIS_VERSION"],
        "redis-tarball-sha256": pinned["REDIS_SHA256"],
        "valkey-version": pinned["VALKEY_VERSION"],
        "valkey-tarball-sha256": pinned["VALKEY_SHA256"],
    }
    for key, value in expected.items():
        if header.get(key) != value:
            raise SystemExit(
                f"{path}: {key} is {header.get(key)!r}, expected the pinned {value!r};"
                " regenerate with scripts/upstream_command_inventory.py"
            )
    counts = {
        "union-rows": len(rows),
        "redis-command-rows": sum(1 for row in rows if row.redis != 0),
        "valkey-command-rows": sum(1 for row in rows if row.valkey != 0),
    }
    for key, value in counts.items():
        if int(header.get(key, "0")) != value:
            raise SystemExit(f"{path}: {key} does not match the counted {value}")
    return rows, header


def registry(connection: Connection) -> dict[str, tuple[int, tuple[str, ...]]]:
    """Return the advertised command names with arity and flags."""
    kind, entries = connection.request("COMMAND")
    if kind != "array" or entries is None:
        raise SystemExit("COMMAND must return an array")
    advertised: dict[str, tuple[int, tuple[str, ...]]] = {}
    for entry in entries:
        info_kind, info = entry
        if info_kind != "array" or info is None or len(info) < 3:
            raise SystemExit("every COMMAND entry must carry name, arity and flags")
        name = info[0][1]
        if not isinstance(name, bytes):
            raise SystemExit("COMMAND entry names must be bulk strings")
        flags: list[str] = []
        flag_kind, flag_entries = info[2]
        if flag_kind != "array" or flag_entries is None:
            raise SystemExit("COMMAND entry flags must be an array")
        for flag in flag_entries:
            if flag[0] not in ("status", "bulk"):
                raise SystemExit("COMMAND entry flags must be simple or bulk strings")
            value = flag[1]
            flags.append(
                value.decode() if isinstance(value, bytes) else str(value)
            )
        advertised[name.decode()] = (int(info[1][1]), tuple(flags))
    return advertised


def probe_arguments(name: str, reference_arity: int) -> list[str]:
    """Build a minimal-arity request for a name the tested head does not advertise."""
    tokens = name.split("|")
    minimum = abs(reference_arity)
    return tokens + ["x"] * max(0, minimum - len(tokens))


@dataclass(frozen=True)
class Observation:
    name: str
    reply: str
    message: str
    klass: str


def observe(connection: Connection, name: str, arity: int) -> Observation:
    kind, value = connection.request(*probe_arguments(name, arity))
    if kind == "error":
        message = str(value)
        if UNKNOWN_COMMAND.fullmatch(message.strip()):
            klass = "unknown-command"
        elif NAMED_EXCLUSION.search(message):
            klass = "named-exclusion"
        elif REJECTED_SUBCOMMAND.search(message):
            klass = "rejected-subcommand"
        else:
            klass = "rejected-request"
        return Observation(name, "error", message, klass)
    # A definite non-error reply. Record only its RESP type: payloads can carry
    # volatile values (ids, counts, random passwords) and are not evidence.
    return Observation(name, kind, "", f"answered-{kind}")


def phrase(name: str) -> re.Pattern[str]:
    tokens = name.replace("|", " ").split()
    return re.compile(
        r"\b" + r"\s+".join(re.escape(token) for token in tokens) + r"\b", re.I
    )


def doc_mentions(root: Path, name: str) -> bool:
    """True when a profile document records the name or its parent container.

    A subcommand of an excluded container is covered by the container-level
    exclusion statement, so the parent name is accepted as its profile entry.
    """
    wanted = [phrase(name)]
    if "|" in name:
        wanted.append(phrase(name.split("|")[0]))
    for path in (UNSUPPORTED, PROFILE):
        candidate = root / path
        if not candidate.is_file():
            continue
        text = candidate.read_text(encoding="utf-8")
        if any(pattern.search(text) for pattern in wanted):
            return True
    return False


def cell(text: str) -> str:
    return text.replace("\\", "\\\\").replace("|", "\\|")


def arity_cell(value: int) -> str:
    return "absent" if value == 0 else str(value)


def render(
    rows: list[Row],
    advertised: dict[str, tuple[int, tuple[str, ...]]],
    observations: dict[str, Observation],
    pinned: dict[str, str],
) -> str:
    by_name = {row.name: row for row in rows}
    upstream = set(by_name)
    registered = set(advertised)
    missing_from_upstream = sorted(registered - upstream)
    absent = sorted(upstream - registered)
    absent_top = [name for name in absent if "|" not in name]
    absent_sub = [name for name in absent if "|" in name]
    container_sub = [
        name for name in absent_sub if name.split("|")[0] in registered
    ]
    excluded_sub = [
        name for name in absent_sub if name.split("|")[0] not in registered
    ]
    top_unknown = [n for n in absent_top if observations[n].klass == "unknown-command"]
    top_named = [n for n in absent_top if observations[n].klass == "named-exclusion"]
    top_other = [
        n
        for n in absent_top
        if observations[n].klass not in ("unknown-command", "named-exclusion")
    ]
    answered = [n for n in container_sub if observations[n].reply != "error"]
    named_sub = [n for n in container_sub if observations[n].klass == "named-exclusion"]
    rejected_sub = [
        n for n in container_sub if observations[n].klass == "rejected-subcommand"
    ]
    other_sub = [
        n for n in container_sub if observations[n].klass == "rejected-request"
    ]
    style = [
        "# Command and subcommand differential matrix",
        "",
        "**Status: generated compatibility evidence, not a readiness or",
        "compatibility claim.** Open items stay in the canonical",
        "[production-readiness gaps](../production-readiness-gaps.md); this file is the",
        "C5 command-surface record and the profile entry for excluded names.",
        "",
        "The tested head advertises the command names and metadata that `COMMAND`",
        "returns. For every name reported by a pinned reference and not advertised",
        "here, the matrix records the actual wire reply to a minimal-arity request.",
        "`tests/command_matrix.rs` fails when this file and the tested head disagree,",
        "when an absent command is silently accepted, or when a named exclusion is",
        "not recorded in [unsupported surfaces](../unsupported.md) or the",
        "[production profile](../production-profile.md).",
        "",
        "## Method",
        "",
        f"- References: Redis `{pinned['REDIS_VERSION']}` and Valkey",
        f"  `{pinned['VALKEY_VERSION']}`, both pinned in",
        "  [`benchmarks/targets.mk`](../../benchmarks/targets.mk) and queried through",
        "  their own `COMMAND` reply. The committed inventory is",
        "  [`tests/fixtures/upstream-command-inventory.tsv`](../../tests/fixtures/upstream-command-inventory.tsv),",
        "  generated by `scripts/upstream_command_inventory.py` from the verified",
        "  source archives.",
        "- The candidate is the tested head in its default, fixed-one-worker and",
        "  four-worker modes; all three modes must produce this file byte for byte.",
        "- A row classified `advertised` is returned by `COMMAND`. Every other row is",
        "  probed once with the reference minimum arity (`name` plus `x` placeholders)",
        "  and the exact error text is recorded. Successes record the RESP reply type",
        "  only, because payloads can carry volatile values.",
        "- Rows are grouped by their parent container. Subcommands of containers the",
        "  head does not advertise at all are covered by that container's own row and",
        "  are listed separately.",
        "- Area documents, not this matrix, define option-level support. An",
        "  `answered` subcommand row proves the container accepted the request, not",
        "  that every documented option or edge case is implemented. Some probes",
        "  (`acl|setuser`, `script|flush`, `function|flush`) do change state, so run",
        "  the check only against a disposable instance or the spawned binary, never",
        "  against a deployment that holds data or production credentials.",
        "- The checked head must be able to serve the probes: a timeout, a closed",
        "  connection or a missing reply fails the check instead of being recorded.",
        "",
        "Regenerate after an intentional surface change:",
        "",
        "```text",
        "python3 scripts/command_matrix.py --spawn target/debug/rillcache --write",
        "```",
        "",
        "## Summary",
        "",
        "| Row set | Count |",
        "| --- | --- |",
        f"| Pinned upstream names | {len(rows)} |",
        f"| Advertised by the tested head | {len(advertised)} |",
        f"| Advertised and absent from both references | {len(missing_from_upstream)} |",
        f"| Upstream top-level commands not advertised | {len(absent_top)} |",
        f"| ... returning the standard unknown-command error | {len(top_unknown)} |",
        f"| ... returning a named capability error | {len(top_named)} |",
        f"| ... returning another argument error | {len(top_other)} |",
        f"| Subcommands of advertised containers | {len(container_sub)} |",
        f"| ... answered by the container | {len(answered)} |",
        f"| ... rejected with a named capability error | {len(named_sub)} |",
        f"| ... rejected as an unknown/unsupported subcommand | {len(rejected_sub)} |",
        f"| ... rejected by argument validation | {len(other_sub)} |",
        f"| Subcommands of excluded containers | {len(excluded_sub)} |",
        "",
        "## Advertised commands",
        "",
        f"All {len(advertised)} names returned by `COMMAND`, with the advertised",
        "metadata and whether each pinned reference also reports the name.",
        "",
        "| Command | Arity | Flags | Redis | Valkey |",
        "| --- | --- | --- | --- | --- |",
    ]
    for name in sorted(advertised):
        arity, flags = advertised[name]
        flags_cell = ", ".join(flags) if flags else "-"
        redis = by_name[name].redis if name in by_name else 0
        valkey = by_name[name].valkey if name in by_name else 0
        style.append(
            f"| `{cell(name)}` | {arity} | {cell(flags_cell)} |"
            f" {arity_cell(redis)} | {arity_cell(valkey)} |"
        )
    style += [
        "",
        "## Upstream top-level commands not advertised",
        "",
        "Each row is a command the tested head does not return from `COMMAND`. The",
        "reply column is the exact wire error for a minimal-arity request, so every",
        "absent command fails explicitly instead of silently succeeding or hanging.",
        "",
        "| Command | Redis | Valkey | Reply |",
        "| --- | --- | --- | --- |",
    ]
    for name in absent_top:
        row = by_name[name]
        style.append(
            f"| `{cell(name)}` | {arity_cell(row.redis)} | {arity_cell(row.valkey)} |"
            f" `{cell(observations[name].message)}` |"
        )
    style += [
        "",
        "## Subcommands of advertised containers",
        "",
        "The container is advertised, so each of these rows was probed individually.",
        "`answered` rows return a definite reply and remain governed by their area",
        "document; every other row returns the recorded error.",
        "",
    ]
    for parent in sorted({name.split("|")[0] for name in container_sub}):
        group = [name for name in container_sub if name.split("|")[0] == parent]
        style += [
            f"### `{parent.upper()}`",
            "",
            "| Subcommand | Redis | Valkey | Reply |",
            "| --- | --- | --- | --- |",
        ]
        for name in group:
            row = by_name[name]
            observation = observations[name]
            shown = (
                f"`{cell(observation.message)}`"
                if observation.reply == "error"
                else f"answered: `{observation.reply}`"
            )
            style.append(
                f"| `{cell(name)}` | {arity_cell(row.redis)} |"
                f" {arity_cell(row.valkey)} | {shown} |"
            )
        style.append("")
    style += [
        "## Subcommands of excluded containers",
        "",
        "These names belong to containers the head does not advertise at all, so the",
        "container-level row already covers them; they are listed for completeness.",
        "",
        "| Subcommand | Redis | Valkey | Reply |",
        "| --- | --- | --- | --- |",
    ]
    for name in excluded_sub:
        row = by_name[name]
        style.append(
            f"| `{cell(name)}` | {arity_cell(row.redis)} | {arity_cell(row.valkey)} |"
            f" `{cell(observations[name].message)}` |"
        )
    style += [
        "",
        "## Names reported only by the tested head",
        "",
        (
            "None: every advertised name also exists in at least one pinned reference."
            if not missing_from_upstream
            else "| Command | Notes |\n| --- | --- |\n"
            + "\n".join(
                f"| `{cell(name)}` | not reported by either pinned reference |"
                for name in missing_from_upstream
            )
        ),
        "",
    ]
    return "\n".join(style)


def verify_observations(
    root: Path,
    rows: list[Row],
    advertised: dict[str, tuple[int, tuple[str, ...]]],
    observations: dict[str, Observation],
) -> list[str]:
    """Return wire-independent invariant violations for the recorded surface."""
    errors: list[str] = []
    by_name = {row.name: row for row in rows}
    upstream = set(by_name)
    registered = set(advertised)
    if registered - upstream:
        errors.append(
            "advertised commands missing from the pinned inventory: "
            + ", ".join(sorted(registered - upstream))
        )
    absent = sorted(upstream - registered)
    for name in absent:
        if name not in observations:
            errors.append(f"{name}: not probed")
    absent_top = [name for name in absent if "|" not in name]
    for name in absent_top:
        observation = observations.get(name)
        if observation is None:
            continue
        if observation.reply != "error":
            errors.append(
                f"{name}: absent command answered with {observation.reply};"
                " advertise it or reject it explicitly"
            )
        elif observation.klass == "named-exclusion" and not doc_mentions(root, name):
            errors.append(
                f"{name}: named capability error is not recorded in"
                f" {UNSUPPORTED} or {PROFILE}"
            )
    for name in absent:
        observation = observations.get(name)
        if observation is None or name in absent_top:
            continue
        if observation.klass == "named-exclusion" and not doc_mentions(root, name):
            errors.append(
                f"{name}: named capability error is not recorded in"
                f" {UNSUPPORTED} or {PROFILE}"
            )
    return errors


def verify_connection(checked: Connection, absent_top: list[str]) -> list[str]:
    """Return invariant violations that need a live connection."""
    errors: list[str] = []
    # `COMMAND INFO` must not describe a name the registry does not advertise.
    kind, info = checked.request("COMMAND", "INFO", *absent_top)
    if kind != "array" or info is None or len(info) != len(absent_top):
        errors.append("COMMAND INFO did not return one entry per absent command")
    else:
        for name, entry in zip(absent_top, info):
            if entry[1] is not None:
                errors.append(
                    f"{name}: absent from COMMAND but described by COMMAND INFO"
                )
    # The probe pass must leave an ordinary session usable and the keyspace empty.
    if checked.request("PING") != ("status", "PONG"):
        errors.append("the probe pass left the connection unusable")
    kind, size = checked.request("DBSIZE")
    if kind != "integer" or int(size) != 0:
        errors.append(f"the probe pass created keyspace data: {size!r}")
    return errors


def main(argv: list[str] | None = None, root: Path = ROOT) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, default=root / FIXTURE)
    parser.add_argument("--matrix", type=Path, default=root / MATRIX)
    parser.add_argument("--addr", help="already running HOST:PORT to check")
    parser.add_argument("--spawn", help="server binary to start on loopback")
    parser.add_argument("--threads", type=int, help="worker count for --spawn")
    parser.add_argument(
        "--write",
        action="store_true",
        help="rewrite the matrix instead of failing on a mismatch",
    )
    arguments = parser.parse_args(argv)
    if (arguments.addr is None) == (arguments.spawn is None):
        parser.error("pass exactly one of --addr or --spawn")

    rows, header = load_fixture(arguments.fixture, root)
    pinned = dict(PIN.findall((root / TARGETS).read_text(encoding="utf-8")))
    target = Target(arguments.addr, arguments.spawn, arguments.threads)
    try:
        connection = target.connect()
        advertised = registry(connection)
        registered = set(advertised)
        absent = sorted({row.name for row in rows} - registered)
        observations: dict[str, Observation] = {}
        for name in absent:
            row = next(row for row in rows if row.name == name)
            observations[name] = observe(
                connection, name, row.redis or row.valkey
            )
        errors = verify_observations(root, rows, advertised, observations)
        errors += verify_connection(
            connection, [name for name in absent if "|" not in name]
        )
        rendered = render(rows, advertised, observations, pinned)
    finally:
        target.close()

    if errors:
        print("command matrix invariant violations:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    existing = arguments.matrix.read_text(encoding="utf-8") if arguments.matrix.is_file() else ""
    if arguments.write:
        arguments.matrix.parent.mkdir(parents=True, exist_ok=True)
        arguments.matrix.write_text(rendered, encoding="utf-8")
        print(f"wrote {arguments.matrix}")
        return 0
    if existing != rendered:
        print(
            f"{arguments.matrix} does not match the tested head;"
            " regenerate with scripts/command_matrix.py --spawn <binary> --write"
            " and review the diff",
            file=sys.stderr,
        )
        return 1
    print(
        f"command matrix matches {arguments.fixture.name}:"
        f" {len(advertised)} advertised, {len(observations)} absent names probed"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
