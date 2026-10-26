# Password authentication and transport boundaries

Rillcache supports one operator-configured password for the implicit Redis
`default` user. This is a real access-control boundary for a private deployment,
not a complete Redis security design. It does not add users, per-command or
per-key permissions, TLS, rate limiting, an audit log, or credential persistence.

## What is implemented

- **Explicit configuration.** `--requirepass PASSWORD` sets the password for
  this process. `--requirepass-file PATH` reads it from a file and strips at most
  one trailing `\n` or `\r\n`; an empty or larger-than-1024-byte file is a startup
  error, not a silent no-password mode.
- **Startup default.** Without either option the server remains an unauthenticated
  loopback service. Containers pass `--bind 0.0.0.0:6379` by default, so operators
  must add authentication before exposing them beyond a trusted private network.
- **Required authentication.** When a password is configured, a new connection is
  unauthenticated. Only `AUTH`, `HELLO` (including `HELLO ... AUTH ...`) and
  `QUIT` execute; every other command, including `MULTI`, `INFO`, `CONFIG`,
  `COMMAND`, reads and writes, returns `NOAUTH Authentication required.` before
  touching the store.
- **Credential handling.** `AUTH [username] password` accepts only `default`
  (case-insensitive) and compares SHA-256 digests in constant time. The server
  stores only the digest. Wrong usernames, wrong passwords and disabled
  credentials return the generic `WRONGPASS` error. Calling `AUTH` when no
  password exists returns Redis's explicit no-password configuration error.
- **HELLO negotiation.** `HELLO <2|3> AUTH default <password> [SETNAME name]`
  validates every option and the credential before changing protocol, metadata or
  authentication state. A failed `HELLO` leaves all three unchanged.
- **Runtime rotation.** `CONFIG SET requirepass <value>` sets a new password;
  an empty value disables authentication. Changes are atomic with other
  `CONFIG SET` options: a rejected pair leaves both collection thresholds and the
  credential unchanged. `CONFIG GET requirepass` never returns the secret:
  unset reports an empty value, configured reports `<redacted>`.
- **No secret in diagnostics.** `Config`'s `Debug` representation, startup lines,
  errors and `CONFIG GET` redact the password. The password is never logged.

## Deliberate limits

- **One user only.** ACL users, `ACL` commands, command categories, key patterns,
  pub/sub channel permissions and per-user passwords are not implemented. Do not
  expose administrative credentials to tenants that need isolation.
- **No native TLS.** Rust's standard library exposes TCP, not TLS. The project
  forbids external Rust dependencies and FFI, so native TLS has no implementation
  path without an explicit dependency-policy decision. An externally operated TLS
  terminator is a documented deployment boundary, not equivalent to Redis native
  TLS; traffic inside that perimeter is plaintext.
- **No brute-force protection.** There is no delay, lockout, connection budget per
  source or credential-attempt audit. Use network policy/firewalling and a strong,
  unique password. Do not expose the port directly to untrusted clients.
- **No live deauthentication.** Rotating or disabling the password affects new
  authentication decisions; a connection that already authenticated remains
  authenticated, as in Redis. Close/reconnect administrative clients when rotating.
  Existing unauthenticated connections begin receiving `NOAUTH` immediately after
  a password is set.
- **Process arguments are visible.** `--requirepass VALUE` can appear in process
  listings and shell history. Prefer `--requirepass-file` with a mode-`0400` secret
  mount. The file is read only at startup; later rotation uses authenticated
  `CONFIG SET`.
- **No persistence.** The password is not written to disk and does not survive a
  restart unless the deployment supplies the flag/file again. The server also has
  no data persistence or replication.
- **Single-process scope.** This is process-local admission control. It does not
  provide distributed identity, failover, fencing tokens or protection against a
  host/container administrator.

## Deployment guidance

1. Keep the default loopback bind for local development.
2. Inside Docker/Kubernetes, mount a secret file read-only and append
   `--requirepass-file /run/secrets/rillcache`; keep the root filesystem read-only.
3. Restrict ingress with a network policy or firewall. Prefer a TLS terminator for
   anything crossing an untrusted network, and document that inner hops are plaintext.
4. Use a long random password from a secret manager, not a short human password;
   SHA-256 comparison does not make weak passwords hard to guess offline.
5. Rotate with authenticated `CONFIG SET requirepass`, then restart clients, and
   verify a new connection needs the new credential.

## Evidence

Unit tests cover known SHA-256 vectors, constant-time digest equality, disabled and
rotated credentials, exact Redis-compatible errors, `HELLO` atomicity, `NOAUTH`
before data/admin access, unauthenticated `QUIT`, and `CONFIG` redaction/rotation.
Wire tests run in thread-per-client, one-worker and four-worker modes, including
pipelined rejection/authentication/order and a full runtime rotation cycle.

References: [Redis AUTH](https://redis.io/docs/latest/commands/auth/),
[Redis security](https://redis.io/docs/latest/operate/oss_and_stack/management/security/),
[Redis TLS](https://redis.io/docs/latest/operate/oss_and_stack/management/security/encryption/).
