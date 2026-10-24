---
name: Small feature story
about: Propose one independently useful, testable increment
---

## Story

As a …, I want …, so that … .

## INVEST check

- Independent user-visible outcome and explicit prerequisites:
- Negotiable implementation choices:
- Value:
- Estimate (XS/S, or split the story):
- Small scope / deliberately excluded behavior:
- Testable outcome:

## EARS acceptance criteria

- When …, the server shall … .
- While …, the server shall … .
- If …, then the server shall … .

## Compatibility and edge cases

Binary/empty/missing values, invalid arity/options, resource limits, concurrency,
connection behavior, and any differences from current Redis/Valkey.

## Validation

Named unit/wire tests, relevant benchmark workload/parameters, and documentation
updates. Safe Rust and zero external dependencies are non-negotiable.
