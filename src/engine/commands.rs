use super::*;
use options::{Condition, ExpireFlags, NOT_INTEGER, OOM, SetOptions, parse_integer};

impl Store {
    pub(super) fn now(&self, data: &Data) -> i64 {
        if data.expirations.is_empty() {
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
        if data.entries.get(key).is_none_or(|entry| entry.alive(now)) {
            encode_bulk(
                data.entries.get(key).map(|entry| entry.value.as_slice()),
                output,
            );
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
        let allowed = match options.condition {
            Condition::Always => true,
            Condition::Missing => existing.is_none(),
            Condition::Present => existing.is_some(),
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
        let Some(payload) = data.reserve(key, value.len(), self.max_payload_bytes, now) else {
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
        data.put(key, value, expires, payload);
    }

    pub(super) fn getdel(&self, key: &[u8], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
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
        let Some(payload) = data.reserve(arguments[1], encoded.len(), self.max_payload_bytes, now)
        else {
            output.extend_from_slice(OOM);
            return;
        };
        data.put(arguments[1], encoded.as_bytes(), expires, payload);
        integer(value, output);
    }
}
