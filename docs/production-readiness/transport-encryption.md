# Transport encryption

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Transport encryption | Partial implementation (S101): native in-process TLS 1.3 using the rustls-only dependency exception. TLS 1.3-only enforcement, certificate/key loading, optional required client certificates, bounded handshakes and shared connection admission, with no OpenSSL, cgo or sidecar. No HTTP/2/HTTP/3 ALPN (Redis clients use RESP); plaintext and TLS listeners can coexist. Still required: certificate hot reload/SNI, OCSP/CRL, client-certificate-to-ACL mapping, TLS interoperability/rotation drills and independent security review. |
