"""Bounded process/TCP helpers; Python stdlib only, no application dependency."""

import contextlib
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import socket
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / "target/upstream"
EVIDENCE = WORK / "evidence"
PINS = json.loads((ROOT / "ci/upstream/pins.json").read_text())
SCOPE = json.loads((ROOT / "ci/upstream/scope.json").read_text())


def save(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def group_signal(proc, sig):
    # Sessions are owned by this harness. Never pkill arbitrary runner processes.
    try:
        os.killpg(proc.pid, sig)
    except ProcessLookupError:
        pass


def terminate(proc):
    group_signal(proc, signal.SIGTERM)
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        group_signal(proc, signal.SIGKILL)
        proc.wait(timeout=5)
    # Tcl children can outlive the test coordinator, even after a normal exit.
    group_signal(proc, signal.SIGKILL)
    return proc.returncode


def interrupted(signum, _frame):
    raise KeyboardInterrupt(f"signal {signum}")


signal.signal(signal.SIGTERM, interrupted)
signal.signal(signal.SIGINT, interrupted)


def command(argv, directory, *, cwd=ROOT, timeout=600, env=None):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    result = {"argv": list(map(str, argv)), "cwd": str(cwd), "timeout_seconds": timeout}
    save(directory / "command.json", result)
    start = time.monotonic()
    with (directory / "stdout.log").open("wb") as out, (directory / "stderr.log").open("wb") as err:
        proc = subprocess.Popen(argv, cwd=cwd, stdout=out, stderr=err, start_new_session=True, env=env)
        try:
            result["exit_code"] = proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            result["timed_out"] = True
            result["exit_code"] = terminate(proc)
        finally:
            terminate(proc)
            result["wall_seconds"] = time.monotonic() - start
            result.setdefault("exit_code", proc.returncode)
            save(directory / "command.json", result)
    return result


def checked(argv, directory, **kwargs):
    result = command(argv, directory, **kwargs)
    if result["exit_code"] != 0 or result.get("timed_out"):
        raise RuntimeError(f"command failed: {argv[0]}; see {directory}")
    return result


def source(name):
    return WORK / f"{name}-{PINS['upstreams'][name]['version']}"


def binary(name):
    return WORK / "rillcache" if name == "rillcache" else source(name) / "src" / f"{name}-server"


def resp_command(*args):
    args = [a if isinstance(a, bytes) else str(a).encode() for a in args]
    return b"*%d\r\n" % len(args) + b"".join(b"$%d\r\n" % len(a) + a + b"\r\n" for a in args)


def read_reply(stream):
    prefix = stream.read(1)
    if not prefix:
        raise ConnectionError("EOF before RESP reply")
    line = stream.readline(1024 * 1024 + 1)
    if not line.endswith(b"\r\n"):
        raise ValueError("invalid or oversized RESP line")
    payload = line[:-2]
    if prefix == b"-":
        raise RuntimeError(payload.decode(errors="replace"))
    if prefix == b"+":
        return payload
    if prefix == b":":
        return int(payload)
    if prefix == b"$":
        length = int(payload)
        if length == -1:
            return None
        if not 0 <= length <= 2 * 1024 * 1024:
            raise ValueError("invalid bulk length")
        data = stream.read(length)
        if len(data) != length or stream.read(2) != b"\r\n":
            raise ValueError("truncated bulk reply")
        return data
    if prefix == b"*":
        length = int(payload)
        if not 0 <= length <= 16384:
            raise ValueError("invalid array length")
        return [read_reply(stream) for _ in range(length)]
    raise ValueError(f"unexpected RESP prefix {prefix!r}")


class Client:
    def __init__(self, port):
        self.socket = socket.create_connection(("127.0.0.1", port), timeout=2)
        self.stream = self.socket.makefile("rb")

    def call(self, *args):
        self.socket.sendall(resp_command(*args))
        return read_reply(self.stream)

    def batch(self, commands):
        # Bound setup/validation I/O; never interleave with a load generator.
        commands = list(commands)
        self.socket.sendall(b"".join(resp_command(*args) for args in commands))
        return [read_reply(self.stream) for _ in commands]

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.stream.close()
        self.socket.close()


def available_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def process_stats(pid):
    stat = Path(f"/proc/{pid}/stat").read_text().rsplit(") ", 1)[1].split()
    status = dict(line.split(":", 1) for line in Path(f"/proc/{pid}/status").read_text().splitlines())
    return {
        "cpu_seconds": (int(stat[11]) + int(stat[12])) / os.sysconf("SC_CLK_TCK"),
        "rss_kib": int(status["VmRSS"].split()[0]),
        "peak_rss_kib": int(status["VmHWM"].split()[0]),
        "threads": int(status["Threads"].strip()),
    }


@contextlib.contextmanager
def server(name, directory, *, port=None, affinity=None, compatibility=False):
    directory = Path(directory).resolve()
    directory.mkdir(parents=True, exist_ok=True)
    port = port or available_port()
    argv = [str(binary(name))]
    if name == "rillcache":
        argv += ["--bind", f"127.0.0.1:{port}", "--threads", "1", "--max-clients", "128",
                 "--max-data-bytes", "268435456", "--idle-timeout-ms", "15000"]
    else:
        argv += ["--bind", "127.0.0.1", "--port", str(port), "--protected-mode", "yes",
                 "--save", "", "--appendonly", "no", "--daemonize", "no", "--dir", str(directory),
                 "--maxclients", "128", "--maxmemory", "268435456", "--maxmemory-policy", "noeviction",
                 "--io-threads", "1", "--timeout", "15"]
        if compatibility:
            argv += ["--enable-debug-command", "yes"]
    if affinity is not None:
        argv = ["taskset", "-c", str(affinity), *argv]
    identity = {"argv": argv, "port": port, "persistence": "disabled", "affinity": affinity}
    save(directory / "server.json", identity)
    with (directory / "server.log").open("wb") as log:
        proc = subprocess.Popen(argv, cwd=directory, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        identity["pid"] = proc.pid
        try:
            deadline = time.monotonic() + 20
            while True:
                if proc.poll() is not None:
                    raise RuntimeError(f"{name} exited during startup: {proc.returncode}")
                try:
                    with Client(port) as client:
                        if client.call("PING") != b"PONG":
                            raise ValueError("readiness PING did not return PONG")
                    break
                except (OSError, RuntimeError, ValueError):
                    if time.monotonic() >= deadline:
                        raise RuntimeError(f"{name} readiness deadline exceeded") from None
                    time.sleep(0.1)
            yield proc, port
        finally:
            identity["exit_before_cleanup"] = proc.poll()
            identity["exit_after_cleanup"] = terminate(proc)
            identity["cleanup"] = "SIGTERM, bounded wait, process-group SIGKILL fallback; joined direct child"
            save(directory / "server.json", identity)


def environment():
    metadata = {key: os.environ.get(key) for key in (
        "GITHUB_SHA", "GITHUB_HEAD_REF", "GITHUB_REF", "GITHUB_BASE_REF", "GITHUB_RUN_ID",
        "GITHUB_RUN_ATTEMPT", "GITHUB_REPOSITORY", "ImageOS", "ImageVersion", "RUNNER_ARCH", "SOURCE_HEAD_SHA")}
    metadata.update({"platform": platform.platform(), "python": platform.python_version(),
                     "cpu_affinity": sorted(os.sched_getaffinity(0)), "pins": PINS, "scope": SCOPE})
    for label, argv in {
        "git_head": ["git", "rev-parse", "HEAD"], "rust": ["rustc", "-Vv"], "cargo": ["cargo", "-V"],
        "cc": ["cc", "--version"], "linker": ["ld", "--version"], "libc": ["ldd", "--version"],
        "tcl": ["tclsh", "-c", ""], "cpu": ["lscpu"], "os": ["uname", "-a"],
    }.items():
        result = subprocess.run(argv, cwd=ROOT, capture_output=True, text=True, timeout=30, check=False)
        metadata[label] = {"exit_code": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
    metadata["os_release"] = Path("/etc/os-release").read_text()
    for name in ("meminfo", "loadavg"):
        metadata[name] = Path(f"/proc/{name}").read_text()
    metadata["tcl_patchlevel"] = subprocess.run(["tclsh"], input="puts [info patchlevel]\n", capture_output=True,
                                               text=True, timeout=5, check=True).stdout.strip()
    return metadata


def summary(text, path):
    Path(path).parent.mkdir(parents=True, exist_ok=True)
    Path(path).write_text(text)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as stream:
            stream.write(text + "\n")
    print(text, flush=True)
