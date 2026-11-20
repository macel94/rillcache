import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

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

    def test_client_dependency_is_pinned_and_missing_dependency_fails_explicitly(self):
        self.assertEqual(HARNESS.CLIENT_VERSION, "5.2.1")
        with patch.dict("sys.modules", {"redis": None}):
            with self.assertRaisesRegex(RuntimeError, "redis-py is required"):
                HARNESS.require_client()


if __name__ == "__main__":
    unittest.main()
