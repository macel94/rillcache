#!/usr/bin/env python3
"""Profiles only: no repeated normal measurements or required-gate verdict."""
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('selector_diagnostic', ROOT / 'scripts/pr154-selector-study.py')
S = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = S
spec.loader.exec_module(S)
G, N = S.G, S.N
S.OUT = ROOT / 'target/owned-profiles'


def main():
    assert os.environ.get('GITHUB_ACTIONS') == 'true'
    assert os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch'
    S.OUT.mkdir(parents=True, exist_ok=False)
    m = {'diagnostic_only': True, 'instrumented_only': True, 'complete': False,
         'sources': {'failed': S.SOURCES['failed']}, 'binaries': {}, 'profiles': []}
    try:
        hardware = N.HARDWARE['probe']()
        N.validate_build_environment(hardware)
        G.save(S.OUT / 'hardware.json', hardware)
        cpus, clients = G.placement(sorted(os.sched_getaffinity(0)))
        m.update(server_cpus=cpus, client_cpus=clients)
        S.build(m['binaries'], variants=('failed',))
        hashes = {v: G.digest(p) for v, p in m['binaries'].items()}
        m['binary_sha256'] = hashes
        perf, event = N.perf_tool()
        m.update(perf=perf, cpu_event=event)
        for variant in ('failed', 'dragonfly'):
            for mode in ('syscalls', 'cpu'):
                folder = S.OUT / f'{variant}-{mode}'
                folder.mkdir()
                role = 'dragonfly' if variant == 'dragonfly' else 'rillcache'
                with G.owned_server(m['binaries'][variant], role, cpus, folder) as (server, port):
                    argv = G.generator_argv(m['binaries']['generator'], port, 'ping', 1, None, clients)
                    argv[argv.index('--duration') + 1] = '20'
                    metadata = N.measured_load(argv, folder / 'measured', server, mode, perf, event)
                    if mode == 'cpu':
                        for who in ('server', 'generator'):
                            for children in (False, True):
                                N.cpu_report(perf, folder / 'measured' / f'{who}.perf.data',
                                             folder / f'{who}-{children}-cpu.txt', children)
                    m['profiles'].append({'variant': variant, 'mode': mode, 'directory': str(folder.name),
                                           'server_pid': server.pid, 'generator_pid': metadata['pid']})
                G.save(S.OUT / 'manifest.json', m)
        assert all(G.digest(path) == hashes[v] for v, path in m['binaries'].items())
        m['complete'] = True
    except BaseException as error:
        m['error'] = repr(error)
        raise
    finally:
        m['evidence_sha256'] = {str(p.relative_to(S.OUT)): G.digest(p) for p in S.OUT.rglob('*')
                               if p.is_file() and p.name != 'manifest.json'}
        G.save(S.OUT / 'manifest.json', m)


if __name__ == '__main__':
    main()
