use super::*;

impl Store {
    /// Execute keyspace commands; return false without touching output if unknown.
    pub(super) fn execute_keyspace(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> bool {
        let Some(command) = arguments.first() else {
            return false;
        };
        let (name, mode) = if command.eq_ignore_ascii_case(b"TYPE") {
            ("type", 0)
        } else if command.eq_ignore_ascii_case(b"RENAME") {
            ("rename", 1)
        } else if command.eq_ignore_ascii_case(b"RENAMENX") {
            ("renamenx", 2)
        } else if command.eq_ignore_ascii_case(b"TOUCH") {
            ("touch", 3)
        } else if command.eq_ignore_ascii_case(b"DBSIZE") {
            ("dbsize", 4)
        } else if command.eq_ignore_ascii_case(b"COPY") {
            ("copy", 5)
        } else {
            return false;
        };
        match mode {
            0 => {
                if arguments.len() != 2 {
                    arity(name, output);
                } else {
                    self.key_type(arguments[1], output);
                }
            }
            1 | 2 => {
                if arguments.len() != 3 {
                    arity(name, output);
                } else {
                    self.rename(arguments[1], arguments[2], mode == 2, output);
                }
            }
            5 => self.copy(arguments, output),
            3 => {
                if arguments.len() < 2 {
                    arity(name, output);
                } else {
                    self.touch(&arguments[1..], output);
                }
            }
            _ => {
                if arguments.len() != 1 {
                    arity(name, output);
                } else {
                    self.dbsize(output);
                }
            }
        }
        true
    }

    fn key_type(&self, key: &[u8], output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        if let Some(entry) = data.entries.get(key) {
            output.extend_from_slice(if entry.stream.is_some() {
                b"+stream\r\n"
            } else {
                b"+string\r\n"
            });
        } else {
            output.extend_from_slice(b"+none\r\n");
        }
    }

    fn touch(&self, keys: &[&[u8]], output: &mut Vec<u8>) {
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

    fn dbsize(&self, output: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.reap(now, usize::MAX);
        integer(data.entries.len() as i64, output);
    }

    fn copy(&self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() < 3 {
            arity("copy", output);
            return;
        }
        let mut replace = false;
        let mut i = 3;
        while i < arguments.len() {
            if arguments[i].eq_ignore_ascii_case(b"REPLACE") && !replace {
                replace = true;
                i += 1;
            } else if arguments[i].eq_ignore_ascii_case(b"DB")
                && i + 1 < arguments.len()
                && arguments[i + 1] == b"0"
            {
                i += 2;
            } else {
                output.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(arguments[1], now);
        data.expire_key(arguments[2], now);
        let Some(source) = data.entries.get(arguments[1]) else {
            integer(0, output);
            return;
        };
        if arguments[1] == arguments[2] {
            integer(0, output);
            return;
        }
        if data.entries.contains_key(arguments[2]) && !replace {
            integer(0, output);
            return;
        }
        let source_bytes = arguments[2].len().checked_add(source.payload_len);
        let destination_bytes = data
            .entries
            .get(arguments[2])
            .and_then(|e| arguments[2].len().checked_add(e.payload_len))
            .unwrap_or(0);
        let Some(total) = source_bytes
            .and_then(|n| {
                data.payload_bytes
                    .checked_sub(destination_bytes)
                    .and_then(|base| base.checked_sub(arguments[1].len()))
                    .and_then(|base| base.checked_add(n))
            })
            .filter(|n| *n <= self.max_payload_bytes)
        else {
            output.extend_from_slice(options::OOM);
            return;
        };
        let value = source.value.clone();
        let stream = source.stream.clone();
        let expires = source.expires;
        if replace {
            data.remove(arguments[2]);
        }
        if let Some(stream) = stream {
            data.put_stream(arguments[2], stream, expires, total);
        } else {
            data.put(arguments[2], &value, expires, total);
        }
        output.extend_from_slice(b"+OK\r\n");
    }

    fn rename(
        &self,
        source: &[u8],
        destination: &[u8],
        only_if_absent: bool,
        output: &mut Vec<u8>,
    ) {
        let Ok(mut data) = self.data.write() else {
            internal_error(output);
            return;
        };
        let now = self.now(&data);
        data.expire_key(source, now);
        if source == destination {
            if data.entries.contains_key(source) {
                if only_if_absent {
                    integer(0, output);
                } else {
                    output.extend_from_slice(b"+OK\r\n");
                }
            } else {
                output.extend_from_slice(b"-ERR no such key\r\n");
            }
            return;
        }
        data.expire_key(destination, now);
        let Some(entry) = data.entries.get(source) else {
            output.extend_from_slice(b"-ERR no such key\r\n");
            return;
        };
        let cloned_value = entry.value.clone();
        let cloned_stream = entry.stream.clone();
        let cloned_expiry = entry.expires;
        let cloned_payload_len = entry.payload_len;
        if only_if_absent && data.entries.contains_key(destination) {
            integer(0, output);
            return;
        }

        // Compute the final exact payload before removing either key. This makes
        // an over-budget longer destination an atomic failure.
        let source_bytes = source.len().checked_add(entry.payload_len);
        let destination_bytes = data
            .entries
            .get(destination)
            .and_then(|old| destination.len().checked_add(old.payload_len))
            .unwrap_or(0);
        let Some(payload) = source_bytes
            .and_then(|bytes| data.payload_bytes.checked_sub(bytes))
            .and_then(|remaining| remaining.checked_sub(destination_bytes))
            .and_then(|remaining| remaining.checked_add(destination.len()))
            .and_then(|remaining| remaining.checked_add(entry.payload_len))
            .filter(|bytes| *bytes <= self.max_payload_bytes)
        else {
            output.extend_from_slice(options::OOM);
            return;
        };

        data.remove(source).expect("source checked while locked");
        data.remove(destination);
        if let Some(stream) = cloned_stream {
            data.put_stream(destination, stream, cloned_expiry, payload);
        } else {
            debug_assert_eq!(cloned_payload_len, cloned_value.len());
            data.put(destination, &cloned_value, cloned_expiry, payload);
        }
        if only_if_absent {
            integer(1, output);
        } else {
            output.extend_from_slice(b"+OK\r\n");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};

    struct TestClock(AtomicI64);
    impl Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn call(store: &Store, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        assert!(store.execute_keyspace(args, &mut out));
        out
    }

    #[test]
    fn commands_are_binary_safe_and_touch_counts_duplicates() {
        let store = Store::new(100);
        assert_eq!(call(&store, &[b"TYPE", b"\xff\0"]), b"+none\r\n");
        store.set(
            b"\xff\0",
            b"v\0",
            SetOptions::default(),
            "set",
            false,
            &mut Vec::new(),
        );
        assert_eq!(call(&store, &[b"TYPE", b"\xff\0"]), b"+string\r\n");
        assert_eq!(
            call(&store, &[b"TOUCH", b"\xff\0", b"\xff\0", b"missing"]),
            b":2\r\n"
        );
        assert_eq!(call(&store, &[b"DBSIZE"]), b":1\r\n");
    }

    #[test]
    fn rename_preserves_ttl_and_oom_is_atomic() {
        let clock = Arc::new(TestClock(AtomicI64::new(1000)));
        let store = Store::with_clock(7, clock.clone());
        store.set(
            b"a",
            b"123",
            SetOptions {
                expiry: Some((ExpiryKind::Px, b"100")),
                ..SetOptions::default()
            },
            "set",
            false,
            &mut Vec::new(),
        );
        store.set(
            b"x",
            b"v",
            SetOptions::default(),
            "set",
            false,
            &mut Vec::new(),
        );
        assert!(call(&store, &[b"RENAME", b"a", b"long"]).starts_with(b"-OOM"));
        assert_eq!(store.data.read().unwrap().payload_bytes, 6);
        assert_eq!(call(&store, &[b"RENAME", b"a", b"b"]), b"+OK\r\n");
        assert_eq!(
            store
                .data
                .read()
                .unwrap()
                .entries
                .get(b"b".as_slice())
                .unwrap()
                .expires,
            Some(1100)
        );
        clock.0.store(1101, Ordering::SeqCst);
        assert_eq!(call(&store, &[b"DBSIZE"]), b":1\r\n");
    }

    #[test]
    fn rename_variants_handle_missing_existing_and_self_keys() {
        let store = Store::new(100);
        assert_eq!(
            call(&store, &[b"RENAMENX", b"missing", b"dst"]),
            b"-ERR no such key\r\n"
        );
        store.set(
            b"src",
            b"s",
            SetOptions::default(),
            "set",
            false,
            &mut Vec::new(),
        );
        store.set(
            b"dst",
            b"d",
            SetOptions::default(),
            "set",
            false,
            &mut Vec::new(),
        );
        assert_eq!(call(&store, &[b"RENAMENX", b"src", b"dst"]), b":0\r\n");
        assert_eq!(call(&store, &[b"RENAME", b"src", b"src"]), b"+OK\r\n");
        assert_eq!(call(&store, &[b"RENAMENX", b"src", b"src"]), b":0\r\n");
        assert_eq!(call(&store, &[b"RENAME", b"src", b"dst"]), b"+OK\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 4);
    }

    #[test]
    fn unknown_command_leaves_output_untouched() {
        let store = Store::new(10);
        let mut output = b"prefix".to_vec();
        assert!(!store.execute_keyspace(&[b"NOTKEYSPACE"], &mut output));
        assert_eq!(output, b"prefix");
    }
}
