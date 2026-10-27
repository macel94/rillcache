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
fn bound_id(raw: &[u8], start: bool) -> Option<StreamId> {
    if raw == b"-" {
        return Some(StreamId { ms: 0, seq: 0 });
    }
    if raw == b"+" {
        return Some(StreamId {
            ms: u64::MAX,
            seq: u64::MAX,
        });
    }
    let value = id(raw)?;
    if raw.contains(&b'-') {
        Some(value)
    } else {
        Some(StreamId {
            ms: value.ms,
            seq: if start { 0 } else { u64::MAX },
        })
    }
}

fn parse_i64(raw: &[u8]) -> Option<i64> {
    std::str::from_utf8(raw).ok()?.parse().ok()
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
        } else if cmd.eq_ignore_ascii_case(b"XACK") {
            self.xack(a, out);
        } else if cmd.eq_ignore_ascii_case(b"XCLAIM") {
            self.xclaim(a, out);
        } else if cmd.eq_ignore_ascii_case(b"XAUTOCLAIM") {
            self.xautoclaim(a, out);
        } else if cmd.eq_ignore_ascii_case(b"XLEN") {
            self.xlen(a, out);
        } else if cmd.eq_ignore_ascii_case(b"XPENDING") {
            self.xpending(a, out);
        } else if cmd.eq_ignore_ascii_case(b"XRANGE") || cmd.eq_ignore_ascii_case(b"XREVRANGE") {
            self.xrange(a, cmd.eq_ignore_ascii_case(b"XREVRANGE"), out);
        } else if cmd.eq_ignore_ascii_case(b"XREAD") {
            self.xread(a, out);
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
        data.put_stream(
            a[1],
            stream,
            existing_expiry,
            payload,
            now.min(i64::MAX as u64) as i64,
        );
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
                // Redis XDEL removes the message, not its pending-delivery
                // references. Those IDs remain in group/consumer PEL metadata.
            }
        }
        let bytes = stream.payload_bytes().unwrap_or(0);
        let old_len = entry.payload_len;
        entry.payload_len = bytes;
        data.payload_bytes = data.payload_bytes - old_len + bytes;
        data.touch(a[1]);
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
    fn xack(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() < 4 {
            arity("xack", out);
            return;
        }
        let mut ids = Vec::new();
        for raw in &a[3..] {
            match id(raw) {
                Some(id) => ids.push(id),
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
        let Some(group) = stream.groups.get_mut(a[2]) else {
            integer(0, out);
            return;
        };
        let mut acknowledged = 0;
        for id in ids {
            if let Some(pending) = group.pending.remove(&id) {
                if let Some(consumer) = group.consumers.get_mut(&pending.consumer) {
                    consumer.pending.remove(&id);
                }
                acknowledged += 1;
            }
        }
        if acknowledged > 0 {
            data.touch(a[1]);
        }
        integer(acknowledged, out);
    }

    fn xpending(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() < 3 {
            arity("xpending", out);
            return;
        }
        let mut index = 3;
        let mut min_idle = 0i64;
        if a.get(index)
            .is_some_and(|arg| arg.eq_ignore_ascii_case(b"IDLE"))
        {
            let Some(raw) = a.get(index + 1) else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            };
            let Some(value) = std::str::from_utf8(raw)
                .ok()
                .and_then(|text| text.parse::<i64>().ok())
                .filter(|value| *value >= 0)
            else {
                out.extend_from_slice(b"-ERR value is not an integer or out of range\r\n");
                return;
            };
            min_idle = value;
            index += 2;
        }
        let extended = index < a.len();
        let mut start = None;
        let mut end = None;
        let mut count = usize::MAX;
        let mut consumer = None;
        if extended {
            if a.len() < index + 3 {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
            let (Some(first), Some(last)) =
                (bound_id(a[index], true), bound_id(a[index + 1], false))
            else {
                id_error(out);
                return;
            };
            start = Some(first);
            end = Some(last);
            let Some(parsed) = parse_i64(a[index + 2]).filter(|value| *value > 0) else {
                out.extend_from_slice(b"-ERR value is not an integer or out of range\r\n");
                return;
            };
            count = usize::try_from(parsed).unwrap_or(usize::MAX);
            if a.len() > index + 3 {
                if a.len() != index + 4 {
                    out.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                consumer = Some(a[index + 3]);
            }
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.clock.now_ms().max(0);
        data.expire_key(a[1], now);
        let Some(entry) = data.entries.get(a[1]) else {
            out.extend_from_slice(b"-NOGROUP No such key or consumer group\r\n");
            return;
        };
        let Some(stream) = entry.stream.as_ref() else {
            out.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        };
        let Some(group) = stream.groups.get(a[2]) else {
            out.extend_from_slice(b"-NOGROUP No such consumer group\r\n");
            return;
        };
        if !extended {
            let total = group.pending.len();
            array(4, out);
            integer(total as i64, out);
            if let Some((id, _)) = group.pending.first_key_value() {
                encode_bulk(Some(&id_text(*id)), out);
            } else {
                encode_bulk(None, out);
            }
            if let Some((id, _)) = group.pending.last_key_value() {
                encode_bulk(Some(&id_text(*id)), out);
            } else {
                encode_bulk(None, out);
            }
            let consumers: Vec<(&[u8], usize)> = group
                .consumers
                .iter()
                .filter(|(_, value)| !value.pending.is_empty())
                .map(|(name, value)| (name.as_slice(), value.pending.len()))
                .collect();
            array(consumers.len(), out);
            for (name, pending) in consumers {
                array(2, out);
                encode_bulk(Some(name), out);
                integer(pending as i64, out);
            }
            return;
        }
        let selected: Vec<(StreamId, &Pending)> = group
            .pending
            .range((
                std::ops::Bound::Included(start.expect("extended")),
                std::ops::Bound::Included(end.expect("extended")),
            ))
            .filter(|(_, pending)| {
                consumer.is_none_or(|name| pending.consumer == name)
                    && now.saturating_sub(pending.delivered_at) >= min_idle
            })
            .take(count)
            .map(|(id, pending)| (*id, pending))
            .collect();
        array(selected.len(), out);
        for (id, pending) in selected {
            array(4, out);
            encode_bulk(Some(&id_text(id)), out);
            encode_bulk(Some(&pending.consumer), out);
            integer(now.saturating_sub(pending.delivered_at), out);
            integer(pending.deliveries as i64, out);
        }
    }

    fn xrange(&self, a: &[&[u8]], reverse: bool, out: &mut Vec<u8>) {
        if a.len() < 4 {
            arity(if reverse { "xrevrange" } else { "xrange" }, out);
            return;
        }
        let (Some(start), Some(end)) = (
            bound_id(if reverse { a[3] } else { a[2] }, true),
            bound_id(if reverse { a[2] } else { a[3] }, false),
        ) else {
            id_error(out);
            return;
        };
        let mut count = usize::MAX;
        if a.len() > 4 {
            if a.len() != 6 || !a[4].eq_ignore_ascii_case(b"COUNT") {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
            let Some(parsed) = parse_i64(a[5]).filter(|value| *value > 0) else {
                out.extend_from_slice(b"-ERR value is not an integer or out of range\r\n");
                return;
            };
            count = usize::try_from(parsed).unwrap_or(usize::MAX);
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(a[1], now);
        let Some(entry) = data.entries.get(a[1]) else {
            array(0, out);
            return;
        };
        let Some(stream) = entry.stream.as_ref() else {
            out.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        };
        if start > end {
            array(0, out);
            return;
        }
        let entries: Vec<(StreamId, &StreamEntry)> = if reverse {
            stream
                .entries
                .range((
                    std::ops::Bound::Included(start),
                    std::ops::Bound::Included(end),
                ))
                .rev()
                .take(count)
                .map(|(id, entry)| (*id, entry))
                .collect()
        } else {
            stream
                .entries
                .range((
                    std::ops::Bound::Included(start),
                    std::ops::Bound::Included(end),
                ))
                .take(count)
                .map(|(id, entry)| (*id, entry))
                .collect()
        };
        array(entries.len(), out);
        for (id, entry) in entries {
            reply_entry(id, entry, out);
        }
    }

    fn xread(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        let mut count = usize::MAX;
        let mut index = 1;
        while index < a.len() && !a[index].eq_ignore_ascii_case(b"STREAMS") {
            if a[index].eq_ignore_ascii_case(b"COUNT") {
                let Some(raw) = a.get(index + 1) else {
                    out.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                };
                let Some(parsed) = parse_i64(raw).filter(|value| *value > 0) else {
                    out.extend_from_slice(b"-ERR value is not an integer or out of range\r\n");
                    return;
                };
                count = usize::try_from(parsed).unwrap_or(usize::MAX);
                index += 2;
            } else if a[index].eq_ignore_ascii_case(b"BLOCK") {
                out.extend_from_slice(b"-ERR blocking XREAD is not supported\r\n");
                return;
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        if index >= a.len() || !a[index].eq_ignore_ascii_case(b"STREAMS") {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        let remaining = &a[index + 1..];
        if remaining.is_empty() || !remaining.len().is_multiple_of(2) {
            out.extend_from_slice(b"-ERR Unbalanced 'XREAD' STREAMS list\r\n");
            return;
        }
        let streams = remaining.len() / 2;
        let keys = &remaining[..streams];
        let starts = &remaining[streams..];
        let mut parsed_ids = Vec::with_capacity(streams);
        for (_key, raw) in keys.iter().zip(starts) {
            if *raw == b"$" {
                parsed_ids.push(None);
            } else {
                let Some(id) = id(raw) else {
                    id_error(out);
                    return;
                };
                parsed_ids.push(Some(id));
            }
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut replies = Vec::new();
        for ((key, _raw), start) in keys.iter().zip(starts).zip(parsed_ids) {
            data.expire_key(key, now);
            let Some(entry) = data.entries.get(*key) else {
                continue;
            };
            let Some(stream) = entry.stream.as_ref() else {
                out.extend_from_slice(
                    b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
                );
                return;
            };
            let start = start.unwrap_or(stream.last_generated);
            let selected: Vec<(StreamId, StreamEntry)> = stream
                .entries
                .range((std::ops::Bound::Excluded(start), std::ops::Bound::Unbounded))
                .take(count)
                .map(|(id, entry)| (*id, entry.clone()))
                .collect();
            if selected.is_empty() {
                continue;
            }
            replies.push((*key, selected));
        }
        if replies.is_empty() {
            out.extend_from_slice(b"*-1\r\n");
            return;
        }
        array(replies.len(), out);
        for (key, entries) in replies {
            array(2, out);
            encode_bulk(Some(key), out);
            array(entries.len(), out);
            for (id, entry) in &entries {
                reply_entry(*id, entry, out);
            }
        }
    }

    fn xclaim(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() < 6 {
            arity("xclaim", out);
            return;
        }
        let Some(min_idle) = parse_i64(a[4]) else {
            out.extend_from_slice(b"-ERR Invalid min-idle-time argument for XCLAIM\r\n");
            return;
        };
        let min_idle = min_idle.max(0);
        let now = self.clock.now_ms().max(0);
        let mut ids = Vec::new();
        let mut index = 5;
        while index < a.len() {
            let Some(value) = id(a[index]) else {
                break;
            };
            ids.push(value);
            index += 1;
        }
        let mut delivery = None;
        let mut retry = None;
        let mut force = false;
        let mut justid = false;
        let mut last_id = StreamId::default();
        while index < a.len() {
            let token = a[index];
            if token.eq_ignore_ascii_case(b"FORCE") {
                force = true;
                index += 1;
            } else if token.eq_ignore_ascii_case(b"JUSTID") {
                justid = true;
                index += 1;
            } else if token.eq_ignore_ascii_case(b"IDLE") && index + 1 < a.len() {
                let Some(value) = parse_i64(a[index + 1]) else {
                    out.extend_from_slice(b"-ERR Invalid IDLE option argument for XCLAIM\r\n");
                    return;
                };
                delivery = Some(now.saturating_sub(value));
                index += 2;
            } else if token.eq_ignore_ascii_case(b"TIME") && index + 1 < a.len() {
                let Some(value) = parse_i64(a[index + 1]) else {
                    out.extend_from_slice(b"-ERR Invalid TIME option argument for XCLAIM\r\n");
                    return;
                };
                delivery = Some(value);
                index += 2;
            } else if token.eq_ignore_ascii_case(b"RETRYCOUNT") && index + 1 < a.len() {
                let Some(value) = parse_i64(a[index + 1]) else {
                    out.extend_from_slice(
                        b"-ERR Invalid RETRYCOUNT option argument for XCLAIM\r\n",
                    );
                    return;
                };
                retry = (value >= 0).then_some(value as u64);
                index += 2;
            } else if token.eq_ignore_ascii_case(b"LASTID") && index + 1 < a.len() {
                let Some(value) = id(a[index + 1]) else {
                    id_error(out);
                    return;
                };
                last_id = value;
                index += 2;
            } else {
                out.extend_from_slice(b"-ERR Unrecognized XCLAIM option '");
                out.extend_from_slice(token);
                out.extend_from_slice(b"'\r\n");
                return;
            }
        }
        let delivery = delivery
            .map(|value| if value < 0 || value > now { now } else { value })
            .unwrap_or(now);
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        data.expire_key(a[1], now);
        let Some(entry) = data.entries.get_mut(a[1]) else {
            out.extend_from_slice(b"-NOGROUP No such key or consumer group\r\n");
            return;
        };
        let Some(stream) = entry.stream.as_mut() else {
            out.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        };
        let Some(group) = stream.groups.get_mut(a[2]) else {
            out.extend_from_slice(b"-NOGROUP No such consumer group\r\n");
            return;
        };
        group
            .consumers
            .entry(a[3].to_vec())
            .or_insert_with(|| Consumer {
                seen_at: now,
                pending: Default::default(),
            })
            .seen_at = now;
        let mut changed = false;
        if last_id > group.last_delivered {
            group.last_delivered = last_id;
            changed = true;
        }
        let mut replies = Vec::new();
        for id in ids {
            let Some(current) = stream.entries.get(&id).cloned() else {
                if let Some(pending) = group.pending.remove(&id) {
                    if let Some(consumer) = group.consumers.get_mut(&pending.consumer) {
                        consumer.pending.remove(&id);
                    }
                    changed = true;
                }
                continue;
            };
            let previous = group.pending.get(&id).cloned();
            if let Some(pending) = &previous {
                if min_idle > 0 && now.saturating_sub(pending.delivered_at) < min_idle {
                    continue;
                }
            } else if !force {
                continue;
            }
            let old_deliveries = previous.as_ref().map_or(0, |pending| pending.deliveries);
            let deliveries = retry.unwrap_or_else(|| {
                if justid {
                    old_deliveries
                } else {
                    old_deliveries.saturating_add(1)
                }
            });
            if let Some(pending) = &previous
                && pending.consumer != a[3]
                && let Some(consumer) = group.consumers.get_mut(&pending.consumer)
            {
                consumer.pending.remove(&id);
            }
            let pending = Pending {
                consumer: a[3].to_vec(),
                deliveries,
                delivered_at: delivery,
            };
            group.pending.insert(id, pending.clone());
            let consumer = group
                .consumers
                .entry(a[3].to_vec())
                .or_insert_with(|| Consumer {
                    seen_at: now,
                    pending: Default::default(),
                });
            consumer.seen_at = now;
            consumer.pending.insert(id, pending);
            replies.push((id, current));
            changed = true;
        }
        array(replies.len(), out);
        for (id, entry) in replies {
            if justid {
                encode_bulk(Some(&id_text(id)), out);
            } else {
                reply_entry(id, &entry, out);
            }
        }
        if changed {
            data.touch(a[1]);
        }
    }

    fn xautoclaim(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() < 6 {
            arity("xautoclaim", out);
            return;
        }
        let Some(min_idle) = parse_i64(a[4]) else {
            out.extend_from_slice(b"-ERR Invalid min-idle-time argument for XAUTOCLAIM\r\n");
            return;
        };
        let min_idle = min_idle.max(0);
        let Some(start) = bound_id(a[5], true) else {
            id_error(out);
            return;
        };
        let mut count = 100usize;
        let mut justid = false;
        let mut index = 6;
        while index < a.len() {
            let token = a[index];
            if token.eq_ignore_ascii_case(b"COUNT") && index + 1 < a.len() {
                let Some(value) = parse_i64(a[index + 1]).filter(|value| *value > 0) else {
                    out.extend_from_slice(b"-ERR COUNT must be > 0\r\n");
                    return;
                };
                count = usize::try_from(value).unwrap_or(usize::MAX);
                index += 2;
            } else if token.eq_ignore_ascii_case(b"JUSTID") {
                justid = true;
                index += 1;
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        let now = self.clock.now_ms().max(0);
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        data.expire_key(a[1], now);
        let Some(entry) = data.entries.get_mut(a[1]) else {
            out.extend_from_slice(b"-NOGROUP No such key or consumer group\r\n");
            return;
        };
        let Some(stream) = entry.stream.as_mut() else {
            out.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        };
        let Some(group) = stream.groups.get_mut(a[2]) else {
            out.extend_from_slice(b"-NOGROUP No such consumer group\r\n");
            return;
        };
        group
            .consumers
            .entry(a[3].to_vec())
            .or_insert_with(|| Consumer {
                seen_at: now,
                pending: Default::default(),
            })
            .seen_at = now;
        let attempts = count.saturating_mul(10).min(group.pending.len());
        let candidates: Vec<(StreamId, Pending)> = group
            .pending
            .range((std::ops::Bound::Included(start), std::ops::Bound::Unbounded))
            .take(attempts)
            .map(|(id, pending)| (*id, pending.clone()))
            .collect();
        let mut remaining = count;
        let mut last_examined = None;
        let mut replies = Vec::new();
        let mut deleted = Vec::new();
        let mut changed = false;
        for (id, pending) in candidates {
            last_examined = Some(id);
            let Some(current) = stream.entries.get(&id).cloned() else {
                group.pending.remove(&id);
                if let Some(consumer) = group.consumers.get_mut(&pending.consumer) {
                    consumer.pending.remove(&id);
                }
                deleted.push(id);
                changed = true;
                remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    break;
                }
                continue;
            };
            if min_idle > 0 && now.saturating_sub(pending.delivered_at) < min_idle {
                continue;
            }
            if pending.consumer != a[3]
                && let Some(consumer) = group.consumers.get_mut(&pending.consumer)
            {
                consumer.pending.remove(&id);
            }
            let deliveries = if justid {
                pending.deliveries
            } else {
                pending.deliveries.saturating_add(1)
            };
            let updated = Pending {
                consumer: a[3].to_vec(),
                deliveries,
                delivered_at: now,
            };
            group.pending.insert(id, updated.clone());
            let consumer = group
                .consumers
                .entry(a[3].to_vec())
                .or_insert_with(|| Consumer {
                    seen_at: now,
                    pending: Default::default(),
                });
            consumer.seen_at = now;
            consumer.pending.insert(id, updated);
            replies.push((id, current));
            changed = true;
            remaining = remaining.saturating_sub(1);
            if remaining == 0 {
                break;
            }
        }
        let cursor = last_examined
            .and_then(|id| {
                group
                    .pending
                    .range((std::ops::Bound::Excluded(id), std::ops::Bound::Unbounded))
                    .next()
                    .map(|(id, _)| *id)
            })
            .unwrap_or_default();
        array(3, out);
        encode_bulk(Some(&id_text(cursor)), out);
        array(replies.len(), out);
        for (id, entry) in replies {
            if justid {
                encode_bulk(Some(&id_text(id)), out);
            } else {
                reply_entry(id, &entry, out);
            }
        }
        array(deleted.len(), out);
        for id in deleted {
            encode_bulk(Some(&id_text(id)), out);
        }
        if changed {
            data.touch(a[1]);
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
            data.put_stream(a[2], stream, existing_expiry, total, now);
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
            data.touch(a[2]);
            simple(b"OK", out);
        } else if a[1].eq_ignore_ascii_case(b"DESTROY") {
            if a.len() != 4 {
                arity("xgroup", out);
                return;
            }
            let Ok(mut data) = self.data.write() else {
                internal_error(out);
                return;
            };
            let now = self.now(&data);
            data.expire_key(a[2], now);
            let Some(e) = data.entries.get_mut(a[2]) else {
                integer(0, out);
                return;
            };
            let Some(s) = e.stream.as_mut() else {
                out.extend_from_slice(
                    b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
                );
                return;
            };
            let removed = s.groups.remove(a[3]).is_some();
            if removed {
                data.touch(a[2]);
            }
            integer(i64::from(removed), out);
        } else if a[1].eq_ignore_ascii_case(b"DELCONSUMER") {
            if a.len() != 5 {
                arity("xgroup", out);
                return;
            }
            let Ok(mut data) = self.data.write() else {
                internal_error(out);
                return;
            };
            let now = self.now(&data);
            data.expire_key(a[2], now);
            let Some(e) = data.entries.get_mut(a[2]) else {
                out.extend_from_slice(b"-NOGROUP No such key or consumer group\r\n");
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
            let removed = g.consumers.remove(a[4]);
            let pending = removed
                .as_ref()
                .map_or(0, |consumer| consumer.pending.len());
            g.pending.retain(|_, pending| pending.consumer != a[4]);
            data.touch(a[2]);
            integer(pending as i64, out);
        } else if a[1].eq_ignore_ascii_case(b"CREATECONSUMER") {
            if a.len() != 5 {
                arity("xgroup", out);
                return;
            }
            let Ok(mut data) = self.data.write() else {
                internal_error(out);
                return;
            };
            let now = self.now(&data);
            data.expire_key(a[2], now);
            let Some(e) = data.entries.get_mut(a[2]) else {
                out.extend_from_slice(b"-NOGROUP No such key or consumer group\r\n");
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
            let created = g
                .consumers
                .insert(
                    a[4].to_vec(),
                    Consumer {
                        seen_at: now,
                        pending: Default::default(),
                    },
                )
                .is_none();
            if created {
                data.touch(a[2]);
            }
            integer(i64::from(created), out);
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
        data.touch(key);
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
    fn xinfo_groups(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(a[2], now);
        let Some(entry) = data.entries.get(a[2]) else {
            out.extend_from_slice(b"-ERR no such key\r\n");
            return;
        };
        let Some(stream) = entry.stream.as_ref() else {
            out.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        };
        array(stream.groups.len(), out);
        for (name, group) in &stream.groups {
            array(12, out);
            encode_bulk(Some(b"name"), out);
            encode_bulk(Some(name), out);
            encode_bulk(Some(b"consumers"), out);
            integer(group.consumers.len() as i64, out);
            encode_bulk(Some(b"pending"), out);
            integer(group.pending.len() as i64, out);
            encode_bulk(Some(b"last-delivered-id"), out);
            encode_bulk(Some(&id_text(group.last_delivered)), out);
            encode_bulk(Some(b"entries-read"), out);
            encode_bulk(None, out);
            encode_bulk(Some(b"lag"), out);
            integer(
                stream
                    .entries
                    .range((
                        std::ops::Bound::Excluded(group.last_delivered),
                        std::ops::Bound::Unbounded,
                    ))
                    .count() as i64,
                out,
            );
        }
    }

    fn xinfo_consumers(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(a[2], now);
        let Some(entry) = data.entries.get(a[2]) else {
            out.extend_from_slice(b"-ERR no such key\r\n");
            return;
        };
        let Some(stream) = entry.stream.as_ref() else {
            out.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        };
        let Some(group) = stream.groups.get(a[3]) else {
            out.extend_from_slice(b"-NOGROUP No such consumer group\r\n");
            return;
        };
        array(group.consumers.len(), out);
        for (name, consumer) in &group.consumers {
            array(8, out);
            encode_bulk(Some(b"name"), out);
            encode_bulk(Some(name), out);
            encode_bulk(Some(b"pending"), out);
            integer(consumer.pending.len() as i64, out);
            encode_bulk(Some(b"idle"), out);
            integer(now.saturating_sub(consumer.seen_at).max(0), out);
            encode_bulk(Some(b"inactive"), out);
            integer(now.saturating_sub(consumer.seen_at).max(0), out);
        }
    }

    fn xinfo(&self, a: &[&[u8]], out: &mut Vec<u8>) {
        if a.len() < 2 {
            arity("xinfo", out);
            return;
        }
        if a[1].eq_ignore_ascii_case(b"GROUPS") {
            if a.len() != 3 {
                out.extend_from_slice(b"-ERR syntax error\r\n");
            } else {
                self.xinfo_groups(a, out);
            }
            return;
        }
        if a[1].eq_ignore_ascii_case(b"CONSUMERS") {
            if a.len() != 4 {
                out.extend_from_slice(b"-ERR syntax error\r\n");
            } else {
                self.xinfo_consumers(a, out);
            }
            return;
        }
        if !a[1].eq_ignore_ascii_case(b"STREAM") {
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
                // This subset has no XNACK/unowned pending entries.
                integer(0, out);
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
    fn xdel_preserves_pending_references_in_groups_and_consumers() {
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
            after
                .windows(pending_entry.len())
                .any(|w| w == pending_entry)
        );
        assert_eq!(run(&store, &[b"XLEN", b"s"]), b":0\r\n");
        let data = store.data.read().unwrap();
        let group = &data.entries[b"s".as_slice()]
            .stream
            .as_ref()
            .unwrap()
            .groups[b"g".as_slice()];
        assert_eq!(group.pending.len(), 1);
        assert_eq!(group.consumers[b"c".as_slice()].pending.len(), 1);
    }
}
