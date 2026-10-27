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
        if command.eq_ignore_ascii_case(b"CONFIG") {
            config(self, args, out);
            return true;
        }
        if self.execute_slowlog(args, out) {
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
        if !args
            .get(1)
            .is_some_and(|sub| sub.eq_ignore_ascii_case(b"USAGE"))
        {
            out.extend_from_slice(b"-ERR unsupported MEMORY subcommand\r\n");
            return;
        }
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
    }
}

fn flush_mode(arg: &[u8]) -> bool {
    arg.eq_ignore_ascii_case(b"SYNC") || arg.eq_ignore_ascii_case(b"ASYNC")
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
}
