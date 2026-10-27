# Transport encryption

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Transport encryption | Partial implementation (S101): bundled Go standard-library TLS 1.3 sidecar (`tlsproxy`) with TLS 1.3-only enforcement, hostname/certificate validation by clients, optional required client certificates, bounded handshakes/connections and graceful signal shutdown. It uses no OpenSSL, cgo or external Go modules; RESP is proxied to a plaintext loopback backend. No HTTP/2/HTTP/3 ALPN (Redis clients use RESP). Still required: native Rust TLS if the dependency policy changes, certificate hot reload/SNI, failure/rotation interoperability drills, backend authentication and independent security review. |
