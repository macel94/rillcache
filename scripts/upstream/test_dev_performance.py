"""Setup regressions for the isolated diagnostic, with no privileged I/O."""
from pathlib import Path
import json
import os
import runpy
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

NATIVE = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'dev-performance.py'))


class NativeSetupTests(unittest.TestCase):
    def probe(self, visible, cycles):
        with patch.object(NATIVE['shutil'], 'which', return_value='/usr/bin/perf'), \
             patch.object(NATIVE['subprocess'], 'run', return_value=subprocess.CompletedProcess([], 0)), \
             patch.object(NATIVE['Path'], 'exists', return_value=False), \
             patch.object(NATIVE['COMMON'], 'command', side_effect=[{'exit_code': visible}, {'exit_code': cycles}]), \
             patch.object(NATIVE['COMMON'], 'checked') as checked:
            result = NATIVE['perf_tool']()
            return result, [call.args[0] for call in checked.call_args_list]

    def test_root_visible_mount_is_not_remounted_despite_unprivileged_invisibility(self):
        result, commands = self.probe(0, 0)
        self.assertEqual(result, ('/usr/bin/perf', 'cycles'))
        self.assertFalse(any('mount' in command for command in commands))

    def test_absent_tracefs_uses_only_a_read_only_mount(self):
        _, commands = self.probe(1, 0)
        mounts = [command for command in commands if 'mount' in command]
        self.assertEqual(mounts, [['sudo', '-n', 'mount', '-t', 'tracefs', '-o', 'ro', 'tracefs', '/sys/kernel/tracing']])

    def test_unavailable_hardware_counter_uses_the_software_cpu_clock(self):
        result, _ = self.probe(0, 1)
        self.assertEqual(result, ('/usr/bin/perf', 'cpu-clock'))


class NativeBuildTests(unittest.TestCase):
    def compile_with_status(self, status):
        function = NATIVE['compile_application']
        with tempfile.TemporaryDirectory() as temporary:
            def checked(argv, directory, **_):
                if argv[1] == 'build':
                    directory.mkdir(parents=True)
                    (directory / 'stderr.log').write_text(status)
                return {'exit_code': 0}
            with patch.dict(function.__globals__, {'OUT': Path(temporary) / 'evidence'}), \
                 patch.object(NATIVE['COMMON'], 'checked', side_effect=checked) as command:
                function(Path(temporary) / 'source', 'candidate')
                return [call.args[0] for call in command.call_args_list]

    def test_only_application_release_artifacts_are_cleaned_before_build(self):
        commands = self.compile_with_status('Compiling rillcache v0.1.0\nFinished release profile')
        self.assertEqual(commands, [
            ['cargo', 'clean', '--locked', '--offline', '--release', '--package', 'rillcache'],
            ['cargo', 'build', '--locked', '--offline', '--release', '--bin', 'rillcache']])

    def test_a_stale_cargo_finished_status_is_rejected(self):
        with self.assertRaisesRegex(AssertionError, 'application was not rebuilt'):
            self.compile_with_status('Finished release profile in 0.01s')

    def test_report_reads_only_the_specified_trusted_capture_with_owner_override(self):
        with tempfile.TemporaryDirectory() as temporary:
            data = Path(temporary) / 'owned.perf.data'
            with patch.object(NATIVE['subprocess'], 'run') as command:
                NATIVE['cpu_report']('perf', data, Path(temporary) / 'report.txt', False)
            self.assertEqual(command.call_args.args[0], [
                'sudo', '-n', 'perf', 'report', '--force', '--stdio', '--percent-limit', '0.5',
                '--no-children', '-i', str(data)])
            self.assertTrue(command.call_args.kwargs['check'])
            self.assertEqual(command.call_args.kwargs['env'], NATIVE['SAFE_ENV'])


class NativeEvidenceTests(unittest.TestCase):
    def test_idle_capture_has_only_the_owned_server_and_no_generator(self):
        argv = NATIVE['idle_argv']('perf', 123, Path('/owned/idle/syscalls.txt'))
        self.assertEqual(argv[:4], ['sudo', '-n', 'perf', 'stat'])
        self.assertEqual(argv[argv.index('-p') + 1], '123')
        self.assertEqual(argv[argv.index('-o') + 1], '/owned/idle/syscalls.txt')
        self.assertEqual(argv[-3:], ['--', 'sleep', '2'])
        events = argv[argv.index('-e') + 1].split(',')
        self.assertIn('syscalls:sys_enter_accept4', events)
        self.assertIn('syscalls:sys_enter_clock_nanosleep', events)
        self.assertNotIn('-a', argv)
        self.assertNotIn('--duration', argv)

    def test_syscall_capture_includes_readv_and_only_the_owned_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            with patch.object(NATIVE['subprocess'], 'Popen'):
                jobs = NATIVE['profiler_jobs']('perf', 'cpu-clock', 'syscalls', 123, 124, Path(temporary))
            for (role, _, argv), pid in zip(jobs, (123, 124)):
                self.assertIn(role, ('server', 'generator'))
                self.assertIn('syscalls:sys_exit_readv', argv)
                self.assertIn('syscalls:sys_enter_readv', argv)
                self.assertIn('syscalls:sys_enter_sched_yield', argv)
                self.assertEqual(argv.count('--filter'), 3)
                self.assertEqual(argv[argv.index('-p') + 1], str(pid))
                self.assertNotIn('-a', argv)

    def test_capture_is_runner_owned_even_if_privileged_profiler_cannot_start(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            with patch.object(NATIVE['subprocess'], 'Popen', side_effect=OSError('simulated start failure')):
                with self.assertRaises(OSError):
                    NATIVE['profiler_jobs']('perf', 'cpu-clock', 'cpu', 123, 124, directory)
            path = directory / 'server.perf.data'
            self.assertEqual(path.stat().st_uid, os.getuid())
            self.assertEqual(path.read_bytes(), b'')
            with patch.object(NATIVE['COMMON'], 'command', return_value={'exit_code': 0}) as command:
                NATIVE['capture_permissions'](directory)
            self.assertEqual(command.call_args.args[0], ['sudo', '-n', 'chmod', '0644', str(path)])

    def test_profiler_signal_failure_still_finalizes_capture_permissions(self):
        finalize = Mock(return_value={})
        def failed_profiler(*_):
            argv = [sys.executable, '-c', 'import os, signal; os.kill(os.getpid(), signal.SIGTERM)']
            proc = subprocess.Popen(argv, start_new_session=True, env=NATIVE['SAFE_ENV'])
            return [('generator', proc, argv)]
        function = NATIVE['measured_load']
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / 'measured'
            with patch.dict(function.__globals__, {'profiler_jobs': failed_profiler, 'capture_permissions': finalize}):
                with self.assertRaisesRegex(RuntimeError, 'generator profiler failed: -15'):
                    function([sys.executable, '-c', 'import time; time.sleep(0.8)'], directory,
                             SimpleNamespace(pid=os.getpid()), 'cpu', 'unused-perf', 'cpu-clock')
            finalize.assert_called_once_with(directory)
            saved = json.loads((directory / 'command.json').read_text())
            self.assertEqual(saved['exit_code'], 0)
            self.assertEqual(saved['capture_permissions'], {})

    def test_partial_capture_start_failure_joins_the_first_owned_child(self):
        first = Mock(name='first owned profiler')
        terminate = Mock()
        with tempfile.TemporaryDirectory() as temporary:
            with patch.object(NATIVE['subprocess'], 'Popen', side_effect=[first, OSError('second start failed')]), \
                 patch.dict(NATIVE['G'], {'terminate': terminate}):
                with self.assertRaisesRegex(OSError, 'second start failed'):
                    NATIVE['profiler_jobs']('perf', 'cpu-clock', 'cpu', 123, 124, Path(temporary))
            terminate.assert_called_once_with(first)

    def test_runtime_child_environment_contains_no_actions_credentials(self):
        self.assertEqual(set(NATIVE['SAFE_ENV']), {'PATH', 'LANG', 'LC_ALL'})
        self.assertEqual(NATIVE['SAFE_ENV']['LANG'], 'C')
        self.assertEqual(NATIVE['SAFE_ENV']['LC_ALL'], 'C')
