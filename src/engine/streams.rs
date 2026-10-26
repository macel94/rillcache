use super::data::{Consumer, ConsumerGroup, Pending, Stream, StreamEntry, StreamId};
use super::{Store, arity, integer, internal_error};
use crate::protocol::encode_bulk;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

fn id(raw: &[u8]) -> Option<StreamId> {
    match raw.iter().position(|byte| *byte == b'-') {
        Some(separator) => Some(StreamId {
            ms: parse_u64(&raw[..separator])?,
            seq: parse_u64(&raw[separator + 1..])?,
        }),
        None => Some(StreamId {
            ms: parse_u64(raw)?,
            seq: 0,
        }),
    }
}
fn parse_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}
fn id_text(id: StreamId) -> Vec<u8> {
    format!("{}-{}", id.ms, id.seq).into_bytes()
}
fn array(n: usize, out: &mut Vec<u8>) {
    write!(out, "*{n}\r\n").expect("Vec write");
}
fn simple(text: &[u8], out: &mut Vec<u8>) {
    out.push(b'+');
    out.extend_from_slice(text);
    out.extend_from_slice(b"\r\n");
}
fn id_error(out: &mut Vec<u8>) {
    out.extend_from_slice(b"-ERR Invalid stream ID specified as stream command argument\r\n");
}

impl Store {
    pub(super) fn execute_streams(&self, a: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let Some(cmd) = a.first() else { return false };
        if cmd.eq_ignore_ascii_case(b"XADD") {
            self.xadd(a, out);
        } else if cmd.eq_ignore_ascii_case(b"XDEL") {
            self.xdel(a, out);
        } else if cmd.eq_ignore_ascii_case(b"XLEN") {
            self.xlen(a, out);
        } else if cmd.eq_ignore_ascii_case(b"XGROUP") {
            self.xgroup(a, out);
        } else if cmd.eq_ignore_ascii_case(b"XREADGROUP") {
            self.xreadgroup(a, out);
        } else if cmd.eq_ignore_ascii_case(b"XINFO") {
            self.xinfo(a, out);
        } else {
            return false;
        }
        true
    }
    fn xadd(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() < 5 || a.len() % 2 != 1 {
            arity("xadd", out);
            return;
        }
        let explicit = a[2] != b"*";
        let requested = if explicit {
            match id(a[2]) {
                Some(v) => Some(v),
                None => {
                    id_error(out);
                    return;
                }
            }
        } else {
            None
        };
        let fields = a[3..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| (p[0].to_vec(), p[1].to_vec()))
            .collect::<Vec<_>>();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let current = self.now(&data);
        data.expire_key(a[1], current);
        if data.entries.get(a[1]).is_some_and(|e| e.stream.is_none()) {
            out.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        }
        let existing_expiry = data.entries.get(a[1]).and_then(|e| e.expires);
        let mut stream = data
            .entries
            .get(a[1])
            .and_then(|e| e.stream.clone())
            .unwrap_or_default();
        let next = if let Some(v) = requested {
            v
        } else if now > stream.last_generated.ms {
            StreamId { ms: now, seq: 0 }
        } else {
            StreamId {
                ms: stream.last_generated.ms,
                seq: stream.last_generated.seq.saturating_add(1),
            }
        };
        if next == StreamId::default() {
            out.extend_from_slice(b"-ERR The ID specified in XADD must be greater than 0-0\r\n");
            return;
        }
        if next <= stream.last_generated {
            out.extend_from_slice(b"-ERR The ID specified in XADD is equal or smaller than the target stream top item\r\n");
            return;
        }
        stream.entries.insert(next, StreamEntry { fields });
        stream.last_generated = next;
        let Some(stream_bytes) = stream.payload_bytes() else {
            out.extend_from_slice(b"-OOM configured payload limit exceeded\r\n");
            return;
        };
        let previous = data.entries.get(a[1]).map(|e| e.payload_len);
        let Some(payload) = data.reserve(
            a[1],
            stream_bytes,
            previous,
            self.max_payload_bytes,
            current,
        ) else {
            out.extend_from_slice(b"-OOM configured payload limit exceeded\r\n");
            return;
        };
        data.put_stream(a[1], stream, existing_expiry, payload);
        encode_bulk(Some(&id_text(next)), out);
    }
    fn xdel(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() < 3 {
            arity("xdel", out);
            return;
        }
        let mut ids = Vec::new();
        for raw in &a[2..] {
            match id(raw) {
                Some(i) => ids.push(i),
                None => {
                    id_error(out);
                    return;
                }
            }
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(a[1], now);
        let Some(entry) = data.entries.get_mut(a[1]) else {
            integer(0, out);
            return;
        };
        let Some(stream) = entry.stream.as_mut() else {
            out.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        };
        let mut deleted = 0;
        for key in ids {
            if stream.entries.remove(&key).is_some() {
                deleted += 1;
                for group in stream.groups.values_mut() {
                    if let Some(pending) = group.pending.remove(&key)
                        && let Some(consumer) = group.consumers.get_mut(&pending.consumer)
                    {
                        consumer.pending.remove(&key);
                    }
                }
            }
        }
        let bytes = stream.payload_bytes().unwrap_or(0);
        let old_len = entry.payload_len;
        entry.payload_len = bytes;
        data.payload_bytes = data.payload_bytes - old_len + bytes;
        integer(deleted as i64, out);
    }
    fn xlen(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() != 2 {
            arity("xlen", out);
            return;
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(a[1], now);
        match data.entries.get(a[1]) {
            None => integer(0, out),
            Some(e) => match &e.stream {
                Some(s) => integer(s.entries.len() as i64, out),
                None => out.extend_from_slice(
                    b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
                ),
            },
        }
    }
    fn xgroup(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() < 2 {
            arity("xgroup", out);
            return;
        }
        if a[1].eq_ignore_ascii_case(b"CREATE") {
            if a.len() < 5 || a.len() > 6 {
                arity("xgroup", out);
                return;
            }
            let start = if a[4] == b"$" {
                None
            } else {
                match id(a[4]) {
                    Some(x) => Some(x),
                    None => {
                        id_error(out);
                        return;
                    }
                }
            };
            if a.len() == 6 && !a[5].eq_ignore_ascii_case(b"MKSTREAM") {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
            let Ok(mut data) = self.data.write() else {
                internal_error(out);
                return;
            };
            let now = self.now(&data);
            data.expire_key(a[2], now);
            if data.entries.get(a[2]).is_some_and(|e| e.stream.is_none()) {
                out.extend_from_slice(
                    b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
                );
                return;
            }
            let existing_expiry = data.entries.get(a[2]).and_then(|e| e.expires);
            let mut stream = match data.entries.get(a[2]).and_then(|e| e.stream.clone()) {
                Some(stream) => stream,
                None if a.len() == 6 => Stream::default(),
                None => {
                    out.extend_from_slice(b"-ERR The XGROUP subcommand requires the key to exist. Note that for CREATE you may want to use the MKSTREAM option to create an empty stream automatically.\r\n");
                    return;
                }
            };
            if stream.groups.contains_key(a[3]) {
                out.extend_from_slice(b"-BUSYGROUP Consumer Group name already exists\r\n");
                return;
            }
            let last = start.unwrap_or(stream.last_generated);
            stream.groups.insert(
                a[3].to_vec(),
                ConsumerGroup {
                    last_delivered: last,
                    consumers: Default::default(),
                    pending: Default::default(),
                },
            );
            let previous = data.entries.get(a[2]).map(|e| e.payload_len);
            let Some(total) = data.reserve(
                a[2],
                stream.payload_bytes().unwrap_or(0),
                previous,
                self.max_payload_bytes,
                now,
            ) else {
                out.extend_from_slice(b"-OOM configured payload limit exceeded\r\n");
                return;
            };
            data.put_stream(a[2], stream, existing_expiry, total);
            simple(b"OK", out);
        } else if a[1].eq_ignore_ascii_case(b"SETID") {
            if a.len() != 5 {
                arity("xgroup", out);
                return;
            }
            let next = if a[4] == b"$" {
                None
            } else {
                match id(a[4]) {
                    Some(x) => Some(x),
                    None => {
                        id_error(out);
                        return;
                    }
                }
            };
            let Ok(mut data) = self.data.write() else {
                internal_error(out);
                return;
            };
            let now = self.now(&data);
            data.expire_key(a[2], now);
            let Some(e) = data.entries.get_mut(a[2]) else {
                out.extend_from_slice(b"-ERR The XGROUP subcommand requires the key to exist\r\n");
                return;
            };
            let Some(s) = e.stream.as_mut() else {
                out.extend_from_slice(
                    b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
                );
                return;
            };
            let Some(g) = s.groups.get_mut(a[3]) else {
                out.extend_from_slice(b"-NOGROUP No such consumer group\r\n");
                return;
            };
            g.last_delivered = next.unwrap_or(s.last_generated);
            simple(b"OK", out);
        } else {
            out.extend_from_slice(b"-ERR unsupported XGROUP subcommand\r\n")
        }
    }
    fn xreadgroup(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() < 7 || !a[1].eq_ignore_ascii_case(b"GROUP") {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        let (group, consumer) = (a[2], a[3]);
        let mut count = usize::MAX;
        let mut noack = false;
        let mut i = 4;
        while i < a.len() && !a[i].eq_ignore_ascii_case(b"STREAMS") {
            if a[i].eq_ignore_ascii_case(b"COUNT") && i + 1 < a.len() {
                count = parse_u64(a[i + 1])
                    .and_then(|n| usize::try_from(n).ok())
                    .unwrap_or(usize::MAX);
                i += 2
            } else if a[i].eq_ignore_ascii_case(b"NOACK") {
                noack = true;
                i += 1
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        if i + 3 != a.len() || !a[i].eq_ignore_ascii_case(b"STREAMS") {
            out.extend_from_slice(b"-ERR Unbalanced 'XREADGROUP' STREAMS list\r\n");
            return;
        }
        let key = a[i + 1];
        if a[i + 2] != b">" {
            out.extend_from_slice(
                b"-ERR history reads are not supported by this bounded subset\r\n",
            );
            return;
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.clock.now_ms().max(0);
        data.expire_key(key, now);
        let Some(e) = data.entries.get_mut(key) else {
            out.extend_from_slice(b"-NOGROUP No such key or consumer group\r\n");
            return;
        };
        let Some(s) = e.stream.as_mut() else {
            out.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        };
        let Some(g) = s.groups.get_mut(group) else {
            out.extend_from_slice(b"-NOGROUP No such consumer group\r\n");
            return;
        };
        let entries = s
            .entries
            .range((
                std::ops::Bound::Excluded(g.last_delivered),
                std::ops::Bound::Unbounded,
            ))
            .take(count)
            .map(|(id, e)| (*id, e.clone()))
            .collect::<Vec<_>>();
        if entries.is_empty() {
            out.extend_from_slice(b"*-1\r\n");
            return;
        }
        let con = g
            .consumers
            .entry(consumer.to_vec())
            .or_insert_with(|| Consumer {
                seen_at: now,
                pending: Default::default(),
            });
        con.seen_at = now;
        for (id, _) in &entries {
            g.last_delivered = *id;
            if !noack {
                let p = Pending {
                    consumer: consumer.to_vec(),
                    deliveries: 1,
                    delivered_at: now,
                };
                g.pending.insert(*id, p.clone());
                con.pending.insert(*id, p);
            }
        }
        array(1, out);
        array(2, out);
        encode_bulk(Some(key), out);
        array(entries.len(), out);
        for (id, e) in entries {
            array(2, out);
            encode_bulk(Some(&id_text(id)), out);
            array(e.fields.len() * 2, out);
            for (f, v) in e.fields {
                encode_bulk(Some(&f), out);
                encode_bulk(Some(&v), out)
            }
        }
    }
    fn xinfo(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() < 3 || !a[1].eq_ignore_ascii_case(b"STREAM") {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        let full = a.get(3).is_some_and(|x| x.eq_ignore_ascii_case(b"FULL"));
        let mut count = 10usize;
        if (a.len() != 3 && !full) || (full && a.len() != 4 && a.len() != 6) {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        if a.len() == 6 {
            if !a[4].eq_ignore_ascii_case(b"COUNT") {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
            let raw = std::str::from_utf8(a[5])
                .ok()
                .and_then(|v| v.parse::<i64>().ok());
            let Some(raw) = raw else {
                out.extend_from_slice(b"-ERR value is not an integer or out of range\r\n");
                return;
            };
            if raw >= 0 {
                count = usize::try_from(raw).unwrap_or(usize::MAX);
            }
        }
        let limit = if count == 0 { usize::MAX } else { count };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(a[2], now);
        let Some(e) = data.entries.get(a[2]) else {
            out.extend_from_slice(b"-ERR no such key\r\n");
            return;
        };
        let Some(s) = &e.stream else {
            out.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        };
        if full {
            array(6, out);
            encode_bulk(Some(b"length"), out);
            integer(s.entries.len() as i64, out);
            encode_bulk(Some(b"entries"), out);
            array(s.entries.len().min(limit), out);
            for (id, e) in s.entries.iter().take(limit) {
                reply_entry(*id, e, out)
            }
            encode_bulk(Some(b"groups"), out);
            array(s.groups.len(), out);
            for (name, g) in &s.groups {
                array(16, out);
                encode_bulk(Some(b"name"), out);
                encode_bulk(Some(name), out);
                encode_bulk(Some(b"last-delivered-id"), out);
                encode_bulk(Some(&id_text(g.last_delivered)), out);
                encode_bulk(Some(b"entries-read"), out);
                encode_bulk(None, out);
                encode_bulk(Some(b"lag"), out);
                integer(
                    s.entries
                        .range((
                            std::ops::Bound::Excluded(g.last_delivered),
                            std::ops::Bound::Unbounded,
                        ))
                        .count() as i64,
                    out,
                );
                encode_bulk(Some(b"pel-count"), out);
                integer(g.pending.len() as i64, out);
                encode_bulk(Some(b"nacked-count"), out);
                integer(g.pending.len() as i64, out);
                encode_bulk(Some(b"pending"), out);
                array(g.pending.len().min(limit), out);
                for (pid, p) in g.pending.iter().take(limit) {
                    array(4, out);
                    encode_bulk(Some(&id_text(*pid)), out);
                    encode_bulk(Some(&p.consumer), out);
                    integer(p.delivered_at, out);
                    integer(p.deliveries as i64, out)
                }
                encode_bulk(Some(b"consumers"), out);
                array(g.consumers.len(), out);
                for (c, consumer) in &g.consumers {
                    array(10, out);
                    encode_bulk(Some(b"name"), out);
                    encode_bulk(Some(c), out);
                    encode_bulk(Some(b"seen-time"), out);
                    integer(consumer.seen_at, out);
                    encode_bulk(Some(b"active-time"), out);
                    integer(consumer.seen_at, out);
                    encode_bulk(Some(b"pel-count"), out);
                    integer(consumer.pending.len() as i64, out);
                    encode_bulk(Some(b"pending"), out);
                    array(consumer.pending.len().min(limit), out);
                    for (pid, p) in consumer.pending.iter().take(limit) {
                        array(3, out);
                        encode_bulk(Some(&id_text(*pid)), out);
                        integer(p.delivered_at, out);
                        integer(p.deliveries as i64, out)
                    }
                }
            }
        } else {
            array(10, out);
            encode_bulk(Some(b"length"), out);
            integer(s.entries.len() as i64, out);
            encode_bulk(Some(b"last-generated-id"), out);
            encode_bulk(Some(&id_text(s.last_generated)), out);
            encode_bulk(Some(b"groups"), out);
            integer(s.groups.len() as i64, out);
            encode_bulk(Some(b"first-entry"), out);
            if let Some((id, entry)) = s.entries.first_key_value() {
                reply_entry(*id, entry, out);
            } else {
                out.extend_from_slice(b"$-1\r\n");
            }
            encode_bulk(Some(b"last-entry"), out);
            if let Some((id, entry)) = s.entries.last_key_value() {
                reply_entry(*id, entry, out);
            } else {
                out.extend_from_slice(b"$-1\r\n");
            }
        }
    }
}
fn reply_entry(id: StreamId, e: &StreamEntry, out: &mut Vec<u8>) {
    array(2, out);
    encode_bulk(Some(&id_text(id)), out);
    array(e.fields.len() * 2, out);
    for (f, v) in &e.fields {
        encode_bulk(Some(f), out);
        encode_bulk(Some(v), out)
    }
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
    fn group_state_and_stream_entries_are_deep_copied_and_keyspace_aware() {
        let store = Store::new(1_000_000);
        assert_eq!(
            run(&store, &[b"XADD", b"src", b"100", b"a", b"1"]),
            b"$5\r\n100-0\r\n"
        );
        assert_eq!(
            run(&store, &[b"XADD", b"src", b"101", b"b", b"2"]),
            b"$5\r\n101-0\r\n"
        );
        assert_eq!(
            run(&store, &[b"XGROUP", b"CREATE", b"src", b"g", b"0"]),
            b"+OK\r\n"
        );
        let read = run(
            &store,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"alice",
                b"COUNT",
                b"1",
                b"STREAMS",
                b"src",
                b">",
            ],
        );
        assert!(read.starts_with(b"*1\r\n"));
        assert_eq!(run(&store, &[b"COPY", b"src", b"dst"]), b":1\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 14);
        assert_eq!(run(&store, &[b"TYPE", b"dst"]), b"+stream\r\n");
        let info = run(&store, &[b"XINFO", b"STREAM", b"dst", b"FULL"]);
        assert_eq!(info, run(&store, &[b"XINFO", b"STREAM", b"src", b"FULL"]));
        assert!(run(&store, &[b"GET", b"dst"]).starts_with(b"-WRONGTYPE"));
        assert_eq!(run(&store, &[b"DEL", b"src"]), b":1\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 7);
        assert_eq!(info, run(&store, &[b"XINFO", b"STREAM", b"dst", b"FULL"]));
        assert_eq!(run(&store, &[b"RENAME", b"dst", b"moved"]), b"+OK\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 9);
        assert_eq!(run(&store, &[b"XLEN", b"moved"]), b":2\r\n");
    }
    #[test]
    fn mkstream_admission_failure_is_atomic_and_well_framed() {
        let store = Store::new(0);
        assert_eq!(
            run(
                &store,
                &[b"XGROUP", b"CREATE", b"s", b"g", b"$", b"MKSTREAM"],
            ),
            b"-OOM configured payload limit exceeded\r\n"
        );
        assert_eq!(run(&store, &[b"DBSIZE"]), b":0\r\n");
        assert_eq!(run(&store, &[b"TYPE", b"s"]), b"+none\r\n");
    }

    #[test]
    fn xdel_and_noack_change_real_stream_state_without_creating_pending_entries() {
        let store = Store::new(1000);
        for (id, field) in [(b"1".as_slice(), b"a".as_slice()), (b"2", b"b")] {
            run(&store, &[b"XADD", b"s", id, field, b"v"]);
        }
        run(&store, &[b"XGROUP", b"CREATE", b"s", b"g", b"0"]);
        run(
            &store,
            &[
                b"XREADGROUP",
                b"GROUP",
                b"g",
                b"c",
                b"NOACK",
                b"COUNT",
                b"2",
                b"STREAMS",
                b"s",
                b">",
            ],
        );
        assert_eq!(run(&store, &[b"XDEL", b"s", b"1-0"]), b":1\r\n");
        assert_eq!(run(&store, &[b"XLEN", b"s"]), b":1\r\n");
        let info = run(&store, &[b"XINFO", b"STREAM", b"s", b"FULL"]);
        assert!(!info.windows(5).any(|w| w == b"1-0"));
    }
    #[test]
    fn xdel_removes_pending_references_from_groups_and_consumers() {
        let store = Store::new(1000);
        run(&store, &[b"XADD", b"s", b"1", b"f", b"v"]);
        run(&store, &[b"XGROUP", b"CREATE", b"s", b"g", b"0"]);
        run(
            &store,
            &[b"XREADGROUP", b"GROUP", b"g", b"c", b"STREAMS", b"s", b">"],
        );
        let data = store.data.read().unwrap();
        let group = &data.entries[b"s".as_slice()]
            .stream
            .as_ref()
            .unwrap()
            .groups[b"g".as_slice()];
        assert_eq!(group.pending.len(), 1);
        assert_eq!(group.consumers[b"c".as_slice()].pending.len(), 1);
        drop(data);
        let before = run(&store, &[b"XINFO", b"STREAM", b"s", b"FULL"]);
        let pending_entry = b"*4\r\n$3\r\n1-0\r\n$1\r\nc\r\n";
        assert!(
            before
                .windows(pending_entry.len())
                .any(|w| w == pending_entry)
        );
        assert_eq!(run(&store, &[b"XDEL", b"s", b"1-0"]), b":1\r\n");
        let after = run(&store, &[b"XINFO", b"STREAM", b"s", b"FULL"]);
        assert!(
            !after
                .windows(pending_entry.len())
                .any(|w| w == pending_entry)
        );
        let data = store.data.read().unwrap();
        let group = &data.entries[b"s".as_slice()]
            .stream
            .as_ref()
            .unwrap()
            .groups[b"g".as_slice()];
        assert!(group.pending.is_empty());
        assert!(group.consumers[b"c".as_slice()].pending.is_empty());
    }
}
