#!/usr/bin/env python3
"""One hosted, matched diagnostic; never a required gate or an unchanged retry."""
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
OUT = ROOT / 'target/selector-study'
SOURCES = {'main': 'eb0d387b870bb42e2c17b503976c965e2e6b3e1e',
           'failed': 'd7ec5b3f132e30fb222b31e3f6e5e48dfcf9d78f',
           'candidate': '76c224c2d53150c75a66bc97878e8a7d61e1a879'}
VARIANTS = ('main', 'failed', 'candidate', 'dragonfly')
CASES = (('ping', 1), ('ping', 16), ('get', 1), ('get', 16))


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


G = load('selector_dragonfly', 'pr-dragonfly-performance.py')
N = load('selector_native', 'dev-performance.py')
C = N.COMMON


def order():
    for workload, pipeline in CASES:
        for run in range(4):
            for variant in VARIANTS[run:] + VARIANTS[:run]:
                yield workload, pipeline, run + 1, variant


def rows(records):
    result = []
    for workload, pipeline in CASES:
        row = {'workload': workload, 'pipeline': pipeline}
        for variant in VARIANTS:
            rates = [r['result']['measured']['rps'] for r in records
                     if (r['workload'], r['pipeline'], r['variant']) == (workload, pipeline, variant)]
            assert len(rates) == 4
            row[variant] = {'median': statistics.median(rates), 'min': min(rates), 'max': max(rates)}
        row['candidate_vs_failed'] = row['candidate']['median'] / row['failed']['median']
        result.append(row)
    return result


def summary(records):
    table = rows(records)
    text = ['# Advisory matched selector study — not merge acceptance', '',
            'One allocated hosted runner, identical four logical server CPUs; same official generator.',
            'Four Latin-rotation repetitions; all normal samples precede separate instrumentation.',
            'No hardware-family selection or universal/causal cross-host claim.', '',
            '| Workload | P | Main | Failed | Candidate | Dragonfly | Candidate/failed |',
            '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    for row in table:
        text.append('| ' + ' | '.join([row['workload'], str(row['pipeline']),
                    *[f"{row[v]['median']:.2f} [{row[v]['min']:.2f}–{row[v]['max']:.2f}]" for v in VARIANTS],
                    f"{row['candidate_vs_failed']:.4f}"]) + ' |')
    accepted = table[0]['candidate_vs_failed'] >= 1.03 and all(r['candidate_vs_failed'] >= .97 for r in table[1:])
    text += ['', f'Predeclared screen: {"PASS" if accepted else "REJECT"} (PING/P1 ≥3% gain; controls no >3% loss).',
             'Required all-40 comparisons and all-14 contexts remain separate and unchanged.']
    return '\n'.join(text) + '\n'


def check(directory):
    manifest = G.read_json(directory / 'manifest.json')
    assert manifest['diagnostic_only'] is True and manifest['normal_complete'] is True
    assert [(r['workload'], r['pipeline'], r['run'], r['variant']) for r in manifest['records']] == list(order())
    for relative, digest in manifest['evidence_sha256'].items():
        assert '..' not in Path(relative).parts and not Path(relative).is_absolute()
        assert G.digest(directory / relative) == digest
    for record in manifest['records']:
        folder = directory / record['directory']
        sample = G.read_json(folder / 'sample.json')
        server = G.read_json(folder / 'server.json')
        assert sample == record['result'] and sample['valid'] is True and server['joined'] is True
        role = 'dragonfly' if record['variant'] == 'dragonfly' else 'rillcache'
        binary = manifest['binaries'][record['variant']]
        assert server['argv'] == G.server_argv(binary, role, server['port'], manifest['server_cpus'], folder)
        assert server['actual_cpus_before'] == server['actual_cpus_after'] == manifest['server_cpus']
        for phase, count in [('warmup', G.WARMUP), ('measured', None)]:
            argv = G.generator_argv(manifest['binaries']['generator'], server['port'], record['workload'],
                                   record['pipeline'], count, manifest['client_cpus'])
            assert G.read_generator(folder / phase, record['workload'], argv) == sample[phase]
    assert (directory / 'summary.md').read_text() == summary(manifest['records'])
    return manifest


def build(binaries, variants=None):
    tree = Path(os.environ['RUNNER_TEMP']) / 'selector-app-source'
    tree.mkdir()
    N.OUT = OUT
    N.BIN = Path(os.environ['RUNNER_TEMP']) / 'selector-frozen'
    N.BIN.mkdir()
    for variant in variants or SOURCES:
        source = SOURCES[variant]
        resolved = subprocess.check_output(['git', 'rev-parse', source + '^{commit}'], cwd=ROOT).decode().strip()
        assert re.fullmatch('[0-9a-f]{40}', source) and resolved == source
        assert not subprocess.check_output(['git', 'diff', '--diff-filter=D', '--name-only',
                                            SOURCES['main'], source], cwd=ROOT), 'archive overlay has deletions'
        SOURCES[variant] = resolved
        archive = subprocess.Popen(['git', 'archive', resolved], cwd=ROOT, stdout=subprocess.PIPE)
        try:
            subprocess.run(['tar', '-xf', '-', '-C', str(tree)], stdin=archive.stdout, check=True, timeout=120)
        finally:
            archive.stdout.close()
            assert archive.wait(timeout=30) == 0
        N.compile_application(tree, variant)
        path = N.BIN / variant
        shutil.copy2(tree / 'target/release/rillcache', path)
        binaries[variant] = str(path)
        G.validate_candidate_version(subprocess.check_output([str(path), '--version'], env=N.SAFE_ENV).decode().strip())
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
    paths = list(extracted.rglob('dragonfly*'))
    paths = [p for p in paths if p.is_file() and os.access(p, os.X_OK)]
    assert len(paths) == 1
    binaries['dragonfly'] = str(paths[0])
    actual = subprocess.check_output([str(paths[0]), '--version'], env=N.SAFE_ENV).decode().strip()
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
                        result={'measured': {'rps': 104 if v == 'candidate' else 100}})
                   for w, p, r, v in identities]
        assert 'screen: PASS' in summary(records)
        records[0]['result']['measured']['rps'] = 1000000
        assert 'screen: PASS' in summary(records)  # Never select the fastest sample.
        for record in records:
            if record['variant'] == 'candidate':
                record['result']['measured']['rps'] = 99
        assert 'screen: REJECT' in summary(records)
        print('Order, balanced rotations, medians and rejection controls pass.')
        return
    if len(sys.argv) == 3 and sys.argv[1] == '--check':
        print(summary(check(Path(sys.argv[2]))['records']))
        return
    assert os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch'
    OUT.mkdir(parents=True, exist_ok=False)
    manifest = {'diagnostic_only': True, 'complete': False, 'normal_complete': False,
                'records': [], 'profiles_complete': False, 'sources': SOURCES, 'binaries': {}}
    try:
        G.save(OUT / 'hardware.json', N.HARDWARE['probe']())
        cpus, clients = G.placement(sorted(os.sched_getaffinity(0)))
        manifest.update(server_cpus=cpus, client_cpus=clients, shared_clients=bool(set(cpus) & set(clients)))
        build(manifest['binaries'])
        hashes = {v: G.digest(p) for v, p in manifest['binaries'].items()}
        assert len(set(hashes.values())) == len(hashes)
        manifest['binary_sha256'] = hashes
        for workload, pipeline, run, variant in order():
            name = f'{workload}-p{pipeline}-r{run}-{variant}'
            print('normal', name, flush=True)
            role = 'dragonfly' if variant == 'dragonfly' else 'rillcache'
            mapping = {role: manifest['binaries'][variant], 'generator': manifest['binaries']['generator']}
            G.sample(mapping, role, workload, pipeline, cpus, clients, OUT / name)
            result = G.read_json(OUT / name / 'sample.json')
            manifest['records'].append(dict(workload=workload, pipeline=pipeline, run=run, variant=variant,
                                            directory=name, result=result))
            G.save(OUT / 'manifest.json', manifest)
        manifest['normal_complete'] = True
        (OUT / 'summary.md').write_text(summary(manifest['records']))
        # Normal evidence remains available even if optional instrumentation is unavailable.
        G.save(OUT / 'manifest.json', manifest)
        perf, event = N.perf_tool()
        manifest.update(perf=perf, cpu_event=event)
        for variant in ('failed', 'candidate', 'dragonfly'):
            for mode in ('syscalls', 'cpu'):
                folder = OUT / f'profile-{variant}-{mode}'
                folder.mkdir()
                role = 'dragonfly' if variant == 'dragonfly' else 'rillcache'
                with G.owned_server(manifest['binaries'][variant], role, cpus, folder) as (server, port):
                    argv = G.generator_argv(manifest['binaries']['generator'], port, 'ping', 1, None, clients)
                    argv[argv.index('--duration') + 1] = '20'
                    N.measured_load(argv, folder / 'measured', server, mode, perf, event)
                    if mode == 'cpu':
                        for who in ('server', 'generator'):
                            N.cpu_report(perf, folder / 'measured' / f'{who}.perf.data',
                                         folder / f'{who}-cpu.txt', True)
        manifest['profiles_complete'] = True
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
