#!/usr/bin/env python3
"""Safe isolated lifecycle/reconnect scenario; no host data or fault injection."""
from pathlib import Path
import os
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("RILLCACHE_BINARY", ROOT / "target/release/rillcache"))
EVIDENCE = Path(os.environ.get("RILLCACHE_RUNBOOK_EVIDENCE", ROOT / "target/benchmarks/standalone-runbook"))
EVIDENCE.mkdir(parents=True, exist_ok=True)
LOG = EVIDENCE / "incident.log"


def read_line(sock):
    line = bytearray()
    while not line.endswith(b"\r\n"):
        part = sock.recv(1)
        if not part:
            raise AssertionError("server closed before a complete RESP line")
        line.extend(part)
    return bytes(line)


def read_reply(sock):
    prefix = sock.recv(1)
    if prefix in (b"+", b"-", b":"):
        return prefix + read_line(sock)
    if prefix == b"$":
        size = int(read_line(sock)[:-2])
        if size < 0:
            return b"$-1\r\n"
        body = bytearray()
        while len(body) < size + 2:
            body.extend(sock.recv(size + 2 - len(body)))
        return prefix + str(size).encode() + b"\r\n" + bytes(body)
    if prefix == b"*":
        count = int(read_line(sock)[:-2])
        return prefix + str(count).encode() + b"\r\n" + b"".join(read_reply(sock) for _ in range(count))
    raise AssertionError(f"unexpected RESP prefix: {prefix!r}")


def command(port, *args):
    with socket.create_connection(("127.0.0.1", port), timeout=2) as sock:
        sock.settimeout(2)
        sock.sendall(b"*" + str(len(args)).encode() + b"\r\n" + b"".join(
            b"$" + str(len(arg)).encode() + b"\r\n" + arg + b"\r\n" for arg in args))
        return read_reply(sock)


def wait_ready(port, process):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(f"server exited early with {process.returncode}")
        try:
            if command(port, b"PING") == b"+PONG\r\n":
                return
        except (OSError, TimeoutError):
            time.sleep(.05)
    raise AssertionError("server did not become ready")


def main():
    if not BINARY.is_file():
        raise SystemExit(f"missing tested binary: {BINARY}")
    with tempfile.TemporaryDirectory(prefix="rillcache-runbook-") as data_dir, LOG.open("wb") as log:
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        args = [str(BINARY), "--bind", f"127.0.0.1:{port}", "--appendonly", "yes",
                "--appendfsync", "always", "--dir", data_dir]
        process = None
        try:
            process = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT)
            wait_ready(port, process)
            role = command(port, b"ROLE")
            info = command(port, b"INFO", b"replication")
            assert b"master" in role and b"0" in role, role
            assert b"role:master" in info and b"connected_slaves:0" in info, info
            assert command(port, b"SET", b"runbook-key", b"retained") == b"+OK\r\n"
            process.terminate()
            process.wait(timeout=25)
            process = None
            # A fresh connection after orderly stop must fail; this is not a partition or host-loss test.
            with socket.socket() as probe:
                probe.settimeout(1)
                assert probe.connect_ex(("127.0.0.1", port)) != 0, "listener survived shutdown"
            process = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT)
            wait_ready(port, process)
            assert command(port, b"GET", b"runbook-key") == b"$8\r\nretained\r\n"
            assert command(port, b"ROLE").startswith(b"*3\r\n")
            process.terminate()
            process.wait(timeout=25)
            process = None
            print(f"standalone lifecycle/reconnect scenario passed; logs: {LOG}")
        except BaseException:
            log.flush()
            print(f"scenario failed; server output retained at {LOG}")
            raise
        finally:
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=25)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


if __name__ == "__main__":
    main()
