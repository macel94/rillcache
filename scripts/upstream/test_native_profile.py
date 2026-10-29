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

NATIVE = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'pr139-native-profile.py'))


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


class NativeEvidenceTests(unittest.TestCase):
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
