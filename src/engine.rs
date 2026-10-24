//! Atomic in-memory string commands, with a bounded key/value payload budget.

use std::collections::HashMap;
use std::io::Write;
use std::sync::RwLock;

use crate::protocol::encode_bulk;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Continue,
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetCondition {
    Always,
    IfMissing,
    IfPresent,
}

impl SetCondition {
    fn parse(options: &[&[u8]]) -> Option<Self> {
        let mut condition = Self::Always;
        for option in options {
            let next = if option.eq_ignore_ascii_case(b"NX") {
                Self::IfMissing
            } else if option.eq_ignore_ascii_case(b"XX") {
                Self::IfPresent
            } else {
                return None;
            };
            if condition != Self::Always && condition != next {
                return None;
            }
            condition = next;
        }
        Some(condition)
    }
}

#[derive(Default)]
struct Data {
    entries: HashMap<Vec<u8>, Vec<u8>>,
    payload_bytes: usize,
}

pub struct Store {
    data: RwLock<Data>,
    max_payload_bytes: usize,
}

impl Store {
    pub fn new(max_payload_bytes: usize) -> Self {
        Self {
            data: RwLock::new(Data::default()),
            max_payload_bytes,
        }
    }

    /// Append exactly one reply. Locks are never held during socket I/O.
    pub fn execute(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        let Some(command) = arguments.first() else {
            output.extend_from_slice(b"-ERR empty command\r\n");
            return Action::Continue;
        };
        if command.eq_ignore_ascii_case(b"PING") {
            match arguments.len() {
                1 => output.extend_from_slice(b"+PONG\r\n"),
                2 => encode_bulk(Some(arguments[1]), output),
                _ => arity("ping", output),
            }
        } else if command.eq_ignore_ascii_case(b"ECHO") {
            if arguments.len() == 2 {
                encode_bulk(Some(arguments[1]), output);
            } else {
                arity("echo", output);
            }
        } else if command.eq_ignore_ascii_case(b"QUIT") {
            if arguments.len() == 1 {
                output.extend_from_slice(b"+OK\r\n");
                return Action::Close;
            }
            arity("quit", output);
        } else if command.eq_ignore_ascii_case(b"SET") {
            if arguments.len() < 3 {
                arity("set", output);
            } else if let Some(condition) = SetCondition::parse(&arguments[3..]) {
                self.set(arguments[1], arguments[2], condition, output);
            } else {
                // Validate every option before testing the condition or mutating data.
                output.extend_from_slice(b"-ERR syntax error\r\n");
            }
        } else if command.eq_ignore_ascii_case(b"GET") {
            if arguments.len() != 2 {
                arity("get", output);
            } else if let Ok(data) = self.data.read() {
                encode_bulk(data.entries.get(arguments[1]).map(Vec::as_slice), output);
            } else {
                internal_error(output);
            }
        } else if command.eq_ignore_ascii_case(b"DEL") {
            if arguments.len() < 2 {
                arity("del", output);
            } else if let Ok(mut data) = self.data.write() {
                let mut removed = 0usize;
                for &key in &arguments[1..] {
                    if let Some((key, value)) = data.entries.remove_entry(key) {
                        data.payload_bytes -= key.len() + value.len();
                        removed += 1;
                    }
                }
                integer(removed, output);
            } else {
                internal_error(output);
            }
        } else if command.eq_ignore_ascii_case(b"EXISTS") {
            if arguments.len() < 2 {
                arity("exists", output);
            } else if let Ok(data) = self.data.read() {
                // Redis counts repeated existing arguments more than once.
                let count = arguments[1..]
                    .iter()
                    .filter(|&&key| data.entries.contains_key(key))
                    .count();
                integer(count, output);
            } else {
                internal_error(output);
            }
        } else {
            // Never reflect untrusted command bytes into a line-based error.
            output.extend_from_slice(b"-ERR unknown command\r\n");
        }
        Action::Continue
    }

    fn set(&self, key: &[u8], value: &[u8], condition: SetCondition, output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let existing = data.entries.get(key);
        let permitted = match condition {
            SetCondition::Always => true,
            SetCondition::IfMissing => existing.is_none(),
            SetCondition::IfPresent => existing.is_some(),
        };
        if !permitted {
            encode_bulk(None, output);
            return;
        }
        let old_size = existing.map_or(0, Vec::len);
        let key_size = if existing.is_some() { 0 } else { key.len() };
        let new_size = (data.payload_bytes - old_size)
            .checked_add(key_size)
            .and_then(|n| n.checked_add(value.len()));
        let Some(new_size) = new_size.filter(|&n| n <= self.max_payload_bytes) else {
            output.extend_from_slice(b"-OOM configured payload limit exceeded\r\n");
            return;
        };
        if let Some(existing) = data.entries.get_mut(key) {
            *existing = value.to_vec();
        } else {
            data.entries.insert(key.to_vec(), value.to_vec());
        }
        data.payload_bytes = new_size;
        output.extend_from_slice(b"+OK\r\n");
    }
}

fn arity(command: &str, output: &mut Vec<u8>) {
    write!(
        output,
        "-ERR wrong number of arguments for '{command}' command\r\n"
    )
    .expect("writing to a Vec cannot fail");
}

fn integer(value: usize, output: &mut Vec<u8>) {
    write!(output, ":{value}\r\n").expect("writing to a Vec cannot fail");
}

fn internal_error(output: &mut Vec<u8>) {
    output.extend_from_slice(b"-ERR internal store error\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execute(store: &Store, arguments: &[&[u8]]) -> Vec<u8> {
        let mut output = Vec::new();
        assert_eq!(store.execute(arguments, &mut output), Action::Continue);
        output
    }

    #[test]
    fn binary_values_overwrite_and_missing_values_are_distinct() {
        let store = Store::new(100);
        assert_eq!(execute(&store, &[b"gEt", b"\xff"]), b"$-1\r\n");
        assert_eq!(execute(&store, &[b"sEt", b"\xff", b"\0\r\n"]), b"+OK\r\n");
        assert_eq!(execute(&store, &[b"GET", b"\xff"]), b"$3\r\n\0\r\n\r\n");
        assert_eq!(execute(&store, &[b"SET", b"\xff", b""]), b"+OK\r\n");
        assert_eq!(execute(&store, &[b"GET", b"\xff"]), b"$0\r\n\r\n");
    }

    #[test]
    fn duplicates_match_redis_semantics() {
        let store = Store::new(100);
        execute(&store, &[b"SET", b"key", b"value"]);
        assert_eq!(
            execute(&store, &[b"EXISTS", b"key", b"key", b"missing"]),
            b":2\r\n"
        );
        assert_eq!(
            execute(&store, &[b"DEL", b"key", b"key", b"missing"]),
            b":1\r\n"
        );
        assert_eq!(execute(&store, &[b"EXISTS", b"key"]), b":0\r\n");
    }

    #[test]
    fn payload_budget_is_atomic_and_reclaimed_on_overwrite_and_delete() {
        let store = Store::new(5);
        assert_eq!(execute(&store, &[b"SET", b"k", b"1234"]), b"+OK\r\n");
        assert!(execute(&store, &[b"SET", b"k", b"12345"]).starts_with(b"-OOM"));
        assert_eq!(execute(&store, &[b"GET", b"k"]), b"$4\r\n1234\r\n");
        assert!(execute(&store, &[b"SET", b"x", b""]).starts_with(b"-OOM"));
        assert_eq!(execute(&store, &[b"SET", b"k", b"1"]), b"+OK\r\n");
        assert_eq!(execute(&store, &[b"SET", b"x", b"12"]), b"+OK\r\n");
        assert_eq!(execute(&store, &[b"DEL", b"k", b"x"]), b":2\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 0);
        assert_eq!(execute(&store, &[b"SET", b"new", b"12"]), b"+OK\r\n");
    }

    #[test]
    fn errors_do_not_mutate_data_or_inject_resp_frames() {
        let store = Store::new(100);
        assert_eq!(
            execute(&store, &[b"SET", b"k", b"v", b"EX", b"1"]),
            b"-ERR syntax error\r\n"
        );
        assert_eq!(execute(&store, &[b"GET", b"k"]), b"$-1\r\n");
        assert_eq!(
            execute(&store, &[b"\r\n+INJECTED\r\n"]),
            b"-ERR unknown command\r\n"
        );
        for (arguments, command) in [
            (vec![b"PING".as_slice(), b"a", b"b"], "ping"),
            (vec![b"ECHO".as_slice()], "echo"),
            (vec![b"GET".as_slice()], "get"),
            (vec![b"SET".as_slice(), b"key"], "set"),
            (vec![b"DEL".as_slice()], "del"),
            (vec![b"EXISTS".as_slice()], "exists"),
            (vec![b"QUIT".as_slice(), b"extra"], "quit"),
        ] {
            assert_eq!(
                execute(&store, &arguments),
                format!("-ERR wrong number of arguments for '{command}' command\r\n").as_bytes()
            );
        }
    }

    #[test]
    fn conditional_sets_distinguish_missing_existing_and_empty_values() {
        for key in [b"".as_slice(), b"\0\xff"] {
            let store = Store::new(100);
            assert_eq!(execute(&store, &[b"SET", key, b"no", b"XX"]), b"$-1\r\n");
            assert_eq!(execute(&store, &[b"GET", key]), b"$-1\r\n");
            assert_eq!(execute(&store, &[b"SET", key, b"", b"nX"]), b"+OK\r\n");
            assert_eq!(execute(&store, &[b"SET", key, b"no", b"NX"]), b"$-1\r\n");
            assert_eq!(execute(&store, &[b"GET", key]), b"$0\r\n\r\n");
            assert_eq!(
                execute(&store, &[b"SET", key, b"\0\xff", b"xX"]),
                b"+OK\r\n"
            );
            assert_eq!(execute(&store, &[b"GET", key]), b"$2\r\n\0\xff\r\n");
            assert_eq!(execute(&store, &[b"DEL", key]), b":1\r\n");
            assert_eq!(execute(&store, &[b"SET", key, b"no", b"XX"]), b"$-1\r\n");
            assert_eq!(execute(&store, &[b"SET", key, b"new", b"NX"]), b"+OK\r\n");
        }
    }

    #[test]
    fn identical_conditional_flags_may_repeat() {
        let store = Store::new(100);
        assert_eq!(
            execute(&store, &[b"SET", b"k", b"first", b"NX", b"nx"]),
            b"+OK\r\n"
        );
        assert_eq!(
            execute(&store, &[b"SET", b"k", b"no", b"nx", b"NX"]),
            b"$-1\r\n"
        );
        assert_eq!(
            execute(&store, &[b"SET", b"k", b"last", b"XX", b"xx"]),
            b"+OK\r\n"
        );
        assert_eq!(execute(&store, &[b"GET", b"k"]), b"$4\r\nlast\r\n");
    }

    #[test]
    fn all_set_options_are_validated_even_when_a_condition_would_reject() {
        let invalid: &[&[&[u8]]] = &[
            &[b"NX", b"XX"],
            &[b"XX", b"NX"],
            &[b"NX", b"nx", b"XX"],
            &[b"NX", b"GET"],
            &[b"XX", b"EX", b"1"],
            &[b""],
            &[b"NXX"],
            &[b"NX "],
            &[b"NX\0suffix"],
            &[b"XX\0suffix"],
            &[b"\xff"],
            &[b"PX", b"1"],
            &[b"KEEPTTL"],
        ];
        for present in [false, true] {
            let store = Store::new(100);
            if present {
                execute(&store, &[b"SET", b"k", b"old"]);
            }
            for flags in invalid {
                let mut arguments: Vec<&[u8]> = vec![b"SET", b"k", b"new"];
                arguments.extend_from_slice(flags);
                assert_eq!(
                    execute(&store, &arguments),
                    b"-ERR syntax error\r\n",
                    "{flags:?}"
                );
                assert_eq!(
                    execute(&store, &[b"GET", b"k"]),
                    if present {
                        b"$3\r\nold\r\n".as_slice()
                    } else {
                        b"$-1\r\n"
                    }
                );
            }
        }
    }

    #[test]
    fn conditional_rejection_does_not_consume_payload_budget() {
        let store = Store::new(5);
        assert_eq!(execute(&store, &[b"SET", b"k", b"1234", b"NX"]), b"+OK\r\n");
        for arguments in [
            [b"SET".as_slice(), b"k", b"123456", b"NX"],
            [b"SET".as_slice(), b"missing", b"123456", b"XX"],
        ] {
            assert_eq!(execute(&store, &arguments), b"$-1\r\n");
            assert_eq!(store.data.read().unwrap().payload_bytes, 5);
        }
        assert!(execute(&store, &[b"SET", b"k", b"12345", b"XX"]).starts_with(b"-OOM"));
        assert_eq!(execute(&store, &[b"GET", b"k"]), b"$4\r\n1234\r\n");
        assert_eq!(execute(&store, &[b"SET", b"k", b"1", b"XX"]), b"+OK\r\n");
        assert_eq!(execute(&store, &[b"SET", b"x", b"12", b"NX"]), b"+OK\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 5);
        assert_eq!(execute(&store, &[b"DEL", b"k", b"x"]), b":2\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 0);
    }

    #[test]
    fn ping_echo_and_quit() {
        let store = Store::new(100);
        assert_eq!(execute(&store, &[b"PING"]), b"+PONG\r\n");
        assert_eq!(execute(&store, &[b"PING", b""]), b"$0\r\n\r\n");
        assert_eq!(execute(&store, &[b"ECHO", b"a\0b"]), b"$3\r\na\0b\r\n");
        let mut output = Vec::new();
        assert_eq!(store.execute(&[b"quit"], &mut output), Action::Close);
        assert_eq!(output, b"+OK\r\n");
    }
}
