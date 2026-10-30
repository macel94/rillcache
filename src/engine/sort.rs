//! Bounded SORT/SORT_RO over list, set and sorted-set source values.
//!
//! Supports ASC/DESC, ALPHA, LIMIT, STORE, dynamic BY weights and GET output
//! patterns. Derived BY/GET keys are readable only by users with all-key ACL
//! access, matching Redis's dynamic-key rule; every lookup and output remains
//! bounded by the source collection, request and payload limits.

use super::collections::encode_list;
use super::data::{Collection, Data};
use super::options::NOT_INTEGER;
use super::{ShardExecutor, arity, integer, internal_error, wrong_type};
use crate::protocol::encode_bulk;
use std::io::Write;

const NUMERIC_ERROR: &[u8] = b"-ERR One or more scores can't be converted into double\r\n";

struct Item {
    value: Vec<u8>,
    number: f64,
    compare: Vec<u8>,
}

impl ShardExecutor {
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
        let mut by = None;
        let mut gets = Vec::new();
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
            } else if token.eq_ignore_ascii_case(b"BY") {
                if index + 1 >= args.len() {
                    out.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                by = Some(args[index + 1]);
                index += 2;
            } else if token.eq_ignore_ascii_case(b"GET") {
                if index + 1 >= args.len() {
                    out.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                gets.push(args[index + 1]);
                index += 2;
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        let Ok(mut data) = self.data.try_borrow_mut() else {
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
        let dontsort = by.is_some_and(|pattern| !pattern.contains(&b'*'));
        let mut items = Vec::with_capacity(values.len());
        for value in values {
            let weight = if dontsort {
                None
            } else if let Some(pattern) = by {
                lookup_pattern(&mut data, pattern, &value, now)
            } else {
                Some(value.clone())
            };
            let (number, compare) = if dontsort {
                (0.0, Vec::new())
            } else if alpha {
                (0.0, weight.unwrap_or_default())
            } else {
                match weight {
                    None => (0.0, Vec::new()),
                    Some(weight) => {
                        let Some(number) = sort_number(&weight) else {
                            out.extend_from_slice(NUMERIC_ERROR);
                            return;
                        };
                        (number, Vec::new())
                    }
                }
            };
            items.push(Item {
                value,
                number,
                compare,
            });
        }
        if dontsort {
            if desc {
                items.reverse();
            }
        } else {
            items.sort_by(|left, right| {
                let ordering = if alpha {
                    left.compare
                        .cmp(&right.compare)
                        .then_with(|| left.value.cmp(&right.value))
                } else {
                    left.number
                        .total_cmp(&right.number)
                        .then_with(|| left.value.cmp(&right.value))
                };
                if desc { ordering.reverse() } else { ordering }
            });
        }
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
        let values: Vec<Vec<u8>> = items.into_iter().map(|item| item.value).collect();
        if gets.is_empty() {
            if let Some(destination) = store {
                self.store_sorted(&mut data, destination, values, now, out);
            } else {
                write!(out, "*{}\r\n", values.len()).expect("Vec write");
                for value in values {
                    encode_bulk(Some(&value), out);
                }
            }
            return;
        }
        let mut results = Vec::with_capacity(values.len() * gets.len());
        for value in &values {
            for pattern in &gets {
                if *pattern == b"#" {
                    results.push(Some(value.clone()));
                } else {
                    results.push(lookup_pattern(&mut data, pattern, value, now));
                }
            }
        }
        if let Some(destination) = store {
            let values = results
                .into_iter()
                .map(|value| value.unwrap_or_default())
                .collect();
            self.store_sorted(&mut data, destination, values, now, out);
            return;
        }
        write!(out, "*{}\r\n", results.len()).expect("Vec write");
        for result in results {
            encode_bulk(result.as_deref(), out);
        }
    }

    fn store_sorted(
        &self,
        data: &mut Data,
        destination: &[u8],
        values: Vec<Vec<u8>>,
        now: i64,
        out: &mut Vec<u8>,
    ) {
        data.expire_key(destination, now);
        if values.is_empty() {
            data.remove(destination);
            integer(0, out);
            return;
        }
        let count = values.len();
        let bytes = values.iter().map(Vec::len).sum::<usize>();
        let previous = data.entries.get(destination).map(|entry| entry.payload_len);
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_list(values.into_iter().collect(), config.list_max_listpack_size);
        let Some(payload) = data.reserve(destination, bytes, previous, self.payload_limit(), now)
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
        let mut values = collection.set_items()?;
        values.sort_unstable();
        return Some(values);
    }
    if collection.is_zset() {
        let mut values = collection.zset_items()?;
        values.sort_unstable_by(|(member_a, score_a), (member_b, score_b)| {
            score_a
                .total_cmp(score_b)
                .then_with(|| member_a.cmp(member_b))
        });
        return Some(
            values
                .into_iter()
                .map(|(member, _)| member.to_vec())
                .collect(),
        );
    }
    None
}

fn lookup_pattern(data: &mut Data, pattern: &[u8], element: &[u8], now: i64) -> Option<Vec<u8>> {
    if pattern == b"#" {
        return Some(element.to_vec());
    }
    let star = pattern.iter().position(|byte| *byte == b'*')?;
    let field_at = pattern[star + 1..]
        .windows(2)
        .position(|window| window == b"->")
        .map(|position| star + 1 + position)
        .filter(|position| position + 2 < pattern.len());
    let key_end = field_at.unwrap_or(pattern.len());
    let mut key = Vec::with_capacity(star + element.len() + key_end - star - 1);
    key.extend_from_slice(&pattern[..star]);
    key.extend_from_slice(element);
    key.extend_from_slice(&pattern[star + 1..key_end]);
    data.expire_key(&key, now);
    let entry = data.entries.get(&key)?;
    if let Some(field_at) = field_at {
        entry
            .collection
            .as_ref()
            .and_then(|collection| collection.hash_get(&pattern[field_at + 2..]))
            .map(<[u8]>::to_vec)
    } else {
        entry.is_string().then(|| entry.value.clone())
    }
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

    fn run(store: &ShardExecutor, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        assert_eq!(store.execute(args, &mut out), Action::Continue);
        out
    }

    #[test]
    fn sort_orders_lists_sets_and_sorted_sets_with_limits() {
        let store = ShardExecutor::new(1024);
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
        let store = ShardExecutor::new(64);
        run(&store, &[b"RPUSH", b"list", b"bad", b"1"]);
        assert_eq!(run(&store, &[b"SORT", b"list"]), NUMERIC_ERROR);
        run(&store, &[b"RPUSH", b"nums", b"2", b"1"]);
        run(&store, &[b"SET", b"w_1", b"10"]);
        run(&store, &[b"SET", b"w_2", b"5"]);
        assert_eq!(
            run(&store, &[b"SORT", b"nums", b"BY", b"w_*"]),
            b"*2\r\n$1\r\n2\r\n$1\r\n1\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"SORT", b"nums", b"BY", b"w_*", b"GET", b"#", b"GET", b"w_*"
                ]
            ),
            b"*4\r\n$1\r\n2\r\n$1\r\n5\r\n$1\r\n1\r\n$2\r\n10\r\n"
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
        let small = ShardExecutor::new(2);
        assert_eq!(run(&small, &[b"RPUSH", b"l", b"a"]), b":1\r\n");
        let out = run(&small, &[b"SORT", b"l", b"ALPHA", b"STORE", b"dest"]);
        assert!(out.starts_with(b"-OOM"), "{out:?}");
    }
}
