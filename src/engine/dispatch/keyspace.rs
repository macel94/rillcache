//! Key lifecycle, expiry and keyspace-wide commands.

use super::super::{Action, Store, arity, integer, internal_error};

pub(super) fn execute(
    store: &Store,
    command: &[u8],
    arguments: &[&[u8]],
    output: &mut Vec<u8>,
) -> Option<Action> {
    if command.eq_ignore_ascii_case(b"DELEX") {
        store.delex(arguments, false, output);
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"DELIFEQ") {
        store.delex(arguments, true, output);
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"DIGEST") {
        if arguments.len() == 2 {
            store.digest(arguments[1], output);
        } else {
            arity("digest", output);
        }
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"DEL") || command.eq_ignore_ascii_case(b"UNLINK") {
        if arguments.len() < 2 {
            arity(
                if command.eq_ignore_ascii_case(b"UNLINK") {
                    "unlink"
                } else {
                    "del"
                },
                output,
            );
        } else if let Ok(mut data) = store.data.write() {
            let now = store.now(&data);
            let mut count = 0;
            for key in &arguments[1..] {
                data.expire_key(key, now);
                count += i64::from(data.remove(key).is_some());
            }
            integer(count, output);
        } else {
            internal_error(output);
        }
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"EXISTS") {
        if arguments.len() < 2 {
            arity("exists", output);
        } else {
            store.exists(&arguments[1..], output);
        }
        return Some(Action::Continue);
    }
    if command.eq_ignore_ascii_case(b"PERSIST") {
        if arguments.len() != 2 {
            arity("persist", output);
        } else {
            store.persist(arguments[1], output);
        }
        return Some(Action::Continue);
    }
    if let Some((_, name, ms, absolute)) = [
        (b"TTL".as_slice(), "ttl", false, false),
        (b"PTTL".as_slice(), "pttl", true, false),
        (b"EXPIRETIME".as_slice(), "expiretime", false, true),
        (b"PEXPIRETIME".as_slice(), "pexpiretime", true, true),
    ]
    .into_iter()
    .find(|(token, _, _, _)| command.eq_ignore_ascii_case(token))
    {
        if arguments.len() != 2 {
            arity(name, output);
        } else {
            store.ttl(arguments[1], ms, absolute, output);
        }
        return Some(Action::Continue);
    }
    if let Some((_, name, seconds, absolute)) = [
        (b"EXPIRE".as_slice(), "expire", true, false),
        (b"PEXPIRE".as_slice(), "pexpire", false, false),
        (b"EXPIREAT".as_slice(), "expireat", true, true),
        (b"PEXPIREAT".as_slice(), "pexpireat", false, true),
    ]
    .into_iter()
    .find(|(token, _, _, _)| command.eq_ignore_ascii_case(token))
    {
        store.expire(arguments, name, seconds, absolute, output);
        return Some(Action::Continue);
    }
    if store.execute_keyspace(arguments, output) {
        return Some(Action::Continue);
    }
    None
}
