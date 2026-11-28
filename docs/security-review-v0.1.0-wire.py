"""Loopback-only v0.1.0 security review evidence; uses synthetic credentials."""

import os
import re
import secrets
import select
import socket
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

binary = Path(sys.argv[1]).resolve()
os.umask(0o077)
with tempfile.TemporaryDirectory(prefix='rillcache-review-') as root:
    root = Path(root)
    admin, reader = (secrets.token_hex(32).encode() for _ in range(2))
    data = root / 'data'
    data.mkdir()
    acl = root / 'users.acl'
    acl.write_bytes(b'user default on >' + admin + b' ~* &* +@all\n'
                    b'user probe on >' + reader + b' %R~safe:* -@all +get +ping +info|persistence\n')
    password = root / 'password'
    password.write_bytes(reader + b'\n')
    cfg = root / 'server.conf'
    cfg.write_text('config-version 1\nsecure-secret-files yes\nbind 127.0.0.1:0\n'
                   f'aclfile {acl}\nappendonly yes\nappendfsync always\ndir {data}\n')
    args = ['--config', str(cfg)]
    def run(*args):
        return subprocess.run([binary, *args], capture_output=True, timeout=5)
    out = run('--check-config', *args)
    assert out.returncode == 0 and b'server not started' in out.stdout and not list(data.iterdir())
    acl.chmod(0o640)
    out = run('--check-config', *args)
    assert out.returncode == 2 and b'PermissionDenied' in out.stderr
    assert str(acl).encode() not in out.stderr and reader not in out.stderr
    acl.chmod(0o600)
    print('CONFIG: valid preflight creates no journal; strict mode 0640 ACL -> exit 2 / PermissionDenied, no path or credential')
    proc = subprocess.Popen([binary, *args], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        assert select.select([proc.stderr], [], [], 5)[0], 'server readiness timeout'
        first = proc.stderr.readline()
        addr = re.search(rb'listening on (127\.0\.0\.1):(\d+)\r?\n$', first)
        assert addr, first
        address = ('127.0.0.1', int(addr[2]))
        endpoint = f'{address[0]}:{address[1]}'
        def conn():
            return socket.create_connection(address, timeout=3)
        def call(sock, stream, *parts):
            frame = b'*%d\r\n' % len(parts)
            frame += b''.join(b'$%d\r\n%s\r\n' % (len(part), part) for part in parts)
            sock.sendall(frame)
            line = stream.readline(65536)
            if line.startswith(b'$') and not line.startswith(b'$-'):
                return stream.read(int(line[1:-2]) + 2)
            return line
        with conn() as a, a.makefile('rb') as fa, conn() as b, b.makefile('rb') as fb:
            assert call(a,fa,b'GET',b'safe:key') == b'-NOAUTH Authentication required.\r\n'
            errors = [call(a,fa,b'AUTH',name,b'incorrect') for name in (b'default',b'not-a-user')]
            assert errors[0] == errors[1] == b'-WRONGPASS invalid username-password pair or user is disabled.\r\n'
            assert call(a,fa,b'AUTH',admin) == b'+OK\r\n'
            assert call(a,fa,b'SET',b'safe:key',b'review-sample') == b'+OK\r\n'
            assert call(b,fb,b'AUTH',b'probe',reader) == b'+OK\r\n'
            assert call(b,fb,b'GET',b'safe:key') == b'review-sample\r\n'
            denied = call(b,fb,b'GET',b'outside-scope-marker')
            assert denied.startswith(b'-NOPERM ') and b'outside-scope-marker' not in denied
            assert call(b,fb,b'SET',b'safe:key',b'denied').startswith(b'-NOPERM ')
            assert call(a,fa,b'GET',b'safe:key') == b'review-sample\r\n'
            info = call(a,fa,b'INFO',b'ALL')
            assert b'aof_enabled:1' in info and all(x not in info for x in (admin,reader,str(acl).encode()))
        print('WIRE: NOAUTH, generic WRONGPASS, scoped GET, NOPERM for other keys/SET, state unchanged, INFO ALL omits credential/path markers')
        probe = ['--healthcheck',endpoint,'--healthcheck-require-aof','--healthcheck-aof-write-probe',str(data)]
        failure = run(*probe)
        assert failure.returncode == 1 and not failure.stdout and b'health check failed' in failure.stderr
        assert reader not in failure.stderr and admin not in failure.stderr
        success = run(*probe,'--healthcheck-user','probe','--healthcheck-password-file',str(password))
        assert success.returncode == 0 and not success.stdout and not success.stderr
        print('PROBE: unauthenticated AOF runbook probe exit 1; authenticated PING + INFO|PERSISTENCE + write probe exit 0, quiet')
        for path in (data / 'appendonly.aof', data / 'appendonly.aof.lock'):
            assert stat.S_IMODE(path.stat().st_mode) == 0o600
        journal = (data / 'appendonly.aof').read_bytes()
        assert b'review-sample' in journal and reader not in journal and admin not in journal
        print('JOURNAL: new AOF and lock 0600; sample data readable on disk, credential fixtures absent')
    finally:
        proc.terminate()
        try:
            _, stderr = proc.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.communicate()
            raise
    assert proc.returncode == 0 and b'graceful shutdown completed' in stderr
    assert all(x not in first + stderr for x in (admin, reader))
    print('LOG: graceful stop; fixture credentials absent in startup/shutdown stderr')
