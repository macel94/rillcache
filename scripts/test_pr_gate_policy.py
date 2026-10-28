"""Offline fail-closed coverage/drift tests. Never writes GitHub settings."""
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("gate_policy", ROOT / "scripts/pr_gate_policy.py")
gate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gate)


class FakeAPI:
    def __init__(self):
        self.policy = json.loads((ROOT / ".github/main-ruleset.json").read_text())
        self.effective = copy.deepcopy(self.policy["rules"])
        self.workflows = {"total_count": 11, "workflows": [
            {"path": f".github/workflows/{path}", "state": "active"}
            for path in gate.load_inventory()
        ] + [{"path": "dynamic/dependabot/update-graph", "state": "active"}]}
        self.calls = []

    def get(self, path):
        self.calls.append(path)
        if path == f"rulesets/{gate.RULESET_ID}": return self.policy
        if path == "rules/branches/main": return self.effective
        if path == "actions/workflows?per_page=100": return self.workflows
        raise AssertionError(path)


class GatePolicyTests(unittest.TestCase):
    def fixture(self, root):
        shutil.copytree(ROOT / ".github/workflows", root / ".github/workflows")
        for path in ("main-ruleset.json", "pr-workflow-gates.json"):
            shutil.copyfile(ROOT / ".github" / path, root / ".github" / path)

    def test_all_ten_workflows_six_pr_workflows_and_fourteen_contexts_are_accounted_for(self):
        inventory = gate.audit_local()
        self.assertEqual(10, len(inventory))
        self.assertEqual(6, len(gate.required_workflows()))
        checks = gate.required_checks()
        self.assertEqual(14, len(checks))
        self.assertTrue(all(c["integration_id"] == 15368 for c in checks))
        self.assertIn({"context": "Valkey 4-CPU performance gate", "integration_id": 15368}, checks)
        self.assertEqual(6, sum(c["context"].startswith("Tcl ") for c in checks))

    def test_live_rules_effective_branch_and_workflow_activation_all_must_match(self):
        api = FakeAPI()
        gate.audit_live(api)
        self.assertEqual([f"rulesets/{gate.RULESET_ID}", "rules/branches/main", "actions/workflows?per_page=100"], api.calls)

    def test_the_actual_two_gate_merge_gap_is_detected(self):
        api = FakeAPI()
        checks = next(r["parameters"] for r in api.policy["rules"] if r["type"] == "required_status_checks")
        checks["required_status_checks"] = checks["required_status_checks"][:2]
        with self.assertRaisesRegex(gate.PolicyError, "LIVE RULESET DRIFT.*Valkey 4-CPU"):
            gate.audit_live(api)

    def test_every_missing_or_wrong_app_required_context_fails(self):
        for index in range(14):
            for mode in ("missing", "wrong-app"):
                with self.subTest(index=index, mode=mode):
                    api = FakeAPI()
                    checks = next(r["parameters"]["required_status_checks"] for r in api.policy["rules"] if r["type"] == "required_status_checks")
                    if mode == "missing": checks.pop(index)
                    else: checks[index]["integration_id"] = None
                    with self.assertRaisesRegex(gate.PolicyError, "LIVE RULESET DRIFT"):
                        gate.audit_live(api)

    def test_bypass_disabled_enforcement_or_relaxed_freshness_and_reviews_fail(self):
        for mutate in (
            lambda p: p.update(bypass_actors=[{"actor_id": 1}]),
            lambda p: p.update(enforcement="evaluate"),
            lambda p: p["conditions"]["ref_name"]["exclude"].append("refs/heads/main"),
            lambda p: next(r["parameters"] for r in p["rules"] if r["type"] == "required_status_checks").update(strict_required_status_checks_policy=False),
            lambda p: next(r["parameters"] for r in p["rules"] if r["type"] == "pull_request").update(require_code_owner_review=False),
        ):
            api = FakeAPI(); mutate(api.policy)
            with self.assertRaises(gate.PolicyError): gate.audit_live(api)

    def test_effective_branch_rules_are_checked_not_only_ruleset_json(self):
        api = FakeAPI()
        api.effective = [r for r in api.effective if r["type"] != "required_status_checks"]
        with self.assertRaisesRegex(gate.PolicyError, "effective main"):
            gate.audit_live(api)

    def test_missing_or_disabled_required_workflow_fails_but_platform_jobs_are_not_pr_gates(self):
        for path in gate.required_workflows():
            api = FakeAPI()
            next(w for w in api.workflows["workflows"] if w["path"] == f".github/workflows/{path}")["state"] = "disabled_manually"
            with self.subTest(path=path), self.assertRaisesRegex(gate.PolicyError, "absent/disabled"):
                gate.audit_live(api)
        gate.audit_live(FakeAPI())

    def test_new_workflow_or_pr_job_cannot_be_silently_advisory(self):
        for change in ("workflow", "job"):
            with tempfile.TemporaryDirectory() as name:
                root = Path(name); self.fixture(root)
                if change == "workflow":
                    (root / ".github/workflows/forgotten.yml").write_text("on:\n  pull_request:\njobs:\n  forgotten:\n    runs-on: ubuntu-latest\n")
                else:
                    p = root / ".github/workflows/ci.yml"
                    p.write_text(p.read_text() + "\n  forgotten:\n    runs-on: ubuntu-latest\n")
                with self.subTest(change=change), self.assertRaisesRegex(gate.PolicyError, "uncatalogued"):
                    gate.audit_local(root)

    def test_optional_job_cannot_reuse_a_required_context(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name); self.fixture(root)
            p = root / ".github/workflows/copilot-auth-check.yml"
            p.write_text(p.read_text().replace("  check:\n", "  check:\n    name: Tests and offline CI\n"))
            with self.assertRaisesRegex(gate.PolicyError, "collides"):
                gate.audit_local(root)

    def test_new_matrix_member_must_be_required_too(self):
        for path, old, new in (
            ("container.yml", "          - arch: arm64", "          - arch: riscv64\n            runner: ubuntu-24.04\n          - arch: arm64"),
            ("upstream-compatibility.yml", "reference: [redis, valkey]", "reference: [redis, valkey, other]"),
        ):
            with tempfile.TemporaryDirectory() as name:
                root = Path(name); self.fixture(root)
                p = root / ".github/workflows" / path
                p.write_text(p.read_text().replace(old, new))
                with self.subTest(path=path), self.assertRaisesRegex(gate.PolicyError, "matrix contexts differ"):
                    gate.audit_local(root)

    def test_path_exemptions_skipped_required_jobs_and_continue_on_error_fail(self):
        for old, new in (
            ("  pull_request:\n", "  pull_request:\n    paths: ['src/**']\n"),
            ("  verify:\n", "  verify:\n    if: false\n"),
            ("  verify:\n", "  verify:\n    if:\n      false\n"),
            ("  verify:\n", "  verify:\n    continue-on-error: true\n"),
        ):
            with tempfile.TemporaryDirectory() as name:
                root = Path(name); self.fixture(root)
                p = root / ".github/workflows/ci.yml"
                p.write_text(p.read_text().replace(old, new))
                with self.assertRaises(gate.PolicyError): gate.audit_local(root)

    def test_publication_job_exemption_cannot_enable_writer_on_pr(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name); self.fixture(root)
            p = root / ".github/workflows/container.yml"
            p.write_text(p.read_text().replace("github.event_name != 'pull_request'", "true"))
            with self.assertRaisesRegex(gate.PolicyError, "publication/writer"):
                gate.audit_local(root)

    def test_api_adapter_is_bounded_get_only_and_does_not_echo_secrets(self):
        response = subprocess.CompletedProcess([], 0, '{"ok": true}', "")
        with patch.object(subprocess, "run", return_value=response) as run:
            self.assertEqual({"ok": True}, gate.GitHub().get("rules/branches/main"))
            args, kwargs = run.call_args
            self.assertEqual(["gh", "api", "--method", "GET", f"repos/{gate.REPO}/rules/branches/main"], args[0])
            self.assertEqual(20, kwargs["timeout"])
        response = subprocess.CompletedProcess([], 1, "sensitive body", "private token")
        with patch.object(subprocess, "run", return_value=response), self.assertRaises(gate.PolicyError) as failure:
            gate.GitHub().get("rules/branches/main")
        self.assertNotIn("sensitive", str(failure.exception))
        self.assertNotIn("private token", str(failure.exception))


if __name__ == "__main__":
    unittest.main()
