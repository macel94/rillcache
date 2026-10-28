# The builder is not part of the runtime image. Digest pins both native architectures.
FROM rust:1.99.1-alpine3.23@sha256:2a941e16ec6e357ff7112e92336087772584122b6ae4b30eb36cb9adff5b5458 AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo ./.cargo
COPY vendor ./vendor
COPY src ./src
COPY benches ./benches
RUN apk add --no-cache build-base perl
ENV CARGO_PROFILE_RELEASE_LTO=thin \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
    CARGO_PROFILE_RELEASE_STRIP=symbols \
    RUSTFLAGS="-C target-feature=+crt-static"
RUN cargo build --locked --offline --release --bin rillcache

FROM scratch
ARG VCS_REF=unknown
LABEL org.opencontainers.image.title="Rillcache" \
      org.opencontainers.image.description="Experimental safe Rust Redis-compatible in-memory server; optional password/ACL-file auth, native rustls TLS 1.3 and append-only journal, no RDB" \
      org.opencontainers.image.source="https://github.com/macel94/rillcache" \
      org.opencontainers.image.revision="${VCS_REF}"
COPY --from=build /build/target/release/rillcache /rillcache
COPY container/notices/ /usr/share/licenses/rillcache-runtime/
USER 65532:65532
EXPOSE 6379/tcp
# Data is volatile. There is no safe-std portable signal handler or graceful SIGTERM path yet.
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=2s --start-period=5s --retries=3 CMD ["/rillcache", "--healthcheck", "127.0.0.1:6379"]
# Keep defaults in ENTRYPOINT: `docker run IMAGE --threads 4` still binds correctly.
# A later --bind overrides this; standalone/library defaults remain loopback-only.
ENTRYPOINT ["/rillcache", "--bind", "0.0.0.0:6379"]
