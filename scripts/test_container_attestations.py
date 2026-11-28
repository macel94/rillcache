#!/usr/bin/env python3
"""Static policy checks for the trusted OCI attestation workflow."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / ".github/workflows/container.yml").read_text()
SECURITY = (ROOT / "SECURITY.md").read_text()
CONTAINERS = (ROOT / "docs/containers.md").read_text()
RUST_POLICY = (ROOT / "tests/container_policy.rs").read_text()


class ContainerAttestationPolicyTests(unittest.TestCase):
    def test_sbom_describes_the_tested_image_and_is_carried_with_its_archive(self):
        self.assertIn("image: rillcache:tested-${{ matrix.arch }}", WORKFLOW)
        self.assertIn("syft-version: v1.57.2", WORKFLOW)
        self.assertIn("upload-artifact: false", WORKFLOW)
        self.assertIn("upload-release-assets: false", WORKFLOW)
        self.assertIn("target/rillcache-${{ matrix.arch }}.spdx.json", WORKFLOW)
        self.assertIn("target/images/rillcache-amd64.spdx.json", WORKFLOW)
        self.assertIn("target/images/rillcache-arm64.spdx.json", WORKFLOW)

    def test_attestations_are_scoped_to_the_trusted_publisher(self):
        publisher = WORKFLOW.split("  publish:\n", 1)[1].split("  public-pull:\n", 1)[0]
        self.assertIn("github.event_name != 'pull_request'", publisher)
        self.assertIn("id-token: write", publisher)
        self.assertIn("attestations: write", publisher)
        self.assertIn("packages: write", publisher)
        self.assertEqual(publisher.count("actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6"), 3)
        self.assertIn("subject-digest: ${{ steps.publish.outputs.index_digest }}", publisher)
        self.assertEqual(publisher.count("push-to-registry: true"), 3)
        self.assertEqual(publisher.count("create-storage-record: false"), 3)
        self.assertNotIn("Promote aliases", publisher)
        self.assertLess(
            publisher.index("Sign the tested arm64 image SBOM"),
            WORKFLOW.index("promote:"),
        )

    def test_consumer_has_no_publishing_permissions_and_verifies_registry_bundles(self):
        consumer = WORKFLOW.split("  public-pull:\n", 1)[1].split("  promote:\n", 1)[0]
        self.assertIn("permissions:\n      contents: read", consumer)
        self.assertNotIn("packages: write", consumer)
        self.assertNotIn("id-token: write", consumer)
        self.assertIn("DOCKER_CONFIG=\"$config\" docker image pull", consumer)
        self.assertIn("export DOCKER_CONFIG=\"$config\"", consumer)
        self.assertIn("GH_TOKEN: ${{ github.token }}", consumer)
        self.assertIn("gh attestation verify", consumer)
        self.assertIn("--bundle-from-oci", consumer)
        self.assertIn("--signer-workflow \"$signer\"", consumer)
        self.assertIn("--predicate-type https://spdx.dev/Document/v2.3", consumer)
        self.assertIn("--source-digest \"$SOURCE_SHA\"", consumer)

    def test_alias_promotion_waits_for_read_only_public_verification(self):
        public_check = WORKFLOW.index("  public-pull:\n")
        promotion = WORKFLOW.split("  promote:\n", 1)[1]
        self.assertGreater(WORKFLOW.index("  promote:\n"), public_check)
        self.assertIn("needs: [publish, public-pull]", promotion)
        self.assertIn("if: github.event_name != 'pull_request'", promotion)
        self.assertIn("packages: write", promotion)
        self.assertIn("IMAGE_NAME@$INDEX_DIGEST", promotion)
        self.assertIn("[[ \"$promoted\" == \"$INDEX_DIGEST\" ]]", promotion)

    def test_rust_policy_checks_the_latest_guard_in_the_promotion_job(self):
        self.assertIn('assert!(promotion.contains("\\\"$head\\\" == \\\"$GITHUB_SHA\\\""));', RUST_POLICY)
        self.assertNotIn('assert!(publish.contains("\\\"$head\\\"', RUST_POLICY)

    def test_security_policy_uses_private_reporting_and_disclaims_unprovided_coverage(self):
        self.assertIn("security/advisories/new", SECURITY)
        self.assertIn("not an SLA", SECURITY)
        self.assertIn("not a vulnerability scan", SECURITY)
        self.assertIn("SECURITY.md", CONTAINERS)


if __name__ == "__main__":
    unittest.main()
