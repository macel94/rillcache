# Transport encryption

Open items are tracked in the canonical [production-readiness gaps](../production-readiness-gaps.md); this file records current behavior and boundaries, not a second checklist.

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Transport encryption | Partial implementation (S101): native in-process TLS 1.3 using the reviewed rustls dependency exception. TLS 1.3-only enforcement, certificate/key loading, exact-name SNI selection, optional required client certificates, bounded handshakes and shared connection admission, plus file-triggered atomic rotation of default and SNI identities and verified client-leaf fingerprint mapping to enabled ACL users. Unmapped client certificates are rejected and normal ACL checks remain active. Existing connections retain their TLS snapshots; invalid replacements retain the active generation; mappings are startup-only. There is no OpenSSL, cgo or sidecar, and no HTTP/2/HTTP/3 ALPN (Redis clients use RESP); plaintext and TLS listeners can coexist. Optional DER CRL validation for mTLS client chains fails closed on revoked, unknown or expired status and participates in atomic TLS generation reload; server-certificate OCSP validation is not implemented. Remaining work is tracked in the canonical gaps file (S5). |
