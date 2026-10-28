# Native TLS 1.3 with rustls

Rillcache has a native TLS 1.3 listener implemented with
[`rustls`](https://github.com/rustls/rustls) and its `ring` provider. There is no
OpenSSL, no cgo and no separate TLS sidecar. TLS is part of the server process and
uses the normal RESP session/worker path.

The repository dependency policy allows **rustls and the transitive crates
required to build it** as its sole external-dependency exception. The crates are
vendored for offline builds. Project code remains `unsafe_code = "forbid"`.

## Protocol scope

- TLS **1.3 only** for the Rillcache listener. TLS 1.0, 1.1 and 1.2 handshakes
  are rejected.
- RESP is carried directly over TLS. **HTTP/1.1, HTTP/2 and HTTP/3 are not
  supported or negotiated.** HTTP/2 is an ALPN application protocol and HTTP/3 is
  QUIC over UDP; neither is a transport for Redis clients.
- One server certificate/chain is supported. SNI-based certificate selection,
  hot reload and OCSP/CRL integration are not implemented.
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
| `--tls-handshake-timeout-ms N` | Handshake deadline in milliseconds (default 5000) |

Example:

```sh
rillcache \
  --bind 127.0.0.1:6379 \
  --tls-bind 0.0.0.0:6380 \
  --tls-cert-file /etc/rillcache/server-chain.pem \
  --tls-key-file /etc/rillcache/server-key.pem
```

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
- There is no certificate hot reload; restart during a controlled window to
  rotate material.
- There is no SNI, no ALPN, no PROXY protocol and no client-certificate-to-ACL
  mapping.
- TLS connections use the per-client blocking worker path. The fixed
  worker-pool setting continues to govern plaintext connections; TLS connections
  are still bounded by the shared client limit.
- TLS adds encryption/decryption and handshake cost. It is not free throughput.
- This is transport encryption, not durability or availability.

## Tests

```sh
cargo test --locked --offline --test native_tls
```

The tests cover TLS 1.3 RESP in all execution modes, plaintext coexistence, TLS
1.2 rejection, absent HTTP ALPN, required client certificates and invalid TLS
configuration.
