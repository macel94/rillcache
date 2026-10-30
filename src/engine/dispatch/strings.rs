//! String commands, including string-encoded bitfields and HyperLogLogs.
//!
//! GET, SET and ECHO stay at the front of this entry point so the hottest
//! Redis commands retain their short direct path.

use super::super::options::{Condition, ExpiryKind, SetOptions};
use super::super::{Action, ShardExecutor, arity};
use crate::protocol::encode_bulk;

#[inline(always)]
pub(super) fn execute_data(
    store: &ShardExecutor,
    command: &[u8],
    arguments: &[&[u8]],
    output: &mut Vec<u8>,
) -> Option<Action> {
    if command.eq_ignore_ascii_case(b"GET") {
        if arguments.len() != 2 {
            arity("get", output);
        } else {
            store.get(arguments[1], output);
        }
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"SET") {
        if arguments.len() < 3 {
            arity("set", output);
        } else {
            match SetOptions::parse(&arguments[3..], false) {
                Ok(options) => store.set(arguments[1], arguments[2], options, "set", false, output),
                Err(error) => output.extend_from_slice(error),
            }
        }
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"ECHO") {
        if arguments.len() == 2 {
            encode_bulk(Some(arguments[1]), output);
        } else {
            arity("echo", output);
        }
        return Some(Action::Continue);
    }
    None
}

/// Every remaining string-family command. Deliberately not `#[inline]` so the
/// large match does not bloat the GET / SET / ECHO path.
pub(super) fn execute_other(
    store: &ShardExecutor,
    command: &[u8],
    arguments: &[&[u8]],
    output: &mut Vec<u8>,
) -> Option<Action> {
    if command.eq_ignore_ascii_case(b"GETEX") {
        if arguments.len() < 2 {
            arity("getex", output);
        } else {
            match SetOptions::parse(&arguments[2..], true) {
                Ok(options) => store.getex(arguments[1], options, output),
                Err(error) => output.extend_from_slice(error),
            }
        }
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"GETDEL") {
        if arguments.len() != 2 {
            arity("getdel", output);
        } else {
            store.getdel(arguments[1], output);
        }
        return Some(Action::Continue);
    }
    if let Some((_, name, kind)) = [
        (b"SETEX".as_slice(), "setex", ExpiryKind::Ex),
        (b"PSETEX".as_slice(), "psetex", ExpiryKind::Px),
    ]
    .into_iter()
    .find(|(token, _, _)| command.eq_ignore_ascii_case(token))
    {
        if arguments.len() != 4 {
            arity(name, output);
        } else {
            store.set(
                arguments[1],
                arguments[3],
                SetOptions {
                    expiry: Some((kind, arguments[2])),
                    ..SetOptions::default()
                },
                name,
                false,
                output,
            );
        }
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"SETNX") || command.eq_ignore_ascii_case(b"GETSET") {
        let nx = command.eq_ignore_ascii_case(b"SETNX");
        let name = if nx { "setnx" } else { "getset" };
        if arguments.len() != 3 {
            arity(name, output);
        } else {
            store.set(
                arguments[1],
                arguments[2],
                SetOptions {
                    condition: if nx {
                        Condition::Missing
                    } else {
                        Condition::Always
                    },
                    get: !nx,
                    ..SetOptions::default()
                },
                name,
                nx,
                output,
            );
        }
        return Some(Action::Continue);
    }
    if let Some((_, name)) = [
        (b"INCR".as_slice(), "incr"),
        (b"DECR".as_slice(), "decr"),
        (b"INCRBY".as_slice(), "incrby"),
        (b"DECRBY".as_slice(), "decrby"),
    ]
    .into_iter()
    .find(|(token, _)| command.eq_ignore_ascii_case(token))
    {
        store.increment(arguments, name, output);
        return Some(Action::Continue);
    }
    if store.execute_string_ranges(arguments, output)
        || store.execute_bitfield(arguments, output)
        || store.execute_string_algorithms(arguments, output)
        || store.execute_hll(arguments, output)
        || store.execute_multikey(arguments, output)
    {
        return Some(Action::Continue);
    }
    None
}
