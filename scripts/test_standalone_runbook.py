#!/usr/bin/env python3
"""Structural acceptance checks for the standalone operator runbook (not fault tests)."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
DOC = ROOT / "docs/standalone-incident-runbook.md"
text = DOC.read_text()
required = {
    "service/node loss": r"Service or node unavailable",
    "disk failure boundary": r"Disk errors or suspected corruption",
    "single-node network interruption": r"Network interruption at one node",
    "client reconnect": r"Client reconnect",
    "no promotion/reconciliation": r"No promotion or reconciliation procedure is supported or exercised",
    "no proven durability": r"not evidence of survival across every",
    "ambiguous mutation retry": r"outcome ambiguous",
}
for name, pattern in required.items():
    assert re.search(pattern, text, re.IGNORECASE), f"missing runbook acceptance: {name}"
for link in re.findall(r"\]\(([^)#]+\.md)(?:#[^)]*)?\)", text):
    assert (DOC.parent / link).resolve().is_file(), f"broken local link: {link}"
print(f"standalone runbook checks passed ({len(required)} acceptance checks)")
