"""Offline tests for the source-grounded OpenWiki freshness audit."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "check_openwiki_freshness.mjs"


RESOLVER = r'''
import { readFile } from "node:fs/promises";
import path from "node:path";
export class RepositoryEvidenceResolver {
  constructor({rootDir}) { this.root = rootDir; }
  async resolve(resource, previousVersion) {
    const body = resource.slice("repo://".length).split("#", 1)[0];
    if (body.startsWith("../") || path.isAbsolute(body)) throw new Error("unsafe path");
    let text;
    try { text = await readFile(path.join(this.root, body), "utf8"); }
    catch (error) { if (error.code === "ENOENT") return null; throw error; }
    if (text.includes("UNSAFE-SYMLINK")) throw new Error("symbolic link rejected");
    const version = text.includes("changed") ? "sha256:changed" : previousVersion;
    return {evidence: {resource, version}, content: text};
  }
}
'''


def make_fixture(root):
    module = root / "fake-openwiki/dist/claims/evidence/repository"
    module.mkdir(parents=True)
    (module / "resolver.js").write_text(RESOLVER)
    claims = root / "repo/openwiki/.claims"
    claims.mkdir(parents=True)
    (root / "repo/openwiki/page.md").write_text("---\ntype: test\n---\nWiki page.\n")
    (root / "repo/openwiki/.page-manifest.json").write_text(json.dumps({
        "schemaVersion": 1,
        "pages": {"/openwiki/page.md": {"pageVersion": "v1"}},
    }))
    (root / "repo/src").mkdir()
    (root / "repo/src/current.rs").write_text("current source\n")
    entries = []
    for name, resource, version in (
        ("ok", "repo://src/current.rs#L1-L1", "sha256:old"),
        ("changed", "repo://src/changed.rs#L1-L1", "sha256:old"),
        ("missing", "repo://src/missing.rs#L1-L1", "sha256:old"),
        ("unsafe", "repo://../outside.rs#L1-L1", "sha256:old"),
    ):
        if name == "changed":
            (root / "repo/src/changed.rs").write_text("changed source\n")
        entries.append({"id": name, "statement": name, "evidence": [{"resource": resource, "version": version}]})
    (claims / "page.json").write_text(json.dumps({"pageVersion":"v1", "claims":entries}))
    return root / "repo", root / "fake-openwiki"


class OpenWikiFreshnessTests(unittest.TestCase):
    def run_check(self, repo, module):
        env = dict(os.environ, OPENWIKI_MODULE_ROOT=str(module))
        return subprocess.run(["node", str(SCRIPT), str(repo)], text=True,
                              capture_output=True, env=env, timeout=15)

    def test_clean_claims_pass_and_stale_missing_and_unsafe_sources_are_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, module = make_fixture(Path(tmp))
            result = self.run_check(repo, module)
            self.assertEqual(1, result.returncode, result.stderr)
            report = json.loads(result.stdout)
            self.assertEqual(4, report["claims_checked"])
            self.assertEqual({"changed", "missing", "unsafe"},
                             {item["claim"] for item in report["stale_evidence"]})
            (repo / "src/changed.rs").write_text("same evidence\n")
            # A deterministic resolver that returns the recorded version marks
            # these fixture references current; missing/unsafe remain findings.
            result = self.run_check(repo, module)
            self.assertEqual(1, result.returncode)
            report = json.loads(result.stdout)
            reasons = " ".join(item["reason"] for item in report["stale_evidence"])
            self.assertIn("source or range disappeared", reasons)

    def test_clean_audit_keeps_stdout_json_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, module = make_fixture(Path(tmp))
            claims_path = repo / "openwiki/.claims/page.json"
            sidecar = json.loads(claims_path.read_text())
            sidecar["claims"] = [claim for claim in sidecar["claims"] if claim["id"] == "ok"]
            claims_path.write_text(json.dumps(sidecar))
            result = self.run_check(repo, module)
            self.assertEqual(0, result.returncode, result.stderr)
            report = json.loads(result.stdout)
            self.assertEqual([], report["stale_evidence"])
            self.assertIn("OpenWiki freshness:", result.stderr)
            self.assertNotIn("OpenWiki freshness:", result.stdout)

    def test_missing_manifest_or_malformed_claims_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, module = make_fixture(Path(tmp))
            (repo / "openwiki/.claims/page.json").write_text("{}")
            result = self.run_check(repo, module)
            self.assertEqual(1, result.returncode)
            self.assertIn("invalid Claims sidecar", result.stderr)

    def test_missing_resolver_location_is_not_silently_skipped(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, module = make_fixture(Path(tmp))
            (module / "dist/claims/evidence/repository/resolver.js").unlink()
            result = self.run_check(repo, module)
            self.assertNotEqual(0, result.returncode)


if __name__ == "__main__":
    unittest.main()
