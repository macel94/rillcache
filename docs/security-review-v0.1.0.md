# Rillcache v0.1.0 — independent automated security review

**Reviewed 2026-11-27 UTC. No production approval.**

- **Reviewer:** OpenAI Codex `gpt-6.2-sol` (`openai-codex/gpt-6.2-sol`, as reported by the review harness), acting as an automated reviewer. This was a separate model in a fresh, separate session from the authoring agents. The reviewer did not author the candidate. This is **not a human sign-off**, a comprehensive penetration test, or the maintainer's acceptance of release risk. The checks cover selected paths; they cannot establish absence of vulnerabilities, correct deployment at a particular site, or independence equivalent to an external human assessment.
- **Candidate:** [`b27da3441219514a2e9badb777be3dfbcd0904dd`](https://github.com/macel94/rillcache/tree/b27da3441219514a2e9badb777be3dfbcd0904dd), clean `HEAD` matching freshly fetched `origin/main` at review start. The review PR is documentation and evidence only. It does not fix, rebuild, or approve a different release candidate.
- **Scope:** draft [`rillcache-standalone-0.1` v1](production-profile.md), one process, one host, DB 0, Linux x86-64 glibc. Source review and loopback checks of default Mio/per-client and fixed-worker paths, native TLS 1.3, ACL/password authentication, authorization, command/frame/connection admission, diagnostics, logs and audit, config and journal files, and the locked dependency graph. Tokio is experimental and outside the profile; selected tests exercised it as a cross-check. Artifact scope: exact-candidate Linux Actions archive; exact-candidate amd64/arm64 container build archives and the attempted signed GHCR publication; Dockerfile, Compose, Kubernetes base/password overlay, systemd unit and runbook; selected backup and journal integrity tests. Inspecting an arm64 build artifact does not add an arm64 platform support claim.
- **Explicitly out of scope:** real credentials or production traffic; scanning or testing other systems; exhaustive command/ACL bypass discovery, exploit development, sustained DoS or fuzzing campaigns, side-channel and cryptographic implementation audits; kernel, toolchain, dependency source and GitHub infrastructure audits; live Kubernetes/CNI, service-manager sandbox, external TLS terminator and secret-manager setup; end-to-end audit custody, durability/availability certification, replication, clustering and scripting implementations. Excluded command entry points were checked for rejection, not assessed as implementations.

## Threat model

| Boundary | What must be protected | Assumptions and limits |
| --- | --- | --- |
| Unauthenticated network peer → listener | Credentials in transit, admission slots, parser and server availability | Use TLS with identity verification across an untrusted network. The plaintext listener still exists; keep it local or explicitly isolated. Do not use unauthenticated examples as production defaults. No assumption that per-IP limits stop distributed guessing. |
| Authenticated client → shared store, commands and observers | Key/channel access, writes and admin operations | Least privilege by command, subcommand, key and channel. ACL is not tenant isolation: keyspace, eviction, process availability and some introspection are shared. Restrict `INFO`, `MONITOR`, `SLOWLOG`, audit and ACL administration. Commands can themselves carry sensitive application data. |
| Operator/filesystem → config, TLS material, journal, backups and audit | Credential and data confidentiality; policy and replay integrity | Trust the service UID and administrators. Use protected parent directories, owner-only files/volumes, file-based random credentials, controlled rotation and storage encryption where needed. Journal HMAC/anchor are integrity controls, not encryption or independent custody. Plain UDP audit is best-effort; HMAC does not make it confidential or reliable. |
| Source/runner/registry → deployed binary or image | Candidate identity, integrity and provenance | Pin immutable digest and verify attestation before use. A tarball checksum matches bytes against its adjacent checksum, not an independent signature. Neither a mutable tag nor an unsigned SBOM is provenance. |

## Findings and dispositions

**Closed** means the stated control check passed on this candidate, not that the whole attack surface is closed. **Accepted** means the reviewer explicitly limits the scope in which the residual risk is tolerable; it does not mark a defect fixed or authorize deployment. An accepted exclusion can still leave a release prerequisite open; in particular, this review does not accept shipping the unverified OCI artifact.

| ID | Severity / status | Observation, disposition and exact evidence or retest |
| --- | --- | --- |
| SR-01 | Informational / **closed** | Authorization precedes ordinary dispatch ([dispatch:149–204](../src/session/dispatch.rs#L149-L204)); key/channel permission errors omit the offending values and command names are constrained ([command:53–64](../src/session/command.rs#L53-L64)). Run `cargo test --locked --offline --test auth --test acl --test acl_aggregate_store --test acl_keyspace_safety --test acl_category_selector_audit --test acl_ops --test network_modes` (31 passed as part of the focused run below). Independent wire probe additionally observed `NOAUTH`, identical `WRONGPASS` for wrong/unknown users, scoped `GET`, `NOPERM` with no denied-key echo, and unchanged seeded state. No timing-equivalence claim. |
| SR-02 | Informational / **closed** | TLS 1.3, cert/key match, exact SNI, mTLS/CRL, certificate-to-ACL mapping and atomic invalid-generation rejection are exercised ([TLS setup:113–220](../src/server/tls.rs#L113-L220)). `cargo test --locked --offline --test native_tls --test cli` ran 38 tests including `tls12_is_rejected_and_client_certificates_can_be_required`, `client_crl_policy_allows_current_nonrevoked_and_rejects_revoked_or_expired`, `invalid_sni_key_does_not_partially_publish_tls_staples` and `tls_healthcheck_verifies_identity_and_supports_acl_and_journal_checks`. Independent TLS probe on source and CI binary: verified TLS returned 0, wrong name returned 1. This result does not close SR-03. OCSP is stapling, not server verification of status, signature or freshness; CRL changes do not evict established sessions. |
| SR-03 | Medium / **accepted only with reload disabled** | Confirmed TLS file hardening gap on both binaries. Private draft `GHSA-jg8h-2cq8-cfc8` tracks exact source lines, commands, retained retest output and fix criteria per [SECURITY.md](../SECURITY.md). Accepted only where the optional live reload path is disabled, files are staged owner-only in trusted directories and rotation uses checked restart. The gap needs remediation and a negative reload retest before relaxing that restriction; not a reported remote authentication bypass. |
| SR-04 | Low / **accepted** | The startup configuration and existing journal are operator-owned permission boundaries. Strict mode checks the named secret paths, not the primary `--config` file ([config load:71–98](../src/config_file.rs#L71-L98), [strict paths:590–645](../src/server.rs#L590-L645)); `0600` on journal creation is not a mode check or repair for existing volumes ([journal:778–801](../src/engine/journal.rs#L778-L801)). Observed primary config mode `640` with strict preflight exit 0 (command below); wire probe observed an ACL file at `0640` rejected without echo and a new AOF and lock at `0600`. Accept only with an owner-only config/data directory, checked existing files/restores and no inline secrets. Provisioning guidance and regression follow-up: [#671](https://github.com/macel94/rillcache/issues/671). |
| SR-05 | Informational / **closed** for sampled redaction paths | Password credential digests use SHA-256 with constant-time digest comparison ([auth:549–577](../src/engine/auth.rs#L549-L577)); this is unsalted and does not make leaked digests safe. CLI passwords remain visible in argv; startup config can store cleartext, and successful config loading does not imply memory erasure. `cargo test --locked --offline --test monitor --test slowlog --test acl_audit --test audit_hmac --test secret_sources --test cli`: 29 passed. `monitor_streams_bounded_redacted_commands_and_reset_leaves_the_mode`, `slowlog_redacts_credentials` and `config_preflight_rejects_lax_secret_permissions_without_echoing_paths_or_values` passed. See [MONITOR redaction](../src/engine/monitor.rs#L44), [SLOWLOG redaction](../src/engine/slowlog.rs#L224), [CONFIG GET redaction](../src/engine/collection_config.rs#L305-L311) and [preflight](../src/main.rs#L787-L800). Wire probe found its generated passwords absent from `INFO ALL`, sampled stderr and AOF, and its ACL path absent from `INFO ALL` and failed preflight. Normal startup/reload errors are local diagnostics and can include paths; don't publish them verbatim. Audit records can contain encoded bounded denial metadata, not necessarily anonymized. AOF stores application data in cleartext. |
| SR-06 | Medium / **accepted as a deployment limit** | Passing frame/bulk/argument limits, queue and error-isolation tests does not prove a process memory or CPU bound. The transaction gate counts queued commands, not their aggregate retained bytes ([queue:159–199](../src/session/transaction.rs#L159-L199)). `cargo test --locked --offline --test server --test network_modes --test protocol_fuzz --test frame_assembly_deadline --test queue_policy --test auth_failure_budget --test auth_source_limit --test auth_process_isolation --test tenant_boundary --test error_dialect --test security_gate`: 64 passed, including live caps, per-user caps, auth lockout isolation, early payload rejection and peer isolation on protocol errors. This was bounded verification, not resource exhaustion testing. Accept only for trusted authenticated clients, scoped command grants and externally enforced memory/CPU/ingress budgets; track sizing and enforcement under [#471](https://github.com/macel94/rillcache/issues/471). Plaintext and unauthenticated examples must stay isolated. |
| SR-07 | Low / **accepted** | `cargo audit --no-fetch --db "$DB" --file Cargo.lock` checked 42 crates and reported one unmaintained warning, `rustls-pemfile 2.2.0` ([RUSTSEC-2025-0134](https://rustsec.org/advisories/RUSTSEC-2025-0134)), no matched vulnerability and exit 0. Bounded PEM reads use operator-supplied files; no fixed upgrade was reported, and TLS control tests passed. Review a supported replacement and rerun audit and TLS tests in [#672](https://github.com/macel94/rillcache/issues/672). Cargo/package-only posture, not a clean bill of health for transitive unsafe, OS or builder inputs. `cargo test --locked --offline --test policy --test source_hygiene`: 2 passed; application unsafe is forbidden ([Cargo.toml:23](../Cargo.toml#L23)), and dependencies are restricted to the explicit reviewed exception set. |
| SR-08 | Low / **accepted** | Unit configures an ACL file but the [systemd runbook probe](containers.md#single-node-deployment-path-and-runbook) supplies no credentials. The independent wire probe reproduced exit 1 against an authenticated AOF instance; the same probe with a named password-file user and only `PING` / `INFO\|PERSISTENCE` returned 0 quietly. Use that least-privilege probe in the documented operator path, not a `nopass` default user. Follow-up [#670](https://github.com/macel94/rillcache/issues/670); no server bypass or startup failure was demonstrated. |
| SR-09 | High (release evidence) / **accepted only as an explicit OCI distribution exclusion** | `gh run view 46045730657 --log-failed` ([exact-candidate workflow](https://github.com/macel94/rillcache/actions/runs/46045730657)): both native builds passed, but publisher exited 1: `annotations are not supported for Docker manifest lists; use "oci-mediatypes=true" when building the source images`. `podman manifest inspect ghcr.io/macel94/rillcache:sha-b27da3441219514a2e9badb777be3dfbcd0904dd` returned 125 / `manifest unknown`; anonymous registry lookup returned HTTP 404 / `MANIFEST_UNKNOWN`. No signed multi-platform index, signature/digest verification, anonymous native pulls or alias promotion for this SHA was established. Do not ship from `latest`, a staging tag, or a build tar as a substitute. Fix/retest: [#673](https://github.com/macel94/rillcache/issues/673), tracked under [#511](https://github.com/macel94/rillcache/issues/511). Closure requires exact-candidate publish, verification of index and children by digest and the signed provenance/SBOMs, plus anonymous native pulls. Not a runtime security defect. Accepting the evidence-only review without this artifact does not waive the **open** OCI release blocker. |
| SR-10 | Informational (unrelated baseline defect) / **accepted** | `python3 scripts/check_docs_index.py` returned 1 with identical output in a clean `git archive` of the candidate and the review worktree: 17 existing problems (unlinked acceptance runbook and duplicate `R426`–`R441`). The report adds neither requirement IDs nor an indexed area file. Accepted for this review only; repair and rerun the actual index check in [#674](https://github.com/macel94/rillcache/issues/674). |

## Method and retest results

Read the trust-boundary code and manifests, ran locked/offline fixture tests on Linux x86-64 using repository-pinned Rust `1.100.0` (`0ef1bc386`), and wrote separate subprocess probes with synthetic credentials, private temp directories, ephemeral loopback listeners and bounded timeouts. Repeated the independent probes against the downloaded release-built binary. These are not repetitions to obtain favorable performance results. Focused command (`173 passed, 0 failed, 0 ignored`, 33 suites):

```sh
RILLCACHE_TEST_THREADS=4 CARGO_BUILD_JOBS=4 cargo test --locked --offline \
  --test auth --test auth_failure_budget --test auth_source_limit --test auth_process_isolation \
  --test acl --test acl_category_selector_audit --test acl_keyspace_safety \
  --test acl_aggregate_store --test acl_ops --test acl_audit --test audit_hmac \
  --test cli --test config_schema --test native_tls --test network_modes \
  --test protocol_fuzz --test frame_assembly_deadline --test queue_policy --test server \
  --test source_hygiene --test policy --test monitor --test slowlog --test secret_sources \
  --test tenant_boundary --test journal_ownership --test container_policy \
  --test deployment_artifacts --test aof_auth --test aof_backup --test security_gate \
  --test error_dialect --test runtime_config -- --test-threads=2
```

Additional evidence:

- `python3 scripts/upstream/profile_matrix.py --check`: 56 rows, 12 exclusions, 14 client rows. `python3 scripts/upstream/acl_matrix.py --check`: 17 scenarios, two declared divergences. These validate manifests; they are **not** fresh differential executions against Redis and Valkey.
- `python3 scripts/test_standalone_runbook.py`: `standalone runbook checks passed (7 acceptance checks)`. `PYTHONPATH=scripts python3 -m unittest discover -s scripts -p test_container_attestations.py -v` and the same command with `test_verify_container_index.py` / `test_production_readiness.py`: 6 / 4 / 8 passed. The initial namespaced unittest invocation had two import errors; the corrected discovery command passed. These policy tests do not substitute for the failed live publisher or the independently reproduced probe mismatch.
- `cargo-audit` 0.23.0, RustSec database `c5279d0b0c9e8b70df666eb6d1edbabcfd1176bd` (`git clone --depth 1 https://github.com/RustSec/advisory-db.git "$DB"` and verify the database revision). `Cargo.lock` SHA-256 `5a7e94bf466dabc5f56ca0d4c037515b1ab32dc852763cb8b39b39ed10611beb`. Output: `Loaded 1571 security advisories`, `Scanning Cargo.lock for vulnerabilities (42 crate dependencies)`, `warning: 1 allowed warning found`. No database update or warning suppression was used in the scan.
- `python3 scripts/pr_gate_policy.py --live`: `Full live protection verified (bypass list visible and empty)`. This checks merge protection, not the security of deployed Rillcache.
- Local `make ci` initially stopped before formatting because Cargo was not on `PATH`. With the pinned toolchain on `PATH`, formatting, Clippy and the first test-mode pass succeeded; the remaining local run had not completed when this report was written. The exact candidate's hosted `Tests and offline CI` passed (run 46045730710). Neither result substitutes for the new PR's required checks.

Public independent checks (run from the candidate checkout or with the packaged binary):

```sh
python3 docs/security-review-v0.1.0-wire.py ./target/debug/rillcache
# When the review doc lives in another checkout, give its script path explicitly.
# Or substitute the verified Actions archive's extracted rillcache.
```

Output was identical on both; marker values are intentionally not retained:

```text
CONFIG: valid preflight creates no journal; strict mode 0640 ACL -> exit 2 / PermissionDenied, no path or credential
WIRE: NOAUTH, generic WRONGPASS, scoped GET, NOPERM for other keys/SET, state unchanged, INFO ALL omits credential/path markers
PROBE: unauthenticated AOF runbook probe exit 1; authenticated PING + INFO|PERSISTENCE + write probe exit 0, quiet
JOURNAL: new AOF and lock 0600; sample data readable on disk, credential fixtures absent
LOG: graceful stop; fixture credentials absent in startup/shutdown stderr
```

Primary config permission confirmation (the credential text is a dummy fixture):

```sh
(
  umask 077
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  printf 'config-version 1\nsecure-secret-files yes\nrequirepass test-only-mode-marker-not-a-real-secret\n' > "$tmp/rillcache.conf"
  chmod 0640 "$tmp/rillcache.conf"
  ./target/debug/rillcache --check-config --config "$tmp/rillcache.conf"
  stat -c '%a' "$tmp/rillcache.conf"
)
```

Observed `rillcache: startup options validated (server not started)`, exit 0 from the binary and mode `640`.

## Artifact verification

- **Linux archive:** [run 46045730710](https://github.com/macel94/rillcache/actions/runs/46045730710) has the reviewed `headSha`; all jobs, including `Tests and offline CI`, restore, storage faults and upgrade/rollback, concluded `success`. `gh run download 46045730710 -n "rillcache-linux-x86_64-$SHA" -D "$DIST"`; `cd "$DIST" && sha256sum -c rillcache-linux-x86_64.tar.gz.sha256` printed `rillcache-linux-x86_64.tar.gz: OK`. Tar SHA-256 `6d0c3ef8d2016e59060cae3a6a0548b80028b83067467040cfaa2c124be73acd`; server SHA-256 `b81a7c3096875a7b056a3d997de37139fe85afdbef6d1455875587599e9b4934` (x86-64 dynamically linked ELF). `cmp` passed for Compose, systemd, Kubernetes base, kustomization and password patch against the reviewed tree. Backup regression tests ran against source builds, not the archive; the archive contains `rillcache` and `rillcache-bench`, not `rillcache-backup`.
- **Container archives:** `gh run download 46045730657 -p 'rillcache-container-*' -D "$DIST"`. The locally inspected `image-amd64.tar` and `image-arm64.tar` SHA-256 were `2f2cb2fb575c5ba3e4b24a1d067a92cb8cf7784e2f9e90a61b5c62da9d71e5be` and `4e67d81c1322d007e47bb4792978602d7545af9620f7e947aed1f382086159a0`. Both had the candidate revision label, Linux target architecture, numeric `65532:65532`, exec-form probe, six runtime files (server plus notices) and adjacent SPDX 2.3 inventory. Local Podman on amd64 loaded the tested archive in isolated storage, ran it with `--network=none --read-only --cap-drop=ALL --security-opt=no-new-privileges`, and `podman exec "$CID" /rillcache --healthcheck 127.0.0.1:6379` returned 0. arm64 native execution is hosted build evidence, not a local retest. [The publish failure](https://github.com/macel94/rillcache/actions/runs/46045730657) is retained; neither a `v0.1.0` tag nor a GitHub release was present at review time.
- **Deployment examples:** `systemd-analyze verify --man=no "$TMP/rillcache.service"` passed after substituting only the extracted binary path for `/usr/local/bin/rillcache` in a disposable copy; no unit was installed or launched. The unit binds loopback, uses `StateDirectoryMode=0700`, `UMask=0077` and strict ACL-file mode. Compose published host loopback with read-only root, dropped capabilities and a 256 MiB cap; no AUTH arguments by default. The Kubernetes base is single replica/nonroot with read-only root, no service-account token, ClusterIP, and opt-in same-namespace ingress requiring a NetworkPolicy-capable CNI. `k3s kubectl kustomize deploy` rendered the password overlay successfully: server and both probes use the same read-only password mount, mode `0440` with `fsGroup: 65532`, **not** the server's owner-only strict mode. Neither `latest` example pins an immutable release digest. No workload was applied to a live cluster.

## Outstanding independent signature

This document is the completed in-repo artifact set for S1: scoped threat model,
review scope and date, findings with dispositions, and the retest commands and
artifact verifications above. The one item it cannot supply is the third-party
signature S1 names: an **independent human security reviewer** (or external
penetration test) recording findings here and accepting the residual risk for a
release candidate. That signature stays recorded as `security / unassigned` with
findings `F-001` in `docs/acceptance/signoffs.tsv`; `python3
scripts/check_acceptance.py --outstanding` prints it. S1 therefore remains open,
and closing the review task when this PR merges is administrative only.

## Conclusion — 2026-11-27 UTC

The sampled auth/ACL, TLS, parsing, admission and redaction controls held on the reviewed paths under the stated assumptions. No remotely exploitable authorization bypass was demonstrated. This review accepts the listed conditional residual risks for the narrowly scoped report, not for arbitrary deployments. Signed image distribution is unverified and remains open. No runtime code, workflow, protection rule or readiness checkbox is changed by this PR. Closing the review task when the PR merges is administrative; it does not satisfy a human sign-off, close the release-distribution gap, or change the profile's **NOT PRODUCTION READY** status. A new runtime or artifact candidate needs candidate-bound retests and a maintainer decision on these findings before a production claim.
