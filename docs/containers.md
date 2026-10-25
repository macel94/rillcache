# Thin Docker and Kubernetes delivery

**Experimental, volatile cache—not production Redis compatibility.** There is no
AUTH/ACL, native TLS, persistence, replication or graceful signal-driven shutdown.
Use only a private/trusted network. Publishing an image does not deploy a service.

## Run it

Once the GHCR package is public, no login, configuration file or extra command is needed:

```sh
docker run --rm --init -p 127.0.0.1:6379:6379 ghcr.io/macel94/rillcache:latest
```

The image already runs `/rillcache --bind 0.0.0.0:6379`; the binary's standalone
and library defaults remain loopback-only. The host port above is loopback-only.
`EXPOSE` alone does not publish a host port. Do not use an unrestricted host
`-p 6379:6379` on an untrusted network: this release has no authentication.
A Docker-network client can instead connect to the container name on port 6379.

Options append naturally without losing the container bind default:

```sh
docker run --rm --init -p 127.0.0.1:6379:6379 ghcr.io/macel94/rillcache:latest --threads 4
docker run --rm ghcr.io/macel94/rillcache:latest --help
docker compose up -d
```

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
replication. An overloaded/full client limit can make it fail; leave probe headroom.
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
without it, other cluster workloads may reach this unauthenticated service.
There is intentionally no PING liveness restart that could erase the cache merely
because its client slots are full. Kubernetes still restarts an exited process.

Use one replica and `Recreate`: independent replicas do not share data, and a
Service over several would randomly route keys to unrelated caches. Upgrades and
restarts lose all data. There is no PVC because mounting a directory does not
create persistence. Prefer a digest-pinned image for repeatable deployments:
use the `ghcr.io/macel94/rillcache@sha256:...` reference printed by the workflow
in place of `:latest`. `latest` is a convenience main-build tag, not a stable release.

**Termination:** the current safe-std server has no portable signal-handler API.
Docker's `--init` / Compose `init: true` forwards termination to the application as
a normal child; this ends the process but is not a graceful drain. As Kubernetes
PID 1 the default SIGTERM may be ignored; the example allows five seconds before
forced termination. In-flight replies may be lost. This distribution increment
does not close the graceful-shutdown/durability production gates.

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
