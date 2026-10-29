#!/usr/bin/env python3
"""One read-only native diagnostic; never a required-gate result or retry."""
import contextlib
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

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/upstream/evidence/pr139-native'
BIN = OUT / 'frozen-binaries'
BASE = '956e5fededdbc4aa7539e69d205d3cc69ed5eecc'
CANDIDATE = 'd9ecad9b805503af949e6fde65b1d029c9f751a4'
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


def build_sources():
    G['save'](OUT / 'initial.json', {'diagnostic_only': True, 'complete': False,
                                   'base': BASE, 'candidate': CANDIDATE})
    COMMON.checked(['git', 'fetch', '--no-tags', 'origin', BASE, CANDIDATE], OUT / 'fetch', timeout=180)
    BIN.mkdir(parents=True, exist_ok=False)
    # Reuse one absolute source/build path; freeze each binary before overlaying.
    # No deleted source may survive an overlay and contaminate the candidate.
    deleted = subprocess.check_output(['git', 'diff', '--diff-filter=D', '--name-only', BASE, CANDIDATE], cwd=ROOT)
    assert not deleted, 'source deletions require a fresh extraction strategy'
    tree = Path(os.environ['RUNNER_TEMP']) / 'pr139-native-source'
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
    # The advisory prerequisite build uses libc; the protected gate uses the
    # upstream defaults. Build the exact protected Makefile targets instead.
    pin = COMMON.PINS['upstreams']['valkey']
    version = pin['version']
    reference = ROOT / f'target/competitors/valkey-{version}'
    archive = ROOT / f'target/upstream-downloads/valkey-{version}-{pin["sha256"]}.tar.gz'
    assert G['digest'](archive) == pin['sha256']
    reference.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(archive, reference.parent / f'valkey-{version}.tar.gz')
    COMMON.checked(['make', str(reference / 'src/valkey-server')], OUT / 'build/valkey-protected', timeout=900)
    COMMON.checked(['make', '-C', str(reference / 'src'), '-j2', 'valkey-benchmark'],
                   OUT / 'build/generator-protected', timeout=900)
    G['save'](OUT / 'reference-identity.json', {
        'source_archive_sha256': pin['sha256'], 'version': version,
        'benchmark_c_sha256': G['digest'](reference / 'src/valkey-benchmark.c'),
        'build_policy': 'same protected Makefile targets and defaults; no allocator/optimization/TLS overrides'})
    shutil.copy2(reference / 'src/valkey-server', BIN / 'valkey')
    shutil.copy2(reference / 'src/valkey-benchmark', BIN / 'generator')
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
    for role, pid in [('server', server_pid), ('generator', generator_pid)]:
        prefix = ['sudo', '-n', perf]
        if mode == 'syscalls':
            argv = prefix + ['stat', '-x', ';', '--no-big-num', '-e', 'syscalls:sys_exit_recvfrom',
                             '--filter', 'ret == -11', '-e', 'syscalls:sys_exit_read', '--filter', 'ret == -11',
                             '-e', 'syscalls:sys_exit_readv', '--filter', 'ret == -11']
            for event in ['sys_enter_recvfrom', 'sys_enter_read', 'sys_enter_readv', 'sys_enter_sendto', 'sys_enter_write',
                          'sys_enter_ioctl', 'sys_enter_futex', 'sys_enter_epoll_wait',
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


def sample(binaries, hashes, variant, run, mode, cpus, perf=None, cpu_event=None):
    directory = OUT / mode / f'r{run}-{variant}'
    directory.mkdir(parents=True)
    result = {'variant': variant, 'run': run, 'mode': mode, 'valid': False}
    try:
        assert all(G['digest'](path) == hashes[name] for name, path in binaries.items())
        with owned_server(binaries[variant], variant, cpus, directory) as (server, port):
            G['validate_dataset'](port, 'ping', preload=True)
            warm = G['generator_argv'](binaries['generator'], port, 'ping', 1, G['WARMUP'], cpus)
            COMMON.checked(warm, directory / 'warmup', timeout=180, env=SAFE_ENV)
            G['read_generator'](directory / 'warmup', 'ping', warm)
            before = snapshot(server.pid)
            argv = G['generator_argv'](binaries['generator'], port, 'ping', 1, None, cpus)
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
                print('NATIVE_RAW_CSV ' + json.dumps({'variant': variant, 'run': run, 'mode': mode,
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


def main():
    assert os.environ.get('GITHUB_ACTIONS') == 'true', 'native study is Actions-only'
    assert os.environ.get('GITHUB_REF') == 'refs/heads/diagnostic/pr139-native-io', 'diagnostic branch only'
    assert re.fullmatch(r'[0-9a-f]{40}', BASE) and re.fullmatch(r'[0-9a-f]{40}', CANDIDATE)
    OUT.mkdir(parents=True, exist_ok=False)
    manifest = {'diagnostic_only': True, 'complete': False, 'profiles_complete': False,
                'base_source': BASE, 'candidate_source': CANDIDATE, 'normal_samples': [], 'profile_samples': [],
                'idle_samples': [], 'idle_complete': False,
                'normal_duration_seconds': 10, 'profile_load_seconds': 20, 'capture_seconds': 8,
                'scope': 'PING/P1 only; default servers, official Valkey generator, 8 clients/2 threads, shared four CPUs; NOT a required gate'}
    try:
        cpus = sorted(os.sched_getaffinity(0))[:4]
        assert len(cpus) == 4, 'four allowed CPUs required'
        perf, cpu_event = perf_tool()
        metadata = COMMON.environment()
        metadata['build_environment'] = {key: os.environ.get(key) for key in (
            'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_TARGET_DIR', 'CARGO_INCREMENTAL',
            'CARGO_PROFILE_RELEASE_LTO', 'CARGO_PROFILE_RELEASE_CODEGEN_UNITS',
            'CARGO_BUILD_JOBS', 'RUSTUP_TOOLCHAIN')}
        metadata.pop('scope', None)
        metadata['advisory_prerequisite_pins'] = metadata.pop('pins')
        metadata['reference_build_policy'] = 'protected Makefile Valkey targets, upstream default allocator/optimization; not advisory libc flags'
        metadata['selected_server_and_generator_cpus'] = cpus
        smt = Path('/sys/devices/system/cpu/smt/active')
        metadata['smt_active'] = smt.read_text()[:3] if smt.exists() else 'unavailable'
        G['save'](OUT / 'environment.json', metadata)
        binaries = build_sources()
        hashes = {name: G['digest'](path) for name, path in binaries.items()}
        manifest.update(binaries=binaries, binary_sha256=hashes, cpus=cpus)
        manifest.update(perf=perf, cpu_profile_event=cpu_event)
        G['save'](OUT / 'manifest.json', manifest)
        variants = ('baseline', 'candidate', 'valkey')
        for run in range(1, 4):
            offset = run - 1
            for variant in variants[offset:] + variants[:offset]:
                print('normal', run, variant, flush=True)
                manifest['normal_samples'].append(sample(binaries, hashes, variant, run, 'normal', cpus))
                G['save'](OUT / 'manifest.json', manifest)
        manifest['complete'] = True
        for mode in ('syscalls', 'cpu'):
            for variant in variants:
                print(mode, variant, flush=True)
                manifest['profile_samples'].append(sample(binaries, hashes, variant, 1, mode, cpus, perf, cpu_event))
                G['save'](OUT / 'manifest.json', manifest)
        manifest['profiles_complete'] = True
        for variant in variants:
            print('idle', variant, flush=True)
            manifest['idle_samples'].append(idle_sample(binaries, hashes, variant, cpus, perf))
            G['save'](OUT / 'manifest.json', manifest)
        manifest['idle_complete'] = True
        assert all(G['digest'](path) == hashes[name] for name, path in binaries.items())
        text = '# Native shared-four-CPU PING diagnostic (NOT a merge gate)\n\n'
        text += 'Separate uninstrumented 10s comparisons and instrumented 20s loads with 8s syscall/CPU captures; default servers, one owned server at a time. All raw samples retained.\n\n'
        text += '| Server | Median [min-max] req/s | Server CPU seconds | Generator CPU seconds |\n|---|---:|---:|---:|\n'
        for variant in variants:
            rows = [s for s in manifest['normal_samples'] if s['variant'] == variant]
            rates = [s['metrics']['rps'] for s in rows]
            generator = [s['generator']['generator_user_seconds'] + s['generator']['generator_system_seconds'] for s in rows]
            text += f'| {variant} | {statistics.median(rates):.2f} [{min(rates):.2f}-{max(rates):.2f}] | {statistics.median(s["server_cpu_seconds"] for s in rows):.3f} | {statistics.median(generator):.3f} |\n'
        text += '\nSeparate no-load, owned-process accept/sleep/context-switch counters are retained under idle/; they contain no generator and make no throughput claim.\n'
        text += '\nThis subset/profiling study cannot satisfy either protected performance gate. No throughput claim uses instrumented samples; no CPU/physical-core or universal-performance guarantee.\n'
        COMMON.summary(text, OUT / 'summary.md')
        return 0
    except BaseException as error:
        manifest['error'] = str(error)
        raise
    finally:
        G['save'](OUT / 'manifest.json', manifest)
        print('NATIVE_MANIFEST ' + json.dumps(manifest, sort_keys=True), flush=True)


if __name__ == '__main__':
    raise SystemExit(main())
