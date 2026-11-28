# Native TLS 1.3 with rustls

Rillcache has a native TLS 1.3 listener implemented with
[`rustls`](https://github.com/rustls/rustls) and its `ring` provider. There is no
OpenSSL, no cgo and no separate TLS sidecar. TLS is part of the server process and
uses the normal RESP session/worker path.

The repository dependency policy allows **rustls**, **Tokio** (with Mio as its
readiness backend) and **`core_affinity`**, plus the transitive crates required
to build them. The crates are vendored for offline builds. Application code
remains `unsafe_code = "forbid"`.

## Protocol scope

- TLS **1.3 only** for the Rillcache listener. TLS 1.0, 1.1 and 1.2 handshakes
  are rejected.
- RESP is carried directly over TLS. **HTTP/1.1, HTTP/2 and HTTP/3 are not
  supported or negotiated.** HTTP/2 is an ALPN application protocol and HTTP/3 is
  QUIC over UDP; neither is a transport for Redis clients.
- The configured `--tls-cert-file`/`--tls-key-file` pair is the default when no
  additional SNI identities are configured. Optional exact-name SNI mappings
  select distinct certificate chains at startup. When mappings are configured,
  an unknown SNI name is rejected; there is no wildcard matching or fallback to
  the default identity for an unknown name. Optional operator-triggered atomic
  reload is available. A DER CRL can fail closed for verified client certificates. Native servers can staple operator-provisioned OCSP bytes and validate their status, freshness window and response type at load and reload time (see [stapled status validation](#stapled-status-validation)); they do not verify the OCSP signature or the `certID` certificate binding, which stay client-side checks.
- Plaintext and TLS listeners can run together. Put the TLS listener on the
  exposed interface and bind the plaintext listener to loopback/private space, or
  firewall the plaintext port.

## Configuration

| Flag | Meaning |
| --- | --- |
| `--tls-bind ADDRESS` | Native TLS listener; must differ from `--bind` |
| `--tls-cert-file PATH` | PEM server certificate followed by intermediates |
| `--tls-key-file PATH` | Unencrypted PEM private key |
| `--tls-ocsp-file PATH` | Optional DER OCSP response stapled with the default identity |
| `--tls-client-ca-file PATH` | Optional client CA; enables required mTLS |
| `--tls-client-crl-file PATH` | Optional DER CRL; requires the client CA and rejects revoked, unknown or expired client status |
| `--tls-client-cert-user SHA256=USER` | Repeatable startup-only SHA-256 leaf-certificate fingerprint to ACL username mapping; requires the client CA |
| `--tls-client-cert-users-file PATH` | Bounded UTF-8 `SHA256=ACL-USERNAME` map reloaded atomically with TLS files; requires client CA and reload trigger; mutually exclusive with inline mappings |
| `--tls-reload-trigger PATH` | Optional operator-controlled reload token file |
| `--tls-sni-cert-file NAME=PATH` | Repeatable exact SNI DNS name and PEM certificate chain |
| `--tls-sni-key-file NAME=PATH` | Repeatable matching SNI DNS name and PEM private key |
| `--tls-sni-ocsp-file NAME=PATH` | Optional DER response for that exact SNI identity |
| `--tls-handshake-timeout-ms N` | Handshake deadline in milliseconds (default 5000) |
| `--frame-assembly-timeout-ms N` | Deadline after incomplete RESP plaintext reaches the parser (default 60000; `0` disables) |
| `--write-stall-timeout-ms N` | Pending-reply write-stall deadline; TLS sockets close a non-draining reader after the same bound (default 30000; `0` disables) |

Example:

```sh
rillcache \
  --bind 127.0.0.1:6379 \
  --tls-bind 0.0.0.0:6380 \
  --tls-cert-file /etc/rillcache/server-chain.pem \
  --tls-key-file /etc/rillcache/server-key.pem \
  --tls-ocsp-file /etc/rillcache/server.ocsp.der \
  --tls-sni-cert-file cache.example.com=/etc/rillcache/cache-chain.pem \
  --tls-sni-key-file cache.example.com=/etc/rillcache/cache-key.pem \
  --tls-sni-ocsp-file cache.example.com=/etc/rillcache/cache.ocsp.der
```

The configured handshake timeout is one monotonic budget across the complete
initial TLS handshake. Every underlying socket read and write uses the remaining
time, including rustls handshake flights and partial record I/O; handshake
progress does not restart the budget. After successful completion, the handshake
deadline is cleared and the ordinary idle timeout applies.

After the TLS handshake, the configured idle timeout bounds the complete period
without application plaintext progress. The deadline is absolute across rustls'
underlying socket reads and is refreshed only when rustls delivers plaintext to
the RESP session; partial TLS records and complete control records that produce
no plaintext do not restart it. Receiving plaintext resumes the idle budget from
that progress. Incomplete RESP plaintext is additionally covered by the configured
frame-assembly deadline. These bounds do not establish a global connection or
resource budget, and other slow-client and O(N) gaps remain; this is not a
production-readiness claim.

Clients connect to the TLS port with their normal Redis TLS options, for example:

```sh
redis-cli --tls --cacert ca.pem -h cache.example.com -p 6380 PING
```

The certificate must be valid for the hostname clients use. A private CA is fine
as long as clients trust it.

## Mutual TLS

```sh
rillcache --tls-bind 0.0.0.0:6380 \
  --bind 127.0.0.1:6379 \
  --tls-cert-file server-chain.pem \
  --tls-key-file server-key.pem \
  --tls-client-ca-file client-ca.pem
```

With a client CA configured, every TLS client must present a certificate that
chains to it. Add `--tls-client-crl-file client-ca.crl.der` to require a signed,
applicable CRL for the presented chain. Unknown revocation status and CRLs past
`nextUpdate` fail the handshake; CRL distribution points are not fetched. The
file must contain one DER-encoded CRL no larger than 1 MiB. To map certificates to ACL identities, add one or more
`--tls-client-cert-user SHA256=USER` entries. The 64-hex-digit fingerprint is
SHA-256 over the leaf certificate's DER bytes (for example, compute it with
`openssl x509 -in client.pem -outform DER | sha256sum`). The mapped ACL user must
be enabled at startup. A verified but unmapped certificate is rejected before
RESP commands are served. The mapped user is authenticated for the connection,
and all its ACL command/key/channel restrictions still apply. `RESET` returns
to the certificate-mapped identity; `AUTH` may explicitly switch users.

Inline fingerprint mappings are startup-only. For rotation without restart,
configure `--tls-client-cert-users-file PATH` with both the client CA and
`--tls-reload-trigger PATH`; do not combine it with inline mappings. The file is
at most 1 MiB, contains up to 4096 UTF-8 records of `SHA256=ACL-USERNAME`, and
allows blank lines and full-line `#` comments. Fingerprints are SHA-256 over
verified leaf DER. Duplicate/invalid fingerprints, malformed records and users
that are missing or disabled reject the whole candidate generation. An empty
file intentionally maps no certificate and rejects every mTLS identity. With
`--secure-secret-files`, the file must be regular and owner-only.

After staging server/client certificate, CA/CRL, OCSP/SNI and mapping files,
atomically publish a new trigger token. The TLS configuration and mapping table
are validated and published together; every handshake uses one immutable
snapshot. Established sessions keep the ACL identity captured at handshake,
including after RESET; new handshakes use the new mapping. A failed candidate
retains the previous TLS and mapping generation. Inline mappings remain
supported for deployments that do not need mapping reload.

## Development certificate

The tests use committed, test-only certificates under
`tests/fixtures/tls/`. For local development they can be used directly:

```sh
rillcache \
  --bind 127.0.0.1:6379 \
  --tls-bind 127.0.0.1:6380 \
  --tls-cert-file tests/fixtures/tls/server.crt \
  --tls-key-file tests/fixtures/tls/server.key
```

For a deployment, generate certificates with your normal PKI. Do not reuse the
test keys.

## Operational limits

- TLS does not cover replication or cluster links because those features do not
  exist in this server.
- SNI mappings are startup-only, exact DNS names, bounded to 128 identities and
  require a matching certificate/key pair for each name. Duplicate, malformed,
  missing or mismatched identities fail startup. If mappings are configured,
  unknown SNI names abort the handshake; the default identity is used only when
  the ClientHello has no SNI. If no mappings are configured, the single default
  certificate preserves existing behavior for any SNI name.
- Optional certificate, client-CA/CRL and mapping rotation uses
  `--tls-reload-trigger PATH`. The trigger must exist at startup and contain a
  unique 1..=64-byte visible-ASCII token. Stage replacement files (each at most
  1 MiB) by atomic rename, then atomically replace the trigger with a new token.
  The next accepted TLS connection attempts one bounded reload of the default
  and every configured SNI identity, associated OCSP response files, the
  optional client CA and CRL, and (when configured) the client-certificate ACL
  mapping file. The TLS verifier, identities and map publish as one generation;
  each handshake uses one snapshot, while established sessions keep their
  handshake-time ACL identity. Each OCSP response follows only its configured
  certificate/key identity.
- If reload parsing, CA/CRL validation, mapped-user validation or a certificate/key
  match fails, the last valid TLS-and-mapping generation remains active and the
  error is written to stderr. That token is not retried; after fixing files,
  publish a different token. New handshakes continue using the last valid
  identities, client certificate policy, and stapled bytes while a bad
  generation is present. A valid replacement client CRL applies to new
  handshakes; it does not disconnect established sessions. A bad key
  for a single SNI name also prevents publication of the staged default identity.
  Verify the served identities after rotating. Reload is process-local, not a distributed
  rollout or readiness guarantee. With `--secure-secret-files`, the trigger
  and configured mapping file must also be owner-only (for example, mode 0600).
  Inline certificate mappings remain startup-only.
- OCSP files must be nonempty, no larger than 1 MiB, and have one bounded DER
  SEQUENCE envelope. Beyond framing, the server enforces the status and freshness
  policy described in [stapled status validation](#stapled-status-validation);
  a staple that fails it rejects the TLS generation, so a stale or unusable
  staple is never served. The server does not verify the OCSP signature,
  responder authorization or `certID` binding, and it does not fetch status.
  Operators must obtain each response from a trusted PKI workflow, keep it
  current, and renew/stage it with the matching certificate and key. Clients must
  request and independently validate stapled status according to their policy;
  clients may ignore a missing or unusable staple. Stapling is not a substitute
  for client-side CRL policy or an external TLS terminator's security review.
  Server-certificate OCSP/CRL validation initiated by the server and automatic
  OCSP fetching remain unsupported.

### Stapled status validation

`src/server/ocsp.rs` parses the bounded DER subset of RFC 6960 that the server can
enforce without a new dependency, and rejects a staple at load or atomic reload
time when any of these hold:

- `OCSPResponse.responseStatus` is not `successful(0)`,
- the response type is not `id-pkix-ocsp-basic`,
- `producedAt` or any `thisUpdate` is in the future beyond a 300-second clock
  skew,
- any `SingleResponse` reports `revoked` or `unknown`,
- a `SingleResponse` carries no `nextUpdate` (freshness cannot be established),
- a `nextUpdate` is already in the past, or
- the structure is truncated, uses indefinite lengths, or has trailing bytes.

Rejection fails the generation: with `--tls-reload-trigger`, the previous valid
generation keeps serving instead of publishing unverifiable status, the same
fail-closed shape as an invalid SNI key. The unit tests in `src/server/ocsp.rs`
cover fresh/good acceptance, revoked, unknown, one-revoked-of-two, missing
`nextUpdate`, expired, future `thisUpdate`, future `producedAt`, non-successful
response status, wrong response type, empty response list, truncated input,
trailing bytes, indefinite lengths and a non-Zulu timestamp.

Deliberate limits: signature and `certID` verification need the issuer's subject
and public key, which would require an X.509 parser outside the reviewed
dependency set. RFC 6960 already requires the client to verify the response
signature and that `certID` names the presented certificate, so the server's
enforced policy is "never serve status that is not successfully structured, good
and fresh", not "the staple is authoritative".

### Non-production rotation acceptance drill

Use a disposable instance and copies of the committed test-only fixtures, never
production keys. Start with `server.crt`/`server.key`, a unique generation token,
and (for mTLS) `ca.crt` plus `client-good.crl.der`. Record the served leaf SHA-256
fingerprint using `openssl s_client -connect 127.0.0.1:6380 -servername localhost
-showcerts </dev/null | openssl x509 -noout -fingerprint -sha256`. Replace the
certificate and key using staged files and atomic renames, publish a different
token by atomically renaming its staged file over the trigger, then reconnect and
verify the expected new fingerprint and a successful RESP PING. For the failure
case, stage a mismatched certificate/key pair and publish another unique token;
verify stderr reports rejection, the old fingerprint remains served, and an
existing connection still exchanges RESP. Restore a valid pair, advance the token
again, and verify recovery. The automated `tls_reload_trigger_rotates_new_handshakes_and_keeps_last_valid_config_on_failure` and client-CRL handshake tests cover the repeatable parts; this drill is not a production certification.

- OCSP stapling can attach operator-provisioned opaque bytes to a server identity. rustls 0.23.46 does not validate their signature, certificate status, certificate identity or freshness for this server's acceptance policy. Revocation of the server certificate therefore depends on clients validating OCSP (which this server cannot enforce) or an independently operated TLS terminator that validates/staples current responses and fails closed. The native CRL option applies only to client certificates during mTLS; it does not revoke the server's own certificate.
- There is no ALPN or PROXY protocol.
- TLS connections use the per-client blocking worker path. The fixed
  worker-pool setting continues to govern plaintext connections; TLS connections
  are still bounded by the shared client limit.
- TLS adds encryption/decryption and handshake cost. It is not free throughput.
- This is transport encryption, not durability or availability.

## Tests

```sh
cargo test --locked --offline --lib handshake_deadline_shrinks_read_and_write_budgets
cargo test --locked --offline --test native_tls
```

The tests cover TLS 1.3 RESP in all execution modes, plaintext coexistence, TLS
1.2 rejection, absent HTTP ALPN, required client certificates, exact SNI
certificate selection with and without mTLS, unknown-name handshake rejection,
atomic certificate rotation and invalid certificate-replacement retention,
valid/revoked/unknown/expired client CRLs, and verified client-certificate-to-ACL mapping with
unmapped-certificate rejection, atomic mapping reload, established-session identity
retention and failed-generation rollback. Server-certificate OCSP **status and
freshness** validation is enforced at load and reload:
`tests/native_tls.rs::stale_unsuccessful_or_revoked_ocsp_staples_fail_closed`
proves a stale, revoked or unknown staple fails the generation with the reason in
the error while a fresh `good` staple is accepted and served, and the sixteen
`src/server/ocsp.rs` unit tests cover the structural and freshness matrix.
OCSP signature/responder verification and `certID` binding remain client-side
checks (see [stapled status validation](#stapled-status-validation)), and
server-initiated status fetching is not implemented.
