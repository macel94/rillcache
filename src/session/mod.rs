//! Per-connection protocol negotiation and bounded client metadata.
//!
//! RESP3 support is deliberately limited to the reply types emitted by the
//! current command set: strings, integers, errors, arrays and nulls.
//!
//! The implementation is split by concern: `dispatch` routes commands,
//! `transaction` owns MULTI/EXEC/WATCH state, `auth` owns AUTH/ACL, `pubsub`
//! owns subscriptions and delivery, `blocking` owns deferred blocking commands
//! and wakeups, `pause` defers commands during CLIENT PAUSE, `client` owns
//! HELLO and CLIENT metadata/replies, `reply` converts replies to RESP3, and
//! `command` holds shared classification helpers.

mod auth;
mod blocking;
mod client;
mod command;
mod dispatch;
mod pause;
mod pubsub;
mod reply;
mod transaction;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::Instant;

use crate::engine::{Action, Store, WatchState};

use blocking::Blocked;
use command::arity;
use pause::PausedCommand;
use transaction::Multi;

const MAX_NAME_BYTES: usize = 256;
const MAX_LIBRARY_FIELD_BYTES: usize = 128;
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

/// State belonging to one socket. Move this together with any borrowed client
/// lease; it contains no socket handles and is never shared across clients.
#[derive(Debug)]
pub struct Session {
    protocol: Protocol,
    id: u64,
    name: Option<Vec<u8>>,
    library_name: Option<Vec<u8>>,
    library_version: Option<Vec<u8>>,
    multi: Option<Multi>,
    watched: WatchState,
    authenticated_user: Option<Vec<u8>>,
    peer_ip: Option<IpAddr>,
    failed_auth: usize,
    close_requested: bool,
    pubsub_sender: Option<SyncSender<Vec<u8>>>,
    monitoring: bool,
    pubsub_channels: BTreeSet<Vec<u8>>,
    pubsub_patterns: BTreeSet<Vec<u8>>,
    pubsub_shard_channels: BTreeSet<Vec<u8>>,
    blocked: Option<Blocked>,
    paused: Option<PausedCommand>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Protocol {
    Resp2,
    Resp3,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            protocol: Protocol::Resp2,
            id: NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed),
            name: None,
            library_name: None,
            library_version: None,
            multi: None,
            watched: WatchState::default(),
            authenticated_user: None,
            peer_ip: None,
            failed_auth: 0,
            close_requested: false,
            pubsub_sender: None,
            monitoring: false,
            pubsub_channels: BTreeSet::new(),
            pubsub_patterns: BTreeSet::new(),
            pubsub_shard_channels: BTreeSet::new(),
            blocked: None,
            paused: None,
        }
    }
    /// Record the accepted peer's IP for per-source authentication throttling.
    pub fn set_peer(&mut self, peer: SocketAddr) {
        self.peer_ip = Some(peer.ip());
    }

    /// Dispatch connection commands locally and all data commands through the
    /// stable public RESP2 Store API, adapting only this session's reply.
    pub fn execute(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        let start = output.len();
        // The benchmarked default mode has no auth, observers, pause, MULTI,
        // subscriber or monitor context. Keep those cold checks out of the
        // hottest GET/SET/ECHO/PING replies. Any enabled feature falls through
        // to the normal fully checked path.
        if self.multi.is_none()
            && self.authenticated_user.is_none()
            && self.paused.is_none()
            && self.blocked.is_none()
            && !self.is_subscribed()
            && !self.is_monitoring()
            && store.plain_fast()
            && arguments.first().is_some_and(|command| {
                command.eq_ignore_ascii_case(b"PING")
                    || command.eq_ignore_ascii_case(b"GET")
                    || command.eq_ignore_ascii_case(b"SET")
                    || command.eq_ignore_ascii_case(b"ECHO")
            })
        {
            store.record_command(self.id);
            if arguments[0].eq_ignore_ascii_case(b"PING") {
                match arguments.len() {
                    1 => output.extend_from_slice(b"+PONG\r\n"),
                    2 => crate::protocol::encode_bulk(Some(arguments[1]), output),
                    _ => arity("ping", output),
                }
                if output.get(start) == Some(&b'-') {
                    store.record_error(&output[start..]);
                }
                return Action::Continue;
            }
            let action = self.execute_store(store, arguments, output);
            if output.get(start) == Some(&b'-') {
                store.record_error(&output[start..]);
            }
            return action;
        }
        if self.pause_command(store, arguments) {
            return Action::Continue;
        }
        let (slowlog, latency) = arguments.first().map_or((false, false), |command| {
            let queued = self.multi.is_some() && !command.eq_ignore_ascii_case(b"EXEC");
            (
                !queued && store.slowlog_enabled(),
                !queued && store.latency_enabled(),
            )
        });
        // CONFIG, or EXEC containing a queued CONFIG, can enable commandstats
        // during this dispatch. Check these cold exceptions only if no observer
        // already needs a timer; an individually queued CONFIG changes nothing.
        let observed = slowlog || latency || store.commandstats_enabled();
        let may_enable = !observed
            && arguments.first().is_some_and(|name| {
                name.eq_ignore_ascii_case(b"EXEC")
                    || (self.multi.is_none() && name.eq_ignore_ascii_case(b"CONFIG"))
            });
        let started = (observed || may_enable).then(Instant::now);
        let reply_start = output.len();
        let action = self.execute_inner(store, arguments, output);
        if let Some(started) = started {
            let reply = &output[reply_start..];
            if latency || (store.commandstats_enabled() && reply != b"+QUEUED\r\n") {
                let elapsed = started.elapsed();
                if reply != b"+QUEUED\r\n" {
                    store.record_command_stats(arguments.first().copied(), elapsed, reply);
                }
                if latency {
                    store.record_command_latency(elapsed);
                }
            }
            if slowlog {
                if let Some(blocked) = self.blocked.as_mut() {
                    blocked.slowlog = Some((
                        started,
                        arguments.iter().map(|argument| argument.to_vec()).collect(),
                    ));
                } else {
                    self.record_slowlog(store, arguments, started);
                }
            }
        }
        if output.get(start) == Some(&b'-') {
            store.record_error(&output[start..]);
        }
        action
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    /// Disconnect-equivalent server-side context reset (Redis RESET).
    pub(super) fn reset(
        &mut self,
        store: &Store,
        arguments: &[&[u8]],
        output: &mut Vec<u8>,
    ) -> Action {
        if arguments.len() != 1 {
            arity("reset", output);
            return Action::Continue;
        }
        self.multi = None;
        self.watched = WatchState::default();
        self.blocked = None;
        self.sync_blocked(store);
        self.monitoring = false;
        store.remove_monitor(self.id);
        for channel in std::mem::take(&mut self.pubsub_channels) {
            store.pubsub_unsubscribe(false, &channel, self.id);
        }
        for pattern in std::mem::take(&mut self.pubsub_patterns) {
            store.pubsub_unsubscribe(true, &pattern, self.id);
        }
        for channel in std::mem::take(&mut self.pubsub_shard_channels) {
            store.pubsub_unsubscribe_shard(&channel, self.id);
        }
        self.protocol = Protocol::Resp2;
        self.authenticated_user = None;
        self.failed_auth = 0;
        self.close_requested = false;
        self.sync_client_metadata(store);
        output.extend_from_slice(b"+RESET\r\n");
        Action::Continue
    }
}
