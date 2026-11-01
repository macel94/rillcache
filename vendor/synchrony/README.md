# Synchrony

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/synchrony/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/synchrony)](https://crates.io/crates/synchrony)
[![docs.rs](https://img.shields.io/badge/docs.rs-synchrony-latest)](https://docs.rs/synchrony)
[![Check](https://github.com/compio-rs/synchrony/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/synchrony/actions/workflows/ci_check.yml)
[![Telegram](https://img.shields.io/badge/Telegram-compio--rs-blue?logo=telegram)](https://t.me/compio_rs)
[![Discord](https://img.shields.io/discord/1429103613602435114?logo=discord&label=Discord)](https://discord.gg/bGG8EF9v)

A library that provides both sync and unsync versions of common synchronization primitives.

## Features

All of the following primitives are provided in both sync and unsync versions:

- Shared (`Rc`/`Arc`)
- Atomic Scalars
- Watch
- Waker Slot (`AtomicWaker` and its unsync counterpart)
- Mutex
- Async Mutex
- BiLock
- Flag (specialized `AtomicBool`)
- Event (`event-listener` and `local-event`)
- Async Flag

## Loom Testing Support

This library includes built-in support for [loom](https://github.com/tokio-rs/loom), a testing tool for concurrent Rust code that helps verify the correctness of concurrent algorithms.

When compiled with `--cfg loom`, the library automatically switches to use loom's implementations of synchronization primitives instead of the standard library versions. This enables you to:

- Detect data races and concurrency bugs
- Verify lock-free algorithms
- Test different thread interleavings exhaustively

For more information about loom and how to write loom tests, see the [loom documentation](https://docs.rs/loom).

### Supported Types

The following types automatically use loom implementations when the `loom` cfg is enabled:

- `std::sync::atomic::*` types (`AtomicBool`, `AtomicUsize`, etc.)
- `std::sync::Arc` → `loom::sync::Arc`
- `std::sync::Mutex` → `loom::sync::Mutex`
- `std::cell::UnsafeCell` → `loom::cell::UnsafeCell`
- `std::cell::Cell` → `loom::cell::Cell`
