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
  reload is available. A DER CRL can fail closed for verified client certificates; OCSP validation of the server identity is not implemented.
- Plaintext and TLS listeners can run together. Put the TLS listener on the
  exposed interface and bind the plaintext listener to loopback/private space, or
  firewall the plaintext port.

## Configuration

| Flag | Meaning |
| --- | --- |
| `--tls-bind ADDRESS` | Native TLS listener; must differ from `--bind` |
| `--tls-cert-file PATH` | PEM server certificate followed by intermediates |
| `--tls-key-file PATH` | Unencrypted PEM private key |
| `--tls-client-ca-file PATH` | Optional client CA; enables required mTLS |
| `--tls-client-crl-file PATH` | Optional DER CRL; requires the client CA and rejects revoked, unknown or expired client status |
| `--tls-client-cert-user SHA256=USER` | Repeatable SHA-256 leaf-certificate fingerprint to ACL username mapping; requires the client CA |
| `--tls-reload-trigger PATH` | Optional operator-controlled reload token file |
| `--tls-sni-cert-file NAME=PATH` | Repeatable exact SNI DNS name and PEM certificate chain |
| `--tls-sni-key-file NAME=PATH` | Repeatable matching SNI DNS name and PEM private key |
| `--tls-handshake-timeout-ms N` | Handshake deadline in milliseconds (default 5000) |
| `--frame-assembly-timeout-ms N` | Deadline after incomplete RESP plaintext reaches the parser (default 60000; `0` disables) |

Example:

```sh
rillcache \
  --bind 127.0.0.1:6379 \
  --tls-bind 0.0.0.0:6380 \
  --tls-cert-file /etc/rillcache/server-chain.pem \
  --tls-key-file /etc/rillcache/server-key.pem \
  --tls-sni-cert-file cache.example.com=/etc/rillcache/cache-chain.pem \
  --tls-sni-key-file cache.example.com=/etc/rillcache/cache-key.pem
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

Fingerprint mappings are startup configuration and are not changed by the TLS
file reload trigger. When replacing a client certificate, update its mapping
and restart the process; the reload trigger rotates server identities and the
client-CA verifier only.

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
- Optional certificate and client-CA/CRL rotation uses `--tls-reload-trigger PATH`. The trigger
  must exist at startup and contain a unique 1..=64-byte visible-ASCII token.
  Stage replacement PEM files (each at most 1 MiB) by atomic rename, then
  atomically replace the trigger with a new token. The next accepted TLS
  connection attempts one bounded reload of the default and every configured
  SNI identity, together with the optional client CA and CRL. Existing connections
  retain their established rustls configuration snapshot.
- If reload parsing, CA validation or a certificate/key match fails, the last
  valid generation remains active and the error is written to stderr. That token
  is not retried; after fixing files, publish a different token. New handshakes
  continue using the last valid identities while a bad generation is present.
  Verify the served identities after rotating. Reload is process-local, not a
  distributed rollout or readiness guarantee. With `--secure-secret-files`, the
  trigger must also be owner-only (for example, mode 0600). CRL updates are
  included in the new verifier generation; certificate-to-ACL mappings require
  explicit leaf fingerprints and are startup-only.

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

- OCSP stapling is not configured. rustls 0.23.46 can transmit opaque OCSP bytes attached to a server identity, but does not validate the response signature, certificate status or freshness for this server's acceptance policy. Revocation of the server certificate therefore depends on clients validating OCSP (which this server cannot enforce) or an independently operated TLS terminator that validates/staples current responses and fails closed. The native CRL option applies only to client certificates during mTLS; it does not revoke the server's own certificate.
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
unmapped-certificate rejection. Server-certificate OCSP validation remains an
explicit deployment boundary, not an implemented control.
