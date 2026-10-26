//! Binary-safe typed collection commands. Collection values remain in the same
//! entry as their key and expiry; mutations stage a new value before admission.
use super::data::{Collection, Entry};
use super::options::{NOT_INTEGER, OOM, parse_integer};
use super::{Store, arity, integer, internal_error, wrong_type};
use crate::protocol::encode_bulk;
use std::collections::{BTreeSet, HashMap};
use std::io::Write;

const MAX_ELEMENTS: usize = 1_000_000;

impl Store {
    pub(super) fn execute_collections(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let Some(cmd) = args.first() else {
            return false;
        };
        if cmd.eq_ignore_ascii_case(b"SADD") {
            if args.len() < 3 {
                arity("sadd", out);
            } else {
                self.set_add(args, out);
            }
            return true;
        }
        if cmd.eq_ignore_ascii_case(b"SMEMBERS")
            || cmd.eq_ignore_ascii_case(b"SCARD")
            || cmd.eq_ignore_ascii_case(b"SISMEMBER")
        {
            let name = if cmd.eq_ignore_ascii_case(b"SMEMBERS") {
                "smembers"
            } else if cmd.eq_ignore_ascii_case(b"SCARD") {
                "scard"
            } else {
                "sismember"
            };
            let valid = if name == "sismember" {
                args.len() == 3
            } else {
                args.len() == 2
            };
            if !valid {
                arity(name, out);
            } else {
                self.set_read(args, name, out);
            }
            return true;
        }
        if cmd.eq_ignore_ascii_case(b"HSET") {
            if args.len() < 4 || !(args.len() - 2).is_multiple_of(2) {
                arity("hset", out);
            } else {
                self.hash_set(args, out);
            }
            return true;
        }
        if cmd.eq_ignore_ascii_case(b"HGET")
            || cmd.eq_ignore_ascii_case(b"HGETALL")
            || cmd.eq_ignore_ascii_case(b"HLEN")
        {
            let name = if cmd.eq_ignore_ascii_case(b"HGET") {
                "hget"
            } else if cmd.eq_ignore_ascii_case(b"HGETALL") {
                "hgetall"
            } else {
                "hlen"
            };
            let valid = if name == "hget" {
                args.len() == 3
            } else {
                args.len() == 2
            };
            if !valid {
                arity(name, out);
            } else {
                self.hash_read(args, name, out);
            }
            return true;
        }
        if cmd.eq_ignore_ascii_case(b"ZADD") {
            if args.len() < 4 || !(args.len() - 2).is_multiple_of(2) {
                arity("zadd", out);
            } else {
                self.zset_add(args, out);
            }
            return true;
        }
        if cmd.eq_ignore_ascii_case(b"ZSCORE")
            || cmd.eq_ignore_ascii_case(b"ZCARD")
            || cmd.eq_ignore_ascii_case(b"ZRANGE")
        {
            let name = if cmd.eq_ignore_ascii_case(b"zscore") {
                "zscore"
            } else if cmd.eq_ignore_ascii_case(b"zcard") {
                "zcard"
            } else {
                "zrange"
            };
            if (name == "zscore" && args.len() != 3)
                || (name == "zcard" && args.len() != 2)
                || (name == "zrange" && !(args.len() == 4 || args.len() == 5))
            {
                arity(name, out);
            } else {
                self.zset_read(args, name, out);
            }
            return true;
        }
        let list_command = [
            b"LPUSH".as_slice(),
            b"RPUSH",
            b"LPOP",
            b"RPOP",
            b"LLEN",
            b"LRANGE",
        ]
        .iter()
        .any(|name| cmd.eq_ignore_ascii_case(name));
        if !list_command {
            return false;
        }
        if cmd.eq_ignore_ascii_case(b"LPUSH") || cmd.eq_ignore_ascii_case(b"RPUSH") {
            let name = if cmd.eq_ignore_ascii_case(b"LPUSH") {
                "lpush"
            } else {
                "rpush"
            };
            if args.len() < 3 {
                arity(name, out);
            } else {
                self.push_list(args, name == "lpush", out);
            }
        } else if cmd.eq_ignore_ascii_case(b"LPOP") || cmd.eq_ignore_ascii_case(b"RPOP") {
            let left = cmd.eq_ignore_ascii_case(b"LPOP");
            let name = if left { "lpop" } else { "rpop" };
            if !(args.len() == 2 || args.len() == 3) {
                arity(name, out);
            } else {
                self.pop_list(args, left, out);
            }
        } else if cmd.eq_ignore_ascii_case(b"LLEN") {
            if args.len() != 2 {
                arity("llen", out);
            } else {
                self.list_len(args[1], out);
            }
        } else {
            if args.len() != 4 {
                arity("lrange", out);
            } else {
                self.list_range(args[1], args[2], args[3], out);
            }
        }
        true
    }

    fn set_add(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let current = data.entries.get(key);
        if current.is_some_and(|entry| !entry.collection.as_ref().is_some_and(Collection::is_set)) {
            wrong_type(out);
            return;
        }
        let mut set: BTreeSet<Vec<u8>> = current
            .and_then(|entry| entry.collection.as_ref().and_then(Collection::set_items))
            .unwrap_or_default()
            .into_iter()
            .collect();
        let before = set.len();
        for member in &args[2..] {
            set.insert(member.to_vec());
        }
        if set.len() > MAX_ELEMENTS {
            out.extend_from_slice(OOM);
            return;
        }
        let added = set.len() - before;
        let expires = current.and_then(|entry| entry.expires);
        let bytes = set.iter().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let compact_ints: Option<BTreeSet<i64>> =
            set.iter().map(|member| parse_integer(member)).collect();
        let encoded = if let Some(values) =
            compact_ints.filter(|values| values.len() <= config.set_max_intset_entries)
        {
            Collection::SetInt(values)
        } else if set.len() <= config.set_max_listpack_entries
            && set
                .iter()
                .all(|member| member.len() <= config.set_max_listpack_value)
        {
            Collection::SetPack(set.into_iter().collect())
        } else {
            Collection::Set(set)
        };
        let previous = current.map(Entry::payload_len);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload);
        integer(added as i64, out);
    }

    fn set_read(&self, args: &[&[u8]], name: &str, out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let collection = match data.entries.get(args[1]) {
            None => {
                if name == "smembers" {
                    out.extend_from_slice(b"*0\r\n");
                } else {
                    integer(0, out);
                }
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_set) => {
                entry.collection.as_ref().expect("set type checked")
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let members = collection.set_items().expect("set type checked");
        match name {
            "scard" => integer(members.len() as i64, out),
            "sismember" => integer(
                i64::from(members.iter().any(|member| member.as_slice() == args[2])),
                out,
            ),
            _ => {
                write!(out, "*{}\r\n", members.len()).expect("Vec write");
                for member in &members {
                    encode_bulk(Some(member), out);
                }
            }
        }
    }

    fn hash_set(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let current = data.entries.get(key);
        if current.is_some_and(|entry| !entry.collection.as_ref().is_some_and(Collection::is_hash))
        {
            wrong_type(out);
            return;
        }
        let mut hash: HashMap<Vec<u8>, Vec<u8>> = current
            .and_then(|entry| entry.collection.as_ref().and_then(Collection::hash_items))
            .unwrap_or_default()
            .into_iter()
            .map(|(field, value)| (field.to_vec(), value.to_vec()))
            .collect();
        let mut added = 0;
        for pair in args[2..].as_chunks::<2>().0 {
            added += i64::from(hash.insert(pair[0].to_vec(), pair[1].to_vec()).is_none());
        }
        if hash.len() > MAX_ELEMENTS {
            out.extend_from_slice(OOM);
            return;
        }
        let expires = current.and_then(|entry| entry.expires);
        let bytes = hash
            .iter()
            .map(|(field, value)| field.len() + value.len())
            .sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let compact = hash.len() <= config.hash_max_listpack_entries
            && hash.iter().all(|(field, value)| {
                field.len() <= config.hash_max_listpack_value
                    && value.len() <= config.hash_max_listpack_value
            });
        let encoded = if compact {
            let mut entries: Vec<_> = hash.into_iter().collect();
            entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            Collection::HashPack(entries)
        } else {
            Collection::Hash(hash)
        };
        let previous = current.map(Entry::payload_len);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload);
        integer(added, out);
    }

    fn hash_read(&self, args: &[&[u8]], name: &str, out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let collection = match data.entries.get(args[1]) {
            None => {
                if name == "hget" {
                    encode_bulk(None, out);
                } else if name == "hgetall" {
                    out.extend_from_slice(b"*0\r\n");
                } else {
                    integer(0, out);
                }
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_hash) => {
                entry.collection.as_ref().expect("hash type checked")
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        match name {
            "hget" => encode_bulk(collection.hash_get(args[2]), out),
            "hlen" => integer(
                collection.hash_items().expect("hash type checked").len() as i64,
                out,
            ),
            _ => {
                let mut fields = collection.hash_items().expect("hash type checked");
                fields.sort_unstable_by(|a, b| a.0.cmp(b.0));
                write!(out, "*{}\r\n", fields.len() * 2).expect("Vec write");
                for (field, value) in fields {
                    encode_bulk(Some(field), out);
                    encode_bulk(Some(value), out);
                }
            }
        }
    }

    fn zset_add(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let mut members = Vec::with_capacity((args.len() - 2) / 2);
        for pair in args[2..].as_chunks::<2>().0 {
            let Some(score) = parse_score(pair[0]) else {
                out.extend_from_slice(b"-ERR value is not a valid float\r\n");
                return;
            };
            members.push((score, pair[1]));
        }
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let current = data.entries.get(key);
        if current.is_some_and(|entry| !entry.collection.as_ref().is_some_and(Collection::is_zset))
        {
            wrong_type(out);
            return;
        }
        let mut zset: HashMap<Vec<u8>, f64> = current
            .and_then(|entry| entry.collection.as_ref().and_then(Collection::zset_items))
            .unwrap_or_default()
            .into_iter()
            .map(|(member, score)| (member.to_vec(), score))
            .collect();
        let mut added = 0;
        for (score, member) in members {
            added += i64::from(zset.insert(member.to_vec(), score).is_none());
        }
        if zset.len() > MAX_ELEMENTS {
            out.extend_from_slice(OOM);
            return;
        }
        let expires = current.and_then(|entry| entry.expires);
        let bytes = zset.keys().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let compact = zset.len() <= config.zset_max_listpack_entries
            && zset
                .keys()
                .all(|member| member.len() <= config.zset_max_listpack_value);
        let encoded = if compact {
            let mut entries: Vec<_> = zset.into_iter().collect();
            entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            Collection::ZSetPack(entries)
        } else {
            Collection::ZSet(zset)
        };
        let previous = current.map(Entry::payload_len);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload);
        integer(added, out);
    }

    fn zset_read(&self, args: &[&[u8]], name: &str, out: &mut Vec<u8>) {
        let (start, stop, with_scores) = if name == "zrange" {
            let (Some(start), Some(stop)) = (parse_count(args[2]), parse_count(args[3])) else {
                out.extend_from_slice(NOT_INTEGER);
                return;
            };
            if args.len() == 5 && !args[4].eq_ignore_ascii_case(b"WITHSCORES") {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
            (start, stop, args.len() == 5)
        } else {
            (0, -1, false)
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let collection = match data.entries.get(args[1]) {
            None => {
                if name == "zscore" {
                    encode_bulk(None, out);
                } else if name == "zcard" {
                    integer(0, out);
                } else {
                    out.extend_from_slice(b"*0\r\n");
                }
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_zset) => {
                entry.collection.as_ref().expect("zset type checked")
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let members = collection.zset_items().expect("zset type checked");
        if name == "zcard" {
            integer(members.len() as i64, out);
            return;
        }
        if name == "zscore" {
            let score = members
                .iter()
                .find(|(member, _)| *member == args[2])
                .map(|(_, score)| score.to_string());
            encode_bulk(score.as_deref().map(str::as_bytes), out);
            return;
        }
        let mut ordered = members;
        ordered.sort_unstable_by(|(member_a, score_a), (member_b, score_b)| {
            score_a
                .total_cmp(score_b)
                .then_with(|| member_a.cmp(member_b))
        });
        let len = ordered.len() as i128;
        let normalize = |i: i64| {
            if i < 0 {
                (len + i as i128).max(0)
            } else {
                i as i128
            }
        };
        let first = normalize(start);
        let last = if stop < 0 {
            len + stop as i128
        } else {
            stop as i128
        };
        if first >= len || first > last {
            out.extend_from_slice(b"*0\r\n");
            return;
        }
        let slice = &ordered[first as usize..=last.min(len - 1) as usize];
        write!(
            out,
            "*{}\r\n",
            slice.len() * if with_scores { 2 } else { 1 }
        )
        .expect("Vec write");
        for (member, score) in slice {
            encode_bulk(Some(member), out);
            if with_scores {
                encode_bulk(Some(score.to_string().as_bytes()), out);
            }
        }
    }

    fn push_list(&self, args: &[&[u8]], left: bool, out: &mut Vec<u8>) {
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let current = data.entries.get(key);
        if current.is_some_and(|entry| !entry.collection.as_ref().is_some_and(Collection::is_list))
        {
            wrong_type(out);
            return;
        }
        let mut list = current
            .and_then(|entry| entry.collection.as_ref().and_then(Collection::list_values))
            .unwrap_or_default();
        if list
            .len()
            .checked_add(args.len() - 2)
            .is_none_or(|n| n > MAX_ELEMENTS)
        {
            out.extend_from_slice(OOM);
            return;
        }
        for value in &args[2..] {
            if left {
                list.push_front(value.to_vec());
            } else {
                list.push_back(value.to_vec());
            }
        }
        let expires = current.and_then(|e| e.expires);
        let bytes = list.iter().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_list(list, config.list_max_listpack_size);
        let previous = current.map(Entry::payload_len);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        let len = encoded.list_items().expect("list encoding").len();
        data.put_collection(key, encoded, expires, payload);
        integer(len as i64, out);
    }

    fn pop_list(&self, args: &[&[u8]], left: bool, out: &mut Vec<u8>) {
        let key = args[1];
        let count = if args.len() == 3 {
            match parse_count(args[2]) {
                Some(n) if n >= 0 => Some(n as usize),
                Some(_) => {
                    out.extend_from_slice(b"-ERR count should be greater than or equal to 0\r\n");
                    return;
                }
                None => {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                }
            }
        } else {
            None
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            if count.is_some() {
                out.extend_from_slice(b"*-1\r\n");
            } else {
                encode_bulk(None, out);
            }
            return;
        };
        let Some(mut list) = entry.collection.as_ref().and_then(Collection::list_values) else {
            wrong_type(out);
            return;
        };
        let expires = entry.expires;
        let previous = entry.payload_len();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let number = count.unwrap_or(1).min(list.len());
        let values: Vec<Vec<u8>> = (0..number)
            .filter_map(|_| {
                if left {
                    list.pop_front()
                } else {
                    list.pop_back()
                }
            })
            .collect();
        if count.is_none() {
            encode_bulk(values.first().map(Vec::as_slice), out);
        } else {
            write!(out, "*{}\r\n", values.len()).expect("Vec write");
            for value in &values {
                encode_bulk(Some(value), out);
            }
        }
        if list.is_empty() {
            data.remove(key);
        } else {
            let bytes = list.iter().map(Vec::len).sum::<usize>();
            let payload = data.payload_bytes - previous + bytes;
            let encoded = encode_list(list, config.list_max_listpack_size);
            data.put_collection(key, encoded, expires, payload);
        }
    }

    fn list_len(&self, key: &[u8], out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        match data.entries.get(key) {
            None => integer(0, out),
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_list) => integer(
                entry
                    .collection
                    .as_ref()
                    .and_then(Collection::list_items)
                    .expect("list type checked")
                    .len() as i64,
                out,
            ),
            Some(_) => wrong_type(out),
        }
    }

    fn list_range(&self, key: &[u8], start: &[u8], stop: &[u8], out: &mut Vec<u8>) {
        let (Some(start), Some(stop)) = (parse_count(start), parse_count(stop)) else {
            out.extend_from_slice(NOT_INTEGER);
            return;
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let list = match data.entries.get(key) {
            None => {
                out.extend_from_slice(b"*0\r\n");
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_list) => entry
                .collection
                .as_ref()
                .and_then(Collection::list_items)
                .expect("list type checked"),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let len = list.len() as i128;
        let normalize = |i: i64| {
            if i < 0 {
                (len + i as i128).max(0)
            } else {
                i as i128
            }
        };
        let first = normalize(start);
        let last = if stop < 0 {
            len + stop as i128
        } else {
            stop as i128
        };
        if first >= len || first > last {
            out.extend_from_slice(b"*0\r\n");
            return;
        }
        let begin = first as usize;
        let end = (last.min(len - 1) as usize) + 1;
        write!(out, "*{}\r\n", end - begin).expect("Vec write");
        for value in list.iter().skip(begin).take(end - begin) {
            encode_bulk(Some(value), out);
        }
    }
}

fn encode_list(list: std::collections::VecDeque<Vec<u8>>, setting: i64) -> Collection {
    let total_bytes: usize = list.iter().map(Vec::len).sum();
    let compact = if setting > 0 {
        list.len() <= setting as usize
    } else {
        total_bytes <= listpack_byte_limit(setting)
    };
    if compact {
        return Collection::ListPack(list);
    }
    let by_count = setting > 0;
    let limit = if by_count {
        setting as usize
    } else {
        listpack_byte_limit(setting)
    };
    let mut nodes = std::collections::VecDeque::new();
    let mut node = std::collections::VecDeque::new();
    let mut node_bytes = 0usize;
    for value in list {
        let full = !node.is_empty()
            && if by_count {
                node.len() >= limit
            } else {
                node_bytes.saturating_add(value.len()) > limit
            };
        if full {
            nodes.push_back(std::mem::take(&mut node));
            node_bytes = 0;
        }
        node_bytes = node_bytes.saturating_add(value.len());
        node.push_back(value);
    }
    if !node.is_empty() {
        nodes.push_back(node);
    }
    Collection::ListQuick(nodes)
}

fn listpack_byte_limit(setting: i64) -> usize {
    if setting >= 0 {
        return setting as usize;
    }
    let level = setting.unsigned_abs().saturating_sub(1).min(4);
    4096usize << level
}

fn parse_score(bytes: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(bytes).ok()?;
    let score = text.parse::<f64>().ok()?;
    score.is_finite().then_some(score)
}

fn parse_count(bytes: &[u8]) -> Option<i64> {
    let text = std::str::from_utf8(bytes).ok()?;
    let value = text.parse::<i64>().ok()?;
    (value.to_string().as_bytes() == bytes).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(store: &Store, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        store.execute(args, &mut out);
        out
    }

    #[test]
    fn lists_are_binary_safe_ordered_and_empty_pop_deletes_key() {
        let store = Store::new(128);
        assert_eq!(run(&store, &[b"LPUSH", b"l", b"a\0", b"\xff"]), b":2\r\n");
        assert_eq!(run(&store, &[b"RPUSH", b"l", b"z"]), b":3\r\n");
        assert_eq!(
            run(&store, &[b"LRANGE", b"l", b"0", b"-1"]),
            b"*3\r\n$1\r\n\xff\r\n$2\r\na\0\r\n$1\r\nz\r\n"
        );
        assert_eq!(
            run(&store, &[b"LPOP", b"l", b"2"]),
            b"*2\r\n$1\r\n\xff\r\n$2\r\na\0\r\n"
        );
        assert_eq!(run(&store, &[b"RPOP", b"l"]), b"$1\r\nz\r\n");
        assert_eq!(run(&store, &[b"EXISTS", b"l"]), b":0\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 0);
    }

    #[test]
    fn list_keys_share_ttl_accounting_and_all_string_commands_reject_wrong_type() {
        let store = Store::new(10);
        assert_eq!(run(&store, &[b"RPUSH", b"k", b"1234"]), b":1\r\n");
        assert_eq!(run(&store, &[b"TYPE", b"k"]), b"+list\r\n");
        assert_eq!(
            run(&store, &[b"GET", b"k"]),
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
        );
        assert_eq!(
            run(&store, &[b"INCR", b"k"]),
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
        );
        assert_eq!(
            run(&store, &[b"APPEND", b"k", b"x"]),
            b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
        );
        assert_eq!(run(&store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 2);
    }

    #[test]
    fn sets_hashes_and_sorted_sets_have_real_binary_safe_semantics() {
        let store = Store::new(128);
        assert_eq!(run(&store, &[b"SADD", b"s", b"b", b"a", b"b"]), b":2\r\n");
        assert_eq!(run(&store, &[b"SCARD", b"s"]), b":2\r\n");
        assert_eq!(run(&store, &[b"SISMEMBER", b"s", b"b"]), b":1\r\n");
        assert_eq!(run(&store, &[b"SISMEMBER", b"s", b"missing"]), b":0\r\n");
        assert_eq!(
            run(&store, &[b"SMEMBERS", b"s"]),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"HSET", b"h", b"field\0", b"v\xff", b"other", b"x"]
            ),
            b":2\r\n"
        );
        assert_eq!(
            run(&store, &[b"HSET", b"h", b"field\0", b"changed"]),
            b":0\r\n"
        );
        assert_eq!(
            run(&store, &[b"HGET", b"h", b"field\0"]),
            b"$7\r\nchanged\r\n"
        );
        assert_eq!(run(&store, &[b"HLEN", b"h"]), b":2\r\n");
        assert_eq!(
            run(&store, &[b"HGETALL", b"h"]),
            b"*4\r\n$6\r\nfield\0\r\n$7\r\nchanged\r\n$5\r\nother\r\n$1\r\nx\r\n"
        );
        assert_eq!(
            run(&store, &[b"ZADD", b"z", b"2", b"b", b"-1.5", b"a"]),
            b":2\r\n"
        );
        assert_eq!(run(&store, &[b"ZADD", b"z", b"3", b"b"]), b":0\r\n");
        assert_eq!(run(&store, &[b"ZCARD", b"z"]), b":2\r\n");
        assert_eq!(
            run(&store, &[b"ZRANGE", b"z", b"0", b"-1", b"WITHSCORES"]),
            b"*4\r\n$1\r\na\r\n$4\r\n-1.5\r\n$1\r\nb\r\n$1\r\n3\r\n"
        );
        assert_eq!(run(&store, &[b"ZSCORE", b"z", b"b"]), b"$1\r\n3\r\n");
        assert_eq!(
            run(&store, &[b"ZADD", b"z", b"NaN", b"broken"]),
            b"-ERR value is not a valid float\r\n"
        );
        assert_eq!(
            run(&store, &[b"MGET", b"s", b"h", b"z"]),
            b"*3\r\n$-1\r\n$-1\r\n$-1\r\n"
        );
        assert_eq!(run(&store, &[b"TYPE", b"s"]), b"+set\r\n");
        assert_eq!(run(&store, &[b"TYPE", b"h"]), b"+hash\r\n");
        assert_eq!(run(&store, &[b"TYPE", b"z"]), b"+zset\r\n");
    }

    #[test]
    fn ranges_with_stops_before_the_first_element_are_empty() {
        let store = Store::new(100);
        run(&store, &[b"RPUSH", b"l", b"one", b"two"]);
        run(&store, &[b"ZADD", b"z", b"1", b"one", b"2", b"two"]);
        for command in [b"LRANGE".as_slice(), b"ZRANGE"] {
            let key = if command == b"LRANGE" { b"l" } else { b"z" };
            for stop in [b"-3".as_slice(), b"-9223372036854775808"] {
                assert_eq!(run(&store, &[command, key, b"0", stop]), b"*0\r\n");
                assert_eq!(run(&store, &[command, key, b"-100", stop]), b"*0\r\n");
            }
            assert_eq!(
                run(&store, &[command, key, b"-100", b"-2"]),
                b"*1\r\n$3\r\none\r\n"
            );
        }
    }

    #[test]
    fn list_rename_preserves_collection_and_ttl_and_failed_push_is_atomic() {
        let store = Store::new(5);
        assert_eq!(run(&store, &[b"RPUSH", b"l", b"123"]), b":1\r\n");
        assert_eq!(run(&store, &[b"PEXPIRE", b"l", b"10000"]), b":1\r\n");
        assert_eq!(
            run(&store, &[b"RPUSH", b"l", b"456"]),
            b"-OOM configured payload limit exceeded\r\n"
        );
        assert_eq!(run(&store, &[b"RENAME", b"l", b"m"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"TYPE", b"m"]), b"+list\r\n");
        let ttl = run(&store, &[b"PTTL", b"m"]);
        let ttl = std::str::from_utf8(&ttl[1..ttl.len() - 2])
            .unwrap()
            .parse::<i64>()
            .unwrap();
        assert!((1..=10000).contains(&ttl));
        assert_eq!(
            run(&store, &[b"LRANGE", b"m", b"0", b"-1"]),
            b"*1\r\n$3\r\n123\r\n"
        );
        assert_eq!(run(&store, &[b"DEL", b"m"]), b":1\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 0);
    }
}
