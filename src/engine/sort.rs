//! Bounded SORT/SORT_RO over list, set and sorted-set source values.
//!
//! This subset supports ASC/DESC, ALPHA, LIMIT and STORE. The dynamic BY/GET
//! pattern lookups are intentionally rejected until their derived-key ACL and
//! lookup bounds have their own design; all reads and stored output remain
//! bounded by the configured collection and payload limits.

use super::collections::encode_list;
use super::data::Collection;
use super::options::NOT_INTEGER;
use super::{Store, arity, integer, internal_error, wrong_type};
use crate::protocol::encode_bulk;
use std::io::Write;

const NUMERIC_ERROR: &[u8] = b"-ERR One or more scores can't be converted into double\r\n";
const PATTERN_ERROR: &[u8] =
    b"-ERR BY and GET options are not supported by this bounded SORT subset\r\n";

struct Item {
    value: Vec<u8>,
    number: f64,
}

impl Store {
    pub(super) fn execute_sort(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let Some(command) = args.first() else {
            return false;
        };
        if !command
            .first()
            .is_some_and(|byte| byte.eq_ignore_ascii_case(&b's'))
        {
            return false;
        }
        let read_only = if command.eq_ignore_ascii_case(b"SORT_RO") {
            true
        } else if command.eq_ignore_ascii_case(b"SORT") {
            false
        } else {
            return false;
        };
        self.sort(args, read_only, out);
        true
    }

    fn sort(&self, args: &[&[u8]], read_only: bool, out: &mut Vec<u8>) {
        if args.len() < 2 {
            arity(if read_only { "sort_ro" } else { "sort" }, out);
            return;
        }
        let mut desc = false;
        let mut alpha = false;
        let mut limit = None;
        let mut store = None;
        let mut index = 2;
        while index < args.len() {
            let token = args[index];
            if token.eq_ignore_ascii_case(b"ASC") {
                desc = false;
                index += 1;
            } else if token.eq_ignore_ascii_case(b"DESC") {
                desc = true;
                index += 1;
            } else if token.eq_ignore_ascii_case(b"ALPHA") {
                alpha = true;
                index += 1;
            } else if token.eq_ignore_ascii_case(b"LIMIT") {
                if index + 2 >= args.len() {
                    out.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                let (Some(offset), Some(count)) =
                    (parse_i64(args[index + 1]), parse_i64(args[index + 2]))
                else {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                };
                limit = Some((offset, count));
                index += 3;
            } else if !read_only && token.eq_ignore_ascii_case(b"STORE") && index + 1 < args.len() {
                store = Some(args[index + 1]);
                index += 2;
            } else if token.eq_ignore_ascii_case(b"BY") || token.eq_ignore_ascii_case(b"GET") {
                out.extend_from_slice(PATTERN_ERROR);
                return;
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let values = match data.entries.get(args[1]) {
            None => None,
            Some(entry) => match entry.collection.as_ref().and_then(source_values) {
                Some(values) => Some(values),
                None => {
                    wrong_type(out);
                    return;
                }
            },
        };
        let Some(values) = values else {
            if let Some(destination) = store {
                data.expire_key(destination, now);
                data.remove(destination);
                integer(0, out);
            } else {
                out.extend_from_slice(b"*0\r\n");
            }
            return;
        };
        let mut items = Vec::with_capacity(values.len());
        for value in values {
            let number = if alpha {
                0.0
            } else {
                let Some(number) = sort_number(&value) else {
                    out.extend_from_slice(NUMERIC_ERROR);
                    return;
                };
                number
            };
            items.push(Item { value, number });
        }
        items.sort_by(|left, right| {
            let ordering = if alpha {
                left.value.cmp(&right.value)
            } else {
                left.number
                    .total_cmp(&right.number)
                    .then_with(|| left.value.cmp(&right.value))
            };
            if desc { ordering.reverse() } else { ordering }
        });
        let (offset, count) = limit.unwrap_or((0, -1));
        let offset = usize::try_from(offset.max(0)).unwrap_or(usize::MAX);
        if offset >= items.len() {
            items.clear();
        } else {
            items.drain(..offset);
            if count >= 0 {
                items.truncate(usize::try_from(count).unwrap_or(usize::MAX));
            }
        }
        let Some(destination) = store else {
            write!(out, "*{}\r\n", items.len()).expect("Vec write");
            for item in items {
                encode_bulk(Some(&item.value), out);
            }
            return;
        };
        data.expire_key(destination, now);
        if items.is_empty() {
            data.remove(destination);
            integer(0, out);
            return;
        }
        let count = items.len();
        let bytes = items.iter().map(|item| item.value.len()).sum::<usize>();
        let previous = data.entries.get(destination).map(|entry| entry.payload_len);
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_list(
            items.into_iter().map(|item| item.value).collect(),
            config.list_max_listpack_size,
        );
        let Some(payload) = data.reserve(destination, bytes, previous, self.max_payload_bytes, now)
        else {
            out.extend_from_slice(b"-OOM configured payload limit exceeded\r\n");
            return;
        };
        data.put_collection(destination, encoded, None, payload, now);
        integer(count as i64, out);
    }
}

fn source_values(collection: &Collection) -> Option<Vec<Vec<u8>>> {
    if collection.is_list() {
        return collection
            .list_items()
            .map(|values| values.into_iter().map(<[u8]>::to_vec).collect());
    }
    if collection.is_set() {
        return collection.set_items();
    }
    if collection.is_zset() {
        return collection.zset_items().map(|values| {
            values
                .into_iter()
                .map(|(member, _)| member.to_vec())
                .collect()
        });
    }
    None
}

fn sort_number(raw: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(raw).ok()?;
    let text = text.trim_start_matches(|character: char| character.is_ascii_whitespace());
    let value = text.parse::<f64>().ok()?;
    (!value.is_nan()).then_some(value)
}

fn parse_i64(raw: &[u8]) -> Option<i64> {
    let text = std::str::from_utf8(raw).ok()?;
    text.parse::<i64>().ok()
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
    fn sort_orders_lists_sets_and_sorted_sets_with_limits() {
        let store = Store::new(1024);
        run(&store, &[b"RPUSH", b"list", b"3", b"1", b"2"]);
        assert_eq!(
            run(&store, &[b"SORT", b"list"]),
            b"*3\r\n$1\r\n1\r\n$1\r\n2\r\n$1\r\n3\r\n"
        );
        assert_eq!(
            run(&store, &[b"SORT", b"list", b"DESC", b"LIMIT", b"0", b"2"]),
            b"*2\r\n$1\r\n3\r\n$1\r\n2\r\n"
        );
        run(&store, &[b"SADD", b"set", b"b", b"a", b"c"]);
        assert_eq!(
            run(&store, &[b"SORT", b"set", b"ALPHA"]),
            b"*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n"
        );
        run(
            &store,
            &[b"ZADD", b"zset", b"9", b"b", b"1", b"a", b"5", b"c"],
        );
        assert_eq!(
            run(&store, &[b"SORT_RO", b"zset", b"ALPHA", b"DESC"]),
            b"*3\r\n$1\r\nc\r\n$1\r\nb\r\n$1\r\na\r\n"
        );
    }

    #[test]
    fn sort_errors_stores_and_pattern_limits_are_bounded() {
        let store = Store::new(64);
        run(&store, &[b"RPUSH", b"list", b"bad", b"1"]);
        assert_eq!(run(&store, &[b"SORT", b"list"]), NUMERIC_ERROR);
        assert_eq!(
            run(&store, &[b"SORT", b"list", b"BY", b"weight_*"]),
            PATTERN_ERROR
        );
        assert!(run(&store, &[b"SORT_RO", b"list", b"STORE", b"out"]).starts_with(b"-ERR syntax"));
        run(&store, &[b"RPUSH", b"ok", b"2", b"1"]);
        assert_eq!(run(&store, &[b"SORT", b"ok", b"STORE", b"out"]), b":2\r\n");
        assert_eq!(
            run(&store, &[b"LRANGE", b"out", b"0", b"-1"]),
            b"*2\r\n$1\r\n1\r\n$1\r\n2\r\n"
        );
        run(&store, &[b"PEXPIRE", b"out", b"100000"]);
        assert_eq!(run(&store, &[b"SORT", b"ok", b"STORE", b"out"]), b":2\r\n");
        assert_eq!(run(&store, &[b"PTTL", b"out"]), b":-1\r\n");
        run(&store, &[b"RPUSH", b"empty", b"x"]);
        run(&store, &[b"DEL", b"empty"]);
        run(&store, &[b"SET", b"out", b"x"]);
        assert_eq!(
            run(&store, &[b"SORT", b"missing", b"STORE", b"out"]),
            b":0\r\n"
        );
        assert_eq!(run(&store, &[b"EXISTS", b"out"]), b":0\r\n");
        let small = Store::new(2);
        assert_eq!(run(&small, &[b"RPUSH", b"l", b"a"]), b":1\r\n");
        let out = run(&small, &[b"SORT", b"l", b"ALPHA", b"STORE", b"dest"]);
        assert!(out.starts_with(b"-OOM"), "{out:?}");
    }
}
