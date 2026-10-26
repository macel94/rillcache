//! Runtime collection encoding thresholds exposed through CONFIG.
use super::{Store, keyspace::glob_match};
use std::io::Write;

#[derive(Clone, Copy, Debug)]
pub(super) struct CollectionConfig {
    pub list_max_listpack_size: i64,
    pub hash_max_listpack_entries: usize,
    pub hash_max_listpack_value: usize,
    pub set_max_intset_entries: usize,
    pub set_max_listpack_entries: usize,
    pub set_max_listpack_value: usize,
    pub zset_max_listpack_entries: usize,
    pub zset_max_listpack_value: usize,
}

impl Default for CollectionConfig {
    fn default() -> Self {
        Self {
            list_max_listpack_size: -2,
            hash_max_listpack_entries: 512,
            hash_max_listpack_value: 64,
            set_max_intset_entries: 512,
            set_max_listpack_entries: 128,
            set_max_listpack_value: 64,
            zset_max_listpack_entries: 128,
            zset_max_listpack_value: 64,
        }
    }
}

const CANONICAL: &[&[u8]] = &[
    b"hash-max-listpack-entries",
    b"hash-max-listpack-value",
    b"list-max-listpack-size",
    b"set-max-intset-entries",
    b"set-max-listpack-entries",
    b"set-max-listpack-value",
    b"zset-max-listpack-entries",
    b"zset-max-listpack-value",
];

const ALIASES: &[(&[u8], &[u8])] = &[
    (b"list-max-ziplist-size", b"list-max-listpack-size"),
    (b"hash-max-ziplist-entries", b"hash-max-listpack-entries"),
    (b"hash-max-ziplist-value", b"hash-max-listpack-value"),
    (b"zset-max-ziplist-entries", b"zset-max-listpack-entries"),
    (b"zset-max-ziplist-value", b"zset-max-listpack-value"),
];

fn canonical_name(name: &[u8]) -> Option<&'static [u8]> {
    CANONICAL
        .iter()
        .copied()
        .find(|canonical| name.eq_ignore_ascii_case(canonical))
        .or_else(|| {
            ALIASES
                .iter()
                .find(|(alias, _)| name.eq_ignore_ascii_case(alias))
                .map(|(_, canonical)| *canonical)
        })
}

impl CollectionConfig {
    fn value(self, name: &[u8]) -> Option<String> {
        let canonical = canonical_name(name)?;
        let value = match canonical {
            b"list-max-listpack-size" => self.list_max_listpack_size,
            b"hash-max-listpack-entries" => self.hash_max_listpack_entries as i64,
            b"hash-max-listpack-value" => self.hash_max_listpack_value as i64,
            b"set-max-intset-entries" => self.set_max_intset_entries as i64,
            b"set-max-listpack-entries" => self.set_max_listpack_entries as i64,
            b"set-max-listpack-value" => self.set_max_listpack_value as i64,
            b"zset-max-listpack-entries" => self.zset_max_listpack_entries as i64,
            b"zset-max-listpack-value" => self.zset_max_listpack_value as i64,
            _ => return None,
        };
        Some(value.to_string())
    }

    fn set(&mut self, name: &[u8], raw: &[u8]) -> Result<(), ()> {
        let Some(canonical) = canonical_name(name) else {
            return Err(());
        };
        let Some(value) = super::options::parse_integer(raw) else {
            return Err(());
        };
        match canonical {
            b"list-max-listpack-size" if i32::try_from(value).is_ok() => {
                self.list_max_listpack_size = value;
            }
            b"hash-max-listpack-entries" if value >= 0 && usize::try_from(value).is_ok() => {
                self.hash_max_listpack_entries = value as usize;
            }
            b"hash-max-listpack-value" if value >= 0 && usize::try_from(value).is_ok() => {
                self.hash_max_listpack_value = value as usize;
            }
            b"set-max-intset-entries" if value >= 0 && usize::try_from(value).is_ok() => {
                self.set_max_intset_entries = value as usize;
            }
            b"set-max-listpack-entries" if value >= 0 && usize::try_from(value).is_ok() => {
                self.set_max_listpack_entries = value as usize;
            }
            b"set-max-listpack-value" if value >= 0 && usize::try_from(value).is_ok() => {
                self.set_max_listpack_value = value as usize;
            }
            b"zset-max-listpack-entries" if value >= 0 && usize::try_from(value).is_ok() => {
                self.zset_max_listpack_entries = value as usize;
            }
            b"zset-max-listpack-value" if value >= 0 && usize::try_from(value).is_ok() => {
                self.zset_max_listpack_value = value as usize;
            }
            _ => return Err(()),
        }
        Ok(())
    }
}

impl Store {
    pub(super) fn execute_collection_config(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        if !args
            .first()
            .is_some_and(|arg| arg.eq_ignore_ascii_case(b"CONFIG"))
        {
            return false;
        }
        match args.get(1) {
            Some(sub) if sub.eq_ignore_ascii_case(b"GET") => {
                self.config_get(args, out);
                true
            }
            Some(sub) if sub.eq_ignore_ascii_case(b"SET") => {
                self.config_set(args, out);
                true
            }
            _ => false,
        }
    }

    fn config_get(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if args.len() < 3 {
            out.extend_from_slice(b"-ERR wrong number of arguments for 'config|get' command\r\n");
            return;
        }
        let Ok(config) = self.collection_config.read() else {
            super::internal_error(out);
            return;
        };
        let mut matches: Vec<(&[u8], String)> = Vec::new();
        for pattern in &args[2..] {
            let folded_pattern: Vec<u8> = pattern.iter().map(u8::to_ascii_lowercase).collect();
            let wildcard = pattern
                .iter()
                .any(|byte| matches!(*byte, b'*' | b'?' | b'['));
            let wants_password = if wildcard {
                glob_match(&folded_pattern, b"requirepass")
            } else {
                pattern.eq_ignore_ascii_case(b"requirepass")
            };
            let wants_policy = if wildcard {
                glob_match(&folded_pattern, b"maxmemory-policy")
            } else {
                pattern.eq_ignore_ascii_case(b"maxmemory-policy")
            };
            if wants_policy {
                matches.push((
                    b"maxmemory-policy",
                    self.eviction_policy().name().to_string(),
                ));
            }
            if wants_password {
                // Never return the secret. The empty value means unset; any
                // configured password is reported as a visible redaction.
                matches.push((
                    b"requirepass",
                    if self.requirepass_configured() {
                        "<redacted>".to_string()
                    } else {
                        String::new()
                    },
                ));
            }
            if !pattern
                .iter()
                .any(|byte| matches!(*byte, b'*' | b'?' | b'['))
            {
                if let Some(value) = config.value(pattern)
                    && !matches
                        .iter()
                        .any(|(existing, _)| existing.eq_ignore_ascii_case(pattern))
                {
                    matches.push((pattern, value));
                }
                continue;
            }
            for name in CANONICAL
                .iter()
                .copied()
                .chain(ALIASES.iter().map(|(alias, _)| *alias))
            {
                if glob_match(&folded_pattern, name)
                    && !matches
                        .iter()
                        .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
                {
                    matches.push((name, config.value(name).expect("known threshold")));
                }
            }
        }
        matches.sort_unstable_by(|a, b| a.0.cmp(b.0));
        let _ = write!(out, "*{}\r\n", matches.len() * 2);
        for (name, value) in matches {
            super::encode_bulk(Some(name), out);
            super::encode_bulk(Some(value.as_bytes()), out);
        }
    }

    fn config_set(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if args.len() < 4 || !(args.len() - 2).is_multiple_of(2) {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        let Ok(mut config) = self.collection_config.write() else {
            super::internal_error(out);
            return;
        };
        let mut candidate = *config;
        let mut password: Option<Option<Vec<u8>>> = None;
        let mut policy = None;
        for pair in args[2..].as_chunks::<2>().0 {
            if pair[0].eq_ignore_ascii_case(b"maxmemory-policy") {
                let Some(parsed) = std::str::from_utf8(pair[1])
                    .ok()
                    .and_then(super::EvictionPolicy::parse)
                else {
                    out.extend_from_slice(b"-ERR invalid maxmemory-policy\r\n");
                    return;
                };
                policy = Some(parsed);
            } else if pair[0].eq_ignore_ascii_case(b"requirepass") {
                if pair[1].len() > 1024 {
                    out.extend_from_slice(b"-ERR requirepass must be at most 1024 bytes\r\n");
                    return;
                }
                password = Some((!pair[1].is_empty()).then(|| pair[1].to_vec()));
            } else if candidate.set(pair[0], pair[1]).is_err() {
                out.extend_from_slice(b"-ERR CONFIG SET option is unsupported or invalid\r\n");
                return;
            }
        }
        *config = candidate;
        if let Some(password) = password
            && !self.set_requirepass(password.as_deref())
        {
            super::internal_error(out);
            return;
        }
        if let Some(policy) = policy
            && !self.set_eviction_policy(policy)
        {
            super::internal_error(out);
            return;
        }
        out.extend_from_slice(b"+OK\r\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(store: &Store, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        assert!(store.execute_collection_config(args, &mut out));
        out
    }

    #[test]
    fn thresholds_choose_compact_and_expanded_storage_without_changing_values() {
        use super::super::data::Collection;
        let store = Store::new(20_000);
        let call = |args: &[&[u8]]| {
            let mut out = Vec::new();
            store.execute(args, &mut out);
            out
        };
        call(&[b"SADD", b"s", b"1", b"2"]);
        call(&[b"HSET", b"h", b"field", b"value"]);
        call(&[b"ZADD", b"z", b"1", b"member"]);
        call(&[b"RPUSH", b"l", b"small"]);
        {
            let data = store.data.read().unwrap();
            assert!(matches!(
                data.entries[b"s".as_slice()].collection.as_ref(),
                Some(Collection::SetInt(_))
            ));
            assert!(matches!(
                data.entries[b"h".as_slice()].collection.as_ref(),
                Some(Collection::HashPack(_))
            ));
            assert!(matches!(
                data.entries[b"z".as_slice()].collection.as_ref(),
                Some(Collection::ZSetPack(_))
            ));
            assert!(matches!(
                data.entries[b"l".as_slice()].collection.as_ref(),
                Some(Collection::ListPack(_))
            ));
        }
        assert_eq!(
            call(&[
                b"CONFIG",
                b"SET",
                b"list-max-listpack-size",
                b"-1",
                b"hash-max-listpack-entries",
                b"0",
                b"set-max-intset-entries",
                b"0",
                b"set-max-listpack-entries",
                b"0",
                b"zset-max-listpack-entries",
                b"0"
            ]),
            b"+OK\r\n"
        );
        call(&[b"HSET", b"h", b"next", b"value"]);
        call(&[b"SADD", b"s", b"3"]);
        call(&[b"ZADD", b"z", b"2", b"other"]);
        call(&[b"RPUSH", b"l", &vec![b'x'; 5000]]);
        {
            let data = store.data.read().unwrap();
            assert!(matches!(
                data.entries[b"s".as_slice()].collection.as_ref(),
                Some(Collection::Set(_))
            ));
            assert!(matches!(
                data.entries[b"h".as_slice()].collection.as_ref(),
                Some(Collection::Hash(_))
            ));
            assert!(matches!(
                data.entries[b"z".as_slice()].collection.as_ref(),
                Some(Collection::ZSet(_))
            ));
            assert!(matches!(
                data.entries[b"l".as_slice()].collection.as_ref(),
                Some(Collection::ListQuick(_))
            ));
        }
        assert_eq!(call(&[b"HGET", b"h", b"field"]), b"$5\r\nvalue\r\n");
        assert_eq!(
            call(&[b"SMEMBERS", b"s"]),
            b"*3\r\n$1\r\n1\r\n$1\r\n2\r\n$1\r\n3\r\n"
        );
        assert_eq!(call(&[b"ZCARD", b"z"]), b":2\r\n");
        assert_eq!(call(&[b"LLEN", b"l"]), b":2\r\n");
    }

    #[test]
    fn config_get_matches_legacy_aliases_and_deduplicates_patterns() {
        let store = Store::new(100);
        assert_eq!(
            run(
                &store,
                &[
                    b"CONFIG",
                    b"GET",
                    b"list-max-ziplist-size",
                    b"list-max-ziplist-size"
                ]
            ),
            b"*2\r\n$21\r\nlist-max-ziplist-size\r\n$2\r\n-2\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"list-max-*", b"list-max-ziplist-size"]),
            b"*4\r\n$22\r\nlist-max-listpack-size\r\n$2\r\n-2\r\n$21\r\nlist-max-ziplist-size\r\n$2\r\n-2\r\n"
        );
        assert!(run(&store, &[b"CONFIG", b"GET", b"*"]).starts_with(b"*30\r\n"));
    }

    #[test]
    fn collection_config_thresholds_round_trip_with_aliases_and_atomic_validation() {
        let store = Store::new(100);
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"list-max-listpack-size"]),
            b"*2\r\n$22\r\nlist-max-listpack-size\r\n$2\r\n-2\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"CONFIG", b"SET", b"hash-max-ziplist-entries", b"0"]
            ),
            b"+OK\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"hash-max-listpack-entries"]),
            b"*2\r\n$25\r\nhash-max-listpack-entries\r\n$1\r\n0\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"CONFIG",
                    b"SET",
                    b"set-max-intset-entries",
                    b"8",
                    b"zset-max-ziplist-entries",
                    b"0"
                ]
            ),
            b"+OK\r\n"
        );
        assert_eq!(run(&store, &[b"CONFIG", b"GET", b"set-max-intset-entries", b"zset-max-listpack-entries"]), b"*4\r\n$22\r\nset-max-intset-entries\r\n$1\r\n8\r\n$25\r\nzset-max-listpack-entries\r\n$1\r\n0\r\n");
        assert!(
            run(
                &store,
                &[
                    b"CONFIG",
                    b"SET",
                    b"set-max-intset-entries",
                    b"10",
                    b"unknown",
                    b"0"
                ]
            )
            .starts_with(b"-ERR")
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"set-max-intset-entries"]),
            b"*2\r\n$22\r\nset-max-intset-entries\r\n$1\r\n8\r\n"
        );
        assert!(
            run(
                &store,
                &[b"CONFIG", b"SET", b"hash-max-listpack-entries", b"-1"]
            )
            .starts_with(b"-ERR")
        );
    }
}
