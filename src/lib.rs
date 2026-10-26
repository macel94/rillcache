#![forbid(unsafe_code)]

//! An incremental Redis-compatible server built exclusively on safe `std` APIs.
//!
//! RESP2 is the default; connection sessions also negotiate a documented RESP3
//! subset and retain common client metadata. This is not a drop-in Redis
//! replacement; see `docs/client-sessions.md` and `docs/requirements.md`.

pub mod engine;
pub mod protocol;
pub mod server;
pub mod session;

pub use server::{Config, Server, ServerHandle};
