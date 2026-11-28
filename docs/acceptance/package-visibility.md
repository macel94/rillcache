# Container package visibility record (L7)

GHCR packages are **private by default**. Every release can pass the hosted
workflows and still be unpullable, because making a package public is a manual
action in the package settings. This record makes that step explicit and checked:

- `python3 scripts/check_package_visibility.py --check` runs in the required CI
  graph (`tests/documentation.rs`) and fails when a released version has no row,
  a row names an unreleased version, a row is malformed, or a `public-verified`
  row is missing the tested digests, the verifying maintainer, the date or the
  method.
- `scripts/cut-release.sh --publish` runs the same checker with
  `--require-public`, so a tag is refused while the package is unverified.
- `--verify-remote --version <x.y.z>` asks the GitHub API for the live package
  visibility and refuses when it is not `public` or when the record disagrees.

## What the release owner does

1. Pull both tested platform digests **anonymously**, for example:
   `docker pull ghcr.io/macel94/rillcache@sha256:<amd64 digest>` and the same for
   arm64, with no registry credentials configured.
2. Make the package public in the package settings if the pull fails, then repeat
   step 1 until both platforms pull without credentials.
3. Record the version, both digests, `public-verified`, the maintainer, the date
   and the method (the exact commands used) in the table below.
4. Run `python3 scripts/check_package_visibility.py --verify-remote --version <x.y.z>`.

## Record

| Version | amd64 digest | arm64 digest | Visibility | Verified by | Verified at | Method |
| --- | --- | --- | --- | --- | --- | --- |
| 0.1.0 | - | - | unverified | - | - | Not verified: no anonymous digest-pinned pull has been recorded for this release yet, so the package may still be private. |

An `unverified` row is honest, not a claim: it blocks the next `--publish` until a
maintainer records the anonymous pull above.
