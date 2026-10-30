"""Offline orchestration/negative controls; synthetic data is not performance evidence."""
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[2]
D = runpy.run_path(str(ROOT / 'scripts/dev-performance.py'))
H = D['HARDWARE']
G = D['G']


def contract():
    return {'selected_cpus': [0, 1, 2, 3], 'cpu': {'model': 'synthetic fixture'},
            'rust': f'rustc {D["COMMON"].PINS["rust"]} (fixture)',
            'build_environment': {key: None for key in H['BUILD_KEYS']}}


class HardwareAndArgumentsTests(unittest.TestCase):
    def test_fingerprint_is_order_independent_but_hardware_changes_fail(self):
        first = contract()
        expected = H['fingerprint'](first)
        self.assertEqual(expected, H['require_match'](dict(reversed(list(first.items()))), expected))
        changed = copy.deepcopy(first)
        changed['cpu']['model'] = 'another hardware family'
        with self.assertRaisesRegex(ValueError, 'contract mismatch'):
            H['require_match'](changed, expected)
        for invalid in (None, '', 'A' * 64, '0' * 63):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                H['require_match'](first, invalid)

    def test_normal_release_overrides_and_wrong_compiler_fail(self):
        for key in ('RUSTFLAGS', 'CFLAGS', 'RUSTC_WRAPPER', 'CARGO_PROFILE_RELEASE_LTO'):
            changed = contract()
            changed['build_environment'][key] = 'override'
            with self.subTest(key=key), self.assertRaisesRegex(AssertionError, 'unsupported overrides'):
                D['validate_build_environment'](changed)
        changed = contract()
        changed['rust'] = 'rustc 1.0.0 (wrong)'
        with self.assertRaisesRegex(AssertionError, 'pinned compiler'):
            D['validate_build_environment'](changed)

    def test_explicit_sources_hardware_and_unique_pipeline_scope_required(self):
        valid = ['--base', 'a' * 40, '--candidate', 'b' * 40, '--expected-hardware', 'c' * 64]
        args = D['parse_args'](valid)
        self.assertFalse(args.profiles)
        self.assertEqual(args.pipelines, [1, 16])
        self.assertTrue(D['parse_args'](valid + ['--profiles']).profiles)
        for invalid in ([], valid[:4], valid + ['--pipelines', '1', '1'],
                        valid + ['--pipelines', '16', '1'],
                        ['--base', 'a' * 40, '--candidate', 'a' * 40, '--expected-hardware', 'c' * 64]):
            with self.subTest(argv=invalid), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                D['parse_args'](invalid)

    def test_each_pipeline_rotates_every_variant_through_each_position(self):
        order = list(D['normal_order']([1, 16]))
        self.assertEqual(len(order), 18)
        self.assertEqual(len(set(order)), 18)
        for pipeline in (1, 16):
            rows = [variant for p, _, variant in order if p == pipeline]
            self.assertEqual(rows, ['baseline', 'candidate', 'valkey', 'candidate', 'valkey', 'baseline',
                                    'valkey', 'baseline', 'candidate'])


class OrchestrationTests(unittest.TestCase):
    def run_study(self, profiles=False, mismatch=False):
        function = D['main']
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binaries = {}
            for name in (*D['VARIANTS'], 'generator'):
                path = root / name
                path.write_text(name)
                binaries[name] = str(path)
            expected = H['fingerprint'](contract())
            argv = ['study', '--base', 'a' * 40, '--candidate', 'b' * 40,
                    '--expected-hardware', '0' * 64 if mismatch else expected]
            if profiles:
                argv.append('--profiles')
            sample = Mock(return_value={'valid': True})
            perf = Mock(return_value=('perf', 'cpu-clock'))
            build = Mock(return_value=binaries)
            summary = Mock()
            idle = Mock(return_value={'valid': True})
            with patch.dict(function.__globals__, {'OUT': root / 'evidence', 'sample': sample,
                    'perf_tool': perf, 'build_sources': build, 'idle_sample': idle,
                    'check_study': Mock(), 'summarize': Mock(return_value='synthetic summary')}), \
                 patch.dict(os.environ, {'GITHUB_ACTIONS': 'true', 'GITHUB_EVENT_NAME': 'workflow_dispatch',
                                         'RUNNER_ENVIRONMENT': 'self-hosted'}), \
                 patch.dict(H, {'probe': Mock(return_value=contract())}), \
                 patch.object(D['COMMON'], 'environment', return_value={}), \
                 patch.object(D['COMMON'], 'summary', summary), \
                 patch.object(D['runpy'], 'run_path', return_value={'validate_pins': Mock()}), \
                 patch.object(D['sys'], 'argv', argv), contextlib.redirect_stdout(io.StringIO()):
                if mismatch:
                    with self.assertRaisesRegex(ValueError, 'contract mismatch'):
                        function()
                    self.assertFalse(build.called)
                    self.assertFalse(perf.called)
                    manifest = G['read_json'](root / 'evidence/manifest.json')
                    self.assertFalse(manifest['complete'])
                    self.assertTrue((root / 'evidence/hardware-observed.json').exists())
                    return
                self.assertEqual(function(), 0)
                manifest = G['read_json'](root / 'evidence/manifest.json')
            self.assertEqual(len(manifest['normal_samples']), 18)
            self.assertEqual(sample.call_count, 24 if profiles else 18)
            self.assertEqual(perf.call_count, int(profiles))
            self.assertEqual(idle.call_count, 3 if profiles else 0)
            self.assertEqual(manifest['profiles_complete'], profiles)
            self.assertTrue(manifest['complete'])

    def test_normal_screening_never_starts_or_installs_a_profiler(self):
        self.run_study()

    def test_profiles_require_an_explicit_flag_and_stay_separate(self):
        self.run_study(profiles=True)

    def test_hardware_mismatch_fails_before_build_and_preserves_failure(self):
        self.run_study(mismatch=True)

    def test_measurement_refuses_the_agent_host(self):
        argv = ['study', '--base', 'a' * 40, '--candidate', 'b' * 40, '--expected-hardware', 'c' * 64]
        with patch.object(D['sys'], 'argv', argv), patch.dict(os.environ, {'GITHUB_ACTIONS': 'false'}):
            with self.assertRaisesRegex(AssertionError, 'Actions-only'):
                D['main']()


class SyntheticEvidenceTests(unittest.TestCase):
    HEADER = '"test","rps","avg_latency_ms","min_latency_ms","p50_latency_ms","p95_latency_ms","p99_latency_ms","max_latency_ms"\n'

    def fixture(self, root):
        hardware = contract()
        hardware_sha = H['fingerprint'](hardware)
        binaries = {name: f'/original/frozen-binaries/{name}' for name in (*D['VARIANTS'], 'generator')}
        for name in binaries:
            path = root / 'frozen-binaries' / name
            path.parent.mkdir(exist_ok=True)
            path.write_text(name)
        hashes = {name: G['digest'](root / 'frozen-binaries' / name) for name in binaries}
        manifest = {'diagnostic_only': True, 'complete': True, 'normal_duration_seconds': 10,
                    'base_source': 'a' * 40, 'candidate_source': 'b' * 40, 'pipelines': [1, 16],
                    'hardware_fingerprint': hardware_sha, 'cpus': [0, 1, 2, 3], 'binaries': binaries,
                    'binary_sha256': hashes, 'launch_directory': '/original', 'normal_samples': [],
                    'profiles_requested': False, 'profiles_complete': False, 'idle_complete': False,
                    'profile_samples': [], 'idle_samples': []}
        G['save'](root / 'environment.json', {'hardware_contract': hardware})
        pin = D['COMMON'].PINS['upstreams']['valkey']
        G['save'](root / 'reference-identity.json', {
            'identity': {'source_archive_sha256': pin['sha256'], 'version': pin['version'],
                         'hardware_fingerprint': hardware_sha},
            'binary_sha256': {name: hashes[name] for name in ('valkey', 'generator')}})
        for variant in ('baseline', 'candidate'):
            path = root / 'build' / variant / 'stderr.log'
            path.parent.mkdir(parents=True)
            path.write_text('Compiling rillcache v0.1.0\nFinished release profile\n')
        for pipeline, run, variant in D['normal_order']([1, 16]):
            name = f'r{run}-{variant}-p{pipeline}'
            directory = root / 'normal' / name
            port, pid = 16379, 1234
            server = {'pid': pid, 'port': port, 'joined': True, 'exit_before_cleanup': None,
                      'argv': G['server_argv'](binaries[variant], 'valkey' if variant == 'valkey' else 'rillcache',
                                              port, [0, 1, 2, 3], Path('/original/normal') / name)}
            G['save'](directory / 'server.json', server)
            for phase, count in (('warmup', G['WARMUP']), ('measured', None)):
                argv = G['generator_argv'](binaries['generator'], port, 'ping', pipeline, count, [0, 1, 2, 3])
                G['save'](directory / phase / 'command.json', {'argv': argv, 'exit_code': 0, 'wall_seconds': 10.1,
                    'generator_user_seconds': 1, 'generator_system_seconds': 1})
                (directory / phase / 'stderr.log').write_text('')
                rate = {'baseline': 100000, 'candidate': 90000, 'valkey': 110000}[variant]
                (directory / phase / 'stdout.log').write_text(self.HEADER + f'"PING","{rate}","0.1","0.01","0.1","0.2","0.3","1.0"\n')
                metrics = G['read_generator'](directory / phase, 'ping', argv)
            record = {'variant': variant, 'run': run, 'pipeline': pipeline, 'mode': 'normal', 'valid': True,
                      'before': {'pid': pid, 'start_ticks': 5, 'actual_cpus': [0, 1, 2, 3],
                                 'clock_ticks_per_second': 100, 'user_ticks': 0, 'system_ticks': 0},
                      'after': {'pid': pid, 'start_ticks': 5, 'actual_cpus': [0, 1, 2, 3],
                                'clock_ticks_per_second': 100, 'user_ticks': 100, 'system_ticks': 0},
                      'metrics': metrics, 'server_cpu_seconds': 1,
                      'generator': G['read_json'](directory / 'measured/command.json')}
            G['save'](directory / 'sample.json', record)
            manifest['normal_samples'].append(record)
        (root / 'summary.md').write_text(D['summarize'](manifest))
        self.persist(root, manifest)
        return manifest

    def persist(self, root, manifest):
        manifest['evidence_sha256'] = D['evidence_hashes'](root)
        G['save'](root / 'manifest.json', manifest)

    def test_a_complete_losing_cohort_is_valid_advisory_data_not_a_gate_pass(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            checked = D['check_study'](root)
            self.assertEqual(len(checked['normal_samples']), 18)
            self.assertNotIn('gate_passed', checked)

    def test_raw_corruption_duplicate_samples_stale_builds_and_affinity_fail(self):
        for change in ('raw', 'duplicate', 'stale', 'affinity', 'profile', 'binary', 'cpu', 'resources'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                manifest = self.fixture(root)
                if change == 'raw':
                    (root / 'normal/r1-baseline-p1/measured/stdout.log').write_text('corrupt')
                elif change == 'duplicate':
                    manifest['normal_samples'][-1] = manifest['normal_samples'][0]
                    self.persist(root, manifest)
                elif change == 'stale':
                    (root / 'build/candidate/stderr.log').write_text('Finished release profile in 0.01s')
                    self.persist(root, manifest)
                elif change == 'affinity':
                    manifest['normal_samples'][0]['before']['actual_cpus'] = [0]
                    G['save'](root / 'normal/r1-baseline-p1/sample.json', manifest['normal_samples'][0])
                    self.persist(root, manifest)
                elif change == 'profile':
                    manifest['profile_samples'].append({'mode': 'cpu', 'variant': 'candidate', 'valid': True})
                    self.persist(root, manifest)
                elif change in ('cpu', 'resources'):
                    record = manifest['normal_samples'][0]
                    if change == 'cpu':
                        record['server_cpu_seconds'] = 99
                    else:
                        record['generator']['generator_user_seconds'] = 99
                    G['save'](root / 'normal/r1-baseline-p1/sample.json', record)
                    self.persist(root, manifest)
                else:
                    (root / 'frozen-binaries/candidate').write_text('changed')
                    self.persist(root, manifest)
                with self.assertRaises((AssertionError, ValueError)):
                    D['check_study'](root)


class ReferenceCacheTests(unittest.TestCase):
    def exercise(self, damage=None):
        function = D['build_reference']
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'benchmarks').mkdir()
            (root / 'Makefile').write_text('synthetic makefile')
            (root / 'benchmarks/targets.mk').write_text('synthetic pins')
            payload = b'synthetic source archive'
            pin = {'version': '9.1.2', 'sha256': D['hashlib'].sha256(payload).hexdigest(), 'url': 'unused'}
            archive = root / f'target/upstream-downloads/valkey-9.1.2-{pin["sha256"]}.tar.gz'
            archive.parent.mkdir(parents=True)
            archive.write_bytes(payload)
            frozen = root / 'frozen'
            frozen.mkdir()
            def checked(argv, directory, **_):
                directory.mkdir(parents=True, exist_ok=True)
                if argv[0] == 'make':
                    source = root / 'target/competitors/valkey-9.1.2/src'
                    source.mkdir(parents=True, exist_ok=True)
                    for name in ('valkey-server', 'valkey-benchmark'):
                        (source / name).write_text(name)
                if '--version' in argv:
                    (directory / 'stdout.log').write_text('valkey-benchmark 9.1.2\n')
                return {'exit_code': 0}
            with patch.dict(function.__globals__, {'ROOT': root, 'OUT': root / 'evidence', 'BIN': frozen}), \
                 patch.dict(H, {'probe': Mock(return_value=contract())}), \
                 patch.object(D['COMMON'], 'PINS', {'upstreams': {'valkey': pin}}), \
                 patch.object(D['COMMON'], 'checked', side_effect=checked) as command:
                function()
                self.assertEqual(sum(call.args[0][0] == 'make' for call in command.call_args_list), 2)
                cache = next((root / 'target/dev-reference-cache').iterdir())
                if damage:
                    (cache / 'generator').write_text('corrupted cached binary')
                    with self.assertRaisesRegex(AssertionError, 'cached reference binary changed'):
                        function()
                    self.assertEqual((cache / 'generator').read_text(), 'corrupted cached binary')
                else:
                    function()
                    self.assertEqual(sum(call.args[0][0] == 'make' for call in command.call_args_list), 2)
                    self.assertTrue(G['read_json'](root / 'evidence/reference-identity.json')['cache_reused'])

    def test_verified_unchanged_reference_is_not_rebuilt(self):
        self.exercise()

    def test_corrupted_reference_fails_without_rebuilding_or_erasing_it(self):
        self.exercise(damage=True)


class DevelopmentWiringTests(unittest.TestCase):
    def test_fast_target_does_not_replace_the_full_ci_graph(self):
        fast = subprocess.check_output(['make', '--no-print-directory', '-n', 'dev-check'], cwd=ROOT, text=True)
        full = subprocess.check_output(['make', '--no-print-directory', '-n', 'ci'], cwd=ROOT, text=True)
        self.assertEqual(fast.count('cargo test --locked --offline --lib'), 1)
        self.assertIn('cargo clippy --locked --offline --all-targets --all-features -- -D warnings', fast)
        self.assertNotIn('for workers in 0 1 4', fast)
        self.assertIn('for workers in 0 1 4', full)
        self.assertIn('taskset', full)
        self.assertIn('rillcache-bench', full)
        self.assertIn('cargo bench --locked --offline --bench core', full)

    def test_study_is_manual_read_only_isolated_and_input_values_are_not_shell_code(self):
        workflow = (ROOT / '.github/workflows/upstream-performance.yml').read_text()
        self.assertIn('workflow_dispatch:', workflow)
        self.assertIn('contents: read', workflow)
        self.assertNotIn('contents: write', workflow)
        self.assertNotIn('pull_request_target:', workflow)
        study = workflow.split('\n  study:\n')[1]
        self.assertIn('if: inputs.study', study)
        self.assertIn('runs-on: [self-hosted, Linux, X64, rillcache-perf]', study)
        self.assertIn('persist-credentials: false', study)
        self.assertIn('if: always()', study)
        self.assertIn('STUDY_BASE: ${{ inputs.study_base }}', study)
        self.assertIn('--base "$STUDY_BASE"', study)
        self.assertNotIn('--base ${{', study)
        self.assertIn('if [[ "$STUDY_PROFILES" == true ]]', study)


if __name__ == '__main__':
    unittest.main()
