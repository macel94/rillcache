//! Bounded runtime clock and object introspection.
//!
//! TIME exposes the wall clock. OBJECT exposes the actual storage encoding
//! selected for a key, a refcount of one for the server's owned values, and
//! explicit unsupported errors for idle-time/LFU frequency rather than invented
//! metrics.

use super::data::Collection;
use super::{Store, arity, internal_error};
use crate::protocol::encode_bulk;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

const IDLETIME_ERROR: &[u8] =
    b"-ERR OBJECT IDLETIME is not supported by this bounded server: read-idle time is not tracked\r\n";
const FREQ_ERROR: &[u8] = b"-ERR An LFU maxmemory policy is not selected, access frequency not tracked. Please note that when switching between maxmemory policies at runtime LFU and LRU data will take some time to adjust.\r\n";

const OBJECT_HELP: &[&[u8]] = &[
    b"ENCODING <key>",
    b"    Return the kind of internal representation used in order to store the value",
    b"    associated with a <key>.",
    b"FREQ <key>",
    b"    Return the access frequency index of the <key>. This is not tracked without",
    b"    an LFU maxmemory policy.",
    b"IDLETIME <key>",
    b"    Idle time is not tracked by this bounded server; the command returns an",
    b"    explicit unsupported error.",
    b"REFCOUNT <key>",
    b"    Return the number of references of the value associated with the specified",
    b"    <key>.",
    b"HELP",
    b"    Print this help.",
];

impl Store {
    pub(super) fn execute_introspection(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let Some(command) = args.first() else {
            return false;
        };
        if command.eq_ignore_ascii_case(b"TIME") {
            if args.len() != 1 {
                arity("time", out);
            } else {
                self.time(out);
            }
        } else if command.eq_ignore_ascii_case(b"OBJECT") {
            self.object(args, out);
        } else {
            return false;
        }
        true
    }

    fn time(&self, out: &mut Vec<u8>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        write!(out, "*2\r\n").expect("Vec write");
        encode_bulk(Some(now.as_secs().to_string().as_bytes()), out);
        encode_bulk(Some(now.subsec_micros().to_string().as_bytes()), out);
    }

    fn object(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if args.len() < 2 {
            arity("object", out);
            return;
        }
        let subcommand = args[1];
        if subcommand.eq_ignore_ascii_case(b"HELP") {
            if args.len() != 2 {
                object_syntax_error(subcommand, out);
                return;
            }
            write!(out, "*{}\r\n", OBJECT_HELP.len()).expect("Vec write");
            for line in OBJECT_HELP {
                encode_bulk(Some(line), out);
            }
            return;
        }
        if args.len() != 3 {
            object_syntax_error(subcommand, out);
            return;
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[2], now);
        let Some(entry) = data.entries.get(args[2]) else {
            encode_bulk(None, out);
            return;
        };
        if subcommand.eq_ignore_ascii_case(b"ENCODING") {
            encode_bulk(Some(encoding(entry)), out);
        } else if subcommand.eq_ignore_ascii_case(b"REFCOUNT") {
            super::integer(1, out);
        } else if subcommand.eq_ignore_ascii_case(b"IDLETIME") {
            out.extend_from_slice(IDLETIME_ERROR);
        } else if subcommand.eq_ignore_ascii_case(b"FREQ") {
            out.extend_from_slice(FREQ_ERROR);
        } else {
            object_syntax_error(subcommand, out);
        }
    }
}

fn encoding(entry: &super::data::Entry) -> &'static [u8] {
    let Some(collection) = entry.collection.as_ref() else {
        return if entry.stream.is_some() {
            b"stream"
        } else {
            b"raw"
        };
    };
    match collection {
        Collection::ListPack(_) => b"listpack",
        Collection::ListQuick(_) => b"quicklist",
        Collection::Set(_) => b"hashtable",
        Collection::SetPack(_) => b"listpack",
        Collection::SetInt(_) => b"intset",
        Collection::Hash(_) => b"hashtable",
        Collection::HashPack(_) => b"listpack",
        Collection::ZSet(_) => b"skiplist",
        Collection::ZSetPack(_) => b"listpack",
    }
}

fn object_syntax_error(subcommand: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(b"-ERR unknown subcommand or wrong number of arguments for '");
    out.extend_from_slice(subcommand);
    out.extend_from_slice(b"'. Try OBJECT HELP.\r\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Action;

    fn run(store: &Store, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        assert_eq!(store.execute(args, &mut out), Action::Continue);
        out
    }

    #[test]
    fn time_and_object_encodings_are_truthful() {
        let store = Store::new(1024);
        let time = run(&store, &[b"TIME"]);
        assert!(time.starts_with(b"*2\r\n$"));
        run(&store, &[b"SET", b"s", b"v"]);
        assert_eq!(
            run(&store, &[b"OBJECT", b"ENCODING", b"s"]),
            b"$3\r\nraw\r\n"
        );
        assert_eq!(run(&store, &[b"OBJECT", b"REFCOUNT", b"s"]), b":1\r\n");
        assert_eq!(
            run(&store, &[b"OBJECT", b"ENCODING", b"missing"]),
            b"$-1\r\n"
        );
        run(&store, &[b"RPUSH", b"l", b"a"]);
        assert_eq!(
            run(&store, &[b"OBJECT", b"ENCODING", b"l"]),
            b"$8\r\nlistpack\r\n"
        );
        run(&store, &[b"SADD", b"i", b"1", b"2"]);
        assert_eq!(
            run(&store, &[b"OBJECT", b"ENCODING", b"i"]),
            b"$6\r\nintset\r\n"
        );
        run(&store, &[b"XADD", b"x", b"1-0", b"f", b"v"]);
        assert_eq!(
            run(&store, &[b"OBJECT", b"ENCODING", b"x"]),
            b"$6\r\nstream\r\n"
        );
        assert_eq!(run(&store, &[b"OBJECT", b"FREQ", b"s"]), FREQ_ERROR);
        assert_eq!(run(&store, &[b"OBJECT", b"IDLETIME", b"s"]), IDLETIME_ERROR);
    }
}
