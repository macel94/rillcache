#!/usr/bin/env python3
"""Owned Actions-only TCP packet-counter diagnostic; never throughput evidence."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time

FIELDS = ("segs_out", "segs_in", "data_segs_out", "data_segs_in", "bytes_sent")


def parse_ss(text, pid, expected=8):
    connections = {}
    for line in text.splitlines():
        if not re.search(rf"\bpid={pid},", line):
            raise ValueError("socket is not attributed to the owned server")
        inode = re.search(r"\bino:(\d+)\b", line)
        if inode is None or inode[1] in connections:
            raise ValueError("missing or duplicate socket inode")
        counters = {}
        for field in FIELDS:
            found = re.search(rf"\b{field}:(\d+)\b", line)
            if found is None:
                raise ValueError(f"missing TCP_INFO counter {field}")
            counters[field] = int(found[1])
        connections[inode[1]] = counters
    if len(connections) != expected:
        raise ValueError(f"expected {expected} owned live sockets, got {len(connections)}")
    return connections


def deltas(before, after):
    if set(before) != set(after):
        raise ValueError("socket identities changed during TCP observation")
    total = dict.fromkeys(FIELDS, 0)
    for inode in before:
        for field in FIELDS:
            difference = after[inode][field] - before[inode][field]
            if difference < 0:
                raise ValueError("TCP counter regressed")
            total[field] += difference
    if total["data_segs_out"] <= 0 or total["segs_out"] < total["data_segs_out"]:
        raise ValueError("invalid outgoing segment counters")
    total["ack_only_out"] = total["segs_out"] - total["data_segs_out"]
    total["ack_only_per_data_out"] = total["ack_only_out"] / total["data_segs_out"]
    return total


def process_start(pid):
    stat = Path(f"/proc/{pid}/stat").read_text()
    return int(stat.rsplit(") ", 1)[1].split()[19])


def ss_argv(port):
    return ["ss", "-H", "-O", "-t", "-n", "-i", "-e", "-p",
            "state", "established", f"( sport = :{port} )"]


def capture(directory, phase, pid, port):
    argv = ss_argv(port)
    result = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            text=True, timeout=5, check=True)
    (directory / f"tcp-{phase}.stdout").write_text(result.stdout)
    (directory / f"tcp-{phase}.stderr").write_text(result.stderr)
    if len(result.stdout) > 1024 * 1024:
        raise ValueError("oversized owned TCP socket observation")
    return {"argv": argv, "time": time.monotonic(),
            "sockets": parse_ss(result.stdout, pid)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("TCP observation is Actions-only")
    if args.pid <= 1 or not 1 <= args.port <= 65535:
        raise ValueError("invalid owned server identity")
    result = {"pid": args.pid, "port": args.port, "valid": False,
              "not_throughput_evidence": True}
    try:
        start = process_start(args.pid)
        result["before"] = capture(args.out, "before", args.pid, args.port)
        time.sleep(8)
        result["after"] = capture(args.out, "after", args.pid, args.port)
        if process_start(args.pid) != start:
            raise ValueError("server PID identity changed")
        result["start_ticks"] = start
        result["delta"] = deltas(result["before"]["sockets"], result["after"]["sockets"])
        result["valid"] = True
    finally:
        (args.out / "tcp-info.json").write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
