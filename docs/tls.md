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
  the default identity for an unknown name. Certificate hot reload and OCSP/CRL
  integration are not implemented.
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
chains to it. A successful certificate handshake is **not** the same as Rillcache
authentication: continue using `AUTH`/ACL rules after connecting. Certificate
identity is not mapped automatically to an ACL user.

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
- There is no certificate hot reload; restart during a controlled window to
  rotate material. OCSP/CRL and client-certificate-to-ACL mapping remain absent.
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
certificate selection with and without mTLS, unknown-name handshake rejection
and invalid TLS configuration.
