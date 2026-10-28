//! Deferred blocking commands, their deadlines and wakeup polling.

use std::io::Write;
use std::time::{Duration, Instant};

use crate::engine::{
    ListMoveOutcome, ListPopOutcome, PopOutcome, Store, StreamId, XReadGroupOptions,
    XReadGroupOutcome, XReadGroupStart, XReadOutcome, ZSetPopOutcome, blocking_lmpop_reply,
    blocking_reply, blocking_zpop_reply, list_move_reply, parse_bzmpop_timeout, parse_list_mpop,
    parse_mpop, parse_side, parse_xread, xread_reply, xreadgroup_reply, zset_pop_single_reply,
};

use super::Protocol;
use super::Session;
use super::command::arity;
use super::reply::convert_resp2_reply;

#[derive(Debug)]
pub(super) struct Blocked {
    pub(super) keys: Vec<Vec<u8>>,
    pub(super) deadline: Option<Instant>,
    pub(super) kind: BlockedKind,
    pub(super) slowlog: Option<(Instant, Vec<Vec<u8>>)>,
}

#[derive(Clone, Debug)]
pub(super) enum BlockedKind {
    List {
        left: bool,
    },
    ListMulti {
        left: bool,
        count: usize,
    },
    ListMove {
        source: Vec<u8>,
        destination: Vec<u8>,
        from_left: bool,
        to_left: bool,
    },
    ZSet {
        max: bool,
        count: usize,
    },
    ZSetSingle {
        max: bool,
    },
    WaitAof {
        local: u64,
    },
    XRead {
        starts: Vec<StreamId>,
        count: usize,
    },
    XReadGroup {
        group: Vec<u8>,
        consumer: Vec<u8>,
        count: usize,
        noack: bool,
    },
}

impl Session {
    pub(super) fn sync_blocked(&self, store: &Store) {
        store.set_client_blocked(self.id, self.blocked.is_some());
    }

    pub(super) fn record_slowlog(&self, store: &Store, arguments: &[&[u8]], started: Instant) {
        if !store.slowlog_enabled() {
            return;
        }
        store.slowlog_push(arguments, started.elapsed(), || {
            (
                store.client_address(self.id).unwrap_or_default(),
                self.name.clone().unwrap_or_default(),
            )
        });
    }
    pub(super) fn blocking(
        &mut self,
        store: &Store,
        arguments: &[&[u8]],
        left: bool,
        output: &mut Vec<u8>,
    ) {
        let name = if left { "blpop" } else { "brpop" };
        if arguments.len() < 2 {
            arity(name, output);
            return;
        }
        let (timeout, keys) = arguments.split_last().expect("nonempty");
        let Some(timeout) = std::str::from_utf8(timeout)
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        else {
            output.extend_from_slice(b"-ERR timeout is not a float or out of range\r\n");
            return;
        };
        let keys: Vec<Vec<u8>> = keys.iter().map(|key| key.to_vec()).collect();
        match store.blocking_pop(&keys.iter().map(Vec::as_slice).collect::<Vec<_>>(), left) {
            PopOutcome::Empty => {}
            outcome => {
                blocking_reply(outcome, output);
                return;
            }
        }
        let deadline = if timeout == 0.0 {
            None
        } else {
            let Some(duration) = Duration::try_from_secs_f64(timeout).ok() else {
                output.extend_from_slice(b"-ERR timeout is out of range\r\n");
                return;
            };
            Instant::now().checked_add(duration)
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys,
            deadline,
            kind: BlockedKind::List { left },
        });
        self.sync_blocked(store);
    }

    pub(super) fn blocking_lmpop(
        &mut self,
        store: &Store,
        arguments: &[&[u8]],
        output: &mut Vec<u8>,
    ) {
        if arguments.len() < 4 {
            arity("blmpop", output);
            return;
        }
        let timeout = match parse_bzmpop_timeout(arguments[0]) {
            Ok(timeout) => timeout,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        let options = match parse_list_mpop(arguments, 1) {
            Ok(options) => options,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        match store.blocking_lmpop(&options.keys, options.left, options.count) {
            ListPopOutcome::Empty => {}
            outcome => {
                blocking_lmpop_reply(outcome, output);
                return;
            }
        }
        let deadline = if timeout == 0.0 {
            None
        } else {
            let Some(duration) = Duration::try_from_secs_f64(timeout).ok() else {
                output.extend_from_slice(b"-ERR timeout is out of range\r\n");
                return;
            };
            Instant::now().checked_add(duration)
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: options.keys.iter().map(|key| key.to_vec()).collect(),
            deadline,
            kind: BlockedKind::ListMulti {
                left: options.left,
                count: options.count,
            },
        });
        self.sync_blocked(store);
    }

    pub(super) fn blocking_list_move(
        &mut self,
        store: &Store,
        arguments: &[&[u8]],
        output: &mut Vec<u8>,
    ) {
        let blmove = arguments[0].eq_ignore_ascii_case(b"BLMOVE");
        let (source, destination, from_left, to_left, timeout_raw) = if blmove {
            if arguments.len() != 6 {
                arity("blmove", output);
                return;
            }
            let (Some(from_left), Some(to_left)) =
                (parse_side(arguments[3]), parse_side(arguments[4]))
            else {
                output.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            };
            (arguments[1], arguments[2], from_left, to_left, arguments[5])
        } else {
            if arguments.len() != 4 {
                arity("brpoplpush", output);
                return;
            }
            (arguments[1], arguments[2], false, true, arguments[3])
        };
        let Some(timeout) = std::str::from_utf8(timeout_raw)
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite())
        else {
            output.extend_from_slice(b"-ERR timeout is not a float or out of range\r\n");
            return;
        };
        if timeout < 0.0 {
            output.extend_from_slice(b"-ERR timeout is negative\r\n");
            return;
        }
        match store.blocking_list_move(source, destination, from_left, to_left) {
            ListMoveOutcome::Empty => {}
            outcome => {
                list_move_reply(outcome, output);
                return;
            }
        }
        let deadline = if timeout == 0.0 {
            None
        } else {
            let Some(duration) = Duration::try_from_secs_f64(timeout).ok() else {
                output.extend_from_slice(b"-ERR timeout is out of range\r\n");
                return;
            };
            Instant::now().checked_add(duration)
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: vec![source.to_vec(), destination.to_vec()],
            deadline,
            kind: BlockedKind::ListMove {
                source: source.to_vec(),
                destination: destination.to_vec(),
                from_left,
                to_left,
            },
        });
        self.sync_blocked(store);
    }

    pub(super) fn blocking_zpop(
        &mut self,
        store: &Store,
        arguments: &[&[u8]],
        output: &mut Vec<u8>,
    ) {
        if arguments.len() < 4 {
            arity("bzmpop", output);
            return;
        }
        let timeout = match parse_bzmpop_timeout(arguments[0]) {
            Ok(timeout) => timeout,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        let options = match parse_mpop(arguments, 1) {
            Ok(options) => options,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        match store.blocking_zpop(&options.keys, options.max, options.count) {
            ZSetPopOutcome::Empty => {}
            outcome => {
                blocking_zpop_reply(outcome, output);
                return;
            }
        }
        let deadline = if timeout == 0.0 {
            None
        } else {
            let Some(duration) = Duration::try_from_secs_f64(timeout).ok() else {
                output.extend_from_slice(b"-ERR timeout is out of range\r\n");
                return;
            };
            Instant::now().checked_add(duration)
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: options.keys.iter().map(|key| key.to_vec()).collect(),
            deadline,
            kind: BlockedKind::ZSet {
                max: options.max,
                count: options.count,
            },
        });
        self.sync_blocked(store);
    }

    pub(super) fn blocking_zset_single(
        &mut self,
        store: &Store,
        arguments: &[&[u8]],
        max: bool,
        output: &mut Vec<u8>,
    ) {
        let name = if max { "bzpopmax" } else { "bzpopmin" };
        let Some((timeout_argument, keys)) = arguments.split_last() else {
            arity(name, output);
            return;
        };
        if keys.is_empty() {
            arity(name, output);
            return;
        }
        let timeout = match parse_bzmpop_timeout(timeout_argument) {
            Ok(timeout) => timeout,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        match store.blocking_zpop(keys, max, 1) {
            ZSetPopOutcome::Empty => {}
            outcome => {
                zset_pop_single_reply(outcome, output);
                return;
            }
        }
        let deadline = if timeout == 0.0 {
            None
        } else {
            let Some(duration) = Duration::try_from_secs_f64(timeout).ok() else {
                output.extend_from_slice(b"-ERR timeout is out of range\r\n");
                return;
            };
            Instant::now().checked_add(duration)
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: keys.iter().map(|key| key.to_vec()).collect(),
            deadline,
            kind: BlockedKind::ZSetSingle { max },
        });
        self.sync_blocked(store);
    }

    pub(super) fn xread(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let options = match parse_xread(arguments) {
            Ok(options) => options,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        let Some((starts, outcome)) = store.xread_start(&options) else {
            output.extend_from_slice(b"-ERR internal store error\r\n");
            return;
        };
        let Some(block) = options.block else {
            xread_reply(outcome, output);
            return;
        };
        match outcome {
            XReadOutcome::Empty => {}
            outcome => {
                xread_reply(outcome, output);
                return;
            }
        }
        let deadline = if block == 0 {
            None
        } else {
            Instant::now().checked_add(Duration::from_millis(block))
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: options.keys.iter().map(|key| key.to_vec()).collect(),
            deadline,
            kind: BlockedKind::XRead {
                starts,
                count: options.count,
            },
        });
        self.sync_blocked(store);
    }

    pub(super) fn blocking_xreadgroup(
        &mut self,
        store: &Store,
        options: &XReadGroupOptions<'_>,
        output: &mut Vec<u8>,
    ) {
        match store.blocking_xreadgroup(options) {
            XReadGroupOutcome::Empty => {}
            outcome => {
                xreadgroup_reply(outcome, options.group, output);
                return;
            }
        }
        let block = options.block.unwrap_or(0);
        let deadline = if block == 0 {
            None
        } else {
            Instant::now().checked_add(Duration::from_millis(block))
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: options.keys.iter().map(|key| key.to_vec()).collect(),
            deadline,
            kind: BlockedKind::XReadGroup {
                group: options.group.to_vec(),
                consumer: options.consumer.to_vec(),
                count: options.count,
                noack: options.noack,
            },
        });
        self.sync_blocked(store);
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked.is_some()
    }

    /// Poll a blocked pop. Returns one complete reply when it succeeded, failed or
    /// timed out, or `None` while the connection must remain blocked.
    pub fn poll_blocked(&mut self, store: &Store) -> Option<Vec<u8>> {
        let blocked = self.blocked.as_ref()?;
        let keys: Vec<&[u8]> = blocked.keys.iter().map(Vec::as_slice).collect();
        let kind = blocked.kind.clone();
        let expired = blocked
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline);
        let mut reply = Vec::new();
        let ready = match kind {
            BlockedKind::List { left } => match store.blocking_pop(&keys, left) {
                PopOutcome::Empty => false,
                outcome => {
                    blocking_reply(outcome, &mut reply);
                    true
                }
            },
            BlockedKind::ListMulti { left, count } => {
                match store.blocking_lmpop(&keys, left, count) {
                    ListPopOutcome::Empty => false,
                    outcome => {
                        blocking_lmpop_reply(outcome, &mut reply);
                        true
                    }
                }
            }
            BlockedKind::ListMove {
                source,
                destination,
                from_left,
                to_left,
            } => match store.blocking_list_move(&source, &destination, from_left, to_left) {
                ListMoveOutcome::Empty => false,
                outcome => {
                    list_move_reply(outcome, &mut reply);
                    true
                }
            },
            BlockedKind::ZSet { max, count } => match store.blocking_zpop(&keys, max, count) {
                ZSetPopOutcome::Empty => false,
                outcome => {
                    blocking_zpop_reply(outcome, &mut reply);
                    true
                }
            },
            BlockedKind::ZSetSingle { max } => match store.blocking_zpop(&keys, max, 1) {
                ZSetPopOutcome::Empty => false,
                outcome => {
                    zset_pop_single_reply(outcome, &mut reply);
                    true
                }
            },
            BlockedKind::WaitAof { local } => {
                if expired {
                    let _ = write!(reply, "*2\r\n:{local}\r\n:0\r\n");
                    true
                } else {
                    false
                }
            }
            BlockedKind::XRead { starts, count } => match store.xread_once(&keys, &starts, count) {
                Some(XReadOutcome::Empty) | None => false,
                Some(outcome) => {
                    xread_reply(outcome, &mut reply);
                    true
                }
            },
            BlockedKind::XReadGroup {
                group,
                consumer,
                count,
                noack,
            } => {
                let options = XReadGroupOptions {
                    count,
                    block: None,
                    noack,
                    group: &group,
                    consumer: &consumer,
                    keys: keys.clone(),
                    starts: vec![XReadGroupStart::New; keys.len()],
                };
                match store.blocking_xreadgroup(&options) {
                    XReadGroupOutcome::Empty => false,
                    outcome => {
                        xreadgroup_reply(outcome, &group, &mut reply);
                        true
                    }
                }
            }
        };
        if !ready && !expired {
            return None;
        }
        let slowlog = self
            .blocked
            .as_mut()
            .and_then(|blocked| blocked.slowlog.take());
        self.blocked = None;
        self.sync_blocked(store);
        if !ready {
            reply.extend_from_slice(b"*-1\r\n");
        }
        if self.protocol == Protocol::Resp3 {
            reply = convert_resp2_reply(&reply);
        }
        if let Some((started, arguments)) = slowlog {
            let borrowed: Vec<&[u8]> = arguments.iter().map(Vec::as_slice).collect();
            self.record_slowlog(store, &borrowed, started);
        }
        Some(reply)
    }

    pub(super) fn waitaof(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 4 {
            arity("waitaof", output);
            return;
        }
        let parse = |raw: &[u8]| {
            std::str::from_utf8(raw)
                .ok()
                .and_then(|text| text.parse::<i64>().ok())
        };
        let (Some(numlocal), Some(numreplicas), Some(timeout)) = (
            parse(arguments[1]),
            parse(arguments[2]),
            parse(arguments[3]),
        ) else {
            output.extend_from_slice(b"-ERR value is not an integer or out of range\r\n");
            return;
        };
        if timeout < 0 {
            output.extend_from_slice(b"-ERR timeout can't be negative\r\n");
            return;
        }
        let (local, _) = match store.waitaof_probe(numlocal, numreplicas) {
            Ok(result) => result,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        if numreplicas == 0 {
            let _ = write!(output, "*2\r\n:{local}\r\n:0\r\n");
            return;
        }
        let deadline = if timeout == 0 {
            None
        } else {
            let millis = u64::try_from(timeout).unwrap_or(u64::MAX);
            Instant::now().checked_add(Duration::from_millis(millis))
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: Vec::new(),
            deadline,
            kind: BlockedKind::WaitAof { local },
        });
        self.sync_blocked(store);
    }
}
