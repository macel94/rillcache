#!/usr/bin/env python3
"""Separate owned-PID mechanism captures; never repeat normal screening."""
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('compio_study', ROOT / 'scripts/pr154-compio-study.py')
S = importlib.util.module_from_spec(spec)
spec.loader.exec_module(S)
G, N = S.G, S.N
OUT = ROOT / 'target/compio-profiles'
VARIANTS = ('failed', 'candidate', 'dragonfly')


def check(directory):
    manifest = G.read_json(directory / 'manifest.json')
    assert manifest['instrumented_only'] is True and manifest['normal_samples_requested'] is False
    assert manifest['complete'] is True
    assert [(c['variant'], c['mode']) for c in manifest['captures']] == [(v, m) for v in VARIANTS for m in ('syscalls', 'cpu')]
    assert manifest['sources'] == {v: S.SOURCES[v] for v in ('failed', 'candidate')}
    for relative, digest in manifest['evidence_sha256'].items():
        assert not Path(relative).is_absolute() and '..' not in Path(relative).parts
        assert G.digest(directory / relative) == digest
    for capture in manifest['captures']:
        folder = directory / capture['directory']
        server = G.read_json(folder / 'server.json')
        assert server['joined'] is True
        assert server['actual_cpus_before'] == server['actual_cpus_after'] == manifest['server_cpus']
        role = 'dragonfly' if capture['variant'] == 'dragonfly' else 'rillcache'
        assert server['argv'] == G.server_argv(manifest['binaries'][capture['variant']], role, server['port'], manifest['server_cpus'], folder)
        scope = G.read_json(folder / 'thread-scope.json')
        if capture['variant'] == 'candidate':
            assert S.MARKER in (folder / 'server.log').read_text()
            assert all(scope[p]['io_uring_fds'] >= 1 for p in ('before', 'after'))
        argv = G.generator_argv(manifest['binaries']['generator'], server['port'], 'ping', 1, None, manifest['client_cpus'])
        argv[argv.index('--duration') + 1] = '20'
        G.read_generator(folder / 'measured', 'ping', argv)
        metadata = G.read_json(folder / 'measured/command.json')
        assert metadata['server_at_capture']['pid'] == server['pid']
        for who, pid in [('server', server['pid']), ('generator', metadata['pid'])]:
            profiler = G.read_json(folder / 'measured' / f'{who}-profiler.json')
            assert profiler['exit_code'] == 0
            assert profiler['argv'][profiler['argv'].index('-p') + 1] == str(pid)
        if capture['mode'] == 'cpu':
            assert (folder / 'server-cpu.txt').stat().st_size > 0
            assert (folder / 'measured/server.perf.data').stat().st_size > 0
    return manifest


def main():
    if len(sys.argv) == 3 and sys.argv[1] == '--check':
        check(Path(sys.argv[2]))
        print('All six separate owned-process captures, hashes, identities and CPU scopes verified.')
        return
    assert not sys.argv[1:]
    assert os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch'
    OUT.mkdir(parents=True, exist_ok=False)
    manifest = {'instrumented_only': True, 'normal_samples_requested': False, 'complete': False,
                'sources': {v: S.SOURCES[v] for v in ('failed', 'candidate')}, 'binaries': {}, 'captures': []}
    try:
        hardware = N.HARDWARE['probe']()
        N.validate_build_environment(hardware)
        G.save(OUT / 'hardware.json', hardware)
        cpus, clients = G.placement(sorted(os.sched_getaffinity(0)))
        manifest.update(server_cpus=cpus, client_cpus=clients, shared_clients=bool(set(cpus) & set(clients)))
        S.OUT = OUT
        S.build(manifest['binaries'], variants=('failed', 'candidate'))
        hashes = {v: G.digest(p) for v, p in manifest['binaries'].items()}
        assert len(set(hashes.values())) == len(hashes)
        manifest['binary_sha256'] = hashes
        G.owned_server = S.observed_server
        perf, event = N.perf_tool()
        N.COMMON.checked(['sudo', '-n', perf, 'stat', '-e', 'io_uring:io_uring_submit_req',
                          '--filter', 'opcode == 26', '--', 'sleep', '0.01'], OUT / 'uring-tracepoint-probe',
                         timeout=20, env=N.SAFE_ENV)
        manifest.update(perf=perf, cpu_event=event)
        for variant in VARIANTS:
            for mode in ('syscalls', 'cpu'):
                name = f'{variant}-{mode}'
                print('separate instrumented capture', name, flush=True)
                folder = OUT / name
                folder.mkdir()
                role = 'dragonfly' if variant == 'dragonfly' else 'rillcache'
                with G.owned_server(manifest['binaries'][variant], role, cpus, folder) as (server, port):
                    argv = G.generator_argv(manifest['binaries']['generator'], port, 'ping', 1, None, clients)
                    argv[argv.index('--duration') + 1] = '20'
                    N.measured_load(argv, folder / 'measured', server, mode, perf, event)
                    if mode == 'cpu':
                        for who in ('server', 'generator'):
                            N.cpu_report(perf, folder / 'measured' / f'{who}.perf.data', folder / f'{who}-cpu.txt', True)
                            N.cpu_report(perf, folder / 'measured' / f'{who}.perf.data', folder / f'{who}-self.txt', False)
                manifest['captures'].append({'variant': variant, 'mode': mode, 'directory': name})
                G.save(OUT / 'manifest.json', manifest)
        assert all(G.digest(p) == hashes[v] for v, p in manifest['binaries'].items())
        manifest['complete'] = True
    except BaseException as error:
        manifest['error'] = repr(error)
        raise
    finally:
        manifest['evidence_sha256'] = {str(p.relative_to(OUT)): G.digest(p) for p in OUT.rglob('*')
                                       if p.is_file() and p.name != 'manifest.json'}
        G.save(OUT / 'manifest.json', manifest)
    check(OUT)


if __name__ == '__main__':
    main()
