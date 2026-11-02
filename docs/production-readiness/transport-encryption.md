# Transport encryption

Open items are tracked in the canonical [production-readiness gaps](../production-readiness-gaps.md); this file records current behavior and boundaries, not a second checklist.

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Transport encryption | Partial implementation (S101): native in-process TLS 1.3 using the reviewed rustls dependency exception. TLS 1.3-only enforcement, certificate/key loading, optional required client certificates, bounded handshakes and shared connection admission, with no OpenSSL, cgo or sidecar. No HTTP/2/HTTP/3 ALPN (Redis clients use RESP); plaintext and TLS listeners can coexist. Open work is tracked in the canonical gaps file (S5). |
