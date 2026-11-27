#!/usr/bin/env python3
"""Validate the security review registry (readiness S1/S2/S5 evidence index).

The registry at ``docs/security/review-registry.json`` records the scoped threat
model, the tracked findings and the sign-off records for the declared profile.
This checker makes those records machine-checkable so an artifact cannot silently
degrade into an attestation:

* every threat needs controls, existing evidence paths and a residual risk,
* every finding needs a known threat, an honest status and -- when it claims
  closure -- both closure and retest evidence that exist in the tree,
* every open finding needs a tracking gap id (``S1``/``O1``/``R5``/...),
* a project sign-off needs a reviewer and date, and
* an independent sign-off can only be ``signed`` with a named reviewer, a date
  and a signature reference, otherwise it must be ``outstanding`` with a
  blocking note. The review document must not claim a completed independent
  review while the registry says the signature is outstanding.

Usage:
    python3 scripts/security_review.py --check
    python3 scripts/security_review.py --check --root <repo>
"""

from __future__ import annotations

import argparse
import datetime
import json
import re
import sys
from pathlib import Path

REGISTRY_PATH = Path("docs/security/review-registry.json")
THREAT_STATUSES = {"mitigated", "accepted", "open"}
FINDING_STATUSES = {"closed", "risk-accepted", "open"}
SIGNOFF_STATUSES = {"signed", "outstanding"}
GAP_ID = re.compile(r"\b[A-Z]\d{1,2}\b")
ISO_DATE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
OVERCLAIM = re.compile(
    r"independent (review|sign-?off) (is )?(complete|completed|signed)",
    re.IGNORECASE,
)


class Invalid(Exception):
    pass


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise Invalid(message)


def _existing_paths(root: Path, values: object, where: str) -> list[str]:
    _require(isinstance(values, list) and values, f"{where}: expected a non-empty list")
    paths = []
    for value in values:
        _require(isinstance(value, str) and value.strip(), f"{where}: empty path")
        target = root / value
        _require(target.exists(), f"{where}: missing evidence path {value}")
        paths.append(value)
    return paths


def _unique(ids: list[str], where: str) -> None:
    seen: set[str] = set()
    for item in ids:
        _require(isinstance(item, str) and item, f"{where}: empty id")
        _require(item not in seen, f"{where}: duplicate id {item}")
        seen.add(item)


def load(root: Path) -> tuple[dict, str]:
    path = root / REGISTRY_PATH
    _require(path.exists(), f"missing registry {REGISTRY_PATH}")
    raw = path.read_text(encoding="utf-8")
    try:
        return json.loads(raw), raw
    except json.JSONDecodeError as error:  # pragma: no cover - reported, not raised
        raise Invalid(f"{REGISTRY_PATH}: invalid JSON: {error}") from error


def validate(root: Path) -> dict:
    data, _ = load(root)
    _require(data.get("schema") == 1, "schema must be 1")

    review = data.get("review")
    _require(isinstance(review, dict), "missing review block")
    _require(bool(review.get("profile")), "review.profile is required")
    for key in ("threat_model", "scope_document"):
        _existing_paths(root, [review.get(key)], f"review.{key}")
    _require(
        ISO_DATE.match(str(review.get("scope_defined", ""))) is not None,
        "review.scope_defined must be an ISO date",
    )
    datetime.date.fromisoformat(review["scope_defined"])
    _require(bool(review.get("status")), "review.status is required")
    _require(bool(review.get("external_signature")), "review.external_signature is required")

    threats = data.get("threats")
    _require(isinstance(threats, list) and threats, "threats must be a non-empty list")
    _unique([t.get("id") for t in threats], "threats")
    threat_ids = {t["id"] for t in threats}
    for threat in threats:
        where = f"threat {threat['id']}"
        _require(threat.get("status") in THREAT_STATUSES, f"{where}: bad status")
        _require(bool(threat.get("surface")), f"{where}: surface is required")
        _require(bool(threat.get("description")), f"{where}: description is required")
        _require(bool(threat.get("residual_risk")), f"{where}: residual_risk is required")
        _require(
            isinstance(threat.get("controls"), list) and threat["controls"],
            f"{where}: controls must be a non-empty list",
        )
        _existing_paths(root, threat.get("evidence"), f"{where}.evidence")

    findings = data.get("findings", [])
    _require(isinstance(findings, list), "findings must be a list")
    _unique([f.get("id") for f in findings], "findings")
    for finding in findings:
        where = f"finding {finding['id']}"
        _require(finding.get("threat") in threat_ids, f"{where}: unknown threat")
        status = finding.get("status")
        _require(status in FINDING_STATUSES, f"{where}: bad status {status!r}")
        _require(bool(finding.get("title")), f"{where}: title is required")
        _require(bool(finding.get("owner")), f"{where}: owner is required")
        if status == "closed":
            _existing_paths(root, finding.get("closure_evidence"), f"{where}.closure_evidence")
            _existing_paths(root, finding.get("retest_evidence"), f"{where}.retest_evidence")
        if status == "open":
            gap = str(finding.get("tracking", ""))
            _require(
                GAP_ID.search(gap) is not None,
                f"{where}: open findings need a checklist gap id in tracking",
            )
        if status == "risk-accepted":
            _require(bool(finding.get("risk_note")), f"{where}: risk_note is required")

    signoffs = data.get("signoffs")
    _require(isinstance(signoffs, list) and signoffs, "signoffs must be a non-empty list")
    _unique([s.get("id") for s in signoffs], "signoffs")
    signed_independent = False
    for signoff in signoffs:
        where = f"signoff {signoff['id']}"
        kind = signoff.get("kind")
        _require(kind in {"project", "independent"}, f"{where}: bad kind {kind!r}")
        status = signoff.get("status")
        _require(status in SIGNOFF_STATUSES, f"{where}: bad status {status!r}")
        if status == "signed":
            _require(bool(signoff.get("reviewer")), f"{where}: reviewer is required")
            _require(
                ISO_DATE.match(str(signoff.get("date", ""))) is not None,
                f"{where}: signed sign-offs need an ISO date",
            )
            _require(
                isinstance(signoff.get("scope"), list) and signoff["scope"],
                f"{where}: signed sign-offs need a scope list",
            )
            if kind == "independent":
                _existing_paths(root, [signoff.get("signature")], f"{where}.signature")
                signed_independent = True
        else:
            _require(bool(signoff.get("blocking_note")), f"{where}: blocking_note is required")
            _require(
                not signoff.get("reviewer"),
                f"{where}: an outstanding sign-off must not name a reviewer",
            )

    _require_single_independent(signoffs)

    scope_doc = (root / review["scope_document"]).read_text(encoding="utf-8")
    threat_doc = (root / review["threat_model"]).read_text(encoding="utf-8")
    for threat in threats:
        _require(threat["id"] in threat_doc, f"threat {threat['id']} is not in the threat model")
        _require(threat["id"] in scope_doc, f"threat {threat['id']} is not in the review scope")
    for finding in findings:
        _require(finding["id"] in scope_doc, f"finding {finding['id']} is not in the review scope")
    if not signed_independent:
        _require(
            OVERCLAIM.search(scope_doc) is None,
            "review scope claims a completed independent review while the sign-off is outstanding",
        )
        _require(
            OVERCLAIM.search(threat_doc) is None,
            "threat model claims a completed independent review while the sign-off is outstanding",
        )
    return data


def _require_single_independent(signoffs: list[dict]) -> None:
    independent = [s for s in signoffs if s.get("kind") == "independent"]
    _require(len(independent) == 1, "exactly one independent sign-off record is required")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="validate the registry")
    parser.add_argument("--root", default=".", help="repository root")
    args = parser.parse_args(argv)
    if not args.check:
        parser.error("--check is required")
    root = Path(args.root).resolve()
    try:
        data = validate(root)
    except Invalid as error:
        print(f"Invalid security review registry: {error}", file=sys.stderr)
        return 1
    signed = [s for s in data["signoffs"] if s["status"] == "signed" and s["kind"] == "independent"]
    verdict = "independent sign-off recorded" if signed else "independent sign-off outstanding"
    print(
        "Security review registry valid: "
        f"{len(data['threats'])} threats, {len(data.get('findings', []))} findings, {verdict}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
