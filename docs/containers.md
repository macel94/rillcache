# Thin Docker and Kubernetes delivery

**Experimental cache—not production Redis compatibility.** Optional single-password
or multi-user ACL-file authentication and an opt-in append-only journal exist,
but there is no native TLS, RDB format, AOF rewrite, replication or graceful
signal-driven shutdown. Use authentication plus a private/trusted network.
Publishing an image does not deploy a service.

## Run it

Once the GHCR package is public, no login, configuration file or extra command is needed:

```sh
docker run --rm --init -p 127.0.0.1:6379:6379 ghcr.io/macel94/rillcache:latest
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
filesystem can stay read-only because only `/data` needs to be writable. There is
no AOF rewrite, so the single journal file grows and must be sized/rotated by the
operator. Restart recovery ignores only an incomplete tail and fails visibly on
mid-file corruption. Stream consumer pending state is not restored. See
[persistence](persistence.md).
Rotate through authenticated `CONFIG SET requirepass` or by restarting with a new
secret. TLS is still absent, so terminate TLS before traffic crosses an untrusted
network and treat the inner connection as plaintext. See [security](security.md).

[`compose.yaml`](../compose.yaml) adds a read-only root filesystem, dropped Linux
capabilities, no-new-privileges, a 256-PID limit, 1 CPU and 256 MiB memory limit.
These are example isolation/resource bounds, not a guaranteed capacity/SLO.
The server defaults remain 128 clients, 64 MiB **key/value payload** and a
30-second socket inactivity timeout. Payload is not RSS: keys, maps, allocator,
threads, expiry indexes and sockets can make actual memory much larger. Adjust
both application limits and runtime memory for the workload; the kernel can OOM-kill.
The default worker mode remains thread-per-client, not an automatic core-count heuristic.

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
TLS, brute-force controls, audit logging or durable backup guarantees.

Use one replica and `Recreate`: independent replicas do not share data, and a
Service over several would randomly route keys to unrelated caches. With
`--appendonly yes --dir /data`, an operator-supplied PersistentVolume can preserve
the journal across container restarts, but the example manifest intentionally has
no PVC and this subset has no replication, rewrite, backup or failover. Prefer a
digest-pinned image for repeatable deployments:
use the `ghcr.io/macel94/rillcache@sha256:...` reference printed by the workflow
in place of `:latest`. `latest` is a convenience main-build tag, not a stable release.

**Termination:** the current safe-std server has no portable signal-handler API.
Docker's `--init` / Compose `init: true` forwards termination to the application as
a normal child; this ends the process but is not a graceful drain. With
`--appendfsync always`, acknowledged records are already synced; with `everysec`
or `no`, termination can lose the unsynced tail. As Kubernetes
PID 1 the default SIGTERM may be ignored; the example allows five seconds before
forced termination. In-flight replies may be lost. This distribution increment
does not close the graceful-shutdown or full durability production gates.

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
