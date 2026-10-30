#!/usr/bin/env python3
"""Bounded matched screening on an approved Actions host; never a merge gate."""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import shutil
import statistics
import subprocess
import sys
import time

if not __debug__:
    raise RuntimeError('evidence validation requires Python without -O')

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/upstream/evidence/dev-performance'
BIN = OUT / 'frozen-binaries'
BASE = None
CANDIDATE = None
HARDWARE = runpy.run_path(str(ROOT / 'scripts/dev_hardware.py'))
G = runpy.run_path(str(ROOT / 'scripts/pr-multicpu-performance.py'))
COMMON = sys.modules['common']
SAFE_ENV = {'PATH': os.environ['PATH'], 'LANG': 'C', 'LC_ALL': 'C'}


def snapshot(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(') ', 1)[1].split()
    return {'pid': pid, 'start_ticks': int(fields[19]), 'user_ticks': int(fields[11]),
            'system_ticks': int(fields[12]), 'threads': int(fields[17]),
            'clock_ticks_per_second': os.sysconf('SC_CLK_TCK'),
            'actual_cpus': sorted(os.sched_getaffinity(pid))}


def compile_application(tree, name):
    # Git archives restore old mtimes: Cargo can otherwise reuse a stale app.
    # Clean only this package's generated release artifacts in the owned CI
    # workspace, retaining dependencies and already-frozen controls.
    COMMON.checked(['cargo', 'clean', '--locked', '--offline', '--release', '--package', 'rillcache'],
                   OUT / 'build' / f'{name}-clean', cwd=tree, timeout=120)
    directory = OUT / 'build' / name
    COMMON.checked(['cargo', 'build', '--locked', '--offline', '--release', '--bin', 'rillcache'],
                   directory, cwd=tree, timeout=600)
    assert 'Compiling rillcache v' in (directory / 'stderr.log').read_text(), 'application was not rebuilt'


def cpu_report(perf, data, output, children):
    with output.open('w') as stream:
        # Only our known, sanitized, runner-owned capture is trusted here. Root
        # perf otherwise rejects a non-root owner's file, even with read access.
        subprocess.run(['sudo', '-n', perf, 'report', '--force', '--stdio', '--percent-limit', '0.5',
                        '--children' if children else '--no-children', '-i', str(data)],
                       stdout=stream, stderr=subprocess.STDOUT, timeout=45, check=True, env=SAFE_ENV)


def build_reference():
    pin = COMMON.PINS['upstreams']['valkey']
    version = pin['version']
    archive = ROOT / f'target/upstream-downloads/valkey-{version}-{pin["sha256"]}.tar.gz'
    if not archive.exists():
        archive.parent.mkdir(parents=True, exist_ok=True)
        temporary = archive.with_suffix('.partial')
        COMMON.checked(['curl', '--proto', '=https', '--tlsv1.2', '--fail', '--location', '--show-error',
                        '--retry', '3', '--connect-timeout', '10', '--max-time', '180',
                        pin['url'], '--output', str(temporary)], OUT / 'build/valkey-download', timeout=600)
        assert G['digest'](temporary) == pin['sha256'], 'Valkey download checksum differs'
        temporary.rename(archive)
    assert G['digest'](archive) == pin['sha256'], 'Valkey cached source checksum differs'
    identity = {'source_archive_sha256': pin['sha256'], 'version': version,
                'hardware_fingerprint': HARDWARE['fingerprint'](HARDWARE['probe']()),
                'makefile_sha256': G['digest'](ROOT / 'Makefile'),
                'targets_sha256': G['digest'](ROOT / 'benchmarks/targets.mk'),
                'build_policy': 'protected Makefile defaults; no allocator/optimization/TLS overrides'}
    key = hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()
    cache = ROOT / 'target/dev-reference-cache' / key
    metadata = cache / 'build.json'
    reused = metadata.exists()
    if reused:
        saved = G['read_json'](metadata)
        assert saved['identity'] == identity, 'cached reference build identity differs'
        assert set(saved['binary_sha256']) == {'valkey', 'generator'}, 'cached reference is incomplete'
        assert all(G['digest'](cache / name) == digest for name, digest in saved['binary_sha256'].items()), 'cached reference binary changed'
    else:
        assert not cache.exists(), 'incomplete reference cache retained; inspect before cleanup'
        reference = ROOT / f'target/competitors/valkey-{version}'
        reference.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(archive, reference.parent / f'valkey-{version}.tar.gz')
        COMMON.checked(['make', str(reference / 'src/valkey-server')], OUT / 'build/valkey-protected', timeout=900)
        COMMON.checked(['make', '-C', str(reference / 'src'), '-j2', 'valkey-benchmark'],
                       OUT / 'build/generator-protected', timeout=900)
        cache.mkdir(parents=True)
        shutil.copy2(reference / 'src/valkey-server', cache / 'valkey')
        shutil.copy2(reference / 'src/valkey-benchmark', cache / 'generator')
        saved = {'identity': identity,
                 'binary_sha256': {name: G['digest'](cache / name) for name in ('valkey', 'generator')}}
        G['save'](metadata, saved)
    for name in ('valkey', 'generator'):
        version_log = OUT / f'build/version-{name}'
        COMMON.checked([str(cache / name), '--version'], version_log, timeout=10)
        assert re.search(r'(?:\bv=|\s)' + re.escape(version) + r'(?:\s|$)',
                         (version_log / 'stdout.log').read_text()), 'cached reference version differs'
        shutil.copy2(cache / name, BIN / name)
    G['save'](OUT / 'reference-identity.json', {**saved, 'cache_reused': reused})


def build_sources():
    G['save'](OUT / 'initial.json', {'diagnostic_only': True, 'complete': False,
                                   'base': BASE, 'candidate': CANDIDATE})
    COMMON.checked(['git', 'fetch', '--no-tags', 'origin', BASE, CANDIDATE], OUT / 'fetch', timeout=180)
    BIN.mkdir(parents=True, exist_ok=False)
    # Reuse one absolute source/build path; freeze each binary before overlaying.
    # No deleted source may survive an overlay and contaminate the candidate.
    deleted = subprocess.check_output(['git', 'diff', '--diff-filter=D', '--name-only', BASE, CANDIDATE], cwd=ROOT)
    assert not deleted, 'source deletions require a fresh extraction strategy'
    tree = Path(os.environ['RUNNER_TEMP']) / 'rillcache-dev-performance-source'
    tree.mkdir()
    for name, sha in [('baseline', BASE), ('candidate', CANDIDATE)]:
        archive = subprocess.Popen(['git', 'archive', sha], cwd=ROOT, stdout=subprocess.PIPE)
        try:
            subprocess.run(['tar', '-x', '-C', str(tree)], stdin=archive.stdout, check=True, timeout=120)
        finally:
            archive.stdout.close()
            if archive.wait(timeout=30) != 0:
                raise RuntimeError('source archive failed')
        compile_application(tree, name)
        shutil.copy2(tree / 'target/release/rillcache', BIN / name)
    assert G['digest'](BIN / 'baseline') != G['digest'](BIN / 'candidate'), 'known code-generating revisions produced the same binary'
    build_reference()
    return {name: str((BIN / name).resolve()) for name in ('baseline', 'candidate', 'valkey', 'generator')}


def perf_tool():
    def find():
        paths = [shutil.which('perf'), *map(str, sorted(Path('/usr/lib/linux-tools').glob('*/perf'), reverse=True))]
        for path in paths:
            if path and subprocess.run([path, '--version'], capture_output=True, timeout=10).returncode == 0:
                return path
        return None
    perf = find()
    if perf is None:
        COMMON.checked(['sudo', '-n', 'apt-get', 'install', '-y', '--no-upgrade', '--no-install-recommends',
                        'linux-tools-generic'], OUT / 'perf-install', timeout=300)
        perf = find()
    if perf is None:
        raise RuntimeError('no usable perf executable; do not invent profiling evidence')
    # Runner users may not search an already-mounted, root-only tracefs.
    visible = COMMON.command(['sudo', '-n', 'test', '-e',
                              '/sys/kernel/tracing/events/syscalls/sys_exit_read/id'],
                             OUT / 'tracefs-root-visibility', timeout=20)
    if visible['exit_code'] != 0:
        COMMON.checked(['sudo', '-n', 'mount', '-t', 'tracefs', '-o', 'ro', 'tracefs',
                        '/sys/kernel/tracing'], OUT / 'tracefs-read-only', timeout=20)
    COMMON.checked(['sudo', '-n', perf, 'stat', '-e', 'syscalls:sys_exit_read', '--filter',
                    'ret == -11', '--', 'sleep', '0.01'], OUT / 'tracepoint-probe', timeout=20)
    result = COMMON.command(['sudo', '-n', perf, 'stat', '-e', 'cycles', '--', 'sleep', '0.01'],
                            OUT / 'cycles-probe', timeout=20)
    return perf, ('cycles' if result['exit_code'] == 0 else 'cpu-clock')


@contextlib.contextmanager
def owned_server(binary, variant, cpus, directory):
    port = G['available_port']()
    kind = 'valkey' if variant == 'valkey' else 'rillcache'
    argv = G['server_argv'](binary, kind, port, cpus, directory)
    identity = {'argv': argv, 'port': port, 'requested_cpus': cpus, 'environment': 'minimal, no Actions credentials'}
    with (directory / 'server.log').open('wb') as log:
        server = subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT,
                                  start_new_session=True, env=SAFE_ENV)
        identity['pid'] = server.pid
        try:
            deadline = time.monotonic() + 10
            while True:
                if server.poll() is not None:
                    raise RuntimeError('owned server exited before readiness')
                try:
                    with G['Client'](port) as client:
                        assert client.call('PING') == b'PONG'
                    break
                except (OSError, ConnectionError):
                    if time.monotonic() >= deadline:
                        raise RuntimeError('readiness deadline') from None
                    time.sleep(.01)
            assert sorted(os.sched_getaffinity(server.pid)) == cpus
            yield server, port
            assert server.poll() is None
            assert sorted(os.sched_getaffinity(server.pid)) == cpus
        finally:
            identity['exit_before_cleanup'] = server.poll()
            identity['exit_after_cleanup'] = G['terminate'](server)
            identity['joined'] = True
            G['save'](directory / 'server.json', identity)


def capture_permissions(directory):
    results = {}
    for role in ('server', 'generator'):
        path = directory / f'{role}.perf.data'
        if path.exists():
            results[role] = COMMON.command(['sudo', '-n', 'chmod', '0644', str(path)],
                                           directory / f'{role}-capture-permission', timeout=15)
    return results


def profiler_jobs(perf, cpu_event, mode, server_pid, generator_pid, directory):
    jobs = []
    try:
        for role, pid in [('server', server_pid), ('generator', generator_pid)]:
            prefix = ['sudo', '-n', perf]
            if mode == 'syscalls':
                argv = prefix + ['stat', '-x', ';', '--no-big-num', '-e', 'syscalls:sys_exit_recvfrom',
                                 '--filter', 'ret == -11', '-e', 'syscalls:sys_exit_read', '--filter', 'ret == -11',
                                 '-e', 'syscalls:sys_exit_readv', '--filter', 'ret == -11']
                for event in ['sys_enter_recvfrom', 'sys_enter_read', 'sys_enter_readv', 'sys_enter_sendto', 'sys_enter_write',
                              'sys_enter_ioctl', 'sys_enter_futex', 'sys_enter_sched_yield', 'sys_enter_epoll_wait',
                              'sys_enter_accept4', 'sys_enter_accept', 'sys_enter_nanosleep', 'sys_enter_clock_nanosleep']:
                    argv += ['-e', 'syscalls:' + event]
                argv += ['-e', 'task-clock,context-switches', '-p', str(pid),
                         '-o', str(directory / f'{role}-syscalls.txt'), '--', 'sleep', '8']
            else:
                # Retain runner ownership even if privileged perf exits abnormally.
                (directory / f'{role}.perf.data').touch(mode=0o600, exist_ok=False)
                argv = prefix + ['record', '-F', '199', '-e', cpu_event, '-g', '--call-graph', 'dwarf,4096',
                                 '-p', str(pid), '-o', str(directory / f'{role}.perf.data'), '--', 'sleep', '8']
            log = (directory / f'{role}-profiler.log').open('wb')
            try:
                proc = subprocess.Popen(argv, stdout=log, stderr=subprocess.STDOUT,
                                        start_new_session=True, env=SAFE_ENV)
            finally:
                log.close()
            jobs.append((role, proc, argv))
    except BaseException:
        # A failure starting the second capture must not lose the first child.
        for _, proc, _ in jobs:
            G['terminate'](proc)
        raise
    return jobs


def measured_load(argv, directory, server, mode, perf, cpu_event):
    directory.mkdir()
    metadata = {'argv': argv, 'timeout_seconds': 180, 'profiling_mode': mode}
    jobs = []
    start = time.monotonic()
    with (directory / 'stdout.log').open('wb') as out, (directory / 'stderr.log').open('wb') as err:
        load = subprocess.Popen(argv, stdout=out, stderr=err, start_new_session=True, env=SAFE_ENV)
        metadata['pid'] = load.pid
        try:
            if mode != 'normal':
                time.sleep(.5)  # Capture delay only; server readiness is protocol-validated.
                metadata['server_at_capture'] = snapshot(server.pid)
                metadata['generator_at_capture'] = snapshot(load.pid)
                jobs = profiler_jobs(perf, cpu_event, mode, server.pid, load.pid, directory)
            deadline = start + 180
            while True:
                pid, status, usage = os.wait4(load.pid, os.WNOHANG)
                if pid:
                    load.returncode = os.waitstatus_to_exitcode(status)
                    metadata.update(exit_code=load.returncode, generator_user_seconds=usage.ru_utime,
                                    generator_system_seconds=usage.ru_stime, generator_peak_rss_kib=usage.ru_maxrss,
                                    generator_voluntary_switches=usage.ru_nvcsw,
                                    generator_involuntary_switches=usage.ru_nivcsw)
                    break
                if time.monotonic() >= deadline:
                    metadata['timed_out'] = True
                    raise RuntimeError('owned generator timed out')
                time.sleep(.01)
            for role, proc, command in jobs:
                code = proc.wait(timeout=20)
                G['save'](directory / f'{role}-profiler.json', {'argv': command, 'exit_code': code})
                if code != 0:
                    raise RuntimeError(f'{role} profiler failed: {code}')
        finally:
            if load.returncode is None:
                G['terminate'](load)
            for _, proc, _ in jobs:
                if proc.poll() is None:
                    G['terminate'](proc)
            metadata['capture_permissions'] = capture_permissions(directory)
            metadata['wall_seconds'] = time.monotonic() - start
            metadata.setdefault('exit_code', load.returncode)
            G['save'](directory / 'command.json', metadata)
    assert metadata['exit_code'] == 0 and not metadata.get('timed_out')
    return metadata


def sample(binaries, hashes, variant, run, mode, cpus, perf=None, cpu_event=None, pipeline=1):
    directory = OUT / mode / f'r{run}-{variant}-p{pipeline}'
    directory.mkdir(parents=True)
    result = {'variant': variant, 'run': run, 'mode': mode, 'pipeline': pipeline, 'valid': False}
    try:
        assert all(G['digest'](path) == hashes[name] for name, path in binaries.items())
        with owned_server(binaries[variant], variant, cpus, directory) as (server, port):
            G['validate_dataset'](port, 'ping', preload=True)
            warm = G['generator_argv'](binaries['generator'], port, 'ping', pipeline, G['WARMUP'], cpus)
            COMMON.checked(warm, directory / 'warmup', timeout=180, env=SAFE_ENV)
            G['read_generator'](directory / 'warmup', 'ping', warm)
            before = snapshot(server.pid)
            argv = G['generator_argv'](binaries['generator'], port, 'ping', pipeline, None, cpus)
            if mode != 'normal':
                # Keep targets alive through perf startup plus its 8s capture.
                argv[argv.index('--duration') + 1] = '20'
            resource = measured_load(argv, directory / 'measured', server, mode, perf, cpu_event)
            assert resource['wall_seconds'] >= (10 if mode == 'normal' else 20)
            after = snapshot(server.pid)
            metrics = G['read_generator'](directory / 'measured', 'ping', argv)
            G['validate_dataset'](port, 'ping', after=True)
            assert before['start_ticks'] == after['start_ticks']
            result.update(valid=True, before=before, after=after, metrics=metrics, generator=resource,
                          server_cpu_seconds=((after['user_ticks']+after['system_ticks'])-
                                              (before['user_ticks']+before['system_ticks'])) / before['clock_ticks_per_second'])
        if mode == 'cpu':
            for role in ('server', 'generator'):
                data = directory / 'measured' / f'{role}.perf.data'
                COMMON.checked(['sudo', '-n', 'chmod', '0644', str(data)],
                               directory / f'{role}-artifact-permission', timeout=15)
                for children in (False, True):
                    suffix = 'children' if children else 'self'
                    cpu_report(perf, data, directory / f'{role}-{suffix}.txt', children)
        return result
    finally:
        G['save'](directory / 'sample.json', result)
        print('NATIVE_SAMPLE ' + json.dumps(result, sort_keys=True), flush=True)
        for phase in ('warmup', 'measured'):
            path = directory / phase / 'stdout.log'
            if path.exists():
                print('NATIVE_RAW_CSV ' + json.dumps({'variant': variant, 'run': run, 'mode': mode, 'pipeline': pipeline,
                                                     'phase': phase, 'csv': path.read_text()}), flush=True)


def idle_argv(perf, pid, output):
    events = ','.join(['syscalls:sys_enter_accept4', 'syscalls:sys_enter_accept',
                       'syscalls:sys_enter_nanosleep', 'syscalls:sys_enter_clock_nanosleep',
                       'context-switches', 'task-clock'])
    return ['sudo', '-n', perf, 'stat', '-x', ';', '--no-big-num', '-e', events,
            '-p', str(pid), '-o', str(output), '--', 'sleep', '2']


def idle_sample(binaries, hashes, variant, cpus, perf):
    directory = OUT / 'idle' / variant
    directory.mkdir(parents=True)
    result = {'variant': variant, 'mode': 'idle', 'valid': False}
    try:
        assert all(G['digest'](path) == hashes[name] for name, path in binaries.items())
        with owned_server(binaries[variant], variant, cpus, directory) as (server, _):
            time.sleep(.2)  # Readiness PING already validated; let its owner retire.
            before = snapshot(server.pid)
            output = directory / 'syscalls.txt'
            output.touch(mode=0o600, exist_ok=False)
            argv = idle_argv(perf, server.pid, output)
            capture = COMMON.checked(argv, directory / 'capture', timeout=20, env=SAFE_ENV)
            after = snapshot(server.pid)
            result.update(valid=True, before=before, after=after, capture=capture)
        return result
    finally:
        G['save'](directory / 'sample.json', result)
        print('NATIVE_IDLE ' + json.dumps(result, sort_keys=True), flush=True)
        output = directory / 'syscalls.txt'
        if output.exists():
            print('NATIVE_IDLE_COUNTERS ' + json.dumps({'variant': variant, 'text': output.read_text()}), flush=True)


VARIANTS = ('baseline', 'candidate', 'valkey')


def normal_order(pipelines):
    for pipeline in pipelines:
        for run in range(1, 4):
            offset = run - 1
            for variant in VARIANTS[offset:] + VARIANTS[:offset]:
                yield pipeline, run, variant


def summarize(manifest):
    text = '# Matched PING screening (NOT a merge gate)\n\n'
    text += 'Uninstrumented 10s samples, three rotating repetitions, default servers, official Valkey generator, 8 clients/2 threads, shared four logical CPUs. All losses and raw samples retained.\n\n'
    text += '| Pipeline | Server | Median [min-max] req/s | Server CPU seconds | Generator CPU seconds |\n|---:|---|---:|---:|---:|\n'
    for pipeline in manifest['pipelines']:
        for variant in VARIANTS:
            rows = [s for s in manifest['normal_samples'] if s['variant'] == variant and s['pipeline'] == pipeline]
            rates = [s['metrics']['rps'] for s in rows]
            generator = [s['generator']['generator_user_seconds'] + s['generator']['generator_system_seconds'] for s in rows]
            text += f'| {pipeline} | {variant} | {statistics.median(rates):.2f} [{min(rates):.2f}-{max(rates):.2f}] | {statistics.median(s["server_cpu_seconds"] for s in rows):.3f} | {statistics.median(generator):.3f} |\n'
    text += '\nProfiles are opt-in and separate: P1 20s loads/8s captures plus owned-server-only 2s idle captures. No throughput claim uses instrumented samples.\n'
    text += '\nThe hardware fingerprint fixes declared SKU/topology/software, not physical-host isolation or stable load. This subset cannot satisfy either protected performance gate or prove a universal win.\n'
    return text


def evidence_hashes(root):
    return {str(path.relative_to(root)): G['digest'](path)
            for path in sorted(Path(root).rglob('*'))
            if path.is_file() and path != Path(root) / 'manifest.json'}


def validate_build_environment(contract):
    overrides = {key: value for key, value in contract['build_environment'].items()
                 if value and key not in ('CARGO_BUILD_JOBS', 'RUSTUP_TOOLCHAIN')}
    assert not overrides, f'production-default builds required; unsupported overrides: {sorted(overrides)}'
    assert contract['rust'].startswith(f'rustc {COMMON.PINS["rust"]} ('), 'Rust differs from pinned compiler'


def check_study(root):
    root = Path(root)
    manifest = G['read_json'](root / 'manifest.json')
    assert manifest['diagnostic_only'] is True and manifest['complete'] is True, 'incomplete advisory study'
    assert manifest['pipelines'] in ([1], [16], [1, 16]), 'unexpected screening scope'
    assert manifest['normal_duration_seconds'] == 10, 'duration policy differs'
    assert manifest['evidence_sha256'] == evidence_hashes(root), 'raw evidence missing or checksum differs'
    assert all(re.fullmatch(r'[0-9a-f]{40}', manifest[k]) for k in ('base_source', 'candidate_source'))
    assert manifest['base_source'] != manifest['candidate_source'], 'same source is not a candidate'
    environment = G['read_json'](root / 'environment.json')
    HARDWARE['require_match'](environment['hardware_contract'], manifest['hardware_fingerprint'])
    validate_build_environment(environment['hardware_contract'])
    assert manifest['cpus'] == environment['hardware_contract']['selected_cpus']
    reference = G['read_json'](root / 'reference-identity.json')
    pin = COMMON.PINS['upstreams']['valkey']
    assert reference['identity']['source_archive_sha256'] == pin['sha256']
    assert reference['identity']['version'] == pin['version']
    assert reference['identity']['hardware_fingerprint'] == manifest['hardware_fingerprint']
    assert reference['binary_sha256'] == {name: manifest['binary_sha256'][name] for name in ('valkey', 'generator')}
    assert set(manifest['binary_sha256']) == {*VARIANTS, 'generator'}, 'missing frozen binary'
    assert manifest['binary_sha256']['baseline'] != manifest['binary_sha256']['candidate'], 'candidate binary is identical to baseline'
    assert all(G['digest'](root / 'frozen-binaries' / name) == digest
               for name, digest in manifest['binary_sha256'].items()), 'frozen binary changed'
    for variant in ('baseline', 'candidate'):
        stderr = (root / 'build' / variant / 'stderr.log').read_text()
        assert 'Compiling rillcache v' in stderr, 'application was not rebuilt'
    expected = list(normal_order(manifest['pipelines']))
    assert [(s['pipeline'], s['run'], s['variant']) for s in manifest['normal_samples']] == expected, 'missing, duplicate or reordered normal sample'
    for recorded in manifest['normal_samples']:
        pipeline, run, variant = recorded['pipeline'], recorded['run'], recorded['variant']
        directory = root / 'normal' / f'r{run}-{variant}-p{pipeline}'
        saved = G['read_json'](directory / 'sample.json')
        assert saved == recorded and saved['valid'] is True and saved['mode'] == 'normal', 'sample record differs'
        server = G['read_json'](directory / 'server.json')
        assert server['joined'] is True and server['exit_before_cleanup'] is None, 'invalid server lifecycle'
        for point in ('before', 'after'):
            assert saved[point]['pid'] == server['pid'] and saved[point]['actual_cpus'] == manifest['cpus'], 'PID/affinity differs'
        assert saved['before']['start_ticks'] == saved['after']['start_ticks'], 'server identity changed'
        before, after = saved['before'], saved['after']
        ticks = before['clock_ticks_per_second']
        assert type(ticks) is int and ticks > 0 and ticks == after['clock_ticks_per_second']
        cpu = ((after['user_ticks'] + after['system_ticks']) -
               (before['user_ticks'] + before['system_ticks'])) / ticks
        assert cpu >= 0 and saved['server_cpu_seconds'] == cpu, 'CPU counters differ'
        assert saved['generator'] == G['read_json'](directory / 'measured/command.json'), 'generator resource record differs'
        launch = Path(manifest['launch_directory']) / 'normal' / directory.name
        assert server['argv'] == G['server_argv'](manifest['binaries'][variant], 'valkey' if variant == 'valkey' else 'rillcache',
                                                 server['port'], manifest['cpus'], launch), 'server command differs'
        for phase, count in (('warmup', G['WARMUP']), ('measured', None)):
            argv = G['generator_argv'](manifest['binaries']['generator'], server['port'], 'ping', pipeline, count, manifest['cpus'])
            metrics = G['read_generator'](directory / phase, 'ping', argv)
            if phase == 'measured':
                assert saved['metrics'] == metrics, 'CSV metrics differ'
    assert (root / 'summary.md').read_text() == summarize(manifest), 'summary differs'
    requested = manifest['profiles_requested']
    assert type(requested) is bool
    assert len(manifest['profile_samples']) == (6 if requested else 0), 'profile scope differs'
    assert len(manifest['idle_samples']) == (3 if requested else 0), 'idle scope differs'
    assert manifest['profiles_complete'] is requested and manifest['idle_complete'] is requested
    assert [(s['mode'], s['variant']) for s in manifest['profile_samples']] == (
        [(mode, variant) for mode in ('syscalls', 'cpu') for variant in VARIANTS] if requested else [])
    assert [s['variant'] for s in manifest['idle_samples']] == (list(VARIANTS) if requested else [])
    assert all(s['valid'] is True for s in manifest['profile_samples'] + manifest['idle_samples'])
    return manifest


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base')
    parser.add_argument('--candidate')
    parser.add_argument('--expected-hardware')
    parser.add_argument('--pipelines', nargs='+', type=int, choices=(1, 16), default=[1, 16])
    parser.add_argument('--profiles', action='store_true')
    parser.add_argument('--check', type=Path)
    args = parser.parse_args(argv)
    if args.check is None:
        for name in ('base', 'candidate'):
            if re.fullmatch(r'[0-9a-f]{40}', getattr(args, name) or '') is None:
                parser.error(f'--{name} requires an exact 40-character source SHA')
        if args.base == args.candidate:
            parser.error('baseline and candidate must differ')
        if re.fullmatch(r'[0-9a-f]{64}', args.expected_hardware or '') is None:
            parser.error('--expected-hardware requires a preselected SHA-256 fingerprint')
        if args.pipelines not in ([1], [16], [1, 16]):
            parser.error('pipelines must be 1, 16, or 1 16 exactly once')
    return args


def main():
    global BASE, CANDIDATE
    args = parse_args()
    if args.check is not None:
        print(summarize(check_study(args.check)))
        return 0
    assert os.environ.get('GITHUB_ACTIONS') == 'true', 'native study is Actions-only'
    assert os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch', 'explicit manual study only'
    assert os.environ.get('RUNNER_ENVIRONMENT') == 'self-hosted', 'use the preselected isolated performance runner'
    BASE, CANDIDATE = args.base, args.candidate
    OUT.mkdir(parents=True, exist_ok=False)
    manifest = {'diagnostic_only': True, 'complete': False, 'normal_complete': False,
                'profiles_requested': args.profiles, 'profiles_complete': False, 'idle_complete': False,
                'base_source': BASE, 'candidate_source': CANDIDATE,
                'pipelines': args.pipelines, 'normal_samples': [], 'profile_samples': [], 'idle_samples': [],
                'normal_duration_seconds': 10, 'profile_load_seconds': 20, 'capture_seconds': 8,
                'launch_directory': str(OUT), 'scope': 'PING subset; advisory only, not a required gate'}
    try:
        contract = HARDWARE['probe']()
        G['save'](OUT / 'hardware-observed.json', {'contract': contract, 'fingerprint': HARDWARE['fingerprint'](contract)})
        manifest['hardware_fingerprint'] = HARDWARE['require_match'](contract, args.expected_hardware)
        validate_build_environment(contract)
        cpus = contract['selected_cpus']
        runpy.run_path(str(ROOT / 'scripts/upstream/prepare.py'))['validate_pins']()
        metadata = COMMON.environment()
        metadata['hardware_contract'] = contract
        metadata.pop('scope', None)
        G['save'](OUT / 'environment.json', metadata)
        binaries = build_sources()
        hashes = {name: G['digest'](path) for name, path in binaries.items()}
        manifest.update(binaries=binaries, binary_sha256=hashes, cpus=cpus)
        G['save'](OUT / 'manifest.json', manifest)
        for pipeline, run, variant in normal_order(args.pipelines):
            print('normal', pipeline, run, variant, flush=True)
            manifest['normal_samples'].append(sample(binaries, hashes, variant, run, 'normal', cpus, pipeline=pipeline))
            G['save'](OUT / 'manifest.json', manifest)
        manifest['normal_complete'] = True
        if args.profiles:
            perf, cpu_event = perf_tool()
            manifest.update(perf=perf, cpu_profile_event=cpu_event)
            for mode in ('syscalls', 'cpu'):
                for variant in VARIANTS:
                    manifest['profile_samples'].append(sample(binaries, hashes, variant, 1, mode, cpus, perf, cpu_event))
                    G['save'](OUT / 'manifest.json', manifest)
            manifest['profiles_complete'] = True
            for variant in VARIANTS:
                manifest['idle_samples'].append(idle_sample(binaries, hashes, variant, cpus, perf))
                G['save'](OUT / 'manifest.json', manifest)
            manifest['idle_complete'] = True
        assert all(G['digest'](path) == hashes[name] for name, path in binaries.items())
        HARDWARE['require_match'](HARDWARE['probe'](), args.expected_hardware)
        COMMON.summary(summarize(manifest), OUT / 'summary.md')
        manifest['evidence_sha256'] = evidence_hashes(OUT)
        manifest['complete'] = True
        G['save'](OUT / 'manifest.json', manifest)
        check_study(OUT)
        return 0
    except BaseException as error:
        manifest.update(complete=False, error=str(error))
        raise
    finally:
        G['save'](OUT / 'manifest.json', manifest)
        print('SCREEN_MANIFEST ' + json.dumps(manifest, sort_keys=True), flush=True)


if __name__ == '__main__':
    raise SystemExit(main())
