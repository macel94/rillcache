"""Setup regressions for the isolated diagnostic, with no privileged I/O."""
from pathlib import Path
import runpy
import subprocess
import unittest
from unittest.mock import patch

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
