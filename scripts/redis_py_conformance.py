#!/usr/bin/env python3
"""Run pinned redis-py against Rillcache: the legacy managed mode and the C7 matrix mode.

Matrix mode runs the shared language-client case set against runner-owned
password/ACL, TLS and wrong-identity candidate listeners and writes one result
per declared case. It never starts its own servers.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import socket
import ssl
import subprocess
import sys
import time
from pathlib import Path

CLIENT_VERSION = "5.2.1"
PASSWORD = "redis-py-test-secret"
USERNAME = "redis_py"
STARTUP_TIMEOUT = 8.0
COMMAND_TIMEOUT = 2.0
TLS_FIXTURES = Path(__file__).resolve().parents[1] / "tests/fixtures/tls"


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


def listener_port(log: bytes, *, tls: bool) -> int | None:
    prefix = rb"rillcache TLS 1\.3" if tls else rb"rillcache [^\s]+"
    match = re.search(rb"^" + prefix + rb" listening on 127\.0\.0\.1:([0-9]{1,5})\r?\n",
                      log, flags=re.MULTILINE)
    if match:
        port = int(match[1])
        if 0 < port <= 65535:
            return port
    return None


class ManagedServer:
    """Owns one subprocess and log; every startup/exit path reaps the child."""

    def __init__(self, binary: Path, mode: str, directory: Path, *, certificate: str | None = None):
        self.binary = binary
        self.mode = mode
        self.directory = directory
        self.port = 0
        self.certificate = certificate
        self.process: subprocess.Popen | None = None
        label = f"{mode}-tls-{certificate}" if certificate else mode
        self.log_path = directory / f"server-{label}.log"
        self.log = None

    def command(self) -> list[str]:
        command = [str(self.binary), "--bind", "127.0.0.1:0",
                   "--aclfile", str(self.directory / "users.acl"), "--idle-timeout-ms", "5000"]
        if self.certificate:
            command += ["--tls-bind", "127.0.0.1:0",
                        "--tls-cert-file", str(TLS_FIXTURES / f"{self.certificate}.crt"),
                        "--tls-key-file", str(TLS_FIXTURES / f"{self.certificate}.key")]
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
                with self.log_path.open("rb") as log:
                    port = listener_port(log.read(64 * 1024), tls=bool(self.certificate))
                if port is not None:
                    try:
                        with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                            if self.process.poll() is None:
                                self.port = port
                                return
                    except OSError:
                        pass
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


def client_options(port: int, tls: bool) -> dict:
    options = {"host": "127.0.0.1", "port": port, "username": USERNAME,
               "password": PASSWORD, "socket_connect_timeout": COMMAND_TIMEOUT,
               "socket_timeout": COMMAND_TIMEOUT, "decode_responses": False}
    if tls:
        options.update(ssl=True, ssl_ca_certs=str(TLS_FIXTURES / "ca.crt"),
                       ssl_cert_reqs="required", ssl_check_hostname=True)
    return options


def check_db_startup(redis, address: dict, protocol: int, db_zero, key: bytes, value: bytes) -> None:
    # redis-py selects a nonzero database during connect, before the first command.
    with redis.Redis(**address, protocol=protocol, db=1) as db_one:
        try:
            db_one.set(key, b"unexpected-db-one-write")
        except redis.exceptions.ResponseError as error:
            if str(error) != "DB index is out of range":
                raise RuntimeError(f"RESP{protocol} DB 1 startup returned the wrong error") from error
        else:
            raise RuntimeError(f"RESP{protocol} DB 1 startup did not reject the write")
    check(db_zero.get(key) == value, f"RESP{protocol} DB 0 value after rejected DB 1 startup")


def exercise(redis, mode: str, port: int, *, tls: bool = False) -> dict:
    address = client_options(port, tls)
    outcomes = []
    for protocol in (2, 3):
        client = redis.Redis(**address, protocol=protocol, db=0)
        try:
            check(client.ping() is True, f"RESP{protocol} PING with named ACL authentication")
            outcomes.append(f"resp{protocol}-named-acl-auth")
            key = f"conformance:{mode}:{protocol}:binary".encode()
            value = b"\\x00\xffredis-py\x00binary"
            check(client.set(key, value) is True, f"RESP{protocol} DB 0 SET succeeds")
            check(client.get(key) == value, "binary GET equals SET value")
            outcomes.append("binary-string-roundtrip")
            check_db_startup(redis, address, protocol, client, key, value)
            outcomes.append(f"resp{protocol}-db-one-startup-rejected-db-zero-unchanged")
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


def wrong_identity(redis, port: int) -> dict:
    with redis.Redis(**client_options(port, tls=True)) as client:
        try:
            client.ping()
        except redis.exceptions.ConnectionError as error:
            # redis-py 5.2.1 wraps the OpenSSL verification exception. A refusal,
            # generic disconnect or trust-chain error cannot pass this control.
            cause = error.__cause__ or error.__context__
            if isinstance(cause, ssl.SSLCertVerificationError) and cause.verify_code == 64:
                return {"status": "rejected", "verify_code": cause.verify_code}
            raise RuntimeError("wrong TLS endpoint did not produce an IP mismatch") from error
    raise RuntimeError("client accepted a certificate without the expected IP SAN")


def run(binary: Path, output: Path, *, tls: bool = False) -> dict:
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
              "modes": records, "checks_per_mode": 12}
    if tls:
        try:
            with ManagedServer(binary, "mio", directory, certificate="server") as server:
                result["tls"] = {"positive": exercise(redis, "mio-tls", server.port, tls=True)}
            with ManagedServer(binary, "mio", directory, certificate="alternate") as server:
                result["tls"]["wrong_identity"] = wrong_identity(redis, server.port)
        except Exception as error:
            result["status"] = "failed"
            result["tls_failure_type"] = type(error).__name__
            (directory / "failed.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
            raise
    path = output / "redis-py-conformance.json"
    path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return result



MATRIX_CASES = (
    "handshake-auth", "acl-allowed", "acl-denied-command", "resp2-roundtrip", "resp3-roundtrip",
    "pipeline", "transaction", "pubsub", "error-wrong-type", "error-unknown-command",
    "error-auth-required", "reconnect", "tls-positive", "tls-wrong-identity",
)


def matrix_case(records, case, body):
    try:
        detail = body()
    except Exception as error:  # one failing case must not abort the others
        records.append({"id": case, "status": "failed", "detail": str(error)[:400]})
        return
    records.append({"id": case, "status": "passed", "detail": detail or ""})


def matrix_options(redis, port, user, password, protocol, tls_ca=None):
    options = {"host": "127.0.0.1", "port": port, "username": user, "password": password,
               "protocol": protocol, "db": 0, "decode_responses": False,
               "socket_connect_timeout": 3.0, "socket_timeout": 3.0, "retry_on_timeout": False}
    if tls_ca is not None:
        options.update(ssl=True, ssl_ca_certs=str(tls_ca), ssl_cert_reqs="required",
                       ssl_check_hostname=True)
    return options


def matrix_run(redis, args, prefix: str) -> dict:
    records = []
    named = (args.named_user, args.named_password)
    limited = (args.limited_user, args.limited_password)
    plain = args.plain_port
    key = lambda label: f"py-matrix:{prefix}:{label}".encode()

    def expect(condition, message):
        check(condition, message)

    def wrong_type_error(client, target):
        try:
            client.lpush(target, b"x")
        except redis.exceptions.ResponseError as error:
            expect("WRONGTYPE" in str(error), f"wrong-type error text: {error}")
        else:
            raise RuntimeError("wrong-type write succeeded")

    def noperm_error(client):
        try:
            client.set(key("denied"), b"value")
        except redis.exceptions.NoPermissionError:
            return
        else:
            raise RuntimeError("limited user was allowed to SET")

    matrix_case(records, "handshake-auth", lambda: _with_client(
        redis, matrix_options(redis, plain, *named, 2),
        lambda client: expect(client.ping() is True, "PING did not return PONG")))
    matrix_case(records, "acl-allowed", lambda: _with_client(
        redis, matrix_options(redis, plain, *named, 2),
        lambda client: (expect(client.set(key("acl"), b"value") is True, "named user SET failed"),
                        expect(client.get(key("acl")) == b"value", "named user GET differed"))))
    matrix_case(records, "acl-denied-command", lambda: _with_client(
        redis, matrix_options(redis, plain, *limited, 2), noperm_error))

    def roundtrip(protocol):
        def body(client):
            expect(client.ping() is True, f"RESP{protocol} PING failed")
            binary = b"\x00\xff\x0d\x0a"
            expect(client.set(key(f"binary-{protocol}"), binary) is True, "binary SET failed")
            expect(client.get(key(f"binary-{protocol}")) == binary, "binary GET differed")
            expect(client.set(key(f"counter-{protocol}"), b"41") is True, "counter SET failed")
            expect(client.incr(key(f"counter-{protocol}")) == 42, "INCR differed")
            client.hset(key(f"hash-{protocol}"), b"field", b"value")
            reply = client.hgetall(key(f"hash-{protocol}"))
            if protocol == 3:
                # The profile's RESP3 subset emits arrays, not maps (C6), so the
                # client surfaces a flat field/value array. Assert that shape so
                # a silent change to either side fails this case.
                expect(reply == [b"field", b"value"], f"RESP3 HGETALL shape differed: {reply!r}")
                return "resp3-hgetall-flat-array: the profile's bounded RESP3 subset returns arrays, not maps (C6)"
            expect(reply == {b"field": b"value"}, f"RESP2 HGETALL differed: {reply!r}")
            return ""
        return lambda: _with_client(redis, matrix_options(redis, plain, *named, protocol), body)

    matrix_case(records, "resp2-roundtrip", roundtrip(2))
    matrix_case(records, "resp3-roundtrip", roundtrip(3))

    def pipeline(client):
        with client.pipeline(transaction=False) as pipe:
            pipe.set(key("p1"), b"one")
            pipe.set(key("p2"), b"two")
            pipe.mget([key("p1"), key("p2")])
            replies = pipe.execute()
        expect(replies == [True, True, [b"one", b"two"]], f"pipeline replies differed: {replies!r}")

    matrix_case(records, "pipeline", lambda: _with_client(
        redis, matrix_options(redis, plain, *named, 2), pipeline))

    def transaction(client):
        with client.pipeline(transaction=True) as pipe:
            pipe.set(key("tx"), b"value")
            pipe.get(key("tx"))
            replies = pipe.execute()
        expect(replies == [True, b"value"], f"transaction replies differed: {replies!r}")
        expect(client.get(key("tx")) == b"value", "transaction state was not committed")

    matrix_case(records, "transaction", lambda: _with_client(
        redis, matrix_options(redis, plain, *named, 2), transaction))

    def pubsub(client):
        channel = key("channel")
        with client.pubsub() as subscriber:
            subscriber.subscribe(channel)
            expect(subscriber.get_message(timeout=3) is not None, "subscribe acknowledgement missing")
            expect(client.publish(channel, b"payload") == 1, "publish subscriber count differed")
            message = subscriber.get_message(timeout=3)
            expect(message is not None and message.get("data") == b"payload", "pub/sub payload differed")

    matrix_case(records, "pubsub", lambda: _with_client(
        redis, matrix_options(redis, plain, *named, 2), pubsub))

    matrix_case(records, "error-wrong-type", lambda: _with_client(
        redis, matrix_options(redis, plain, *named, 2),
        lambda client: (client.set(key("wrong"), b"value"), wrong_type_error(client, key("wrong")))))
    matrix_case(records, "error-unknown-command", lambda: _with_client(
        redis, matrix_options(redis, plain, *named, 2), lambda client: unknown_command_error(redis, client)))
    matrix_case(records, "error-auth-required", lambda: _with_client(
        redis, matrix_options(redis, plain, None, None, 2), lambda client: auth_required_error(redis, client)))

    def reconnect(client):
        expect(client.set(key("reconnect"), b"value") is True, "SET before reconnect failed")
        client.connection_pool.disconnect()
        expect(client.get(key("reconnect")) == b"value", "value missing after reconnect")

    matrix_case(records, "reconnect", lambda: _with_client(
        redis, matrix_options(redis, plain, *named, 2), reconnect))

    def tls_positive(client):
        expect(client.ping() is True, "TLS PING failed")
        expect(client.set(key("tls"), b"value") is True, "TLS SET failed")
        expect(client.get(key("tls")) == b"value", "TLS GET differed")

    matrix_case(records, "tls-positive", lambda: _with_client(
        redis, matrix_options(redis, args.tls_port, *named, 2, tls_ca=args.ca), tls_positive))
    matrix_case(records, "tls-wrong-identity", lambda: tls_wrong_identity(
        redis, matrix_options(redis, args.alt_tls_port, *named, 2, tls_ca=args.ca)))

    return {"client": "redis-py", "version": redis.__version__,
            "protocols": [2, 3], "cases": records}


def _with_client(redis, options, body):
    client = redis.Redis(**options)
    try:
        return body(client)
    finally:
        client.close()


def unknown_command_error(redis, client):
    try:
        client.execute_command("NOSUCHCOMMAND")
    except redis.exceptions.ResponseError as error:
        check("unknown command" in str(error).lower(), f"unknown-command error text: {error}")
    else:
        raise RuntimeError("unknown command succeeded")
    check(client.ping() is True, "connection was unusable after the error")


def auth_required_error(redis, client):
    try:
        client.get(b"py-matrix:noauth")
    except redis.exceptions.AuthenticationError:
        return
    except redis.exceptions.ResponseError as error:
        text = str(error).upper()
        check("NOAUTH" in text or "AUTHENTICATION" in text, f"auth error text: {error}")
    else:
        raise RuntimeError("unauthenticated read succeeded")


def tls_wrong_identity(redis, options):
    client = redis.Redis(**options)
    try:
        client.ping()
    except redis.exceptions.ConnectionError as error:
        cause = error.__cause__ or error.__context__
        if isinstance(cause, ssl.SSLCertVerificationError) and cause.verify_code == 64:
            return
        raise RuntimeError("wrong TLS endpoint did not produce an IP mismatch") from error
    finally:
        client.close()
    raise RuntimeError("certificate without the target IP SAN was accepted")


def matrix_main(redis, args) -> int:
    result = matrix_run(redis, args, "case")
    args.json_out.parent.mkdir(parents=True, exist_ok=True)
    args.json_out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    failed = [case for case in result["cases"] if case["status"] != "passed"]
    print(f"redis-py {redis.__version__}: {len(result['cases']) - len(failed)}/{len(result['cases'])} cases passed")
    for case in failed:
        print(f"FAILED {case['id']}: {case.get('detail', '')}", file=sys.stderr)
    return 0 if not failed else 1

def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix", action="store_true",
                        help="run the shared language-client case set against runner-owned listeners")
    parser.add_argument("--plain-port", type=int, help="matrix mode: plaintext listener port")
    parser.add_argument("--tls-port", type=int, help="matrix mode: TLS listener port")
    parser.add_argument("--alt-tls-port", type=int, help="matrix mode: wrong-identity TLS listener port")
    parser.add_argument("--named-user"), parser.add_argument("--named-password")
    parser.add_argument("--limited-user"), parser.add_argument("--limited-password")
    parser.add_argument("--default-password")
    parser.add_argument("--ca", type=Path, help="matrix mode: test CA certificate")
    parser.add_argument("--json-out", type=Path, help="matrix mode: case results path")
    parser.add_argument("--server", type=Path, default=Path("target/release/rillcache"),
                        help="candidate rillcache executable (legacy managed mode only)")
    parser.add_argument("--output", type=Path, default=Path("target/upstream/redis-py-conformance"),
                        help="directory for JSON result and per-mode server logs")
    parser.add_argument("--tls", action="store_true", help="also verify the native TLS endpoint")
    args = parser.parse_args(argv)
    if args.matrix:
        required = ("plain_port", "tls_port", "alt_tls_port", "named_user", "named_password",
                    "limited_user", "limited_password", "default_password", "ca", "json_out")
        missing = [name for name in required if getattr(args, name) in (None, "")]
        if missing:
            parser.error("--matrix requires: " + ", ".join("--" + name.replace("_", "-") for name in missing))
        try:
            redis = require_client()
            return matrix_main(redis, args)
        except Exception as error:
            print(f"redis-py client matrix FAILED: {error}", file=sys.stderr)
            return 1
    try:
        result = run(args.server, args.output, tls=args.tls)
        print(json.dumps(result, indent=2, sort_keys=True))
        return 0
    except Exception as error:
        print(f"redis-py conformance FAILED: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
