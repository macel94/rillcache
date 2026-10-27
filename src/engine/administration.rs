use super::*;

impl Store {
    pub(super) fn execute_admin(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let Some(command) = args.first() else {
            return false;
        };
        if command.first().is_some_and(|byte| {
            byte.eq_ignore_ascii_case(&b't') || byte.eq_ignore_ascii_case(&b'o')
        }) && self.execute_introspection(args, out)
        {
            return true;
        }
        if command.eq_ignore_ascii_case(b"FLUSHDB") || command.eq_ignore_ascii_case(b"FLUSHALL") {
            if args.len() > 2 || args.get(1).is_some_and(|arg| !flush_mode(arg)) {
                out.extend_from_slice(b"-ERR syntax error\r\n");
            } else if let Ok(mut data) = self.data.write() {
                data.flush();
                out.extend_from_slice(b"+OK\r\n");
            } else {
                internal_error(out);
            }
            return true;
        }
        if command.eq_ignore_ascii_case(b"SHUTDOWN") {
            self.shutdown_command(args, out);
            return true;
        }
        if command.eq_ignore_ascii_case(b"CONFIG") {
            config(self, args, out);
            return true;
        }
        if self.execute_slowlog(args, out) || self.execute_latency(args, out) {
            return true;
        }
        if command.eq_ignore_ascii_case(b"SCRIPT") || command.eq_ignore_ascii_case(b"FUNCTION") {
            let name = if command.eq_ignore_ascii_case(b"SCRIPT") {
                "script"
            } else {
                "function"
            };
            let valid = args
                .get(1)
                .is_some_and(|sub| sub.eq_ignore_ascii_case(b"FLUSH"))
                && args.len() <= 3
                && args.get(2).is_none_or(|arg| flush_mode(arg));
            if valid {
                // No script/function registry or execution capability exists in this server.
                out.extend_from_slice(b"+OK\r\n");
            } else if args.get(1).is_none() {
                arity(name, out);
            } else {
                out.extend_from_slice(b"-ERR unsupported subcommand or syntax error\r\n");
            }
            return true;
        }
        if command.eq_ignore_ascii_case(b"MEMORY") {
            self.memory(args, out);
            return true;
        }
        if command.eq_ignore_ascii_case(b"BGREWRITEAOF") {
            if args.len() != 1 {
                arity("bgrewriteaof", out);
            } else {
                match self.rewrite_journal() {
                    Ok(_) => out.extend_from_slice(b"+OK\r\n"),
                    Err(()) => out.extend_from_slice(
                        b"-ERR append-only journal is not enabled or cannot be rewritten\r\n",
                    ),
                }
            }
            return true;
        }
        false
    }

    fn memory(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some(subcommand) = args.get(1) else {
            out.extend_from_slice(b"-ERR unsupported MEMORY subcommand\r\n");
            return;
        };
        if subcommand.eq_ignore_ascii_case(b"USAGE") {
            if args.len() != 3 {
                arity("memory|usage", out);
                return;
            }
            let Ok(mut data) = self.data.write() else {
                internal_error(out);
                return;
            };
            let now = self.now(&data);
            data.expire_key(args[2], now);
            if let Some(entry) = data.entries.get(args[2]) {
                // Deterministic logical estimate, not allocator/RSS data.
                let header = if usize::BITS == 64 { 16usize } else { 12usize };
                let value_bytes = Some(entry.payload_len());
                if let Some(size) = value_bytes.and_then(|value_bytes| {
                    header
                        .checked_add(args[2].len())
                        .and_then(|n| n.checked_add(value_bytes))
                }) {
                    let _ = write!(out, ":{size}\r\n");
                } else {
                    out.extend_from_slice(b"-ERR memory size overflow\r\n");
                }
            } else {
                out.extend_from_slice(b"$-1\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"STATS") {
            if args.len() != 2 {
                arity("memory|stats", out);
                return;
            }
            self.memory_stats(out);
        } else if subcommand.eq_ignore_ascii_case(b"DOCTOR") {
            if args.len() != 2 {
                arity("memory|doctor", out);
                return;
            }
            self.memory_doctor(out);
        } else if subcommand.eq_ignore_ascii_case(b"HELP") {
            if args.len() != 2 {
                arity("memory|help", out);
                return;
            }
            const HELP: &[&[u8]] = &[
                b"DOCTOR",
                b"    Return a human-readable payload report.",
                b"HELP",
                b"    Show this help.",
                b"STATS",
                b"    Return payload accounting metrics.",
                b"USAGE <key>",
                b"    Estimate the payload bytes used by a key.",
            ];
            let _ = write!(out, "*{}\r\n", HELP.len());
            for line in HELP {
                encode_bulk(Some(line), out);
            }
        } else {
            out.extend_from_slice(b"-ERR unsupported MEMORY subcommand\r\n");
        }
    }

    fn memory_snapshot(&self) -> Option<MemorySnapshot> {
        let data = self.data.read().ok()?;
        let now = self.now(&data);
        let mut keys = 0usize;
        let mut expires = 0usize;
        for entry in data.entries.values() {
            if entry.alive(now) {
                keys += 1;
                if entry.expires.is_some() {
                    expires += 1;
                }
            }
        }
        Some(MemorySnapshot {
            payload: data.payload_bytes,
            peak: data.peak_payload_bytes(),
            keys,
            expires,
            limit: self.payload_limit(),
            policy: data.eviction,
        })
    }

    fn memory_stats(&self, out: &mut Vec<u8>) {
        let Some(snapshot) = self.memory_snapshot() else {
            internal_error(out);
            return;
        };
        let percentage = memory_percentage(snapshot.payload, snapshot.limit);
        let _ = write!(out, "*14\r\n");
        encode_bulk(Some(b"peak.allocated"), out);
        let _ = write!(out, ":{}\r\n", snapshot.peak);
        encode_bulk(Some(b"total.allocated"), out);
        let _ = write!(out, ":{}\r\n", snapshot.payload);
        encode_bulk(Some(b"keys.count"), out);
        let _ = write!(out, ":{}\r\n", snapshot.keys);
        encode_bulk(Some(b"dataset.bytes"), out);
        let _ = write!(out, ":{}\r\n", snapshot.payload);
        encode_bulk(Some(b"dataset.percentage"), out);
        encode_bulk(Some(format!("{percentage:.2}").as_bytes()), out);
        encode_bulk(Some(b"rillcache.payload.limit_bytes"), out);
        let _ = write!(out, ":{}\r\n", snapshot.limit);
        encode_bulk(Some(b"rillcache.payload.scope"), out);
        encode_bulk(Some(MEMORY_SCOPE), out);
    }

    fn memory_doctor(&self, out: &mut Vec<u8>) {
        let Some(snapshot) = self.memory_snapshot() else {
            internal_error(out);
            return;
        };
        let percentage = memory_percentage(snapshot.payload, snapshot.limit);
        let text = if snapshot.keys == 0 && snapshot.payload == 0 {
            format!(
                "Rillcache payload is empty: 0 bytes across 0 live keys. {}",
                MEMORY_SCOPE_SENTENCE
            )
        } else if percentage >= 90.0 && snapshot.policy == EvictionPolicy::NoEviction {
            format!(
                "Payload pressure is high: {} bytes across {} live keys ({percentage:.1}% of the {} byte limit) with noeviction; new writes may fail with OOM. {}",
                snapshot.payload, snapshot.keys, snapshot.limit, MEMORY_SCOPE_SENTENCE
            )
        } else if percentage >= 75.0 {
            format!(
                "Payload is using {:.1} of the {} byte limit across {} live keys. Monitor growth; {}",
                percentage, snapshot.limit, snapshot.keys, MEMORY_SCOPE_SENTENCE
            )
        } else {
            format!(
                "Payload is using {:.1} of the {} byte limit across {} live keys ({} with a deadline); no payload pressure detected. {}",
                percentage, snapshot.limit, snapshot.keys, snapshot.expires, MEMORY_SCOPE_SENTENCE
            )
        };
        encode_bulk(Some(text.as_bytes()), out);
    }
}

struct MemorySnapshot {
    payload: usize,
    peak: usize,
    keys: usize,
    expires: usize,
    limit: usize,
    policy: EvictionPolicy,
}

const MEMORY_SCOPE: &[u8] = b"key_and_value_bytes_not_RSS";
const MEMORY_SCOPE_SENTENCE: &str =
    "This report uses key and value payload accounting, not RSS or allocator metrics.";

fn memory_percentage(payload: usize, limit: usize) -> f64 {
    if limit == 0 {
        return 0.0;
    }
    payload as f64 * 100.0 / limit as f64
}

fn flush_mode(arg: &[u8]) -> bool {
    arg.eq_ignore_ascii_case(b"SYNC") || arg.eq_ignore_ascii_case(b"ASYNC")
}

impl Store {
    /// Parse SHUTDOWN, force a journal flush and, on success, request stop.
    /// Returns without a success reply; the session closes the caller.
    pub(crate) fn shutdown_command(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let mut nosave = false;
        let mut save = false;
        let mut abort = false;
        for argument in &args[1..] {
            if argument.eq_ignore_ascii_case(b"NOSAVE") {
                nosave = true;
            } else if argument.eq_ignore_ascii_case(b"SAVE") {
                save = true;
            } else if argument.eq_ignore_ascii_case(b"NOW")
                || argument.eq_ignore_ascii_case(b"FORCE")
            {
                // Shutdown is immediate in this server; these modifiers are no-ops.
            } else if argument.eq_ignore_ascii_case(b"ABORT") {
                abort = true;
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        if (abort && (nosave || save || args.len() > 2)) || (nosave && save) {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        if abort {
            out.extend_from_slice(b"-ERR No shutdown in progress.\r\n");
            return;
        }
        if self.journal_flush().is_err() {
            out.extend_from_slice(b"-ERR append-only journal sync failed\r\n");
            return;
        }
        self.request_shutdown();
    }
}

fn config(store: &Store, args: &[&[u8]], out: &mut Vec<u8>) {
    let Some(sub) = args.get(1) else {
        arity("config", out);
        return;
    };
    if sub.eq_ignore_ascii_case(b"GET") {
        if args.len() != 3 {
            arity("config|get", out);
            return;
        }
        // No Redis CONFIG runtime settings are implemented.
        out.extend_from_slice(b"*0\r\n");
    } else if sub.eq_ignore_ascii_case(b"SET") {
        if args.len() < 4 || !args.len().is_multiple_of(2) {
            out.extend_from_slice(b"-ERR syntax error\r\n");
        } else {
            out.extend_from_slice(b"-ERR CONFIG SET option is unsupported\r\n");
        }
    } else if sub.eq_ignore_ascii_case(b"RESETSTAT") {
        if args.len() != 2 {
            arity("config|resetstat", out);
        } else if store.reset_stats() {
            out.extend_from_slice(b"+OK\r\n");
        } else {
            internal_error(out);
        }
    } else {
        out.extend_from_slice(b"-ERR unsupported CONFIG subcommand\r\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(store: &Store, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        assert_eq!(store.execute(args, &mut out), Action::Continue);
        out
    }

    #[test]
    fn flush_is_atomic_and_clears_payload_and_expiry_index() {
        let store = Store::new(100);
        assert_eq!(
            run(&store, &[b"SET", b"one", b"v", b"PX", b"50000"]),
            b"+OK\r\n"
        );
        assert_eq!(
            run(&store, &[b"SET", b"two", b"binary\0", b"EX", b"50"]),
            b"+OK\r\n"
        );
        assert_eq!(run(&store, &[b"FLUSHDB", b"ASYNC"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"DBSIZE"]), b":0\r\n");
        let data = store.data.read().unwrap();
        assert_eq!(data.payload_bytes, 0);
        assert!(data.entries.is_empty());
        assert!(data.expirations.is_empty());
        drop(data);
        assert_eq!(run(&store, &[b"SET", b"after", b"flush"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"FLUSHALL"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"DBSIZE"]), b":0\r\n");
    }

    #[test]
    fn malformed_flush_preserves_state_and_registry_flush_arity_is_strict() {
        let store = Store::new(100);
        run(&store, &[b"SET", b"k", b"v"]);
        for args in [
            &[b"FLUSHDB".as_slice(), b"BAD"][..],
            &[b"FLUSHALL".as_slice(), b"SYNC", b"ASYNC"][..],
        ] {
            assert_eq!(run(&store, args), b"-ERR syntax error\r\n");
            assert_eq!(run(&store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
        }
        assert_eq!(run(&store, &[b"SCRIPT", b"FLUSH", b"SYNC"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"FUNCTION", b"FLUSH", b"ASYNC"]), b"+OK\r\n");
        assert!(run(&store, &[b"SCRIPT", b"FLUSH", b"BAD"]).starts_with(b"-ERR"));
        assert!(run(&store, &[b"FUNCTION", b"LOAD"]).starts_with(b"-ERR"));
    }

    #[test]
    fn config_and_memory_responses_are_honest_and_memory_accounts_expiry() {
        let store = Store::new(100);
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"unsupported-setting"]),
            b"*0\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"set-max-intset-entries"]),
            b"*2\r\n$22\r\nset-max-intset-entries\r\n$3\r\n512\r\n"
        );
        assert!(run(&store, &[b"CONFIG", b"SET", b"maxmemory", b"0"]).starts_with(b"-ERR"));
        assert_eq!(run(&store, &[b"CONFIG", b"RESETSTAT"]), b"+OK\r\n");
        let info = run(&store, &[b"INFO", b"server"]);
        let bits = format!("arch_bits:{}\r", usize::BITS);
        assert!(info.windows(bits.len()).any(|part| part == bits.as_bytes()));
        assert!(
            info.windows(b"mem_allocator:system\r".len())
                .any(|part| part == b"mem_allocator:system\r")
        );
        assert_eq!(run(&store, &[b"MEMORY", b"USAGE", b"missing"]), b"$-1\r\n");
        run(&store, &[b"SET", b"key", b"value"]);
        assert_eq!(
            run(&store, &[b"MEMORY", b"USAGE", b"key"]),
            format!(":{}\r\n", usize::BITS as usize / 4 + 8).as_bytes()
        );
        assert_eq!(
            run(&store, &[b"MEMORY", b"USAGE", b"key", b"SAMPLES", b"0"]),
            b"-ERR wrong number of arguments for 'memory|usage' command\r\n"
        );
    }

    #[test]
    fn memory_stats_and_doctor_report_payload_high_water() {
        let store = Store::new(100);
        let empty = run(&store, &[b"MEMORY", b"DOCTOR"]);
        assert!(
            empty
                .windows(b"payload is empty".len())
                .any(|part| part == b"payload is empty")
        );
        let empty_stats = run(&store, &[b"MEMORY", b"STATS"]);
        for field in [
            b"peak.allocated".as_slice(),
            b"total.allocated",
            b"keys.count",
            b"dataset.bytes",
            b"dataset.percentage",
            b"rillcache.payload.limit_bytes",
            b"rillcache.payload.scope",
        ] {
            assert!(
                empty_stats.windows(field.len()).any(|part| part == field),
                "missing {field:?}"
            );
        }
        let value = vec![b'v'; 90];
        assert_eq!(run(&store, &[b"SET", b"key", &value]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"DEL", b"key"]), b":1\r\n");
        {
            let data = store.data.read().unwrap();
            assert_eq!(data.payload_bytes, 0);
            assert!(data.peak_payload_bytes() > 0);
        }
        let stats = run(&store, &[b"MEMORY", b"STATS"]);
        let text = String::from_utf8_lossy(&stats);
        assert!(text.contains("$14\r\npeak.allocated\r\n:"), "{text}");
        assert!(text.contains("$15\r\ntotal.allocated\r\n:0\r\n"), "{text}");
        assert!(text.contains("key_and_value_bytes_not_RSS"), "{text}");

        assert_eq!(run(&store, &[b"SET", b"key", &value]), b"+OK\r\n");
        let doctor = run(&store, &[b"MEMORY", b"DOCTOR"]);
        let text = String::from_utf8_lossy(&doctor);
        assert!(text.contains("Payload pressure is high"), "{text}");
        assert!(text.contains("not RSS"), "{text}");
        let help = run(&store, &[b"MEMORY", b"HELP"]);
        assert!(help.windows(b"STATS".len()).any(|part| part == b"STATS"));
        assert!(run(&store, &[b"MEMORY", b"STATS", b"extra"]).starts_with(b"-ERR"));
    }
}
