Test-only TLS material for the native rustls integration. These keys are public
and must never be used outside tests. `alternate.crt`/`alternate.key` provide a
second CA-signed `alt.test` identity for exact SNI-selection, mTLS and rotation tests.
