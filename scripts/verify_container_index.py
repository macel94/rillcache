#!/usr/bin/env python3
"""Compare raw container index bytes and platform descriptors to trusted digests."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

MAX_INDEX_BYTES = 256 * 1024
INDEX_TYPES = (
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
)
MANIFEST_TYPES = (
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
)
ARCHES = ("amd64", "arm64")
DIGEST = re.compile(r"sha256:[0-9a-f]{64}")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON member")
        result[key] = value
    return result


def invalid_constant(_):
    raise ValueError("invalid JSON constant")


def verify(raw: bytes, index_digest: str, expected: dict[str, str]) -> dict[str, str]:
    if len(raw) > MAX_INDEX_BYTES:
        raise ValueError("index exceeds 256 KiB")
    if f"sha256:{hashlib.sha256(raw).hexdigest()}" != index_digest:
        raise ValueError("raw index digest mismatch")
    index = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object,
                       parse_constant=invalid_constant)
    if (not isinstance(index, dict) or type(index.get("schemaVersion")) is not int
            or index["schemaVersion"] != 2 or index.get("mediaType") not in INDEX_TYPES):
        raise ValueError("expected an OCI index or Docker manifest list with schemaVersion 2")
    manifests = index.get("manifests")
    if not isinstance(manifests, list) or len(manifests) != len(ARCHES):
        raise ValueError("expected exactly two platform descriptors")
    observed = {}
    for manifest in manifests:
        if not isinstance(manifest, dict) or manifest.get("mediaType") not in MANIFEST_TYPES:
            raise ValueError("expected an image manifest descriptor")
        platform = manifest.get("platform")
        if not isinstance(platform, dict) or platform.get("os") != "linux":
            raise ValueError("expected a Linux platform")
        arch = platform.get("architecture")
        if arch not in ARCHES or arch in observed:
            raise ValueError("expected one amd64 and one arm64 descriptor")
        size = manifest.get("size")
        if type(size) is not int or size <= 0:
            raise ValueError("manifest descriptor size must be a positive integer")
        if manifest.get("digest") != expected[arch]:
            raise ValueError(f"linux/{arch} descriptor digest mismatch")
        observed[arch] = expected[arch]
    return observed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("index", type=Path, help="exact registry response body, not reserialized JSON")
    for field in ("digest", *ARCHES):
        parser.add_argument(f"--{field}", required=True, metavar="SHA256")
    args = parser.parse_args(argv)
    expected = {arch: getattr(args, arch) for arch in ARCHES}
    if not all(DIGEST.fullmatch(value) for value in (args.digest, *expected.values())):
        parser.error("expected canonical sha256: digests")
    if len(set(expected.values())) != len(ARCHES):
        parser.error("platform manifest digests must differ")
    try:
        with args.index.open("rb") as source:
            observed = verify(source.read(MAX_INDEX_BYTES + 1), args.digest, expected)
    except (UnicodeError, OSError, RecursionError, json.JSONDecodeError):
        print("container index check failed: unreadable or malformed index", file=sys.stderr)
        return 1
    except ValueError as error:
        print(f"container index check failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps({"index_digest": args.digest,
                      "platform_descriptors": {f"linux/{arch}": digest for arch, digest in observed.items()},
                      "child_manifests_verified": False,
                      "signatures_verified": False, "attestations_verified": False}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
