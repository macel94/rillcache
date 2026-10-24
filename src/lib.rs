#![forbid(unsafe_code)]

//! An incremental Redis-compatible server built exclusively on safe `std` APIs.
//!
//! This first slice supports RESP2 arrays of non-null bulk strings and a small
//! command subset. It is not a drop-in Redis replacement; see `docs/requirements.md`.

pub mod engine;
pub mod protocol;
pub mod server;

pub use server::{Config, Server, ServerHandle};
