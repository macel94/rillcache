#!/usr/bin/env python3
"""Run pinned redis-py against managed Rillcache servers (optional external tooling)."""
from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

CLIENT_VERSION = "5.2.1"
PASSWORD = "redis-py-test-secret"
USERNAME = "redis_py"
STARTUP_TIMEOUT = 8.0
COMMAND_TIMEOUT = 2.0


def require_client():
    try:
        import redis
    except ImportError as error:
        raise RuntimeError("redis-py is required; install redis==5.2.1") from error
    if redis.__version__ != CLIENT_VERSION:
        raise RuntimeError(f"redis-py {CLIENT_VERSION} required, found {redis.__version__}")
    return redis


def check(condition: bool, description: str) -> None:
    if not condition:
        raise RuntimeError(f"conformance check failed: {description}")


def reserve_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


class ManagedServer:
    """Owns one subprocess and log; every startup/exit path reaps the child."""

    def __init__(self, binary: Path, mode: str, directory: Path):
        self.binary = binary
        self.mode = mode
        self.directory = directory
        self.port = reserve_port()
        self.process: subprocess.Popen | None = None
        self.log_path = directory / f"server-{mode}.log"
        self.log = None

    def command(self) -> list[str]:
        command = [str(self.binary), "--bind", f"127.0.0.1:{self.port}",
                   "--aclfile", str(self.directory / "users.acl"), "--idle-timeout-ms", "5000"]
        if self.mode == "fixed-one":
            command += ["--threads", "1"]
        elif self.mode == "tokio":
            command += ["--io-mode", "tokio"]
        elif self.mode != "mio":
            raise ValueError(f"unknown mode: {self.mode}")
        return command

    def start(self) -> None:
        self.log = self.log_path.open("wb")
        try:
            self.process = subprocess.Popen(self.command(), stdout=subprocess.DEVNULL,
                                            stderr=self.log, start_new_session=True)
            deadline = time.monotonic() + STARTUP_TIMEOUT
            while time.monotonic() < deadline:
                if self.process.poll() is not None:
                    raise RuntimeError(f"server exited during startup ({self.process.returncode}); see {self.log_path}")
                try:
                    with socket.create_connection(("127.0.0.1", self.port), timeout=0.1):
                        return
                except OSError:
                    time.sleep(0.025)
            raise TimeoutError(f"server readiness timed out after {STARTUP_TIMEOUT}s; see {self.log_path}")
        except BaseException:
            self.stop()
            raise

    def stop(self) -> None:
        process = self.process
        try:
            if process is not None and process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=2)
            elif process is not None:
                process.wait(timeout=0)
        finally:
            self.process = None
            if self.log is not None:
                self.log.close()
                self.log = None

    def __enter__(self):
        self.start()
        return self

    def __exit__(self, *_):
        self.stop()


def exercise(redis, mode: str, port: int) -> dict:
    address = {"host": "127.0.0.1", "port": port, "username": USERNAME,
               "password": PASSWORD, "socket_connect_timeout": COMMAND_TIMEOUT,
               "socket_timeout": COMMAND_TIMEOUT, "decode_responses": False}
    outcomes = []
    for protocol in (2, 3):
        client = redis.Redis(**address, protocol=protocol)
        try:
            check(client.ping() is True, f"RESP{protocol} PING with named ACL authentication")
            outcomes.append(f"resp{protocol}-named-acl-auth")
            key = f"conformance:{mode}:{protocol}:binary".encode()
            value = b"\\x00\xffredis-py\x00binary"
            client.set(key, value)
            check(client.get(key) == value, "binary GET equals SET value")
            outcomes.append("binary-string-roundtrip")
            with client.pipeline(transaction=False) as pipeline:
                pipeline.set(key + b":p1", b"one")
                pipeline.set(key + b":p2", b"two")
                replies = pipeline.execute()
            check(replies == [True, True], "non-transactional pipeline replies")
            with client.pipeline(transaction=True) as pipeline:
                pipeline.set(key + b":tx", b"before")
                pipeline.get(key + b":tx")
                check(pipeline.execute() == [True, b"before"], "transaction pipeline replies")
            outcomes.append("pipeline-and-transaction")
            client.delete(key + b":channel")
            with client.pubsub() as pubsub:
                pubsub.subscribe(key + b":channel")
                subscribed = pubsub.get_message(timeout=COMMAND_TIMEOUT)
                check(subscribed is not None and subscribed.get("type") == "subscribe", f"subscribe acknowledgement: {subscribed!r}")
                check(client.publish(key + b":channel", b"payload\x00\xff") == 1, "publish subscriber count")
                message = pubsub.get_message(timeout=COMMAND_TIMEOUT)
                check(message is not None and message["type"] == "message", "pub/sub message type")
                check(message["data"] == b"payload\x00\xff", "binary pub/sub payload")
            outcomes.append("pubsub-binary-payload")
            client.connection_pool.disconnect()
            check(client.ping() is True, "PING after pool disconnect/reconnect")
            outcomes.append("connection-pool-reconnect")
        finally:
            client.close()
    return {"mode": mode, "port": port, "protocols": [2, 3], "checks": outcomes}


def run(binary: Path, output: Path) -> dict:
    redis = require_client()
    binary = binary.resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError(f"candidate server executable missing/not executable: {binary}")
    output.mkdir(parents=True, exist_ok=True)
    records = []
    directory = output / f"run-{time.time_ns()}"
    directory.mkdir()
    (directory / "users.acl").write_text(
        f"user default on >default-only-test-secret ~* &* +@all\nuser {USERNAME} on >{PASSWORD} ~* &* +@all\n", encoding="ascii")
    for mode in ("mio", "fixed-one", "tokio"):
        with ManagedServer(binary, mode, directory) as server:
            record = exercise(redis, mode, server.port)
            record["pid"] = server.process.pid
            records.append(record)
    result = {"schema_version": 1, "status": "passed", "client": "redis-py",
              "client_version": CLIENT_VERSION, "server_binary": str(binary),
              "modes": records, "checks_per_mode": 10}
    path = output / "redis-py-conformance.json"
    path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", type=Path, default=Path("target/release/rillcache"),
                        help="candidate rillcache executable (default: target/release/rillcache)")
    parser.add_argument("--output", type=Path, default=Path("target/upstream/redis-py-conformance"),
                        help="directory for JSON result and per-mode server logs")
    args = parser.parse_args(argv)
    try:
        result = run(args.server, args.output)
        print(json.dumps(result, indent=2, sort_keys=True))
        return 0
    except Exception as error:
        print(f"redis-py conformance FAILED: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
