"""Synthetic offline fixtures; no native capture or throughput measurement."""
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import common

spec = importlib.util.spec_from_file_location("owned_profile", common.ROOT / "scripts/reactor-profile.py")
P = importlib.util.module_from_spec(spec)
spec.loader.exec_module(P)


def fixture(root):
    cpus = [0, 1, 2, 3]
    hardware = {"selected_cpus": cpus, "rust": f"rustc {common.PINS['rust']} (fixture)", "build_environment": {}}
    common.save(root / "hardware.json", hardware)
    m = {"complete": True, "instrumented": True, "normal_performance_evidence": False,
         "tcp_observation_schema": 1,
         "application_source": "a" * 40, "cpus": cpus, "samples": [], "binary_paths": {},
         "binary_sha256": {}, "launch_directory": str(root),
         "hardware_fingerprint": P.D.HARDWARE["fingerprint"](hardware)}
    (root / "frozen-binaries").mkdir()
    for name in ("rillcache", "valkey", "generator"):
        p = root / "frozen-binaries" / name
        p.write_text("synthetic fixture, not executable: " + name)
        m["binary_paths"][name] = str(p)
        m["binary_sha256"][name] = common.digest(p)
    for index, identity in enumerate(P.identities()):
        mode, workload, variant = identity
        directory = root / "captures" / "-".join(identity)
        measured = directory / "measured"
        measured.mkdir(parents=True)
        pid, generator_pid, port = 1000 + index, 2000 + index, 20000 + index
        before = {"pid": pid, "start_ticks": 10, "actual_cpus": cpus, "user_ticks": 0, "system_ticks": 0}
        after = {**before, "user_ticks": 10, "system_ticks": 20}
        argv = P.load_argv(m["binary_paths"]["generator"], port, workload, cpus)
        command = {"argv": argv, "pid": generator_pid, "exit_code": 0, "wall_seconds": 20.1,
                   "server_at_capture": before, "generator_at_capture": {"pid": generator_pid}}
        common.save(measured / "command.json", command)
        metrics = {"rps": 100.0, "min_latency_ms": .001, "p50_latency_ms": .002,
                   "p95_latency_ms": .003, "p99_latency_ms": .004,
                   "max_latency_ms": .005, "avg_latency_ms": .002}
        header = P.G["CSV_HEADER"]
        title = " ".join(P.G["workload_command"](workload))
        (measured / "stdout.log").write_text(','.join(f'"{x}"' for x in header) + '\n' +
            f'"{title}",' + ','.join(str(metrics[x]) for x in header[1:]) + '\n')
        (measured / "stderr.log").write_text("")
        for role, target in (("server", pid), ("generator", generator_pid)):
            args = ["sudo", "-n", "/usr/bin/perf", "record" if mode == "cpu" else "stat",
                    "-p", str(target), "--", "sleep", "8"]
            common.save(measured / f"{role}-profiler.json", {"argv": args, "exit_code": 0})
            if mode == "cpu":
                (measured / f"{role}.perf.data").write_text("synthetic raw capture fixture only")
                for suffix in ("self", "children"):
                    (directory / f"{role}-{suffix}.txt").write_text("# Samples: 2 of cpu-clock (synthetic)\n")
            else:
                (measured / f"{role}-syscalls.txt").write_text(
                    "1;;syscalls:sys_enter_epoll_wait\n0;;syscalls:sys_exit_recvfrom\n1;;syscalls:sys_enter_writev\n")
        common.save(directory / "server.json", {"pid": pid, "port": port, "joined": True,
            "exit_before_cleanup": None, "exit_after_cleanup": -15,
            "argv": P.G["server_argv"](m["binary_paths"][variant], variant, port, cpus, directory)})
        tcp = {"pid": pid, "port": port, "valid": True, "start_ticks": 10}
        for phase, count in (("before", 100), ("after", 200)):
            raw = "\n".join(
                f'0 0 127.0.0.1:{port} 127.0.0.1:1234 users:(("app",pid={pid},fd=5)) '
                f'ino:{i} segs_out:{count * 2} segs_in:{count} data_segs_out:{count} '
                f'data_segs_in:{count} bytes_sent:{count * 7}' for i in range(8))
            (measured / f"tcp-{phase}.stdout").write_text(raw)
            tcp[phase] = {"time": 1 if phase == "before" else 9,
                          "argv": P.T.ss_argv(port), "sockets": P.T.parse_ss(raw, pid)}
        tcp["delta"] = P.T.deltas(tcp["before"]["sockets"], tcp["after"]["sockets"])
        common.save(measured / "tcp-info.json", tcp)
        common.save(measured / "tcp-profiler.json", {
            "argv": ["/usr/bin/python3", str(common.ROOT / "scripts/upstream/tcp_info.py"),
                     "--pid", str(pid), "--port", str(port), "--out", str(measured)],
            "exit_code": 0})
        s = {"identity": list(identity), "valid": True, "instrumented": True,
             "before": before, "after": after, "command": command, "metrics": metrics,
             "tcp_info": tcp}
        common.save(directory / "sample.json", s)
        m["samples"].append(s)
    (root / "summary.md").write_text(P.report(m["samples"]))
    m["evidence_sha256"] = P.D.evidence_hashes(root)
    common.save(root / "manifest.json", m)
    return m


class ReactorProfileTests(unittest.TestCase):
    def test_scope_and_generator_are_actual_failing_profile_not_a_gate(self):
        self.assertEqual(8, len(P.identities()))
        self.assertEqual(8, len(set(P.identities())))
        args = P.load_argv("/generator", 1234, "get", [0, 1, 2, 3])
        generator_args = args[args.index("/generator") + 1:]
        for flag, expected in (("--duration", "20"), ("-d", "64"), ("-c", "8"), ("--threads", "2")):
            self.assertEqual(expected, generator_args[generator_args.index(flag) + 1])
        with patch.dict(os.environ, {"GITHUB_ACTIONS": ""}):
            with self.assertRaisesRegex(AssertionError, "Actions-only"):
                P.main(["--source", "a" * 40])

    def test_complete_instrumented_fixture_validates_but_never_reports_medians(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            m = fixture(root)
            self.assertEqual(8, len(P.check(root)["samples"]))
            self.assertIn("NOT throughput qualification", P.report(m["samples"]))
            self.assertNotIn("median req/s", P.report(m["samples"]))

    def test_incomplete_missing_or_changed_evidence_fails_closed(self):
        for kind in ("incomplete", "missing", "changed"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                m = fixture(root)
                if kind == "incomplete": m["complete"] = False
                elif kind == "missing": m["samples"].pop()
                else: (root / "summary.md").write_text("corrupt")
                common.save(root / "manifest.json", m)
                with self.assertRaises(AssertionError): P.check(root)

    def test_rehashed_wrong_pid_duration_or_join_is_rejected(self):
        for kind in ("pid", "duration", "join"):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                m = fixture(root)
                s = m["samples"][0]
                directory = root / "captures" / "-".join(s["identity"])
                if kind == "pid":
                    path = directory / "measured/server-profiler.json"
                    capture = P.G["read_json"](path)
                    capture["argv"][capture["argv"].index("-p") + 1] = "99999"
                    common.save(path, capture)
                elif kind == "duration":
                    args = s["command"]["argv"]
                    args[args.index("--duration") + 1] = "10"
                    common.save(directory / "sample.json", s)
                    common.save(directory / "measured/command.json", s["command"])
                else:
                    path = directory / "server.json"
                    server = P.G["read_json"](path); server["joined"] = False; common.save(path, server)
                m["evidence_sha256"] = P.D.evidence_hashes(root)
                common.save(root / "manifest.json", m)
                with self.assertRaises(AssertionError): P.check(root)

    def test_syscall_capture_includes_vectored_writes_and_only_owned_pids(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(P.D.subprocess, "Popen") as popen:
            jobs = P.D.profiler_jobs("perf", "cpu-clock", "syscalls", 123, 456, Path(tmp))
            self.assertEqual(2, len(jobs))
            for call, target in zip(popen.call_args_list, (123, 456)):
                args = call.args[0]
                self.assertIn("syscalls:sys_enter_writev", args)
                self.assertEqual(str(target), args[args.index("-p") + 1])
                self.assertEqual(["--", "sleep", "8"], args[-3:])

    def test_privileged_cleanup_is_scoped_and_joined(self):
        proc = Mock(args=["sudo", "-n", "/usr/bin/perf", "record"], pid=4321, returncode=-15)
        proc.poll.return_value = None
        with patch.object(P.os, "getpgid", return_value=4321), patch.object(P.subprocess, "run") as run:
            P.terminate_owned(proc)
            self.assertTrue(proc.wait.called)
            self.assertEqual(2, run.call_count)
            self.assertTrue(all(c.args[0][-1] == "-4321" for c in run.call_args_list))
        with patch.object(P.os, "getpgid", return_value=99), patch.object(P.subprocess, "run") as run:
            with self.assertRaises(AssertionError): P.terminate_owned(proc)
            run.assert_not_called()
        ordinary = Mock(args=["/owned/server"], pid=123)
        with patch.object(P, "ORIGINAL_TERMINATE", return_value=0) as stop:
            self.assertEqual(0, P.terminate_owned(ordinary)); stop.assert_called_once_with(ordinary)


if __name__ == "__main__":
    unittest.main()
