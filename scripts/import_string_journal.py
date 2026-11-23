#!/usr/bin/env python3
"""Convert a bounded offline DB 0 string command stream to a Rillcache journal."""

from __future__ import annotations

import argparse
import os
import re
import stat
import sys
import tempfile
from io import BytesIO
from pathlib import Path
from typing import BinaryIO

MAX_INPUT = 16 * 1024 * 1024
MAX_BULK = 1024 * 1024
MAX_FRAME = 2 * MAX_BULK
MAX_DEADLINE = (1 << 63) - 1


def number(raw: bytes, limit: int) -> int:
    if not re.fullmatch(rb"0|[1-9][0-9]{0,18}", raw) or (value := int(raw)) > limit:
        raise ValueError("invalid or out-of-range decimal")
    return value


def header(line: bytes, prefix: bytes, limit: int) -> int:
    if not line.startswith(prefix) or not line.endswith(b"\r\n"):
        raise ValueError("expected a complete RESP array or bulk header")
    return number(line[1:-2], limit)


def write_frame(output: BinaryIO, args: list[bytes]) -> None:
    output.write(b"*%d\r\n" % len(args))
    for item in args:
        output.write(b"$%d\r\n" % len(item))
        output.write(item)
        output.write(b"\r\n")


def convert(data: bytes, output: BinaryIO) -> int:
    source = BytesIO(data)
    count = 0
    while (line := source.readline(24)):
        start = source.tell() - len(line)
        parts = []
        for _ in range(header(line, b"*", 5)):
            length = header(source.readline(24), b"$", MAX_BULK)
            part = source.read(length)
            if len(part) != length or source.read(2) != b"\r\n":
                raise ValueError("truncated or malformed bulk argument")
            parts.append(part)
        if source.tell() - start > MAX_FRAME:
            raise ValueError("command exceeds the default frame limit")
        if start == 0 and len(parts) == 2 and parts[0].upper() == b"SELECT" and parts[1] == b"0":
            continue
        if len(parts) not in (3, 5) or parts[0].upper() != b"SET":
            raise ValueError("only DB 0 SET records are supported")
        if len(parts) == 5:
            if parts[3].upper() != b"PXAT" or number(parts[4], MAX_DEADLINE) == 0:
                raise ValueError("only positive absolute PXAT expiry is supported")
        write_frame(output, [b"SET", parts[1], parts[2]])
        if len(parts) == 5:
            write_frame(output, [b"PEXPIREAT", parts[1], parts[4]])
        count += 1
    if not count:
        raise ValueError("expected at least one SET record")
    return count


def import_file(source: Path, destination: Path) -> int:
    # The operator supplies a quiescent local source and an isolated destination.
    # Check the open descriptor, not only path metadata.
    flags = os.O_RDONLY | getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_BINARY", 0)
    with os.fdopen(os.open(source, flags), "rb") as handle:
        if not stat.S_ISREG(os.fstat(handle.fileno()).st_mode):
            raise ValueError("source must be a regular file")
        data = handle.read(MAX_INPUT + 1)
    if len(data) > MAX_INPUT:
        raise ValueError("source exceeds 16 MiB")
    # NamedTemporaryFile creates mode 0600 on Unix. On unsupported filesystems
    # link fails closed; it never replaces an existing destination.
    with tempfile.NamedTemporaryFile(dir=destination.parent, prefix=".rillcache-import-") as staged:
        count = convert(data, staged)
        staged.flush()
        os.fsync(staged.fileno())
        os.link(staged.name, destination)
    return count


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args(argv)
    try:
        count = import_file(args.source, args.destination)
    except ValueError as error:
        print(f"import refused: {error}", file=sys.stderr)
        return 2
    except OSError as error:
        print(f"import refused: file operation failed (errno {error.errno})", file=sys.stderr)
        return 2
    print(f"Published a legacy Rillcache journal with {count} SET records. Verify in isolation before use.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
