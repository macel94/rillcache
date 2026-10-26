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
                    .and_then(|base| base.checked_add(n))
            })
            .filter(|n| *n <= self.max_payload_bytes)
        else {
            output.extend_from_slice(options::OOM);
            return;
        };
        // COPY must duplicate state, but only after destination rejection and
        // final payload admission. Install the clone directly (no second copy).
        let copied = source.clone();
        data.put_owned(arguments[2], copied, total);
        integer(1, output);
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

        // Transfer ownership only after every rejection/admission check. In
        // particular, never deep-clone a stream tree just to reject RENAMENX or
        // return OOM. Moving also preserves the admitted string buffer capacity.
        let entry = data.remove(source).expect("source checked while locked");
        data.put_owned(destination, entry, payload);
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

    fn exec(store: &Store, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        assert_eq!(store.execute(args, &mut out), Action::Continue);
        out
    }

    fn seed(store: &Store, key: &[u8], stream: bool, deadline: &[u8]) {
        if stream {
            assert_eq!(
                exec(store, &[b"XADD", key, b"1", b"f", b"v\0"]),
                b"$3\r\n1-0\r\n"
            );
            assert_eq!(
                exec(store, &[b"XGROUP", b"CREATE", key, b"group", b"0"]),
                b"+OK\r\n"
            );
            assert!(
                exec(
                    store,
                    &[
                        b"XREADGROUP",
                        b"GROUP",
                        b"group",
                        b"consumer",
                        b"STREAMS",
                        key,
                        b">"
                    ]
                )
                .starts_with(b"*1\r\n")
            );
            assert_eq!(exec(store, &[b"PEXPIREAT", key, deadline]), b":1\r\n");
            // Legal spare capacities act as ownership sentinels. A deep clone
            // preserves contents but not these capacities; no allocator hook or
            // pointer arithmetic is needed to distinguish it from a move.
            let mut data = store.data.write().unwrap();
            let tree = data.entries.get_mut(key).unwrap().stream.as_mut().unwrap();
            for entry in tree.entries.values_mut() {
                entry.fields.reserve_exact(3);
                for (field, value) in &mut entry.fields {
                    field.reserve_exact(1);
                    value.reserve_exact(2);
                }
            }
            for group in tree.groups.values_mut() {
                for pending in group.pending.values_mut() {
                    pending.consumer.reserve_exact(5);
                }
                for consumer in group.consumers.values_mut() {
                    for pending in consumer.pending.values_mut() {
                        pending.consumer.reserve_exact(7);
                    }
                }
            }
        } else {
            // A normal shrink reuses capacity when it is at most twice len.
            assert_eq!(exec(store, &[b"SET", key, b"123456"]), b"+OK\r\n");
            assert_eq!(
                exec(store, &[b"SET", key, b"v\0x", b"PXAT", deadline]),
                b"+OK\r\n"
            );
        }
    }

    fn capacities(entry: &data::Entry) -> Vec<usize> {
        let mut sizes = vec![entry.value.capacity()];
        if let Some(tree) = &entry.stream {
            for item in tree.entries.values() {
                sizes.push(item.fields.capacity());
                for (field, value) in &item.fields {
                    sizes.extend([field.capacity(), value.capacity()]);
                }
            }
            for group in tree.groups.values() {
                for pending in group.pending.values() {
                    sizes.push(pending.consumer.capacity());
                }
                for consumer in group.consumers.values() {
                    for pending in consumer.pending.values() {
                        sizes.push(pending.consumer.capacity());
                    }
                }
            }
        }
        sizes
    }

    #[test]
    fn rename_moves_owned_string_and_stream_capacities_after_admission() {
        for typed in [false, true] {
            for command in [b"RENAME".as_slice(), b"RENAMENX"] {
                let clock = Arc::new(TestClock(AtomicI64::new(1000)));
                let store = Store::with_clock(7, clock.clone());
                seed(&store, b"a", typed, b"1100");
                let (before, buffers, index) = {
                    let data = store.data.read().unwrap();
                    let entry = &data.entries[b"a".as_slice()];
                    (entry.clone(), capacities(entry), data.expirations.clone())
                };
                // Admission is exact: six-byte key + three-byte payload is OOM.
                assert_eq!(exec(&store, &[command, b"a", b"longer"]), options::OOM);
                // Self-key paths must not touch ownership, TTL or accounting.
                assert_eq!(exec(&store, &[b"RENAME", b"a", b"a"]), b"+OK\r\n");
                assert_eq!(exec(&store, &[b"RENAMENX", b"a", b"a"]), b":0\r\n");
                {
                    let data = store.data.read().unwrap();
                    assert_eq!(data.entries.len(), 1);
                    assert_eq!(data.entries[b"a".as_slice()], before);
                    assert_eq!(capacities(&data.entries[b"a".as_slice()]), buffers);
                    assert_eq!(data.expirations, index);
                    assert_eq!(data.payload_bytes, 4);
                }
                let expected = if command == b"RENAME" {
                    b"+OK\r\n".as_slice()
                } else {
                    b":1\r\n"
                };
                // Four-byte destination + three-byte payload exactly fits.
                assert_eq!(exec(&store, &[command, b"a", b"long"]), expected);
                {
                    let data = store.data.read().unwrap();
                    assert_eq!(data.entries.len(), 1);
                    assert_eq!(data.entries[b"long".as_slice()], before);
                    assert_eq!(capacities(&data.entries[b"long".as_slice()]), buffers);
                    assert_eq!(data.payload_bytes, 7);
                    assert_eq!(data.expirations, [(1100, b"long".to_vec())].into());
                }
                clock.0.store(1100, Ordering::SeqCst);
                assert_eq!(exec(&store, &[b"PTTL", b"long"]), b":0\r\n");
                clock.0.store(1101, Ordering::SeqCst);
                assert_eq!(call(&store, &[b"DBSIZE"]), b":0\r\n");
                let data = store.data.read().unwrap();
                assert_eq!(data.payload_bytes, 0);
                assert!(data.expirations.is_empty());
            }
        }
    }

    #[test]
    fn rename_nx_rejection_and_replacement_preserve_owned_state_and_expiry_index() {
        for typed in [false, true] {
            let clock = Arc::new(TestClock(AtomicI64::new(1000)));
            let store = Store::with_clock(12, clock.clone());
            seed(&store, b"a", typed, b"1200");
            seed(&store, b"b", !typed, b"1100");
            let (source, buffers, destination, index) = {
                let data = store.data.read().unwrap();
                (
                    data.entries[b"a".as_slice()].clone(),
                    capacities(&data.entries[b"a".as_slice()]),
                    data.entries[b"b".as_slice()].clone(),
                    data.expirations.clone(),
                )
            };
            assert_eq!(call(&store, &[b"RENAMENX", b"a", b"b"]), b":0\r\n");
            {
                let data = store.data.read().unwrap();
                assert_eq!(data.entries[b"a".as_slice()], source);
                assert_eq!(capacities(&data.entries[b"a".as_slice()]), buffers);
                assert_eq!(data.entries[b"b".as_slice()], destination);
                assert_eq!(data.payload_bytes, 8);
                assert_eq!(data.expirations, index);
            }
            assert_eq!(call(&store, &[b"RENAME", b"a", b"b"]), b"+OK\r\n");
            {
                let data = store.data.read().unwrap();
                assert_eq!(data.entries.len(), 1);
                assert_eq!(data.entries[b"b".as_slice()], source);
                assert_eq!(capacities(&data.entries[b"b".as_slice()]), buffers);
                assert_eq!(data.payload_bytes, 4);
                assert_eq!(data.expirations, [(1200, b"b".to_vec())].into());
            }
            clock.0.store(1101, Ordering::SeqCst);
            assert_eq!(call(&store, &[b"DBSIZE"]), b":1\r\n");
            clock.0.store(1201, Ordering::SeqCst);
            assert_eq!(call(&store, &[b"DBSIZE"]), b":0\r\n");
        }
    }

    #[test]
    fn expired_destination_is_absent_but_expired_source_is_not_renamed() {
        let clock = Arc::new(TestClock(AtomicI64::new(1000)));
        let store = Store::with_clock(12, clock.clone());
        seed(&store, b"a", true, b"1200");
        seed(&store, b"b", false, b"1100");
        clock.0.store(1101, Ordering::SeqCst);
        assert_eq!(call(&store, &[b"RENAMENX", b"a", b"b"]), b":1\r\n");
        assert_eq!(exec(&store, &[b"PEXPIRETIME", b"b"]), b":1200\r\n");
        clock.0.store(1201, Ordering::SeqCst);
        assert_eq!(
            call(&store, &[b"RENAME", b"b", b"a"]),
            b"-ERR no such key\r\n"
        );
        let data = store.data.read().unwrap();
        assert!(data.entries.is_empty());
        assert!(data.expirations.is_empty());
        assert_eq!(data.payload_bytes, 0);
    }

    #[test]
    fn copy_admission_replace_and_expiry_keep_an_independent_value() {
        for typed in [false, true] {
            let clock = Arc::new(TestClock(AtomicI64::new(1000)));
            let store = Store::with_clock(12, clock.clone());
            seed(&store, b"a", typed, b"1200");
            seed(&store, b"b", !typed, b"1100");
            let before = store.data.read().unwrap().entries.clone();
            let index = store.data.read().unwrap().expirations.clone();
            assert_eq!(call(&store, &[b"COPY", b"a", b"longer"]), options::OOM);
            assert_eq!(call(&store, &[b"COPY", b"a", b"b"]), b":0\r\n");
            {
                let data = store.data.read().unwrap();
                assert_eq!(data.entries, before);
                assert_eq!(data.expirations, index);
                assert_eq!(data.payload_bytes, 8);
            }
            assert_eq!(call(&store, &[b"COPY", b"a", b"b", b"REPLACE"]), b":1\r\n");
            {
                let data = store.data.read().unwrap();
                assert_eq!(data.entries[b"a".as_slice()], data.entries[b"b".as_slice()]);
                assert_eq!(data.payload_bytes, 8);
                assert_eq!(
                    data.expirations,
                    [(1200, b"a".to_vec()), (1200, b"b".to_vec())].into()
                );
            }
            if typed {
                assert_eq!(exec(&store, &[b"XDEL", b"a", b"1"]), b":1\r\n");
                assert_eq!(exec(&store, &[b"XLEN", b"b"]), b":1\r\n");
            } else {
                assert_eq!(exec(&store, &[b"SET", b"a", b"new"]), b"+OK\r\n");
                assert_eq!(exec(&store, &[b"GET", b"b"]), b"$3\r\nv\0x\r\n");
            }
            assert_eq!(
                store.data.read().unwrap().entries[b"b".as_slice()],
                before[b"a".as_slice()]
            );
            // Expiring the destination's old deadline must not remove the copy.
            clock.0.store(1101, Ordering::SeqCst);
            assert_eq!(exec(&store, &[b"PEXPIRETIME", b"b"]), b":1200\r\n");
            clock.0.store(1201, Ordering::SeqCst);
            assert_eq!(call(&store, &[b"TYPE", b"b"]), b"+none\r\n");
        }
    }

    #[test]
    fn unknown_command_leaves_output_untouched() {
        let store = Store::new(10);
        let mut output = b"prefix".to_vec();
        assert!(!store.execute_keyspace(&[b"NOTKEYSPACE"], &mut output));
        assert_eq!(output, b"prefix");
    }
}
