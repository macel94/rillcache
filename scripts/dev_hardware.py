#!/usr/bin/env python3
"""Read-only performance-host fingerprint; no host tuning or measurements."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess

BUILD_KEYS = ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_TARGET_DIR',
              'CARGO_INCREMENTAL', 'CARGO_PROFILE_RELEASE_LTO',
              'CARGO_PROFILE_RELEASE_CODEGEN_UNITS', 'CARGO_BUILD_JOBS', 'RUSTUP_TOOLCHAIN', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER',
              'CC', 'CXX', 'AR', 'RANLIB', 'CFLAGS', 'CXXFLAGS', 'CPPFLAGS', 'LDFLAGS',
              'MAKEFLAGS', 'MALLOC', 'BUILD_TLS')
CPU_FIELDS = ('Architecture', 'CPU(s)', 'Vendor ID', 'Model name', 'CPU family',
              'Model', 'Stepping', 'Thread(s) per core', 'Core(s) per socket', 'Socket(s)')


def fingerprint(contract):
    encoded = json.dumps(contract, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()
    return hashlib.sha256(encoded).hexdigest()


def require_match(contract, expected):
    if not re.fullmatch(r'[0-9a-f]{64}', expected or ''):
        raise ValueError('a preselected 64-character hardware fingerprint is required')
    observed = fingerprint(contract)
    if observed != expected:
        raise ValueError(f'hardware/software contract mismatch: expected {expected}, observed {observed}; do not retry for a favorable host')
    return observed


def probe():
    if platform.system() != 'Linux':
        raise ValueError('Linux CPU topology/procfs required')
    env = {'PATH': os.environ['PATH'], 'LANG': 'C', 'LC_ALL': 'C'}
    if os.environ.get('RUSTUP_TOOLCHAIN'):
        env['RUSTUP_TOOLCHAIN'] = os.environ['RUSTUP_TOOLCHAIN']
    def output(argv):
        return subprocess.run(argv, capture_output=True, text=True, check=True,
                              timeout=30, env=env).stdout.strip()
    cpu = {row['field'].rstrip(':'): row['data']
           for row in json.loads(output(['lscpu', '--json']))['lscpu']}
    if any(not cpu.get(field) for field in CPU_FIELDS):
        raise ValueError('incomplete CPU model/topology identity')
    allowed = sorted(os.sched_getaffinity(0))
    if len(allowed) < 4:
        raise ValueError('at least four allowed logical CPUs required')
    topology = {}
    for number in allowed[:4]:
        root = Path(f'/sys/devices/system/cpu/cpu{number}/topology')
        topology[str(number)] = {name: (root / name).read_text().strip() for name in
                                 ('physical_package_id', 'core_id', 'thread_siblings_list')}
    return {'schema_version': 1, 'cpu': {field: cpu[field] for field in CPU_FIELDS},
            'allowed_cpus': allowed, 'selected_cpus': allowed[:4], 'topology': topology,
            'kernel_release': platform.release(), 'os_release': Path('/etc/os-release').read_text(),
            'rust': output(['rustc', '-Vv']), 'cc': output(['cc', '--version']),
            'linker': output(['ld', '--version']), 'libc': output(['ldd', '--version']),
            'build_environment': {key: os.environ.get(key) for key in BUILD_KEYS}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--expected-fingerprint')
    args = parser.parse_args()
    contract = probe()
    print(json.dumps({'fingerprint': fingerprint(contract), 'contract': contract,
                      'scope': 'SKU/topology/software identity, not proof of a dedicated physical host or stable load'},
                     indent=2, sort_keys=True))
    if args.expected_fingerprint is not None:
        require_match(contract, args.expected_fingerprint)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
