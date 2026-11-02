# Container distribution

Open items are tracked in the canonical [production-readiness gaps](../production-readiness-gaps.md); this file records current behavior and boundaries, not a second checklist.

Part of the [production-readiness plan](../production-readiness.md). This file owns
the capability row so independent feature increments edit only their own area.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Container distribution | S14/S15: scratch/non-root static images, shell-free PING probe, native amd64/arm64 mode tests, separate GHCR publisher and anonymous-access gate, restricted Docker/Kubernetes examples. See [delivery and first-package visibility](../containers.md). Distribution alone closes no authentication, durability, topology or production-validation gate. |
