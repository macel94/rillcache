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
        let Some(aof) = self.aof_config() else {
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
            let wants_maxmemory = if wildcard {
                glob_match(&folded_pattern, b"maxmemory")
            } else {
                pattern.eq_ignore_ascii_case(b"maxmemory")
            };
            if wants_maxmemory {
                matches.push((b"maxmemory", self.payload_limit().to_string()));
            }
            let wants_notify = if wildcard {
                glob_match(&folded_pattern, b"notify-keyspace-events")
            } else {
                pattern.eq_ignore_ascii_case(b"notify-keyspace-events")
            };
            if wants_notify {
                matches.push((b"notify-keyspace-events", self.notify_flags().as_string()));
            }
            let wants_slowlog_threshold = if wildcard {
                glob_match(&folded_pattern, b"slowlog-log-slower-than")
            } else {
                pattern.eq_ignore_ascii_case(b"slowlog-log-slower-than")
            };
            let wants_auto_aof_percentage = if wildcard {
                glob_match(&folded_pattern, b"auto-aof-rewrite-percentage")
            } else {
                pattern.eq_ignore_ascii_case(b"auto-aof-rewrite-percentage")
            };
            let wants_auto_aof_min_size = if wildcard {
                glob_match(&folded_pattern, b"auto-aof-rewrite-min-size")
            } else {
                pattern.eq_ignore_ascii_case(b"auto-aof-rewrite-min-size")
            };
            if wants_auto_aof_percentage {
                matches.push((
                    b"auto-aof-rewrite-percentage",
                    self.auto_aof_rewrite_percentage().to_string(),
                ));
            }
            if wants_auto_aof_min_size {
                matches.push((
                    b"auto-aof-rewrite-min-size",
                    self.auto_aof_rewrite_min_size().to_string(),
                ));
            }
            let wants_appendonly = if wildcard {
                glob_match(&folded_pattern, b"appendonly")
            } else {
                pattern.eq_ignore_ascii_case(b"appendonly")
            };
            if wants_appendonly {
                matches.push((
                    b"appendonly",
                    if aof.appendonly { "yes" } else { "no" }.to_string(),
                ));
            }
            let wants_appendfilename = if wildcard {
                glob_match(&folded_pattern, b"appendfilename")
            } else {
                pattern.eq_ignore_ascii_case(b"appendfilename")
            };
            if wants_appendfilename {
                matches.push((b"appendfilename", aof.appendfilename.clone()));
            }
            let wants_appendfsync = if wildcard {
                glob_match(&folded_pattern, b"appendfsync")
            } else {
                pattern.eq_ignore_ascii_case(b"appendfsync")
            };
            if wants_appendfsync {
                matches.push((b"appendfsync", aof.policy.as_str().to_string()));
            }
            let wants_maxclients = if wildcard {
                glob_match(&folded_pattern, b"maxclients")
            } else {
                pattern.eq_ignore_ascii_case(b"maxclients")
            };
            if wants_maxclients {
                matches.push((b"maxclients", self.max_clients().to_string()));
            }
            let wants_latency_threshold = if wildcard {
                glob_match(&folded_pattern, b"latency-monitor-threshold")
            } else {
                pattern.eq_ignore_ascii_case(b"latency-monitor-threshold")
            };
            if wants_latency_threshold {
                matches.push((
                    b"latency-monitor-threshold",
                    self.latency_threshold_ms().to_string(),
                ));
            }
            if wants_slowlog_threshold {
                matches.push((
                    b"slowlog-log-slower-than",
                    self.slowlog_threshold_us().to_string(),
                ));
            }
            let wants_slowlog_max_len = if wildcard {
                glob_match(&folded_pattern, b"slowlog-max-len")
            } else {
                pattern.eq_ignore_ascii_case(b"slowlog-max-len")
            };
            if wants_slowlog_max_len {
                matches.push((b"slowlog-max-len", self.slowlog_max_len().to_string()));
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
        let mut maxmemory = None;
        let mut notify = None;
        let mut slowlog_threshold = None;
        let mut slowlog_max_len = None;
        let mut latency_threshold = None;
        let mut auto_aof_percentage = None;
        let mut auto_aof_min_size = None;
        let mut appendfsync = None;
        let mut maxclients = None;
        for pair in args[2..].as_chunks::<2>().0 {
            if pair[0].eq_ignore_ascii_case(b"notify-keyspace-events") {
                let Some(parsed) = super::NotifyFlags::parse(pair[1]) else {
                    out.extend_from_slice(b"-ERR invalid notify-keyspace-events flags\r\n");
                    return;
                };
                notify = Some(parsed);
            } else if pair[0].eq_ignore_ascii_case(b"maxmemory-policy") {
                let Some(parsed) = std::str::from_utf8(pair[1])
                    .ok()
                    .and_then(super::EvictionPolicy::parse)
                else {
                    out.extend_from_slice(b"-ERR invalid maxmemory-policy\r\n");
                    return;
                };
                policy = Some(parsed);
            } else if pair[0].eq_ignore_ascii_case(b"maxmemory") {
                let Some(value) = std::str::from_utf8(pair[1])
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|value| *value > 0)
                else {
                    out.extend_from_slice(
                        b"-ERR maxmemory must be a positive payload byte budget\r\n",
                    );
                    return;
                };
                maxmemory = Some(value);
            } else if pair[0].eq_ignore_ascii_case(b"slowlog-log-slower-than") {
                let Some(value) = std::str::from_utf8(pair[1])
                    .ok()
                    .and_then(|value| value.parse::<i64>().ok())
                    .filter(|value| *value >= -1)
                else {
                    out.extend_from_slice(b"-ERR invalid slowlog-log-slower-than\r\n");
                    return;
                };
                slowlog_threshold = Some(value);
            } else if pair[0].eq_ignore_ascii_case(b"latency-monitor-threshold") {
                let Some(value) = std::str::from_utf8(pair[1])
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                else {
                    out.extend_from_slice(b"-ERR invalid latency-monitor-threshold\r\n");
                    return;
                };
                latency_threshold = Some(value);
            } else if pair[0].eq_ignore_ascii_case(b"slowlog-max-len") {
                let Some(value) = std::str::from_utf8(pair[1])
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| *value <= super::slowlog::MAX_MAX_LEN)
                else {
                    out.extend_from_slice(
                        b"-ERR slowlog-max-len must be between 0 and 1000000\r\n",
                    );
                    return;
                };
                slowlog_max_len = Some(value);
            } else if pair[0].eq_ignore_ascii_case(b"auto-aof-rewrite-percentage") {
                let Some(value) = std::str::from_utf8(pair[1])
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                else {
                    out.extend_from_slice(b"-ERR invalid auto-aof-rewrite-percentage\r\n");
                    return;
                };
                auto_aof_percentage = Some(value);
            } else if pair[0].eq_ignore_ascii_case(b"auto-aof-rewrite-min-size") {
                let Some(value) = std::str::from_utf8(pair[1])
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                else {
                    out.extend_from_slice(b"-ERR invalid auto-aof-rewrite-min-size\r\n");
                    return;
                };
                auto_aof_min_size = Some(value);
            } else if pair[0].eq_ignore_ascii_case(b"appendfsync") {
                let Some(parsed) = std::str::from_utf8(pair[1])
                    .ok()
                    .and_then(super::FsyncPolicy::parse)
                else {
                    out.extend_from_slice(b"-ERR invalid appendfsync\r\n");
                    return;
                };
                appendfsync = Some(parsed);
            } else if pair[0].eq_ignore_ascii_case(b"maxclients") {
                let Some(value) = std::str::from_utf8(pair[1])
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| *value > 0)
                else {
                    out.extend_from_slice(b"-ERR invalid maxclients\r\n");
                    return;
                };
                maxclients = Some(value);
            } else if pair[0].eq_ignore_ascii_case(b"appendonly")
                || pair[0].eq_ignore_ascii_case(b"appendfilename")
            {
                out.extend_from_slice(
                    b"-ERR appendonly and appendfilename are startup-only; restart with the desired options\r\n",
                );
                return;
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
        if let Some(limit) = maxmemory
            && !self.set_payload_limit(limit)
        {
            super::internal_error(out);
            return;
        }
        if let Some(notify) = notify {
            self.set_notify_flags(notify);
        }
        if let Some(value) = slowlog_threshold {
            self.set_slowlog_threshold_us(value);
        }
        if let Some(value) = slowlog_max_len {
            self.set_slowlog_max_len(value);
        }
        if let Some(value) = auto_aof_percentage {
            self.set_auto_aof_rewrite_percentage(value);
        }
        if let Some(value) = auto_aof_min_size {
            self.set_auto_aof_rewrite_min_size(value);
        }
        if let Some(policy) = appendfsync
            && self.set_aof_policy(policy).is_err()
        {
            out.extend_from_slice(b"-ERR appendfsync update failed\r\n");
            return;
        }
        if let Some(value) = maxclients {
            self.set_max_clients(value);
        }
        if let Some(value) = latency_threshold {
            self.set_latency_threshold_ms(value);
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
    fn runtime_maxmemory_is_visible_and_evicts_to_the_new_budget() {
        let store = Store::new(64);
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"maxmemory"]),
            b"*2\r\n$9\r\nmaxmemory\r\n$2\r\n64\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"CONFIG", b"SET", b"maxmemory-policy", b"allkeys-random"]
            ),
            b"+OK\r\n"
        );
        let mut out = Vec::new();
        store.execute(&[b"SET", b"k1", b"vvvv"], &mut out);
        assert_eq!(out, b"+OK\r\n");
        out.clear();
        store.execute(&[b"SET", b"k2", b"vvvv"], &mut out);
        assert_eq!(out, b"+OK\r\n");
        assert!(store.data.read().unwrap().payload_bytes > 8);
        assert_eq!(
            run(&store, &[b"CONFIG", b"SET", b"maxmemory", b"8"]),
            b"+OK\r\n"
        );
        assert!(store.data.read().unwrap().payload_bytes <= 8);
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"maxmemory"]),
            b"*2\r\n$9\r\nmaxmemory\r\n$1\r\n8\r\n"
        );

        assert!(run(&store, &[b"CONFIG", b"SET", b"maxmemory", b"0"]).starts_with(b"-ERR"));
        assert!(run(&store, &[b"CONFIG", b"SET", b"maxmemory", b"nope"]).starts_with(b"-ERR"));
        assert_eq!(
            run(
                &store,
                &[
                    b"CONFIG",
                    b"SET",
                    b"maxmemory-policy",
                    b"volatile-ttl",
                    b"maxmemory",
                    b"0"
                ]
            ),
            b"-ERR maxmemory must be a positive payload byte budget\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"maxmemory-policy"]),
            b"*2\r\n$16\r\nmaxmemory-policy\r\n$14\r\nallkeys-random\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"maxmemory"]),
            b"*2\r\n$9\r\nmaxmemory\r\n$1\r\n8\r\n"
        );
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
        assert!(run(&store, &[b"CONFIG", b"GET", b"*"]).starts_with(b"*52\r\n"));
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
    #[test]
    fn journal_config_visibility_is_read_only_except_for_appendfsync() {
        let store = Store::new(1024);
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"append*"]),
            b"*6\r\n$14\r\nappendfilename\r\n$14\r\nappendonly.aof\r\n$11\r\nappendfsync\r\n$8\r\neverysec\r\n$10\r\nappendonly\r\n$2\r\nno\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"SET", b"appendfsync", b"no"]),
            b"+OK\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"appendfsync"]),
            b"*2\r\n$11\r\nappendfsync\r\n$2\r\nno\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"SET", b"appendfsync", b"always"]),
            b"+OK\r\n"
        );
        assert!(
            run(&store, &[b"CONFIG", b"SET", b"appendfsync", b"sometimes"]).starts_with(b"-ERR")
        );
        for (name, value) in [
            (&b"appendonly"[..], &b"yes"[..]),
            (b"appendfilename", b"other.aof"),
        ] {
            assert!(
                run(&store, &[b"CONFIG", b"SET", name, value]).starts_with(b"-ERR"),
                "{name:?}"
            );
        }
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"appendonly", b"appendfilename"]),
            b"*4\r\n$14\r\nappendfilename\r\n$14\r\nappendonly.aof\r\n$10\r\nappendonly\r\n$2\r\nno\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"appendfsync"]),
            b"*2\r\n$11\r\nappendfsync\r\n$6\r\nalways\r\n"
        );
    }

    #[test]
    fn runtime_maxclients_roundtrips_and_rejects_invalid_values() {
        let store = Store::new(1024);
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"maxclients"]),
            b"*2\r\n$10\r\nmaxclients\r\n$3\r\n128\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"max*"]),
            b"*6\r\n$10\r\nmaxclients\r\n$3\r\n128\r\n$9\r\nmaxmemory\r\n$4\r\n1024\r\n$16\r\nmaxmemory-policy\r\n$10\r\nnoeviction\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"SET", b"maxclients", b"2"]),
            b"+OK\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"maxclients"]),
            b"*2\r\n$10\r\nmaxclients\r\n$1\r\n2\r\n"
        );
        for value in [&b"0"[..], b"-1", b"x", b""] {
            assert!(
                run(&store, &[b"CONFIG", b"SET", b"maxclients", value])
                    .starts_with(b"-ERR invalid maxclients"),
                "{value:?}"
            );
            assert_eq!(store.max_clients(), 2);
        }
    }

    #[test]
    fn auto_aof_rewrite_config_roundtrips_and_validates() {
        let store = Store::new(1024);
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"auto-aof-rewrite-percentage"]),
            b"*2\r\n$27\r\nauto-aof-rewrite-percentage\r\n$3\r\n100\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"CONFIG",
                    b"SET",
                    b"auto-aof-rewrite-percentage",
                    b"0",
                    b"auto-aof-rewrite-min-size",
                    b"4096"
                ]
            ),
            b"+OK\r\n"
        );
        assert_eq!(
            run(&store, &[b"CONFIG", b"GET", b"auto-aof-rewrite-min-size"]),
            b"*2\r\n$25\r\nauto-aof-rewrite-min-size\r\n$4\r\n4096\r\n"
        );
        assert!(
            run(
                &store,
                &[b"CONFIG", b"SET", b"auto-aof-rewrite-percentage", b"-1"]
            )
            .starts_with(b"-ERR")
        );
        assert!(
            run(
                &store,
                &[b"CONFIG", b"SET", b"auto-aof-rewrite-min-size", b"x"]
            )
            .starts_with(b"-ERR")
        );
    }
}
