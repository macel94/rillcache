# Thin Docker and Kubernetes delivery

Rillcache's image is ready for local evaluation, with optional passwords/ACLs,
native TLS 1.3 and an append-only journal. Full production compatibility is still
in progress: there is no RDB format, background/incremental rewrite, replication
or full client-draining shutdown. Keep the plaintext listener private even when
TLS is enabled. [Production readiness](production-readiness.md) tracks the
remaining work; the [canonical gap checklist](production-readiness-gaps.md) is the
authoritative list.
Publishing an image does not deploy a service.

## Run it

Once the GHCR package is public, no login or configuration file is needed. The
30-second stop timeout accommodates the standalone shutdown deadline:

```sh
docker run --rm --init --stop-timeout 30 -p 127.0.0.1:6379:6379 ghcr.io/macel94/rillcache:latest
```

The image already runs `/rillcache --bind 0.0.0.0:6379`; the binary's standalone
and library defaults remain loopback-only. The host port above is loopback-only.
`EXPOSE` alone does not publish a host port. Do not use an unrestricted host
`-p 6379:6379` on an untrusted network. Authentication is disabled unless you
supply a password; prefer a read-only secret file over a process argument:

Options append naturally without losing the container bind default:

```sh
docker run --rm --init -p 127.0.0.1:6379:6379 ghcr.io/macel94/rillcache:latest --threads 4
docker run --rm ghcr.io/macel94/rillcache:latest --help
docker run --rm --init -p 127.0.0.1:6379:6379 \
  -v /secure/rillcache-password:/run/secrets/rillcache:ro \
  ghcr.io/macel94/rillcache:latest --requirepass-file /run/secrets/rillcache
# Or provide a multi-user ACL file (mutually exclusive with requirepass):
docker run --rm --init -p 127.0.0.1:6379:6379 \
  -v /secure/users.acl:/run/secrets/users.acl:ro \
  ghcr.io/macel94/rillcache:latest --aclfile /run/secrets/users.acl
docker compose up -d
```

`--requirepass` also works but is visible in process listings and shell history.
The file must be nonempty and at most 1024 bytes; one trailing newline is stripped.
ACL files require an enabled `default` user and use Redis-style `user ...` lines.
`ACL SAVE` needs a writable directory, so a read-only ACL mount supports loading
and runtime `ACL SETUSER` but not persisting changes; use a dedicated writable
volume if runtime ACL edits must survive a restart.

Append-only persistence is off by default. To enable it, give the container a
writable data directory and a sync policy:

```sh
docker run --rm --init -p 127.0.0.1:6379:6379 \
  -v rillcache-data:/data \
  ghcr.io/macel94/rillcache:latest --appendonly yes --dir /data
```

`--appendfsync always` acknowledges only after fsync; `everysec` (default) can
lose roughly a second; `no` only writes without an explicit sync. The root
filesystem can stay read-only because only `/data` needs to be writable. There is no
background/incremental rewrite; `BGREWRITEAOF` performs a synchronous full-state
compaction that briefly pauses command service. Configurable maintenance thresholds
can trigger the same synchronous compaction. Restart recovery ignores only an incomplete tail and fails visibly on
mid-file corruption. Stream consumer pending state is not restored. See
[persistence](persistence.md).
Rotate through authenticated `CONFIG SET requirepass` or by restarting with a new
secret. Native TLS 1.3 is available with `--tls-bind`, `--tls-cert-file` and
`--tls-key-file`; mount certificates read-only and keep the plaintext listener on
loopback/private space. See [native TLS](tls.md) and [security](security.md).

[`compose.yaml`](../compose.yaml) adds a read-only root filesystem, dropped Linux
capabilities, no-new-privileges, a 256-PID limit, 1 CPU and 256 MiB memory limit.
These are example isolation/resource bounds, not a guaranteed capacity/SLO.
The server defaults remain 128 clients, 64 MiB **key/value payload** and a
30-second socket inactivity timeout. Payload is not RSS: keys, maps, allocator,
threads, expiry indexes and sockets can make actual memory much larger. Adjust
both application limits and runtime memory for the workload; the kernel can OOM-kill.
Default Unix plain TCP uses up to four Mio socket reactors within the captured
CPU budget; data connections migrate once to the local data owner. You do not
need a worker override to get started. Advanced modes and their actual resource
accounting are documented in [the runtime guide](hot-paths.md).

## What ships

The final stage is `scratch`: one stripped, statically linked musl-target binary
plus runtime license notices. No Linux distribution, shell, package manager,
compiler, redis-cli, benchmark executable, Copilot, credentials or build tree.
It runs as numeric UID/GID `65532:65532`, needs no writable directory or volume,
and includes a shell-free Docker health check. CI rejects images at or above
8 MiB uncompressed and rejects an ELF interpreter/shared-library requirement.
Actual per-architecture sizes and ELF checks are retained as workflow artifacts.
Small is not a vulnerability-free/security-reviewed claim.

The builder uses the repository-pinned Rust version in a digest-pinned multi-arch
Rust/Alpine image. Musl and standard-runtime internals are within the existing
safe-standard-library/runtime boundary, not project FFI or an added Cargo dependency.
Image-only thin LTO, one codegen unit and symbol stripping do not change ordinary
benchmark compilation settings. No panic/overflow-check weakening is applied.

`/rillcache --healthcheck 127.0.0.1:6379` sends only RESP2 PING, accepts PONG, is
quiet on success (exit 0), and returns nonzero for rejection, bad/truncated replies
or timeout. Socket waits share one one-second deadline; this is not a hard
real-time scheduling guarantee. It checks request service, not persistence or
replication, and does not probe the journal. An overloaded/full client limit can make it fail; leave probe headroom.
Changing the bind/port requires updating probe addresses and port mappings too.

For a password-protected server, override the probe command to use a read-only
mounted secret rather than putting the password in process arguments. Without
an explicit user name, AUTH uses the default user:

```sh
/rillcache --healthcheck 127.0.0.1:6379 --healthcheck-password-file /run/secrets/probe-password
```

For least privilege, provision an enabled named ACL user with a password and
only `-@all +ping` command permissions, then select it explicitly:

```sh
/rillcache --healthcheck 127.0.0.1:6379 --healthcheck-user probe --healthcheck-password-file /run/secrets/probe-password
```

Both options must follow the probe address, in either order, without duplicates.
`--healthcheck-user` requires the password-file option and accepts the server's
bounded ACL-name syntax (1..=128 printable ASCII bytes, excluding `|`). A disabled,
unknown or incorrectly authenticated user fails; a user denied PING also fails
after AUTH, so successful authentication alone is not treated as health. The probe reads at most
1027 file bytes and accepts 1..=1024 password bytes after stripping one trailing
LF or CRLF. It validates an exact AUTH `+OK` before sending PING; both exchanges
share the same one-second socket deadline. Success remains silent. A rejected
AUTH, denied PING, missing/empty/oversized file or malformed response fails
without printing the password or server error payload. File/argument errors exit
2; connection, authentication and response failures exit 1. Local file I/O and OS
scheduling are not subject to a hard real-time deadline. Restrict the mounted
file's permissions and keep it outside the image/build context. Without explicit
TLS options this probe uses plaintext TCP: use loopback inside the container, not
an untrusted network. The default image health check stays unauthenticated and
plaintext; deployments enabling AUTH or TLS must explicitly override it.

### Verified TLS probes

Use an explicit read-only PEM CA bundle to probe the TLS listener:

```sh
/rillcache --healthcheck 127.0.0.1:6380 --healthcheck-tls-ca-file /run/secrets/probe-ca.crt --healthcheck-server-name localhost --healthcheck-user probe --healthcheck-password-file /run/secrets/probe-password
```

All probe options follow the numeric IP:port address, in any order, without
duplicates. `--healthcheck-tls-ca-file` enables TLS 1.3 with the existing rustls
implementation; it does not use system roots, skip verification or fall back to
plaintext. The certificate chain must validate against the supplied roots and
match the probe IP, or the explicit DNS/IP identity in `--healthcheck-server-name`.
The name option requires TLS and changes only certificate verification/SNI, not
the numeric connection address; it performs no DNS lookup. The CA file is bounded
to 1 MiB and 128 certificates; missing, empty, invalid or oversized bundles fail
before connecting. TLS handshake and every underlying socket read/write share
the same one-second deadline with AUTH, PING and optional INFO. Authentication is
sent only after a successful handshake. Success is silent, while trust/name,
handshake and authentication failures exit nonzero. CA-file/argument errors exit
2. Local certificate parsing/file I/O and OS scheduling have no hard real-time
guarantee. Client-certificate (mTLS) probe credentials are not supported: a
listener requiring them fails this probe rather than weakening authentication.
TLS adds transport verification, not evidence of journal durability or recovery.

### Require reported journal health

Append `--healthcheck-require-aof` after the address to additionally query
`INFO persistence` and require exactly one of each of these fields:

- `loading:0`
- `aof_enabled:1`
- `aof_last_write_status:ok`
- `aof_last_bgrewrite_status:ok`

```sh
/rillcache --healthcheck 127.0.0.1:6379 --healthcheck-require-aof --healthcheck-user probe --healthcheck-password-file /run/secrets/probe-password
```

This form needs `+info` as well as `+ping`; ordinary PING-only probes need no new
permission. AUTH, PING and INFO share the same socket deadline. The INFO reply
must be a RESP2 bulk no larger than 16 KiB with a header no larger than 32 bytes.
Disabled persistence, reported errors, missing/duplicate required fields, denied
INFO, malformed/truncated/oversized replies and timeouts fail the probe without
reflecting server payloads. Other INFO fields are permitted. A rewrite in
progress is not itself a failure, but a reported failed last rewrite makes this
conservative probe unhealthy even if the live journal can still accept writes. This is a conservative
**reported-status** check only: it performs no write or fsync, cannot discover an
unreported disk fault, and does not prove recovery, backup validity or an RPO/RTO.
The default container probe does not require a journal; opt in explicitly only
for deployments that expect persistence.

## Kubernetes

```sh
kubectl apply -f deploy/kubernetes.yaml
kubectl rollout status deployment/rillcache
kubectl port-forward service/rillcache 6379:6379
```

[`deploy/kubernetes.yaml`](../deploy/kubernetes.yaml) supplies one restricted
non-root/read-only Pod, startup and PING readiness probes, resource bounds, no
service-account token, a private ClusterIP Service and an ingress NetworkPolicy.
Client Pods **in the same namespace** must have `rillcache-client: "true"` to be
allowed through that policy. NetworkPolicy enforcement requires a capable CNI;
without it, other cluster workloads may reach a service that is unauthenticated
unless you configure a password.
There is intentionally no PING liveness restart that could erase the cache merely
because its client slots are full. Kubernetes still restarts an exited process.

To require a password or ACL file, create a secret and mount it read-only, then
append the file option without replacing the image entrypoint:

```sh
kubectl create secret generic rillcache-auth --from-literal=requirepass="$(openssl rand -hex 32)"
kubectl patch deployment rillcache --type=json -p='[
  {"op":"add","path":"/spec/template/spec/volumes","value":[{"name":"auth","secret":{"secretName":"rillcache-auth","defaultMode":256}}]},
  {"op":"add","path":"/spec/template/spec/containers/0/volumeMounts","value":[{"name":"auth","mountPath":"/run/secrets/rillcache","readOnly":true,"subPath":"requirepass"}]},
  {"op":"add","path":"/spec/template/spec/containers/0/args","value":["--requirepass-file","/run/secrets/rillcache"]}
]'
```

For multi-user ACLs, create the secret from an ACL file and use
`--aclfile /run/secrets/users.acl` instead. This keeps the root filesystem
read-only and credentials out of the image and process arguments. It does not add
cross-process/distributed rate limiting, collector sender authentication/TLS/
federation or durable backup guarantees. The optional process-local auth-failure
budget is not shared across replicas. TLS requires explicit certificate
configuration.

Use one replica and `Recreate`: independent replicas do not share data, and a
Service over several would randomly route keys to unrelated caches. With
`--appendonly yes --dir /data`, an operator-supplied PersistentVolume can preserve
the journal across container restarts, but the example manifest intentionally has
no PVC and this subset has no replication, background rewrite, validated
backup/restore workflow or automatic failover. Prefer a
digest-pinned image for repeatable deployments:
use the `ghcr.io/macel94/rillcache@sha256:...` reference printed by the workflow
in place of `:latest`. `latest` is a convenience main-build tag, not a stable release.

**Termination:** the standalone binary handles SIGINT/SIGTERM on Unix and
Ctrl-C on Windows. It stops admission, joins the server/client workers, then
forces a journal flush. The shutdown deadline is 25 seconds; a second termination
signal or an exceeded deadline is logged as forced termination and exits with
status 1. Set the supervisor grace period to at least 30 seconds; the Kubernetes
example does so. Docker's `--init` / Compose `init: true` forwards SIGTERM to the
application as a normal child. Use Docker `--stop-timeout 30` or Compose
`stop_grace_period: 30s` so the supervisor does not kill it before its 25-second
deadline. This is not a full client drain: replies for commands already dispatched are
written before their read-side-closed connections finish, but idle/incomplete
clients, subscriptions, blocked commands and replies that exceed a bounded
transport write timeout or the shutdown deadline may be abandoned. Server lanes
allow one second total for reply draining, then close remaining sockets; Tokio
individual writes are also capped at the smaller of one second and the configured
idle timeout. With `--appendfsync always`, acknowledged records are already
synced; with `everysec` or `no`, a successful signal shutdown flushes the journal
before exit, but forced termination may lose the unsynced tail. The
signal/shutdown path is still partial and does not close the full durability or
graceful-drain production gates.

## Publication and verification

[Container image](../.github/workflows/container.yml) runs on main pushes, version
tags, PRs and manual dispatch. It first runs `make ci`, then builds and tests native
`linux/amd64` and `linux/arm64` images. Both architectures must pass all per-client,
one-worker and four-worker TCP/conformance checks before a separate job can push
those **exact tested archives**, without rebuilding. PRs cannot publish. Only the
trusted publisher has `packages: write`, using the short-lived `GITHUB_TOKEN`;
no repository secret or personal token is required. Actions and the builder image
are pinned. Registry permissions do not grant source-repository write access.

Tags: `sha-<full source SHA>` plus architecture-specific source tags; `latest`
only for the current main head; `v<version>` only when the tag matches Cargo's
real package version. Reruns cannot promote an old source to `latest`. The final
manifest must contain exactly amd64 and arm64. A digest—not a mutable tag—is the
content-immutable deployment identity. OCI labels link the real source revision.
No image signature, SBOM or SLSA attestation is claimed by this workflow.

### One-time GitHub public visibility

GitHub creates new GHCR packages **private**, even for public source repositories.
Linking a repository inherits access permissions, **not package visibility**.
After the first publish, the package owner must open
[package settings](https://github.com/users/macel94/packages/container/rillcache/settings),
choose **Change visibility → Public**, and confirm. GitHub documents this as a
package-settings action; its documented REST/GraphQL APIs do not expose a
visibility-changing mutation. No broad PAT workaround or stored credential is used.

Separate fresh-runner jobs attempt an anonymous pull of the exact published digest
and re-run its live checks on both native architectures. They fail visibly if the
package is private and explain the action above. Re-run failed jobs after changing
visibility. A successful authenticated push alone is **not** proof anyone can pull.

## Build and test locally

Docker (or compatible optional tooling) and network access for the builder are
needed only for this explicit path, not `make ci` or Cargo's dependency graph:

```sh
make container
make container-smoke
# With an existing Podman build:
make container-smoke CONTAINER_ENGINE=podman CONTAINER_IMAGE=rillcache:local
```

`docker build -t rillcache:local .` also works. The build-context allowlist excludes
`.git`, `.env`, host targets and benchmark evidence. Dirty local builds are labelled
by the Make target with a `-dirty` source suffix. Smoke tests use the real image
entrypoint, published loopback host ports, read-only/capability/resource restrictions,
all three worker modes, exact replies and the supported command corpus. Container
measurements are correctness smoke, not the native performance dashboard.

References: [scratch](https://docs.docker.com/build/building/base-images/),
[GHCR](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry),
[package visibility](https://docs.github.com/en/packages/learn-github-packages/configuring-a-packages-access-control-and-visibility),
[Kubernetes probes](https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/).
