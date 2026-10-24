# Original GitHub CI evidence: ed15e2f

Unmodified result files downloaded from:

- [CI 40098539251](https://github.com/macel94/rillcache/actions/runs/40098539251): `smoke/`.
- [Comparisons 40098536127](https://github.com/macel94/rillcache/actions/runs/40098536127): `pipeline-1/` and `pipeline-16/`.

Both ran source commit `ed15e2f3d29bf8db988b984bd1ce1652cd58a7bd` successfully.
No binary archives or credentials are included. Environment files retain the
original source, compiler, hardware, configuration and reference executable
digests. Different reference digests across profiles are one of the discovered
build-harness defects, not a claimed change to upstream source.

See [the review](../../../docs/ci-benchmark-review.md) before interpreting these
numbers. In particular, threading budgets were not normalized, pipelined samples
were short, and smoke versus comparison jobs ran on different CPU models. These
results are not a proof of a general performance advantage or a regression gate.
