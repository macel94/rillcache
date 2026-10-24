# rillcache

An experiment in building a Redis-compatible server from the ground up in Rust.

## Project vision

The initial goal is to implement a useful Redis-compatible server using only Rust’s standard library and runtime—no external dependencies. Over time, the project may explore newer and more advanced functionality, including streams and richer pub/sub capabilities.

Compatibility and behavior will be guided by tests, with the aim of making the server dependable and understandable rather than merely checking off features.

## How we’ll work

This is a collaboration between a human and AI:

- I (the human) will bring the ideas, direction, and product judgment.
- I will validate behavior and proposals with tests.
- All project code must be written by AI; only AI-generated code will be accepted.
- Tests and specifications are how ideas are checked and refined.

## Constraints

- **Language:** Rust
- **Dependencies:** None beyond the Rust standard library and runtime
- **Compatibility target:** Redis protocol and behavior, developed incrementally

## Status

Early-stage project. The goals above describe the intended direction, not a claim of current Redis compatibility or implemented features.
