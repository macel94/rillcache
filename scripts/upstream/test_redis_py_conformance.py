import importlib.util
import ssl
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import MagicMock, Mock, patch

SCRIPT = Path(__file__).resolve().parents[1] / "redis_py_conformance.py"
SPEC = importlib.util.spec_from_file_location("redis_py_conformance", SCRIPT)
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)


class ManagedServerTests(unittest.TestCase):
    def test_modes_are_explicit_and_use_aclfile(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            for mode, expected in (("mio", []), ("fixed-one", ["--threads", "1"]),
                                   ("tokio", ["--io-mode", "tokio"])):
                server = HARNESS.ManagedServer(Path("candidate"), mode, directory)
                command = server.command()
                self.assertIn("--aclfile", command)
                self.assertEqual(command[command.index("--aclfile") + 1], str(directory / "users.acl"))
                if expected:
                    self.assertEqual(command[-len(expected):], expected)
                else:
                    self.assertNotIn("--threads", command)
                    self.assertNotIn("--io-mode", command)

    def test_terminate_timeout_escalates_to_kill_and_joins(self):
        process = Mock()
        process.poll.return_value = None
        process.wait.side_effect = [subprocess.TimeoutExpired("server", 2), 0]
        server = HARNESS.ManagedServer(Path("candidate"), "mio", Path("."))
        server.process = process
        server.stop()
        process.terminate.assert_called_once_with()
        process.kill.assert_called_once_with()
        process.wait.assert_any_call(timeout=2)
        self.assertIsNone(server.process)

    def test_startup_exit_is_reaped_and_logs_close(self):
        process = Mock()
        process.poll.return_value = 9
        process.returncode = 9
        with tempfile.TemporaryDirectory() as temporary:
            server = HARNESS.ManagedServer(Path("candidate"), "mio", Path(temporary))
            with patch.object(HARNESS.subprocess, "Popen", return_value=process):
                with self.assertRaisesRegex(RuntimeError, "exited during startup"):
                    server.start()
            process.wait.assert_called_once_with(timeout=0)
            self.assertIsNone(server.process)
            self.assertIsNone(server.log)

    def test_tls_uses_distinct_loopback_endpoint_and_strict_identity(self):
        server = HARNESS.ManagedServer(Path("candidate"), "mio", Path("."), certificate="alternate")
        command = server.command()
        self.assertEqual(command[command.index("--bind") + 1], "127.0.0.1:0")
        self.assertEqual(command[command.index("--tls-bind") + 1], "127.0.0.1:0")
        self.assertEqual(Path(command[command.index("--tls-cert-file") + 1]).name, "alternate.crt")
        options = HARNESS.client_options(server.port, tls=True)
        self.assertEqual(options["ssl_cert_reqs"], "required")
        self.assertTrue(options["ssl_check_hostname"])
        self.assertEqual(Path(options["ssl_ca_certs"]).name, "ca.crt")

    def test_listener_port_requires_complete_matching_loopback_line(self):
        log = (b"rillcache 0.1.0 listening on 127.0.0.1:31001\n"
               b"rillcache TLS 1.3 listening on 127.0.0.1:31002\n")
        self.assertEqual(HARNESS.listener_port(log, tls=False), 31001)
        self.assertEqual(HARNESS.listener_port(log, tls=True), 31002)
        self.assertIsNone(HARNESS.listener_port(log[:-1], tls=True))
        self.assertIsNone(HARNESS.listener_port(log.replace(b"31002", b"0"), tls=True))
        self.assertIsNone(HARNESS.listener_port(log.replace(b"31002", b"65536"), tls=True))
        self.assertIsNone(HARNESS.listener_port(log.replace(b"127.0.0.1", b"0.0.0.0"), tls=True))

    def test_start_uses_child_tls_endpoint_not_plain_port(self):
        process = Mock()
        process.poll.return_value = None
        with tempfile.TemporaryDirectory() as temporary:
            server = HARNESS.ManagedServer(Path("candidate"), "mio", Path(temporary),
                                           certificate="alternate")

            def start_child(command, *, stderr, **kwargs):
                stderr.write(b"rillcache 0.1.0 listening on 127.0.0.1:31001\n"
                             b"rillcache TLS 1.3 listening on 127.0.0.1:31002\n")
                stderr.flush()
                return process

            with patch.object(HARNESS.subprocess, "Popen", side_effect=start_child):
                with patch.object(HARNESS.socket, "create_connection") as connect:
                    with server:
                        self.assertEqual(server.port, 31002)
                    connect.assert_called_once_with(("127.0.0.1", 31002), timeout=0.1)

    def test_identity_control_distinguishes_ip_mismatch_from_generic_connection_error(self):
        redis = MagicMock()
        redis.exceptions.ConnectionError = ConnectionError
        ping = redis.Redis.return_value.__enter__.return_value.ping
        with self.assertRaisesRegex(RuntimeError, "client accepted"):
            HARNESS.wrong_identity(redis, 10000)
        ping.side_effect = ConnectionError("cannot connect")
        with self.assertRaisesRegex(RuntimeError, "did not produce an IP mismatch"):
            HARNESS.wrong_identity(redis, 10000)
        cause = ssl.SSLCertVerificationError()
        cause.verify_code = 64
        wrapped = ConnectionError("wrapped mismatch")
        wrapped.__context__ = cause
        ping.side_effect = wrapped
        self.assertEqual(HARNESS.wrong_identity(redis, 10000), {"status": "rejected", "verify_code": 64})

    def test_client_dependency_is_pinned_and_missing_dependency_fails_explicitly(self):
        self.assertEqual(HARNESS.CLIENT_VERSION, "5.2.1")
        with patch.dict("sys.modules", {"redis": None}):
            with self.assertRaisesRegex(RuntimeError, "redis-py is required"):
                HARNESS.require_client()


if __name__ == "__main__":
    unittest.main()
