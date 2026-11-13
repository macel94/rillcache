"""Pure synthetic diagnostic fixtures, no native servers or performance load."""
import hashlib
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import common

spec = importlib.util.spec_from_file_location('string_study', common.ROOT / 'scripts/string-mutation-study.py')
S = importlib.util.module_from_spec(spec)
spec.loader.exec_module(S)


def fixture(root):
    cpus = [0, 1, 2, 3]
    hardware = {'selected_cpus': cpus, 'rust': f'rustc {common.PINS["rust"]} (synthetic)', 'build_environment': {}}
    m = dict(complete=True, diagnostic_only=True, protected_gate=False, instrumented=False,
             study='string-controls-v2', base_source='a'*40, candidate_source='b'*40,
             runtime_candidate='c'*40, hardware=hardware,
             hardware_fingerprint=S.D.HARDWARE['fingerprint'](hardware),
             launch_directory=str(root), binaries={}, binary_sha256={})
    (root / 'frozen-binaries').mkdir()
    for name in (*S.VARIANTS, 'generator'):
        file = root / 'frozen-binaries' / name
        file.write_text('non-executable synthetic fixture ' + name)
        m['binaries'][name] = str(file)
        m['binary_sha256'][name] = common.digest(file)
    samples = []
    expected = {case: S.expected_value(case) for case in S.CASES}
    for index, identity in enumerate(S.identities()):
        case, mode, round_number, variant = identity
        server_cpus, client_cpus = S.affinities(mode, cpus)
        path = root / S.name_for(*identity)
        path.mkdir()
        pid, port = 1000 + index, 20000 + index
        before = dict(pid=pid, start_ticks=10, actual_cpus=server_cpus,
                      user_ticks=0, system_ticks=0, clock_ticks_per_second=100)
        times = [100000] * case[4]
        s = dict(valid=True, instrumented=False, case=list(case), mode=mode, round=round_number,
                 variant=variant, port=port, before=before, after=before,
                 generator_before=dict(before, pid=999, actual_cpus=client_cpus),
                 generator_after=dict(before, pid=999, actual_cpus=client_cpus),
                 latencies_ns=times, replies=[S.expected_reply(case, i) for i in range(case[4])],
                 metrics=S.percentiles(times), final_size=len(expected[case]),
                 actual_sha256=hashlib.sha256(expected[case]).hexdigest(),
                 expected_sha256=hashlib.sha256(expected[case]).hexdigest())
        common.save(path / 'sample.json', s)
        kind = 'valkey' if variant == 'valkey' else 'rillcache'
        common.save(path / 'server.json', dict(pid=pid, port=port, joined=True,
            exit_before_cleanup=None, exit_after_cleanup=-15,
            argv=S.G['server_argv'](m['binaries'][variant], kind, port, server_cpus, path)))
        samples.append(s)
    common.save(root / 'sample-index.json', S.sample_index(root))
    (root / 'summary.md').write_text(S.report(samples))
    m['evidence_sha256'] = S.D.evidence_hashes(root)
    common.save(root / 'manifest.json', m)
    return m


class StringStudyTests(unittest.TestCase):
    def test_balanced_complete_36_sample_scope(self):
        ids = list(S.identities())
        self.assertEqual(36, len(ids))
        for case in S.CASES:
            order = [identity[3] for identity in ids if identity[0] == case and identity[1] == S.MODES[0]]
            self.assertEqual(['baseline', 'candidate', 'valkey', 'candidate', 'valkey', 'baseline',
                              'valkey', 'baseline', 'candidate'], order)
        with patch.dict(os.environ, {'GITHUB_ACTIONS': ''}), patch('sys.argv', ['study']):
            with self.assertRaises(AssertionError): S.main()

    def test_semantics_percentiles_and_first_growth_included(self):
        self.assertEqual(S.expected_reply(S.CASES[0], 0), 72)
        self.assertEqual(len(S.expected_value(S.CASES[0])), 64 + 8*200)
        self.assertEqual(S.expected_value(S.CASES[1])[:8], bytes(range(8)))
        self.assertEqual(S.command_for(S.CASES[-1]), ('PING',))
        self.assertEqual(S.expected_reply(S.CASES[0], 200), 72)
        self.assertEqual(S.affinities('disjoint-one', [0, 1, 2, 3]), ([0], [1, 2, 3]))
        result = S.percentiles([10000000] + [100000]*99)
        self.assertEqual(result, dict(p50_ms=.1, p99_ms=.1, max_ms=10.0))
        for invalid in ([], [0], [-1], [float('nan')]):
            with self.assertRaises(AssertionError): S.percentiles(invalid)

    def test_complete_synthetic_evidence_and_rehashed_corruption(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            m = fixture(root)
            self.assertEqual(len(S.check(root)), 36)
            path = root / S.name_for(*next(S.identities())) / 'sample.json'
            sample = S.G['read_json'](path)
            sample['replies'][0] += 1
            common.save(path, sample)
            m['evidence_sha256'] = S.D.evidence_hashes(root)
            common.save(root / 'manifest.json', m)
            with self.assertRaises(AssertionError): S.check(root)

    def test_changed_raw_evidence_or_missing_binary_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            fixture(root)
            (root / 'frozen-binaries/candidate').write_text('changed')
            with self.assertRaises(AssertionError): S.check(root)


if __name__ == '__main__':
    unittest.main()
