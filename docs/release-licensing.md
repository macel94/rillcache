# Release licensing and third-party obligations

This document records the L1 licensing decision, the third-party notice review
and the obligations that distribution channels must satisfy. It is a software
engineering record, not legal advice; a qualified reviewer can still change the
outcome.

## Decision

Rillcache is released under the **BSD 3-Clause License** (`LICENSE` at the
repository root) and declares it in package metadata as
`license = "BSD-3-Clause"` in `Cargo.toml`.

Reasons for that choice:

- It matches the license lineage of the Redis/Valkey ecosystem this server is
  wire-compatible with, so downstream users already have a BSD-3-Clause review
  path and no new copyleft obligation is introduced.
- It is permissive for both source and binary redistribution, including the
  statically linked container image, and keeps the attribution and
  non-endorsement obligations explicit.
- It keeps the licensing of the produced artifact independent of the licenses of
  vendored dependencies, which are all permissive or permissive-compatible.

## Third-party inventory

Vendored crates (`vendor/`) and runtime components were reviewed against their
own license files. The effective obligation for every entry is attribution:
retain the license text and notices with source and binary distributions.

| Component | Version | License | Obligation |
| --- | --- | --- | --- |
| `tokio` | 1.54.1 | MIT | Copyright notice |
| `mio` | 1.2.4 | MIT | Copyright notice |
| `bytes` | 1.10.1 | MIT | Copyright notice |
| `core_affinity` | 0.8.3 | MIT/Apache-2.0 | Notice + Apache-2.0 terms if elected |
| `rustls` | 0.23.46 | Apache-2.0 OR ISC OR MIT | Notice; Apache-2.0 NOTICE text |
| `rustls-pemfile` | 2.2.0 | Apache-2.0 OR ISC OR MIT | Notice; Apache-2.0 NOTICE text |
| `rustls-webpki` | 0.103.x | ISC | Copyright notice |
| `rustls-pki-types` | 1.x | MIT OR Apache-2.0 | Notice |
| `ring` | 0.17.x | Apache-2.0 AND ISC | Both notices; Apache-2.0 NOTICE text |
| `untrusted` | 0.9.x | ISC | Copyright notice |
| `subtle` | 2.x | BSD-3-Clause | Copyright + disclaimer |
| `libc`, `getrandom`, `cfg-if`, `once_cell`, `pin-project-lite`, `socket2`, `num_cpus`, `hermit-abi`, `cc`, `find-msvc-tools`, `shlex`, `wasi` | vendored | MIT OR Apache-2.0 (or `wasi` with LLVM exception) | Notice; LLVM exception text where applicable |
| Rust standard library/runtime 1.99.1 | toolchain | MIT OR Apache-2.0 | `container/notices/Rust-MIT.txt` |
| musl libc 1.2.5 | 1.2.5 | MIT | `container/notices/musl-COPYRIGHT.txt` |
| LLVM libunwind | toolchain | Apache-2.0 WITH LLVM-exception | `container/notices/LLVM-LICENSE.txt` |
| xxHash (XXH3-64 spec) | spec/safe scalar implementation | BSD-2-Clause | `container/notices/xxHash-BSD-2-Clause.txt` |

No vendored component is copyleft, so linking it into the BSD-3-Clause binary
creates no additional source-disclosure obligation. The Apache-2.0 components
(`ring`, `rustls`, `rustls-pemfile`) require their notice text to travel with
binary distributions; the files above satisfy that.

## Distribution obligations

- **Source archive:** ship `LICENSE` together with `container/notices/` so the
  attribution set is complete, and keep the vendored license files untouched.
- **Container image:** the `container/notices/` directory is copied into the
  image; it retains the runtime component notices and now also points at the
  project `LICENSE`. Attribution for the Rust, musl, LLVM and xxHash components
  is preserved exactly as shipped.
- **Package metadata:** `Cargo.toml` declares `license = "BSD-3-Clause"`, so
  scanners and downstream packagers can identify the license without reading the
  repository.
- **Modifications:** redistribution of modified sources must carry the BSD-3
  clause 2 notice and must not imply endorsement by the copyright holder.

## Limits of this record

- This is not a legal opinion and does not cover export-control, patent or
  trademark analysis.
- Only the dependency set vendored in this repository at the reviewed commit was
  inventoried; adding or upgrading a dependency requires repeating the review.
- The container image is an experimental in-memory server and remains
  **NOT PRODUCTION READY** until the remaining checklist items close.

## Evidence

- `LICENSE` — BSD 3-Clause text, copyright 2026 Francesco Belacca.
- `Cargo.toml` — `license = "BSD-3-Clause"`.
- `container/notices/README.txt` — updated notice provenance for the image.
- Vendored license files under `vendor/*/LICENSE*` reviewed per the table above.
