#!/usr/bin/env python3
"""One matched hosted completion-I/O screen, never protected acceptance."""
import contextlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import sys
import tarfile

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/compio-study'
SOURCES = {'main': 'eb0d387b870bb42e2c17b503976c965e2e6b3e1e',
           'failed': 'd7ec5b3f132e30fb222b31e3f6e5e48dfcf9d78f',
           'candidate': 'bf0ed14da3f14f7a5b77ee98d437e370747ad9ed'}
VARIANTS = ('main', 'failed', 'candidate', 'dragonfly')
CASES = (('ping', 1), ('ping', 16), ('get', 1), ('get', 16))
MARKER = 'rillcache: plain TCP compio io_uring; one local socket/data owner'


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


G = load('compio_dragonfly', 'pr-dragonfly-performance.py')
N = load('compio_native', 'dev-performance.py')
C = N.COMMON
ORIGINAL_SERVER = G.owned_server


def process_scope(pid, cpus):
    tasks = {}
    for path in Path(f'/proc/{pid}/task').iterdir():
        try:
            tasks[path.name] = {'cpus': sorted(os.sched_getaffinity(int(path.name))),
                                'name': (path / 'comm').read_text().strip()}
        except (FileNotFoundError, ProcessLookupError):
            continue  # A finished thread is not a live mask observation.
    assert tasks and all(set(t['cpus']) <= set(cpus) for t in tasks.values()), 'thread escaped CPU budget'
    rings = 0
    for fd in Path(f'/proc/{pid}/fd').iterdir():
        try:
            rings += 'io_uring' in os.readlink(fd)
        except FileNotFoundError:
            continue
    return {'tasks': tasks, 'io_uring_fds': rings}


@contextlib.contextmanager
def observed_server(binary, role, cpus, directory):
    with ORIGINAL_SERVER(binary, role, cpus, directory) as (proc, port):
        before = process_scope(proc.pid, cpus)
        candidate = Path(binary).name == 'candidate'
        if candidate:
            assert before['io_uring_fds'] >= 1, 'candidate silently fell back from completion I/O'
            assert len(before['tasks']) >= 2, 'network owner and cold service must be accounted'
            assert MARKER in (directory / 'server.log').read_text()
        try:
            yield proc, port
        finally:
            G.save(directory / 'thread-scope.json', {'before': before, 'after': process_scope(proc.pid, cpus),
                                                    'completion_required': candidate})


def order():
    for workload, pipeline in CASES:
        for run in range(4):
            for variant in VARIANTS[run:] + VARIANTS[:run]:
                yield workload, pipeline, run + 1, variant


def rows(records):
    table = []
    for workload, pipeline in CASES:
        row = {'workload': workload, 'pipeline': pipeline}
        for variant in VARIANTS:
            rates = [r['result']['measured']['rps'] for r in records
                     if (r['workload'], r['pipeline'], r['variant']) == (workload, pipeline, variant)]
            assert len(rates) == 4
            row[variant] = {'median': statistics.median(rates), 'min': min(rates), 'max': max(rates)}
        row['candidate_vs_failed'] = row['candidate']['median'] / row['failed']['median']
        table.append(row)
    return table


def summary(records):
    table = rows(records)
    text = ['# Advisory matched Compio study — not merge acceptance', '',
            'One allocated hosted runner; identical four logical server CPUs and official generator.',
            'Four balanced Latin rotations, production release flags, no normal instrumentation.',
            'No CPU-family selection or cross-host causal/universal claim.', '',
            '| Workload | P | Main | Failed | Candidate | Dragonfly | Candidate/failed |',
            '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    for row in table:
        text.append('| ' + ' | '.join([row['workload'], str(row['pipeline']),
                    *[f"{row[v]['median']:.2f} [{row[v]['min']:.2f}–{row[v]['max']:.2f}]" for v in VARIANTS],
                    f"{row['candidate_vs_failed']:.4f}"]) + ' |')
    passed = table[0]['candidate_vs_failed'] >= 1.10 and all(r['candidate_vs_failed'] >= .97 for r in table[1:])
    text += ['', f'Predeclared screen: {"PASS" if passed else "REJECT"} (PING/P1 ≥10% gain; controls no >3% loss).',
             'All40 protected comparisons/all14 required contexts remain unchanged and separate.']
    return '\n'.join(text) + '\n'


def check(directory):
    manifest = G.read_json(directory / 'manifest.json')
    assert manifest['diagnostic_only'] and manifest['normal_complete'] and manifest['complete']
    assert manifest['sources'] == SOURCES
    assert [(r['workload'], r['pipeline'], r['run'], r['variant']) for r in manifest['records']] == list(order())
    for relative, digest in manifest['evidence_sha256'].items():
        assert '..' not in Path(relative).parts and not Path(relative).is_absolute()
        assert G.digest(directory / relative) == digest
    for variant in SOURCES:
        G.validate_candidate_version((directory / f'{variant}-version.txt').read_text().strip())
        assert 'Compiling rillcache v' in (directory / 'build' / variant / 'stderr.log').read_text()
    for record in manifest['records']:
        folder = directory / record['directory']
        sample, server = G.read_json(folder / 'sample.json'), G.read_json(folder / 'server.json')
        assert sample == record['result'] and sample['valid'] and server['joined']
        role = 'dragonfly' if record['variant'] == 'dragonfly' else 'rillcache'
        assert server['argv'] == G.server_argv(manifest['binaries'][record['variant']], role,
                                              server['port'], manifest['server_cpus'], folder)
        assert server['actual_cpus_before'] == server['actual_cpus_after'] == manifest['server_cpus']
        scope = G.read_json(folder / 'thread-scope.json')
        for point in ('before', 'after'):
            assert all(set(t['cpus']) <= set(manifest['server_cpus']) for t in scope[point]['tasks'].values())
        if record['variant'] == 'candidate':
            assert scope['completion_required'] and MARKER in (folder / 'server.log').read_text()
            assert all(scope[p]['io_uring_fds'] >= 1 and len(scope[p]['tasks']) >= 2 for p in ('before', 'after'))
        for phase, count in [('warmup', G.WARMUP), ('measured', None)]:
            argv = G.generator_argv(manifest['binaries']['generator'], server['port'], record['workload'],
                                   record['pipeline'], count, manifest['client_cpus'])
            assert G.read_generator(folder / phase, record['workload'], argv) == sample[phase]
    assert (directory / 'summary.md').read_text() == summary(manifest['records'])
    return manifest


def build(binaries, variants=None):
    tree = Path(os.environ['RUNNER_TEMP']) / 'compio-app-source'
    tree.mkdir()
    N.OUT = OUT
    N.BIN = Path(os.environ['RUNNER_TEMP']) / 'compio-frozen'
    N.BIN.mkdir()
    for variant, source in SOURCES.items():
        if variants is not None and variant not in variants:
            continue
        assert subprocess.check_output(['git', 'rev-parse', source + '^{commit}'], cwd=ROOT).decode().strip() == source
        assert not subprocess.check_output(['git', 'diff', '--diff-filter=D', '--name-only',
                                            SOURCES['main'], source], cwd=ROOT), 'archive overlay has deletions'
        archive = subprocess.Popen(['git', 'archive', source], cwd=ROOT, stdout=subprocess.PIPE)
        try:
            subprocess.run(['tar', '-xf', '-', '-C', str(tree)], stdin=archive.stdout, check=True, timeout=120)
        finally:
            archive.stdout.close()
            assert archive.wait(timeout=30) == 0
        N.compile_application(tree, variant)  # package-clean and demand actual app compilation
        path = N.BIN / variant
        shutil.copy2(tree / 'target/release/rillcache', path)
        binaries[variant] = str(path)
        version = subprocess.check_output([str(path), '--version'], env=N.SAFE_ENV).decode().strip()
        G.validate_candidate_version(version)
        (OUT / f'{variant}-version.txt').write_text(version + '\n')
    N.build_reference()
    binaries['generator'] = str(N.BIN / 'generator')
    targets = (ROOT / 'benchmarks/targets.mk').read_text()
    version = re.search(r'^DRAGONFLY_VERSION := (\S+)$', targets, re.M)[1]
    expected = re.search(r'^DRAGONFLY_SHA256 := ([0-9a-f]{64})$', targets, re.M)[1]
    url = re.search(r'^DRAGONFLY_URL := (\S+)$', targets, re.M)[1].replace('$(DRAGONFLY_VERSION)', version)
    archive = N.BIN / 'dragonfly.tar.gz'
    C.checked(['curl', '--proto', '=https', '--tlsv1.2', '--fail', '--location', '--show-error',
               '--connect-timeout', '10', '--max-time', '180', url, '-o', str(archive)], OUT / 'dragonfly-download')
    assert G.digest(archive) == expected
    extracted = N.BIN / 'dragonfly-release'
    extracted.mkdir()
    with tarfile.open(archive) as packed:
        packed.extractall(extracted, filter='data')
    executables = [p for p in extracted.rglob('dragonfly*') if p.is_file() and os.access(p, os.X_OK)]
    assert len(executables) == 1
    binaries['dragonfly'] = str(executables[0])
    actual = subprocess.check_output([str(executables[0]), '--version'], env=N.SAFE_ENV).decode().strip()
    assert re.search(r'dragonfly\s+v?' + re.escape(version) + r'(?:-|\s|$)',
                     re.sub(r'\x1b\[[0-9;]*m', '', actual), re.I)
    (OUT / 'dragonfly-version.txt').write_text(actual + '\n')
    shutil.copy2(ROOT / 'benchmarks/notices/Dragonfly-BSL-1.1.txt', OUT / 'Dragonfly-BSL-1.1.txt')
    G.save(OUT / 'dragonfly-pin.json', {'version': version, 'url': url, 'archive_sha256': expected})


def main():
    if sys.argv[1:] == ['--self-test']:
        identities = list(order())
        assert len(identities) == len(set(identities)) == 64
        for case in CASES:
            rotations = [[v for w, p, r, v in identities if (w, p, r) == (*case, run)] for run in range(1, 5)]
            assert all(set(turn) == set(VARIANTS) for turn in zip(*rotations))
        records = [dict(workload=w, pipeline=p, run=r, variant=v,
                        result={'measured': {'rps': 115 if v == 'candidate' else 100}})
                   for w, p, r, v in identities]
        assert 'screen: PASS' in summary(records)
        records[0]['result']['measured']['rps'] = 1000000
        assert 'screen: PASS' in summary(records)  # no fastest-sample selection
        for record in records:
            if record['variant'] == 'candidate' and record['pipeline'] == 16:
                record['result']['measured']['rps'] = 96
        assert 'screen: REJECT' in summary(records)
        print('Balanced order, strict screen and losing-control rejection pass.')
        return
    if len(sys.argv) == 3 and sys.argv[1] == '--check':
        print(summary(check(Path(sys.argv[2]))['records']))
        return
    assert not sys.argv[1:]
    assert os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch'
    OUT.mkdir(parents=True, exist_ok=False)
    manifest = {'diagnostic_only': True, 'complete': False, 'normal_complete': False,
                'records': [], 'profiles_requested': False, 'sources': SOURCES, 'binaries': {}}
    try:
        hardware = N.HARDWARE['probe']()
        N.validate_build_environment(hardware)
        G.save(OUT / 'hardware.json', hardware)
        cpus, clients = G.placement(sorted(os.sched_getaffinity(0)))
        manifest.update(server_cpus=cpus, client_cpus=clients, shared_clients=bool(set(cpus) & set(clients)))
        build(manifest['binaries'])
        hashes = {v: G.digest(p) for v, p in manifest['binaries'].items()}
        assert len(set(hashes.values())) == len(hashes)
        manifest['binary_sha256'] = hashes
        G.owned_server = observed_server
        for workload, pipeline, run, variant in order():
            name = f'{workload}-p{pipeline}-r{run}-{variant}'
            print('normal', name, flush=True)
            role = 'dragonfly' if variant == 'dragonfly' else 'rillcache'
            G.sample({role: manifest['binaries'][variant], 'generator': manifest['binaries']['generator']},
                     role, workload, pipeline, cpus, clients, OUT / name)
            manifest['records'].append(dict(workload=workload, pipeline=pipeline, run=run, variant=variant,
                                            directory=name, result=G.read_json(OUT / name / 'sample.json')))
            G.save(OUT / 'manifest.json', manifest)
        assert all(G.digest(p) == hashes[v] for v, p in manifest['binaries'].items())
        manifest['normal_complete'] = manifest['complete'] = True
        (OUT / 'summary.md').write_text(summary(manifest['records']))
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
