import contextlib
import hashlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from verify_container_index import ARCHES, INDEX_TYPES, MANIFEST_TYPES, MAX_INDEX_BYTES, main, verify

EXPECTED = {"amd64": "sha256:" + "1" * 64, "arm64": "sha256:" + "2" * 64}


def digest(raw):
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def index(kind=0):
    return {"schemaVersion": 2, "mediaType": INDEX_TYPES[kind], "manifests": [
        {"mediaType": MANIFEST_TYPES[kind], "digest": EXPECTED[arch], "size": 1234,
         "platform": {"os": "linux", "architecture": arch}} for arch in reversed(ARCHES)
    ]}


def raw_index(value):
    return json.dumps(value, separators=(",", ":")).encode()


class VerifyContainerIndexTests(unittest.TestCase):
    def test_oci_and_docker_index_check_exact_bytes(self):
        for kind in range(len(INDEX_TYPES)):
            raw = raw_index(index(kind))
            self.assertEqual(verify(raw, digest(raw), EXPECTED), EXPECTED)
            with self.assertRaisesRegex(ValueError, "raw index digest mismatch"):
                verify(raw + b"\n", digest(raw), EXPECTED)

    def test_wrong_platform_and_swapped_digests(self):
        for patch, problem in (
            ({"platform": {"os": "linux", "architecture": "amd64"}, "digest": EXPECTED["amd64"]},
             "one amd64 and one arm64"),
            ({"platform": {"os": "windows", "architecture": "arm64"}}, "Linux platform"),
            ({"digest": EXPECTED["amd64"]}, "linux/arm64 descriptor digest mismatch"),
            ({"size": True}, "size must be a positive integer"),
            ({"mediaType": INDEX_TYPES[0]}, "expected an image manifest descriptor"),
        ):
            value = index()
            value["manifests"][0].update(patch)
            raw = raw_index(value)
            with self.subTest(patch=patch), self.assertRaisesRegex(ValueError, problem):
                verify(raw, digest(raw), EXPECTED)

    def test_missing_extra_and_ambiguous_items_rejected(self):
        for keep in (0, 1, 3):
            value = index()
            value["manifests"] = (value["manifests"] * 2)[:keep]
            raw = raw_index(value)
            with self.assertRaisesRegex(ValueError, "exactly two"):
                verify(raw, digest(raw), EXPECTED)
        for raw, problem in ((b'{"schemaVersion": 2, "schemaVersion": 2}', "duplicate JSON member"),
                             (b'{"schemaVersion": NaN}', "invalid JSON constant"),
                             (b" " * (MAX_INDEX_BYTES + 1), "exceeds 256 KiB")):
            with self.assertRaisesRegex(ValueError, problem):
                verify(raw, digest(raw), EXPECTED)

    def test_cli_reports_scope_without_claiming_signatures_or_child_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "index.json"
            raw = raw_index(index())
            path.write_bytes(raw)
            out = io.StringIO()
            argv = [str(path), "--digest", digest(raw), "--amd64", EXPECTED["amd64"], "--arm64", EXPECTED["arm64"]]
            with contextlib.redirect_stdout(out):
                self.assertEqual(main(argv), 0)
            report = json.loads(out.getvalue())
            self.assertEqual(report["platform_descriptors"], {f"linux/{arch}": value for arch, value in EXPECTED.items()})
            for name in ("child_manifests_verified", "signatures_verified", "attestations_verified"):
                self.assertIs(report[name], False)
            path.write_bytes(raw + b"\n")
            with contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(main(argv), 1)
            self.assertIn("raw index digest mismatch", err.getvalue())


if __name__ == "__main__":
    unittest.main()
