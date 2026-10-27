# TLS 1.3 sidecar

Rillcache itself speaks RESP over plain TCP. This repository now includes
`tlsproxy/`, a small **Go standard-library** TLS 1.3 terminator for RESP. It does
not use OpenSSL, cgo, or any external Go module. The Rust server remains
dependency-free and unchanged; TLS is terminated one hop in front of it.

```text
RESP client == TLS 1.3 ==> tlsproxy == plaintext loopback ==> rillcache
```

## Protocol scope

- TLS **1.3 only**. TLS 1.0, 1.1 and 1.2 handshakes are rejected.
- RESP is proxied as opaque bytes. **HTTP/1.1, HTTP/2 and HTTP/3 are not
  supported or negotiated.** HTTP/2 is an ALPN application protocol and HTTP/3 is
  QUIC over UDP; neither is a transport for Redis clients. The proxy deliberately
  offers no ALPN protocol.
- No native Rust TLS. Native TLS still requires a dependency-policy decision,
  because safe Rust with no external dependencies has no audited TLS stack.
- The Go sidecar is a deployment component. It is not embedded in the `rillcache`
  binary and does not change the Rust dependency graph.

## Build

Go 1.22 or newer is required only to build the sidecar:

```sh
cd tlsproxy
CGO_ENABLED=0 go build -trimpath -ldflags='-s -w' -o tlsproxy .
./tlsproxy --help
```

The resulting binary is static when built with `CGO_ENABLED=0`.

## Run

```sh
./tlsproxy \
  --listen 0.0.0.0:6380 \
  --backend 127.0.0.1:6379 \
  --cert /etc/rillcache/server-chain.pem \
  --key /etc/rillcache/server-key.pem
```

Clients connect to port 6380 with their normal Redis TLS options, for example:

```sh
redis-cli --tls --cacert ca.pem -h cache.example.com -p 6380 PING
```

The certificate file must contain a PEM leaf certificate followed by any
intermediate chain. The private key file must be an unencrypted PEM key accepted
by Go's `crypto/tls` (RSA, ECDSA or Ed25519).

## Mutual TLS

Pass a CA bundle to require and verify a client certificate:

```sh
./tlsproxy --listen 0.0.0.0:6380 --backend 127.0.0.1:6379 \
  --cert server-chain.pem --key server-key.pem \
  --client-ca client-ca.pem
```

Without `--client-ca`, client certificates are neither requested nor required.
There is no CRL/OCSP integration and no certificate hot reload; rotate
certificates by restarting the sidecar during a controlled window.

## Flags

| Flag | Default | Meaning |
| --- | --- | --- |
| `--listen` | `127.0.0.1:6380` | TLS listener address |
| `--backend` | `127.0.0.1:6379` | plaintext RESP backend |
| `--cert` | required | PEM certificate chain |
| `--key` | required | PEM private key |
| `--client-ca` | empty | optional client CA; enables required mTLS |
| `--max-conns` | `1024` | simultaneous TLS connections |
| `--handshake-timeout` | `5s` | TLS handshake deadline |
| `--dial-timeout` | `5s` | backend connect deadline |
| `--shutdown-timeout` | `10s` | graceful drain deadline |
| `--version` | | print version and exit |

The process handles `SIGINT`/`SIGTERM` itself, stops accepting, waits for active
proxies up to the shutdown deadline, then force-closes leftovers.

## Operational limits

- Bind the plaintext backend to loopback or a private network. The sidecar does
  not authenticate the backend and the inner hop is plaintext.
- The backend sees the sidecar's source address, not the original client address.
  `CLIENT LIST`, `CLIENT KILL ADDR/LADDR` and any address-based operation therefore
  observe proxy addresses.
- One proxy process is a single point of failure. Run it under the same supervisor
  as the server; replication and failover remain unsupported.
- There is no SNI-based multi-certificate selection, no PROXY protocol, no
  connection pooling, no metrics endpoint and no dynamic certificate reload.
- TLS session tickets are held in process memory and do not survive a restart.
- There is no proxy-level idle timeout. The plaintext backend's idle timeout
  closes idle connections, which the proxy then mirrors.
- Go chooses the TLS 1.3 cipher suites; no legacy cipher configuration is exposed.
- This is transport encryption, not a durability or availability feature.

## Tests

```sh
cd tlsproxy
go test -race ./...
```

The tests start a real TLS 1.3 listener and backend, verify RESP echo, reject a
TLS 1.2 client, assert that no HTTP ALPN protocol is negotiated, and exercise
required client-certificate authentication and configuration validation.
