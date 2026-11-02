#!/usr/bin/env python3
"""One hosted matched owner-turn screen, not protected performance acceptance."""
import contextlib
import importlib.util
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import sys
import tarfile

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/owner-study'
SOURCES = {'baseline': 'd7ec5b3f132e30fb222b31e3f6e5e48dfcf9d78f',
           'candidate': '868901bbaab0564524ea297cf66de8762b9a7522'}
VARIANTS = ('baseline', 'mio', 'candidate', 'dragonfly')
CASES = (('ping', 1), ('ping', 16), ('get', 1), ('get', 16))


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


G = load('owner_dragonfly', 'pr-dragonfly-performance.py')
N = load('owner_native', 'dev-performance.py')
C = N.COMMON
ORIGINAL_SERVER = G.owned_server
ORIGINAL_ARGV = G.server_argv


def server_argv(binary, role, port, cpus, directory):
    argv = ORIGINAL_ARGV(binary, role, port, cpus, directory)
    if Path(binary).name == 'mio':
        assert role == 'rillcache'
        argv += ['--io-mode', 'mio']
    return argv


G.server_argv = server_argv


def process_scope(pid, cpus):
    tasks = {}
    for path in Path(f'/proc/{pid}/task').iterdir():
        try:
            tasks[path.name] = {'cpus': sorted(os.sched_getaffinity(int(path.name))),
                                'name': (path / 'comm').read_text().strip()}
        except (FileNotFoundError, ProcessLookupError):
            continue
    assert tasks and all(set(t['cpus']) <= set(cpus) for t in tasks.values()), 'thread escaped CPU budget'
    return tasks


@contextlib.contextmanager
def observed_server(binary, role, cpus, directory):
    with ORIGINAL_SERVER(binary, role, cpus, directory) as (proc, port):
        before = process_scope(proc.pid, cpus)
        networks = None
        if Path(binary).name == 'candidate':
            match = re.search(r'Tokio topology networks=(\d+) local_data_owner=true owner_turn_capacity=32',
                              (directory / 'server.log').read_text())
            assert match, 'default did not select the intended complete local-owner transport'
            networks = int(match[1])
            assert 2 <= networks <= len(cpus)
            assert len(before) == networks + 1, 'must count network runtimes plus cold service, no extra data thread'
        elif role == 'rillcache':
            assert len(before) == 1, 'Mio controls must retain their single local owner'
        try:
            yield proc, port
        finally:
            after = process_scope(proc.pid, cpus)
            if networks is not None:
                assert len(after) == networks + 1
            G.save(directory / 'thread-scope.json', {'before': before, 'after': after,
                                                    'network_runtimes': networks, 'extra_data_owner_threads': 0 if networks else None})


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
        row['candidate_vs_baseline'] = row['candidate']['median'] / row['baseline']['median']
        row['candidate_vs_mio'] = row['candidate']['median'] / row['mio']['median']
        table.append(row)
    return table


def summary(records):
    table = rows(records)
    text = ['# Advisory matched asynchronous-owner screen — not merge acceptance', '',
            'One allocated hosted runner; same four logical server CPUs and official generator.',
            'Four balanced Latin rotations; normal release flags and no instrumentation.',
            'Mio control is the EXACT candidate ELF with --io-mode mio; default candidate has no extra flag.',
            'No hardware-family selection, cross-host attribution or universal claim.', '',
            '| Workload | P | d7 Mio | Same-ELF Mio | Default candidate | Dragonfly | Candidate/d7 | Candidate/same-Mio |',
            '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
    for row in table:
        text.append('| ' + ' | '.join([row['workload'], str(row['pipeline']),
                    *[f"{row[v]['median']:.2f} [{row[v]['min']:.2f}–{row[v]['max']:.2f}]" for v in VARIANTS],
                    f"{row['candidate_vs_baseline']:.4f}", f"{row['candidate_vs_mio']:.4f}"]) + ' |')
    ratios = ('candidate_vs_baseline', 'candidate_vs_mio')
    passed = all(table[0][key] >= 1.10 for key in ratios) and all(row[key] >= .97 for row in table[1:] for key in ratios)
    text += ['', f'Predeclared screen: {"PASS" if passed else "REJECT"} (PING/P1 ≥10% vs both controls; no >3% control loss vs either).',
             'Every all40/all14 protected requirement remains separate, current-head and unchanged.']
    return '\n'.join(text) + '\n'


def check(directory):
    manifest = G.read_json(directory / 'manifest.json')
    assert manifest['diagnostic_only'] and manifest['normal_complete'] and manifest['complete']
    assert manifest['sources'] == SOURCES
    hashes = manifest['binary_sha256']
    assert hashes['mio'] == hashes['candidate'] and len(set(hashes.values())) == len(hashes) - 1
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
        assert server['argv'] == server_argv(manifest['binaries'][record['variant']], role,
                                            server['port'], manifest['server_cpus'], folder)
        assert server['actual_cpus_before'] == server['actual_cpus_after'] == manifest['server_cpus']
        scope = G.read_json(folder / 'thread-scope.json')
        for point in ('before', 'after'):
            assert all(set(t['cpus']) <= set(manifest['server_cpus']) for t in scope[point].values())
        if record['variant'] == 'candidate':
            networks = scope['network_runtimes']
            assert 2 <= networks <= len(manifest['server_cpus']) and scope['extra_data_owner_threads'] == 0
            assert all(len(scope[p]) == networks + 1 for p in ('before', 'after'))
            assert f'Tokio topology networks={networks} local_data_owner=true owner_turn_capacity=32' in (folder / 'server.log').read_text()
        for phase, count in [('warmup', G.WARMUP), ('measured', None)]:
            argv = G.generator_argv(manifest['binaries']['generator'], server['port'], record['workload'],
                                   record['pipeline'], count, manifest['client_cpus'])
            assert G.read_generator(folder / phase, record['workload'], argv) == sample[phase]
    assert (directory / 'summary.md').read_text() == summary(manifest['records'])
    return manifest


def build(binaries):
    tree = Path(os.environ['RUNNER_TEMP']) / 'owner-app-source'
    tree.mkdir()
    N.OUT = OUT
    N.BIN = Path(os.environ['RUNNER_TEMP']) / 'owner-frozen'
    N.BIN.mkdir()
    for variant, source in SOURCES.items():
        assert subprocess.check_output(['git', 'rev-parse', source + '^{commit}'], cwd=ROOT).decode().strip() == source
        assert not subprocess.check_output(['git', 'diff', '--diff-filter=D', '--name-only',
                                            SOURCES['baseline'], source], cwd=ROOT), 'archive overlay has deletions'
        archive = subprocess.Popen(['git', 'archive', source], cwd=ROOT, stdout=subprocess.PIPE)
        try:
            subprocess.run(['tar', '-xf', '-', '-C', str(tree)], stdin=archive.stdout, check=True, timeout=120)
        finally:
            archive.stdout.close()
            assert archive.wait(timeout=30) == 0
        N.compile_application(tree, variant)
        path = N.BIN / variant
        shutil.copy2(tree / 'target/release/rillcache', path)
        binaries[variant] = str(path)
        version = subprocess.check_output([str(path), '--version'], env=N.SAFE_ENV).decode().strip()
        G.validate_candidate_version(version)
        (OUT / f'{variant}-version.txt').write_text(version + '\n')
    binaries['mio'] = str(N.BIN / 'mio')
    shutil.copy2(binaries['candidate'], binaries['mio'])
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
        assert server_argv('/owned/mio', 'rillcache', 1234, [0, 1, 2, 3], Path('/owned'))[-2:] == ['--io-mode', 'mio']
        assert '--io-mode' not in server_argv('/owned/candidate', 'rillcache', 1234, [0, 1, 2, 3], Path('/owned'))
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
        assert 'screen: PASS' in summary(records)
        for record in records:
            if record['variant'] == 'mio': record['result']['measured']['rps'] = 120
        assert 'screen: REJECT' in summary(records), 'same-ELF control loss must reject'
        print('Balanced order, both baselines, medians and losing-control rejection pass.')
        return
    if len(sys.argv) == 3 and sys.argv[1] == '--check':
        print(summary(check(Path(sys.argv[2]))['records']))
        return
    assert not sys.argv[1:]
    assert os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch'
    OUT.mkdir(parents=True, exist_ok=False)
    manifest = {'diagnostic_only': True, 'complete': False, 'normal_complete': False,
                'records': [], 'profiles_requested': False, 'sources': SOURCES, 'binaries': {},
                'same_binary_control': 'mio and candidate share one ELF; mio adds only --io-mode mio'}
    try:
        hardware = N.HARDWARE['probe']()
        N.validate_build_environment(hardware)
        G.save(OUT / 'hardware.json', hardware)
        cpus, clients = G.placement(sorted(os.sched_getaffinity(0)))
        manifest.update(server_cpus=cpus, client_cpus=clients, shared_clients=bool(set(cpus) & set(clients)))
        build(manifest['binaries'])
        hashes = {v: G.digest(p) for v, p in manifest['binaries'].items()}
        assert hashes['candidate'] == hashes['mio'] and len(set(hashes.values())) == len(hashes) - 1
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
