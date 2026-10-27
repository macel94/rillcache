use super::data::Entry;
use super::*;
use options::{Condition, ExpireFlags, NOT_INTEGER, OOM, SetOptions, parse_integer};

impl Store {
    pub(super) fn delex(&self, arguments: &[&[u8]], delifeq: bool, output: &mut Vec<u8>) {
        let name = if delifeq { "delifeq" } else { "delex" };
        if (delifeq && arguments.len() != 3)
            || (!delifeq && arguments.len() != 2 && arguments.len() != 4)
        {
            arity(name, output);
            return;
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(arguments[1], now);
        let Some(entry) = data.entries.get(arguments[1]) else {
            integer(0, output);
            return;
        };
        if arguments.len() == 2 {
            data.remove(arguments[1]);
            integer(1, output);
            return;
        }
        if !entry.is_string() {
            output.extend_from_slice(if delifeq {
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
            } else {
                b"-ERR Key should be of string type if conditions are specified\r\n"
            });
            return;
        }
        let token = if delifeq {
            b"IFEQ".as_slice()
        } else {
            arguments[2]
        };
        let expected = if delifeq { arguments[2] } else { arguments[3] };
        let matched = if token.eq_ignore_ascii_case(b"IFEQ") {
            entry.value.as_slice() == expected
        } else if token.eq_ignore_ascii_case(b"IFNE") {
            entry.value.as_slice() != expected
        } else if token.eq_ignore_ascii_case(b"IFDEQ") || token.eq_ignore_ascii_case(b"IFDNE") {
            if expected.len() != 16 {
                output.extend_from_slice(b"-ERR must be exactly 16 hexadecimal characters\r\n");
                return;
            }
            let digest = format!("{:016x}", super::xxh3::xxh3_64(&entry.value));
            let equal = digest.eq_ignore_ascii_case(std::str::from_utf8(expected).unwrap_or(""));
            if token.eq_ignore_ascii_case(b"IFDEQ") {
                equal
            } else {
                !equal
            }
        } else {
            output
                .extend_from_slice(b"-ERR Invalid condition. Use IFEQ, IFNE, IFDEQ, or IFDNE\r\n");
            return;
        };
        if matched {
            data.remove(arguments[1]);
        }
        integer(i64::from(matched), output);
    }

    pub(super) fn digest(&self, key: &[u8], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        if let Some(entry) = data.entries.get(key) {
            if !entry.is_string() {
                output.extend_from_slice(
                    b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
                );
                return;
            }
            let hash = super::xxh3::xxh3_64(&entry.value);
            write!(output, "$16\r\n{hash:016x}\r\n").expect("Vec write");
        } else {
            output.extend_from_slice(b"$-1\r\n");
        }
    }

    pub(super) fn now(&self, data: &Data) -> i64 {
        if data.expirations.is_empty() && data.field_ttls == 0 {
            0
        } else {
            self.clock.now_ms().max(0)
        }
    }
    pub(super) fn get(&self, key: &[u8], output: &mut Vec<u8>) {
        let Ok(data) = self.data.read() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        let entry = data.entries.get(key);
        if entry.is_some_and(|entry| entry.alive(now) && !entry.is_string()) {
            wrong_type(output);
            return;
        }
        if entry.is_none_or(|entry| entry.alive(now)) {
            encode_bulk(entry.map(|entry| entry.value.as_slice()), output);
            return;
        }
        drop(data);
        // Recheck under an exclusive lock: never delete a concurrent replacement.
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        if data
            .entries
            .get(key)
            .is_some_and(|entry| !entry.is_string())
        {
            wrong_type(output);
            return;
        }
        encode_bulk(
            data.entries.get(key).map(|entry| entry.value.as_slice()),
            output,
        );
    }

    pub(super) fn exists(&self, keys: &[&[u8]], output: &mut Vec<u8>) {
        let Ok(data) = self.data.read() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        if keys
            .iter()
            .all(|key| data.entries.get(*key).is_none_or(|entry| entry.alive(now)))
        {
            integer(
                keys.iter()
                    .filter(|key| data.entries.contains_key(**key))
                    .count() as i64,
                output,
            );
            return;
        }
        drop(data);
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        for key in keys {
            data.expire_key(key, now);
        }
        integer(
            keys.iter()
                .filter(|key| data.entries.contains_key(**key))
                .count() as i64,
            output,
        );
    }

    pub(super) fn set(
        &self,
        key: &[u8],
        value: &[u8],
        options: SetOptions<'_>,
        command: &str,
        numeric_reply: bool,
        output: &mut Vec<u8>,
    ) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = if options.expiry.is_some() {
            self.clock.now_ms().max(0)
        } else {
            self.now(&data)
        };
        let expires = match options.deadline(now, command) {
            Ok(value) => value,
            Err(error) => {
                output.extend_from_slice(&error);
                return;
            }
        };
        data.expire_key(key, now);
        let existing = data.entries.get(key);
        if (options.get
            || matches!(
                options.condition,
                Condition::ValueEqual(_)
                    | Condition::ValueNotEqual(_)
                    | Condition::DigestEqual(_)
                    | Condition::DigestNotEqual(_)
            ))
            && existing.is_some_and(|entry| !entry.is_string())
        {
            output.extend_from_slice(
                b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n",
            );
            return;
        }
        let allowed = match options.condition {
            Condition::Always => true,
            Condition::Missing => existing.is_none(),
            Condition::Present => existing.is_some(),
            Condition::ValueEqual(expected) => {
                existing.is_some_and(|entry| entry.value.as_slice() == expected)
            }
            Condition::ValueNotEqual(expected) => {
                existing.is_none_or(|entry| entry.value.as_slice() != expected)
            }
            Condition::DigestEqual(expected) | Condition::DigestNotEqual(expected) => {
                if let Some(entry) = existing {
                    if expected.len() != 16 {
                        output.extend_from_slice(
                            b"-ERR must be exactly 16 hexadecimal characters\r\n",
                        );
                        return;
                    }
                    let digest = format!("{:016x}", super::xxh3::xxh3_64(&entry.value));
                    let equal =
                        digest.eq_ignore_ascii_case(std::str::from_utf8(expected).unwrap_or(""));
                    if matches!(options.condition, Condition::DigestEqual(_)) {
                        equal
                    } else {
                        !equal
                    }
                } else {
                    matches!(options.condition, Condition::DigestNotEqual(_))
                }
            }
        };
        if !allowed {
            if numeric_reply {
                integer(0, output);
            } else {
                encode_bulk(
                    if options.get {
                        existing.map(|entry| entry.value.as_slice())
                    } else {
                        None
                    },
                    output,
                );
            }
            return;
        }
        let expires = if options.keep {
            existing.and_then(|entry| entry.expires)
        } else {
            expires
        };
        if !options.keep && expires.is_some_and(|at| at <= now) {
            if options.get {
                encode_bulk(existing.map(|entry| entry.value.as_slice()), output);
            } else if numeric_reply {
                integer(1, output);
            } else {
                output.extend_from_slice(b"+OK\r\n");
            }
            data.remove(key);
            return;
        }
        let previous = existing.map(Entry::payload_len);
        let Some(payload) = data.reserve(key, value.len(), previous, self.payload_limit(), now)
        else {
            output.extend_from_slice(OOM);
            return;
        };
        if options.get {
            encode_bulk(
                data.entries.get(key).map(|entry| entry.value.as_slice()),
                output,
            );
        } else if numeric_reply {
            integer(1, output);
        } else {
            output.extend_from_slice(b"+OK\r\n");
        }
        data.put(key, value, expires, payload, now);
    }

    pub(super) fn getdel(&self, key: &[u8], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        if data
            .entries
            .get(key)
            .is_some_and(|entry| !entry.is_string())
        {
            wrong_type(output);
            return;
        }
        encode_bulk(
            data.remove(key)
                .as_ref()
                .map(|entry| entry.value.as_slice()),
            output,
        );
    }

    pub(super) fn getex(&self, key: &[u8], options: SetOptions<'_>, output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = if options.expiry.is_some() {
            self.clock.now_ms().max(0)
        } else {
            self.now(&data)
        };
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            encode_bulk(None, output);
            return;
        };
        if !entry.is_string() {
            wrong_type(output);
            return;
        }
        // Like Redis, a missing GETEX key returns null without parsing its TTL value.
        let expires = match options.deadline(now, "getex") {
            Ok(value) => value,
            Err(error) => {
                output.extend_from_slice(&error);
                return;
            }
        };
        encode_bulk(Some(&entry.value), output);
        if expires.is_some_and(|at| at <= now) {
            data.remove(key);
        } else if options.expiry.is_some() || options.persist {
            data.set_expiry(key, expires);
        }
    }

    pub(super) fn ttl(&self, key: &[u8], milliseconds: bool, absolute: bool, output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            integer(-2, output);
            return;
        };
        let Some(at) = entry.expires else {
            integer(-1, output);
            return;
        };
        let value = if absolute { at } else { at - now };
        integer(
            if milliseconds {
                value
            } else {
                value / 1000 + i64::from(value % 1000 >= 500)
            },
            output,
        );
    }

    pub(super) fn persist(&self, key: &[u8], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let changed = data
            .entries
            .get(key)
            .is_some_and(|entry| entry.expires.is_some());
        if changed {
            data.set_expiry(key, None);
        }
        integer(i64::from(changed), output);
    }

    pub(super) fn expire(
        &self,
        arguments: &[&[u8]],
        command: &str,
        seconds: bool,
        absolute: bool,
        output: &mut Vec<u8>,
    ) {
        if arguments.len() < 3 {
            arity(command, output);
            return;
        }
        let flags = match ExpireFlags::parse(&arguments[3..]) {
            Ok(flags) => flags,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        let Some(value) = parse_integer(arguments[2]) else {
            output.extend_from_slice(NOT_INTEGER);
            return;
        };
        let now = self.clock.now_ms().max(0);
        let at = (if seconds {
            value.checked_mul(1000)
        } else {
            Some(value)
        })
        .and_then(|value| value.checked_add(if absolute { 0 } else { now }));
        let Some(at) = at else {
            write!(
                output,
                "-ERR invalid expire time in '{command}' command\r\n"
            )
            .expect("Vec write");
            return;
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        data.expire_key(arguments[1], now);
        let Some(entry) = data.entries.get(arguments[1]) else {
            integer(0, output);
            return;
        };
        let previous = entry.expires;
        if (flags.nx && previous.is_some())
            || (flags.xx && previous.is_none())
            || (flags.gt && previous.is_none_or(|old| at <= old))
            || (flags.lt && previous.is_some_and(|old| at >= old))
        {
            integer(0, output);
            return;
        }
        if at <= now {
            data.remove(arguments[1]);
        } else {
            data.set_expiry(arguments[1], Some(at));
        }
        integer(1, output);
    }

    pub(super) fn increment(&self, arguments: &[&[u8]], command: &str, output: &mut Vec<u8>) {
        let by = matches!(command, "incrby" | "decrby");
        if arguments.len() != if by { 3 } else { 2 } {
            arity(command, output);
            return;
        }
        let amount = if by {
            let Some(amount) = parse_integer(arguments[2]) else {
                output.extend_from_slice(NOT_INTEGER);
                return;
            };
            amount
        } else {
            1
        };
        let amount = if matches!(command, "decr" | "decrby") {
            let Some(amount) = amount.checked_neg() else {
                output.extend_from_slice(b"-ERR decrement would overflow\r\n");
                return;
            };
            amount
        } else {
            amount
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(arguments[1], now);
        let entry = data.entries.get(arguments[1]);
        if entry.is_some_and(|entry| !entry.is_string()) {
            wrong_type(output);
            return;
        }
        let value = match entry {
            Some(entry) => match parse_integer(&entry.value) {
                Some(value) => value,
                None => {
                    output.extend_from_slice(NOT_INTEGER);
                    return;
                }
            },
            None => 0,
        };
        let Some(value) = value.checked_add(amount) else {
            output.extend_from_slice(b"-ERR increment or decrement would overflow\r\n");
            return;
        };
        let expires = entry.and_then(|entry| entry.expires);
        let encoded = value.to_string();
        let previous = entry.map(Entry::payload_len);
        let Some(payload) = data.reserve(
            arguments[1],
            encoded.len(),
            previous,
            self.payload_limit(),
            now,
        ) else {
            output.extend_from_slice(OOM);
            return;
        };
        data.put(arguments[1], encoded.as_bytes(), expires, payload, now);
        integer(value, output);
    }
}
