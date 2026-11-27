# Supported platforms and filesystem contract

**Status: single platform supported for the declared profile.** This page is the
authoritative platform matrix for profile `rillcache-standalone-0.1`; it is a
compatibility boundary, not a portability claim. The canonical
[readiness checklist](production-readiness-gaps.md) tracks the remaining work
(L5).

## Matrix

| Platform | State | Evidence |
| --- | --- | --- |
| Linux `x86_64`, glibc | **Supported and tested** | The full test matrix (`make test`: default, one-worker, four-worker, plus the constrained and async suites), the Tcl/command/client compatibility jobs, the storage-fault and restore drills, and both protected performance gates all run on `ubuntu-24.04` (`x86_64`). |
| Linux `aarch64`, glibc | **Built, behaviourally untested — excluded from the support claim** | The container workflow builds and publishes an `arm64` image on `ubuntu-24.04-arm`, which proves the release build compiles and links for that target; no test, compatibility, fault, restore or performance evidence exists for it. Treat it as an unsupported platform that happens to build. |
| Linux `aarch64`/`x86_64`, musl (release container image) | **Built, behaviourally untested — excluded from the support claim** | The image is statically linked against musl; only the `x86_64` image path is exercised by the container smoke test. |
| macOS, Windows, BSD, Solaris | **Unsupported** | The journal relies on Unix permissions, `flock`-style exclusive ownership, `/proc` for process identity and RSS, and signals for graceful shutdown. No port exists and none is claimed; the non-Unix code paths are compiled stubs, not a supported runtime. |
| Other Linux architectures (32-bit, big-endian, `riscv64`, `s390x`, …) | **Unsupported** | No CI build or test; the journal-encoding and alignment assumptions are only exercised on the two targets above. |
| Non-glibc/non-musl libc, exotic allocators | **Unsupported** | The image is the only supported libc because it is the only one built and smoke-tested. |

A run on an untested platform is not evidence of support. Anything outside the
first row must be validated by the operator, and the profile's availability,
durability and performance statements do not apply to it.

## Filesystem contract

- **Supported:** local block-backed filesystems (ext4, xfs, btrfs on the tested
  kernel) and local NVMe/SATA devices, with the journal directory owned by the
  service account and mode `0700`.
- **Excluded:** network and shared filesystems (NFS, CIFS/SMB, CephFS, GlusterFS,
  object-storage FUSE mounts), because the journal's exclusive ownership lock is a
  local advisory lock and there is no distributed lease. Two hosts must never
  write the same `--dir`.
- **Excluded:** filesystems without working `fsync`/`fdatasync` semantics,
  write-back caches that reorder durable data behind an acknowledged `fsync`, and
  copy-on-write snapshots used as a backup substitute (use the verified archive
  procedure instead).
- **tmpfs is a test/dev medium only:** it honours `fsync` as a no-op, so it
  cannot be used to claim durability; the fault drills use a size-bounded tmpfs
  only to provoke real `ENOSPC`, not as a durability claim.
- **Encryption at rest** is delegated to the platform (LUKS/dm-crypt, cloud
  volume encryption); the journal is plain RESP, as documented in
  [persistence](persistence.md#encryption-at-rest-and-key-management).

## What a platform support claim requires

Adding a row to the supported set requires, on that exact platform: the full test
matrix, the pinned compatibility jobs, the storage-fault and restore drills, and
both protected performance gates — with the results retained. Until then the row
stays excluded, as stated above.
