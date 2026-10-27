//! Additional bounded collection mutation and inspection commands.
//!
//! The core commands in `collections.rs` create and read values. This module
//! completes the common mutation surface without introducing a second storage
//! representation: every change stages the entire typed value, checks the
//! configured payload budget, and publishes it atomically under the store lock.

use super::collection_config::CollectionConfig;
use super::data::{Collection, Data, Entry};
use super::options::{NOT_INTEGER, OOM, SYNTAX, parse_integer};
use super::{Store, arity, integer, internal_error, wrong_type};
use crate::protocol::encode_bulk;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_ELEMENTS: usize = 1_000_000;
static RANDOM_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl Store {
    /// Returns false without touching output for commands owned by another module.
    pub(super) fn execute_collection_commands(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let Some(command) = args.first() else {
            return false;
        };
        if (command
            .first()
            .is_some_and(|byte| byte.eq_ignore_ascii_case(&b'z'))
            || command.eq_ignore_ascii_case(b"BZMPOP"))
            && self.execute_zset_extended(args, out)
        {
            return true;
        }
        if (command.first().is_some_and(|byte| {
            byte.eq_ignore_ascii_case(&b'l') || byte.eq_ignore_ascii_case(&b'b')
        }) || command.eq_ignore_ascii_case(b"RPOPLPUSH"))
            && self.execute_list_extended(args, out)
        {
            return true;
        }
        if command
            .first()
            .is_some_and(|byte| byte.eq_ignore_ascii_case(&b'h'))
            && self.execute_hash_extended(args, out)
        {
            return true;
        }
        if command.eq_ignore_ascii_case(b"HPEXPIRE")
            || command.eq_ignore_ascii_case(b"HEXPIRE")
            || command.eq_ignore_ascii_case(b"HPEXPIREAT")
            || command.eq_ignore_ascii_case(b"HEXPIREAT")
        {
            self.hash_expire(args, out);
        } else if command.eq_ignore_ascii_case(b"HPTTL")
            || command.eq_ignore_ascii_case(b"HTTL")
            || command.eq_ignore_ascii_case(b"HPEXPIRETIME")
            || command.eq_ignore_ascii_case(b"HEXPIRETIME")
        {
            self.hash_ttl(args, out);
        } else if command.eq_ignore_ascii_case(b"HPERSIST") {
            self.hash_persist(args, out);
        } else if command.eq_ignore_ascii_case(b"HSETNX") {
            self.hash_setnx(args, out);
        } else if command.eq_ignore_ascii_case(b"HDEL") {
            if args.len() < 3 {
                arity("hdel", out);
            } else {
                self.hash_del(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"HEXISTS") {
            if args.len() != 3 {
                arity("hexists", out);
            } else {
                self.hash_exists(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"HINCRBY") {
            if args.len() != 4 {
                arity("hincrby", out);
            } else {
                self.hash_incr(args, false, out);
            }
        } else if command.eq_ignore_ascii_case(b"HINCRBYFLOAT") {
            if args.len() != 4 {
                arity("hincrbyfloat", out);
            } else {
                self.hash_incr(args, true, out);
            }
        } else if command.eq_ignore_ascii_case(b"HMGET") {
            if args.len() < 3 {
                arity("hmget", out);
            } else {
                self.hash_mget(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"HKEYS") || command.eq_ignore_ascii_case(b"HVALS") {
            if args.len() != 2 {
                arity(
                    if command.eq_ignore_ascii_case(b"HKEYS") {
                        "hkeys"
                    } else {
                        "hvals"
                    },
                    out,
                );
            } else {
                self.hash_keys_values(args, command.eq_ignore_ascii_case(b"HKEYS"), out);
            }
        } else if command.eq_ignore_ascii_case(b"HSTRLEN") {
            if args.len() != 3 {
                arity("hstrlen", out);
            } else {
                self.hash_strlen(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"LINDEX") {
            if args.len() != 3 {
                arity("lindex", out);
            } else {
                self.list_index(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"LINSERT") {
            if args.len() != 5 {
                arity("linsert", out);
            } else {
                self.list_insert(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"LREM") {
            if args.len() != 4 {
                arity("lrem", out);
            } else {
                self.list_remove(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"LSET") {
            if args.len() != 4 {
                arity("lset", out);
            } else {
                self.list_set(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"LTRIM") {
            if args.len() != 4 {
                arity("ltrim", out);
            } else {
                self.list_trim(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"LPUSHX") || command.eq_ignore_ascii_case(b"RPUSHX")
        {
            if args.len() < 3 {
                arity(
                    if command.eq_ignore_ascii_case(b"LPUSHX") {
                        "lpushx"
                    } else {
                        "rpushx"
                    },
                    out,
                );
            } else {
                self.list_push_existing(args, command.eq_ignore_ascii_case(b"LPUSHX"), out);
            }
        } else if command.eq_ignore_ascii_case(b"SREM") {
            if args.len() < 3 {
                arity("srem", out);
            } else {
                self.set_remove(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"SMISMEMBER") {
            if args.len() < 3 {
                arity("smismember", out);
            } else {
                self.set_mismember(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"SMOVE") {
            if args.len() != 4 {
                arity("smove", out);
            } else {
                self.set_move(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"SINTER")
            || command.eq_ignore_ascii_case(b"SUNION")
            || command.eq_ignore_ascii_case(b"SDIFF")
        {
            let name = if command.eq_ignore_ascii_case(b"SINTER") {
                "sinter"
            } else if command.eq_ignore_ascii_case(b"SUNION") {
                "sunion"
            } else {
                "sdiff"
            };
            if args.len() < 2 {
                arity(name, out);
            } else {
                self.set_algebra(args, name, out);
            }
        } else if command.eq_ignore_ascii_case(b"SINTERCARD") {
            if args.len() < 3 {
                arity("sintercard", out);
            } else {
                self.set_intercard(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"SINTERSTORE")
            || command.eq_ignore_ascii_case(b"SUNIONSTORE")
            || command.eq_ignore_ascii_case(b"SDIFFSTORE")
        {
            let name = if command.eq_ignore_ascii_case(b"SINTERSTORE") {
                "sinterstore"
            } else if command.eq_ignore_ascii_case(b"SUNIONSTORE") {
                "sunionstore"
            } else {
                "sdiffstore"
            };
            if args.len() < 3 {
                arity(name, out);
            } else {
                self.set_store(args, name, out);
            }
        } else if command.eq_ignore_ascii_case(b"SPOP") {
            if !(args.len() == 2 || args.len() == 3) {
                arity("spop", out);
            } else {
                self.set_pop(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"SRANDMEMBER") {
            if !(args.len() == 2 || args.len() == 3) {
                arity("srandmember", out);
            } else {
                self.set_random_member(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZADD") {
            if args.len() < 4 {
                arity("zadd", out);
            } else {
                self.zset_add_extra(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZREM") {
            if args.len() < 3 {
                arity("zrem", out);
            } else {
                self.zset_remove(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZINCRBY") {
            if args.len() != 4 {
                arity("zincrby", out);
            } else {
                self.zset_incr(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZRANK")
            || command.eq_ignore_ascii_case(b"ZREVRANK")
        {
            if !(args.len() == 3 || args.len() == 4) {
                arity(
                    if command.eq_ignore_ascii_case(b"ZRANK") {
                        "zrank"
                    } else {
                        "zrevrank"
                    },
                    out,
                );
            } else {
                self.zset_rank(args, command.eq_ignore_ascii_case(b"ZREVRANK"), out);
            }
        } else if command.eq_ignore_ascii_case(b"ZREVRANGE") {
            if !(args.len() == 4 || args.len() == 5) {
                arity("zrevrange", out);
            } else {
                self.zset_reverse_range(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZCOUNT") {
            if args.len() != 4 {
                arity("zcount", out);
            } else {
                self.zset_count(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZRANGEBYSCORE")
            || command.eq_ignore_ascii_case(b"ZREVRANGEBYSCORE")
        {
            self.zset_range_by_score(args, command.eq_ignore_ascii_case(b"ZREVRANGEBYSCORE"), out);
        } else if command.eq_ignore_ascii_case(b"ZREMRANGEBYRANK")
            || command.eq_ignore_ascii_case(b"ZREMRANGEBYSCORE")
        {
            if args.len() != 4 {
                arity(
                    if command.eq_ignore_ascii_case(b"ZREMRANGEBYRANK") {
                        "zremrangebyrank"
                    } else {
                        "zremrangebyscore"
                    },
                    out,
                );
            } else {
                self.zset_remove_range(
                    args,
                    command.eq_ignore_ascii_case(b"ZREMRANGEBYSCORE"),
                    out,
                );
            }
        } else if command.eq_ignore_ascii_case(b"ZPOPMIN")
            || command.eq_ignore_ascii_case(b"ZPOPMAX")
        {
            if !(args.len() == 2 || args.len() == 3) {
                arity(
                    if command.eq_ignore_ascii_case(b"ZPOPMIN") {
                        "zpopmin"
                    } else {
                        "zpopmax"
                    },
                    out,
                );
            } else {
                self.zset_pop(args, command.eq_ignore_ascii_case(b"ZPOPMAX"), out);
            }
        } else {
            return false;
        }
        true
    }
}

fn array(length: usize, out: &mut Vec<u8>) {
    write!(out, "*{length}\r\n").expect("Vec write");
}

fn normalize_score(score: f64) -> f64 {
    if score == 0.0 { 0.0 } else { score }
}

fn score_bytes(score: f64) -> Vec<u8> {
    normalize_score(score).to_string().into_bytes()
}

fn parse_score(bytes: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(bytes).ok()?;
    let score = text.parse::<f64>().ok()?;
    (!score.is_nan()).then_some(score)
}

fn parse_i64(bytes: &[u8]) -> Option<i64> {
    let text = std::str::from_utf8(bytes).ok()?;
    let value = text.parse::<i64>().ok()?;
    (value.to_string().as_bytes() == bytes).then_some(value)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldTtlMode {
    /// No TTL option: HSETEX clears the written fields' TTLs; HGETEX leaves them.
    Default,
    Keep,
    Persist,
    Ex,
    Px,
    ExAt,
    PxAt,
}

impl FieldTtlMode {
    fn needs_value(self) -> bool {
        matches!(self, Self::Ex | Self::Px | Self::ExAt | Self::PxAt)
    }

    fn absolute(self) -> bool {
        matches!(self, Self::ExAt | Self::PxAt)
    }
}

fn field_ttl_mode(token: &[u8], allow_keep: bool, allow_persist: bool) -> Option<FieldTtlMode> {
    let matches = |candidate: &[u8]| token.eq_ignore_ascii_case(candidate);
    if matches(b"EX") {
        Some(FieldTtlMode::Ex)
    } else if matches(b"PX") {
        Some(FieldTtlMode::Px)
    } else if matches(b"EXAT") {
        Some(FieldTtlMode::ExAt)
    } else if matches(b"PXAT") {
        Some(FieldTtlMode::PxAt)
    } else if matches(b"KEEPTTL") && allow_keep {
        Some(FieldTtlMode::Keep)
    } else if matches(b"PERSIST") && allow_persist {
        Some(FieldTtlMode::Persist)
    } else {
        None
    }
}

struct HashFieldOptions {
    /// `Some(true)` for FNX, `Some(false)` for FXX.
    condition: Option<bool>,
    ttl: FieldTtlMode,
    ttl_value: Option<i64>,
    fields_index: usize,
}

/// Parse the optional condition/TTL clause shared by HSETEX and HGETEX. The
/// order is flexible, but each condition and TTL option may appear only once.
fn parse_hash_field_options(
    args: &[&[u8]],
    allow_condition: bool,
    allow_keep: bool,
    allow_persist: bool,
    out: &mut Vec<u8>,
) -> Option<HashFieldOptions> {
    let mut index = 2;
    let mut condition = None;
    let mut ttl = FieldTtlMode::Default;
    let mut ttl_value = None;
    let mut condition_seen = false;
    let mut ttl_seen = false;
    loop {
        let Some(token) = args.get(index) else {
            out.extend_from_slice(
                b"-ERR Mandatory argument FIELDS is missing or not at the right position\r\n",
            );
            return None;
        };
        if token.eq_ignore_ascii_case(b"FIELDS") {
            break;
        }
        if allow_condition
            && (token.eq_ignore_ascii_case(b"FNX") || token.eq_ignore_ascii_case(b"FXX"))
        {
            if condition_seen {
                out.extend_from_slice(
                    b"-ERR Only one of FXX or FNX arguments can be specified\r\n",
                );
                return None;
            }
            condition_seen = true;
            condition = Some(token.eq_ignore_ascii_case(b"FNX"));
            index += 1;
            continue;
        }
        let Some(mode) = field_ttl_mode(token, allow_keep, allow_persist) else {
            out.extend_from_slice(SYNTAX);
            return None;
        };
        if ttl_seen {
            out.extend_from_slice(if allow_persist {
                b"-ERR Only one of EX, PX, EXAT, PXAT or PERSIST arguments can be specified\r\n"
            } else {
                b"-ERR Only one of EX, PX, EXAT, PXAT or KEEPTTL arguments can be specified\r\n"
            });
            return None;
        }
        ttl_seen = true;
        ttl = mode;
        index += 1;
        if mode.needs_value() {
            let Some(raw) = args.get(index) else {
                out.extend_from_slice(NOT_INTEGER);
                return None;
            };
            let Some(value) = parse_i64(raw) else {
                out.extend_from_slice(NOT_INTEGER);
                return None;
            };
            if value < 0 {
                out.extend_from_slice(b"-ERR invalid expire time, must be >= 0\r\n");
                return None;
            }
            if let Some(value) =
                value.checked_mul(if mode == FieldTtlMode::Ex || mode == FieldTtlMode::ExAt {
                    1000
                } else {
                    1
                })
            {
                ttl_value = Some(value);
            } else {
                out.extend_from_slice(b"-ERR invalid expire time\r\n");
                return None;
            }
            index += 1;
        }
    }
    Some(HashFieldOptions {
        condition,
        ttl,
        ttl_value,
        fields_index: index,
    })
}

/// Parse the mandatory `FIELDS numfields ...` block and return its length and
/// start offset. `pairs` selects field/value pairs (HSETEX) over fields only.
fn parse_hash_fields_block(
    args: &[&[u8]],
    index: usize,
    pairs: bool,
    out: &mut Vec<u8>,
) -> Option<(usize, usize)> {
    if !args
        .get(index)
        .is_some_and(|token| token.eq_ignore_ascii_case(b"FIELDS"))
    {
        out.extend_from_slice(
            b"-ERR Mandatory argument FIELDS is missing or not at the right position\r\n",
        );
        return None;
    }
    let Some(count) = args
        .get(index + 1)
        .and_then(|raw| parse_i64(raw))
        .filter(|count| *count > 0)
    else {
        out.extend_from_slice(b"-ERR Parameter `numFields` should be greater than 0\r\n");
        return None;
    };
    let Ok(count) = usize::try_from(count) else {
        out.extend_from_slice(b"-ERR Parameter `numFields` should be greater than 0\r\n");
        return None;
    };
    let start = index + 2;
    let needed = if pairs {
        count.checked_mul(2)
    } else {
        Some(count)
    };
    if needed.and_then(|needed| start.checked_add(needed)) != Some(args.len()) {
        out.extend_from_slice(
            b"-ERR The `numfields` parameter must match the number of arguments\r\n",
        );
        return None;
    }
    Some((count, start))
}

fn resolve_field_deadline(mode: FieldTtlMode, value: i64, now: i64) -> Option<i64> {
    if mode.absolute() {
        Some(value)
    } else {
        now.checked_add(value)
    }
}

fn parse_score_bound(bytes: &[u8]) -> Option<(f64, bool)> {
    let (body, inclusive) = match bytes.first() {
        Some(b'(') => (&bytes[1..], false),
        _ => (bytes, true),
    };
    if body.eq_ignore_ascii_case(b"-inf") {
        return Some((f64::NEG_INFINITY, inclusive));
    }
    if body.eq_ignore_ascii_case(b"+inf") || body.eq_ignore_ascii_case(b"inf") {
        return Some((f64::INFINITY, inclusive));
    }
    Some((parse_score(body)?, inclusive))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RangeMode {
    Rank,
    Score,
    Lex,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ZSetOp {
    Union,
    Inter,
    Diff,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Aggregate {
    Sum,
    Min,
    Max,
}

fn nan_zero(score: f64) -> f64 {
    normalize_score(if score.is_nan() { 0.0 } else { score })
}

fn parse_zset_numkeys(raw: &[u8], name: &str, out: &mut Vec<u8>) -> Option<usize> {
    let Some(value) = parse_i64(raw) else {
        out.extend_from_slice(NOT_INTEGER);
        return None;
    };
    if value < 1 {
        write!(
            out,
            "-ERR at least 1 input key is needed for '{name}' command\r\n"
        )
        .expect("Vec write");
        return None;
    }
    match usize::try_from(value) {
        Ok(value) => Some(value),
        Err(_) => {
            out.extend_from_slice(NOT_INTEGER);
            None
        }
    }
}

fn parse_zset_algebra_options(
    args: &[&[u8]],
    start: usize,
    count: usize,
    operation: ZSetOp,
    store: bool,
    out: &mut Vec<u8>,
) -> Option<(Vec<f64>, Aggregate, bool)> {
    let mut weights = vec![1.0f64; count];
    let mut aggregate = Aggregate::Sum;
    let mut with_scores = false;
    let mut index = start;
    while index < args.len() {
        let token = args[index];
        if token.eq_ignore_ascii_case(b"WEIGHTS") && operation != ZSetOp::Diff {
            if index + count + 1 > args.len() {
                out.extend_from_slice(SYNTAX);
                return None;
            }
            for (offset, weight) in weights.iter_mut().enumerate() {
                let Some(parsed) = parse_score(args[index + 1 + offset]) else {
                    out.extend_from_slice(b"-ERR weight value is not a float\r\n");
                    return None;
                };
                *weight = parsed;
            }
            index += count + 1;
        } else if token.eq_ignore_ascii_case(b"AGGREGATE") && operation != ZSetOp::Diff {
            let Some(value) = args.get(index + 1) else {
                out.extend_from_slice(SYNTAX);
                return None;
            };
            aggregate = if value.eq_ignore_ascii_case(b"SUM") {
                Aggregate::Sum
            } else if value.eq_ignore_ascii_case(b"MIN") {
                Aggregate::Min
            } else if value.eq_ignore_ascii_case(b"MAX") {
                Aggregate::Max
            } else {
                out.extend_from_slice(SYNTAX);
                return None;
            };
            index += 2;
        } else if token.eq_ignore_ascii_case(b"WITHSCORES") && !store {
            with_scores = true;
            index += 1;
        } else {
            out.extend_from_slice(SYNTAX);
            return None;
        }
    }
    Some((weights, aggregate, with_scores))
}

enum LexBound<'a> {
    Min,
    Max,
    Value(&'a [u8], bool),
}

enum Selector<'a> {
    Rank(i64, i64),
    Score((f64, bool), (f64, bool)),
    Lex(LexBound<'a>, LexBound<'a>),
}

fn parse_selector<'a>(
    mode: RangeMode,
    reverse: bool,
    first: &'a [u8],
    second: &'a [u8],
) -> Result<Selector<'a>, &'static [u8]> {
    Ok(match mode {
        RangeMode::Rank => {
            let (Some(start), Some(stop)) = (parse_i64(first), parse_i64(second)) else {
                return Err(NOT_INTEGER);
            };
            Selector::Rank(start, stop)
        }
        RangeMode::Score => {
            let (min, max) = if reverse {
                (second, first)
            } else {
                (first, second)
            };
            let (Some(min), Some(max)) = (parse_score_bound(min), parse_score_bound(max)) else {
                return Err(b"-ERR min or max is not a float\r\n");
            };
            Selector::Score(min, max)
        }
        RangeMode::Lex => {
            let (min, max) = if reverse {
                (second, first)
            } else {
                (first, second)
            };
            let Some((min, max)) = parse_lex_bounds(min, max) else {
                return Err(b"-ERR min or max not valid string range item\r\n");
            };
            Selector::Lex(min, max)
        }
    })
}

fn parse_lex_bounds<'a>(min: &'a [u8], max: &'a [u8]) -> Option<(LexBound<'a>, LexBound<'a>)> {
    Some((parse_lex_bound(min)?, parse_lex_bound(max)?))
}

fn parse_lex_bound(raw: &[u8]) -> Option<LexBound<'_>> {
    match raw {
        b"-" => Some(LexBound::Min),
        b"+" => Some(LexBound::Max),
        _ => {
            let (&marker, value) = raw.split_first()?;
            match marker {
                b'[' => Some(LexBound::Value(value, true)),
                b'(' => Some(LexBound::Value(value, false)),
                _ => None,
            }
        }
    }
}

fn in_lex_range(member: &[u8], min: &LexBound<'_>, max: &LexBound<'_>) -> bool {
    let above_min = match min {
        LexBound::Min => true,
        LexBound::Max => false,
        LexBound::Value(value, inclusive) => {
            if *inclusive {
                member >= *value
            } else {
                member > *value
            }
        }
    };
    let below_max = match max {
        LexBound::Min => false,
        LexBound::Max => true,
        LexBound::Value(value, inclusive) => {
            if *inclusive {
                member <= *value
            } else {
                member < *value
            }
        }
    };
    above_min && below_max
}

fn select_zset(
    items: Vec<(&[u8], f64)>,
    selector: Selector<'_>,
    reverse: bool,
    limit: Option<(i64, i64)>,
) -> Vec<(Vec<u8>, f64)> {
    let ordered = ordered_zset(items, reverse);
    match selector {
        Selector::Rank(start, stop) => {
            let len = ordered.len() as i128;
            let normalize = |index: i64| {
                if index < 0 {
                    (len + index as i128).max(0)
                } else {
                    index as i128
                }
            };
            let first = normalize(start);
            let last = if stop < 0 {
                len + stop as i128
            } else {
                stop as i128
            };
            if first >= len || first > last {
                return Vec::new();
            }
            let begin = first as usize;
            let end = (last.min(len - 1) as usize) + 1;
            ordered[begin..end].to_vec()
        }
        Selector::Score(min, max) => {
            let mut selected: Vec<_> = ordered
                .into_iter()
                .filter(|(_, score)| in_score_range(*score, min, max))
                .collect();
            apply_limit(&mut selected, limit);
            selected
        }
        Selector::Lex(min, max) => {
            let mut selected: Vec<_> = ordered
                .into_iter()
                .filter(|(member, _)| in_lex_range(member, &min, &max))
                .collect();
            apply_limit(&mut selected, limit);
            selected
        }
    }
}

fn apply_limit(items: &mut Vec<(Vec<u8>, f64)>, limit: Option<(i64, i64)>) {
    let Some((offset, count)) = limit else {
        return;
    };
    if offset < 0 {
        items.clear();
        return;
    }
    let offset = usize::try_from(offset).unwrap_or(usize::MAX);
    if offset >= items.len() {
        items.clear();
        return;
    }
    if count >= 0 {
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        items.truncate(offset.saturating_add(count));
    }
    items.drain(..offset);
}

fn in_score_range(score: f64, min: (f64, bool), max: (f64, bool)) -> bool {
    (min.1 && score >= min.0 || !min.1 && score > min.0)
        && (max.1 && score <= max.0 || !max.1 && score < max.0)
}

fn random_index(length: usize) -> usize {
    if length <= 1 {
        return 0;
    }
    let sequence = RANDOM_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut random = nanos ^ sequence.wrapping_mul(0x9e3779b97f4a7c15);
    random = (random ^ (random >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    random = (random ^ (random >> 27)).wrapping_mul(0x94d049bb133111eb);
    random ^= random >> 31;
    random as usize % length
}

fn encode_set(set: BTreeSet<Vec<u8>>, config: &CollectionConfig) -> Collection {
    let compact_ints: Option<BTreeSet<i64>> =
        set.iter().map(|member| parse_integer(member)).collect();
    if let Some(values) =
        compact_ints.filter(|values| values.len() <= config.set_max_intset_entries)
    {
        Collection::SetInt(values)
    } else if set.len() <= config.set_max_listpack_entries
        && set
            .iter()
            .all(|member| member.len() <= config.set_max_listpack_value)
    {
        Collection::SetPack(set.into_iter().collect())
    } else {
        Collection::Set(set)
    }
}

fn encode_hash(hash: HashMap<Vec<u8>, Vec<u8>>, config: &CollectionConfig) -> Collection {
    let compact = hash.len() <= config.hash_max_listpack_entries
        && hash.iter().all(|(field, value)| {
            field.len() <= config.hash_max_listpack_value
                && value.len() <= config.hash_max_listpack_value
        });
    if compact {
        let mut entries: Vec<_> = hash.into_iter().collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        Collection::HashPack(entries)
    } else {
        Collection::Hash(hash)
    }
}

pub(super) fn encode_zset(zset: HashMap<Vec<u8>, f64>, config: &CollectionConfig) -> Collection {
    let compact = zset.len() <= config.zset_max_listpack_entries
        && zset
            .keys()
            .all(|member| member.len() <= config.zset_max_listpack_value);
    if compact {
        let mut entries: Vec<_> = zset.into_iter().collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        Collection::ZSetPack(entries)
    } else {
        Collection::ZSet(zset)
    }
}

fn encode_list(list: VecDeque<Vec<u8>>, setting: i64) -> Collection {
    let total_bytes: usize = list.iter().map(Vec::len).sum();
    let compact = if setting > 0 {
        list.len() <= setting as usize
    } else {
        total_bytes <= listpack_byte_limit(setting)
    };
    if compact {
        return Collection::ListPack(list);
    }
    let by_count = setting > 0;
    let limit = if by_count {
        setting as usize
    } else {
        listpack_byte_limit(setting)
    };
    let mut nodes = VecDeque::new();
    let mut node = VecDeque::new();
    let mut node_bytes = 0usize;
    for value in list {
        let full = !node.is_empty()
            && if by_count {
                node.len() >= limit
            } else {
                node_bytes.saturating_add(value.len()) > limit
            };
        if full {
            nodes.push_back(std::mem::take(&mut node));
            node_bytes = 0;
        }
        node_bytes = node_bytes.saturating_add(value.len());
        node.push_back(value);
    }
    if !node.is_empty() {
        nodes.push_back(node);
    }
    Collection::ListQuick(nodes)
}

fn listpack_byte_limit(setting: i64) -> usize {
    if setting >= 0 {
        return setting as usize;
    }
    let level = setting.unsigned_abs().saturating_sub(1).min(4);
    4096usize << level
}

fn normalize_index(index: i64, len: usize) -> Option<usize> {
    let len = len as i128;
    let index = if index < 0 {
        len + index as i128
    } else {
        index as i128
    };
    (0..len).contains(&index).then_some(index as usize)
}

pub(crate) struct ListPopOptions<'a> {
    pub keys: Vec<&'a [u8]>,
    pub left: bool,
    pub count: usize,
}

pub(crate) enum ListPopOutcome {
    Popped { key: Vec<u8>, values: Vec<Vec<u8>> },
    Empty,
    WrongType,
    Oom,
    JournalError,
}

pub(crate) fn parse_list_mpop<'a>(
    args: &[&'a [u8]],
    numkeys_index: usize,
) -> Result<ListPopOptions<'a>, &'static [u8]> {
    let Some(numkeys) = args
        .get(numkeys_index)
        .and_then(|raw| parse_i64(raw))
        .filter(|value| *value > 0)
    else {
        return Err(b"-ERR numkeys should be greater than 0\r\n");
    };
    let Ok(numkeys) = usize::try_from(numkeys) else {
        return Err(SYNTAX);
    };
    let keys_start = numkeys_index + 1;
    let Some(where_index) = keys_start.checked_add(numkeys) else {
        return Err(SYNTAX);
    };
    let Some(direction) = args.get(where_index) else {
        return Err(SYNTAX);
    };
    let left = if direction.eq_ignore_ascii_case(b"LEFT") {
        true
    } else if direction.eq_ignore_ascii_case(b"RIGHT") {
        false
    } else {
        return Err(SYNTAX);
    };
    let mut count = 1usize;
    let mut count_set = false;
    let mut index = where_index + 1;
    while index < args.len() {
        if !count_set && args[index].eq_ignore_ascii_case(b"COUNT") && index + 1 < args.len() {
            let Some(value) = parse_i64(args[index + 1]).filter(|value| *value > 0) else {
                return Err(b"-ERR count should be greater than 0\r\n");
            };
            count = usize::try_from(value).unwrap_or(usize::MAX);
            count_set = true;
            index += 2;
        } else {
            return Err(SYNTAX);
        }
    }
    Ok(ListPopOptions {
        keys: args[keys_start..where_index].to_vec(),
        left,
        count,
    })
}

pub(crate) fn list_pop_reply(outcome: ListPopOutcome, out: &mut Vec<u8>) {
    match outcome {
        ListPopOutcome::Popped { key, values } => {
            out.extend_from_slice(b"*2\r\n");
            encode_bulk(Some(&key), out);
            write!(out, "*{}\r\n", values.len()).expect("Vec write");
            for value in &values {
                encode_bulk(Some(value), out);
            }
        }
        ListPopOutcome::Empty => out.extend_from_slice(b"*-1\r\n"),
        ListPopOutcome::WrongType => wrong_type(out),
        ListPopOutcome::Oom => out.extend_from_slice(OOM),
        ListPopOutcome::JournalError => out.extend_from_slice(b"-ERR append-only write failed\r\n"),
    }
}

pub(crate) enum ListMoveOutcome {
    Moved {
        source: Vec<u8>,
        value: Vec<u8>,
        source_emptied: bool,
    },
    Empty,
    WrongType,
    Oom,
    JournalError,
}

pub(crate) fn list_move_reply(outcome: ListMoveOutcome, out: &mut Vec<u8>) {
    match outcome {
        ListMoveOutcome::Moved { value, .. } => encode_bulk(Some(&value), out),
        ListMoveOutcome::Empty => encode_bulk(None, out),
        ListMoveOutcome::WrongType => wrong_type(out),
        ListMoveOutcome::Oom => out.extend_from_slice(OOM),
        ListMoveOutcome::JournalError => {
            out.extend_from_slice(b"-ERR append-only write failed\r\n")
        }
    }
}

pub(crate) fn parse_side(raw: &[u8]) -> Option<bool> {
    if raw.eq_ignore_ascii_case(b"LEFT") {
        Some(true)
    } else if raw.eq_ignore_ascii_case(b"RIGHT") {
        Some(false)
    } else {
        None
    }
}

pub(crate) struct MPopOptions<'a> {
    pub keys: Vec<&'a [u8]>,
    pub max: bool,
    pub count: usize,
}

pub(crate) fn parse_mpop<'a>(
    args: &[&'a [u8]],
    numkeys_index: usize,
) -> Result<MPopOptions<'a>, &'static [u8]> {
    let Some(numkeys) = args
        .get(numkeys_index)
        .and_then(|raw| parse_i64(raw))
        .filter(|value| *value > 0)
    else {
        return Err(b"-ERR numkeys should be greater than 0\r\n");
    };
    let Ok(numkeys) = usize::try_from(numkeys) else {
        return Err(SYNTAX);
    };
    let keys_start = numkeys_index + 1;
    let Some(where_index) = keys_start.checked_add(numkeys) else {
        return Err(SYNTAX);
    };
    let Some(direction) = args.get(where_index) else {
        return Err(SYNTAX);
    };
    let max = if direction.eq_ignore_ascii_case(b"MIN") {
        false
    } else if direction.eq_ignore_ascii_case(b"MAX") {
        true
    } else {
        return Err(SYNTAX);
    };
    let mut count = 1usize;
    let mut count_set = false;
    let mut index = where_index + 1;
    while index < args.len() {
        if !count_set && args[index].eq_ignore_ascii_case(b"COUNT") && index + 1 < args.len() {
            let Some(value) = parse_i64(args[index + 1]).filter(|value| *value > 0) else {
                return Err(b"-ERR count should be greater than 0\r\n");
            };
            count = usize::try_from(value).unwrap_or(usize::MAX);
            count_set = true;
            index += 2;
        } else {
            return Err(SYNTAX);
        }
    }
    Ok(MPopOptions {
        keys: args[keys_start..where_index].to_vec(),
        max,
        count,
    })
}

pub(crate) fn parse_bzmpop_timeout(raw: &[u8]) -> Result<f64, &'static [u8]> {
    let Some(text) = std::str::from_utf8(raw).ok() else {
        return Err(b"-ERR timeout is not a float or out of range\r\n");
    };
    let Ok(value) = text.parse::<f64>() else {
        return Err(b"-ERR timeout is not a float or out of range\r\n");
    };
    if value.is_nan() {
        return Err(b"-ERR timeout is not a float or out of range\r\n");
    }
    if value < 0.0 {
        return Err(b"-ERR timeout is negative\r\n");
    }
    if !value.is_finite() || Duration::try_from_secs_f64(value).is_err() {
        return Err(b"-ERR timeout is out of range\r\n");
    }
    Ok(value)
}

pub(crate) enum ZSetPopOutcome {
    Popped {
        key: Vec<u8>,
        items: Vec<(Vec<u8>, f64)>,
    },
    Empty,
    WrongType,
    Oom,
    JournalError,
}

pub(crate) fn zset_pop_reply(outcome: ZSetPopOutcome, out: &mut Vec<u8>) {
    match outcome {
        ZSetPopOutcome::Popped { key, items } => {
            out.extend_from_slice(b"*2\r\n");
            encode_bulk(Some(&key), out);
            write!(out, "*{}\r\n", items.len()).expect("Vec write");
            for (member, score) in &items {
                out.extend_from_slice(b"*2\r\n");
                encode_bulk(Some(member), out);
                encode_bulk(Some(&score_bytes(*score)), out);
            }
        }
        ZSetPopOutcome::Empty => out.extend_from_slice(b"*-1\r\n"),
        ZSetPopOutcome::WrongType => wrong_type(out),
        ZSetPopOutcome::Oom => out.extend_from_slice(OOM),
        ZSetPopOutcome::JournalError => out.extend_from_slice(b"-ERR append-only write failed\r\n"),
    }
}

fn ordered_zset(items: Vec<(&[u8], f64)>, reverse: bool) -> Vec<(Vec<u8>, f64)> {
    let mut ordered: Vec<(Vec<u8>, f64)> = items
        .into_iter()
        .map(|(member, score)| (member.to_vec(), score))
        .collect();
    ordered.sort_unstable_by(|(member_a, score_a), (member_b, score_b)| {
        score_a
            .total_cmp(score_b)
            .then_with(|| member_a.cmp(member_b))
    });
    if reverse {
        ordered.reverse();
    }
    ordered
}

impl Store {
    fn execute_hash_extended(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let command = args.first().expect("nonempty command");
        if command.eq_ignore_ascii_case(b"HRANDFIELD") {
            if !(2..=4).contains(&args.len()) {
                arity("hrandfield", out);
            } else {
                self.hash_random_field(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"HGETDEL") {
            if args.len() < 5 {
                arity("hgetdel", out);
            } else {
                self.hash_getdel(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"HSETEX") {
            if args.len() < 6 {
                arity("hsetex", out);
            } else {
                self.hash_setex(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"HGETEX") {
            if args.len() < 5 {
                arity("hgetex", out);
            } else {
                self.hash_getex(args, out);
            }
        } else {
            return false;
        }
        true
    }

    fn hash_random_field(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let with_values = args.len() == 4 && args[3].eq_ignore_ascii_case(b"WITHVALUES");
        if args.len() == 4 && !with_values {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        let count = if args.len() >= 3 {
            let Some(value) = parse_i64(args[2]) else {
                out.extend_from_slice(NOT_INTEGER);
                return;
            };
            Some(value)
        } else {
            None
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let items = match data.entries.get(args[1]) {
            None => None,
            Some(entry) => match entry.collection.as_ref().and_then(Collection::hash_items) {
                Some(items) => Some(
                    items
                        .into_iter()
                        .map(|(field, value)| (field.to_vec(), value.to_vec()))
                        .collect::<Vec<_>>(),
                ),
                None => {
                    wrong_type(out);
                    return;
                }
            },
        };
        let Some(items) = items else {
            if count.is_some() {
                out.extend_from_slice(b"*0\r\n");
            } else {
                encode_bulk(None, out);
            }
            return;
        };
        let Some(count) = count else {
            let index = random_index(items.len());
            encode_bulk(items.get(index).map(|(field, _)| field.as_slice()), out);
            return;
        };
        if count == 0 || items.is_empty() {
            out.extend_from_slice(b"*0\r\n");
            return;
        }
        let limit = usize::try_from(count.unsigned_abs()).unwrap_or(usize::MAX);
        if count > 0 {
            let mut pool = items;
            let take = limit.min(pool.len());
            for index in 0..take {
                let swap = index + random_index(pool.len() - index);
                pool.swap(index, swap);
            }
            write!(out, "*{}\r\n", take * if with_values { 2 } else { 1 }).expect("Vec write");
            for (field, value) in pool.iter().take(take) {
                encode_bulk(Some(field), out);
                if with_values {
                    encode_bulk(Some(value), out);
                }
            }
        } else {
            let take = limit.min(MAX_ELEMENTS);
            write!(out, "*{}\r\n", take * if with_values { 2 } else { 1 }).expect("Vec write");
            for _ in 0..take {
                let index = random_index(items.len());
                let (field, value) = &items[index];
                encode_bulk(Some(field), out);
                if with_values {
                    encode_bulk(Some(value), out);
                }
            }
        }
    }

    fn hash_getdel(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if !args[2].eq_ignore_ascii_case(b"FIELDS") {
            out.extend_from_slice(
                b"-ERR Mandatory argument FIELDS is missing or not at the right position\r\n",
            );
            return;
        }
        let Some(numfields) = parse_i64(args[3]).filter(|value| *value > 0) else {
            out.extend_from_slice(b"-ERR Number of fields must be a positive integer\r\n");
            return;
        };
        let Ok(numfields) = usize::try_from(numfields) else {
            out.extend_from_slice(
                b"-ERR The `numfields` parameter must match the number of arguments\r\n",
            );
            return;
        };
        if numfields != args.len() - 4 {
            out.extend_from_slice(
                b"-ERR The `numfields` parameter must match the number of arguments\r\n",
            );
            return;
        }
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            write!(out, "*{numfields}\r\n").expect("Vec write");
            for _ in 0..numfields {
                encode_bulk(None, out);
            }
            return;
        };
        let Some(collection) = entry.collection.as_ref() else {
            wrong_type(out);
            return;
        };
        let Some(items) = collection.hash_items() else {
            wrong_type(out);
            return;
        };
        let expires = entry.expires;
        let previous = entry.payload_len();
        let mut hash: HashMap<Vec<u8>, Vec<u8>> = items
            .into_iter()
            .map(|(field, value)| (field.to_vec(), value.to_vec()))
            .collect();
        let mut values = Vec::with_capacity(numfields);
        let mut deleted = 0usize;
        for field in &args[4..] {
            match hash.remove(*field) {
                Some(value) => {
                    deleted += 1;
                    values.push(Some(value));
                }
                None => values.push(None),
            }
        }
        if deleted > 0 {
            data.remove_field_ttls(key, &args[4..]);
            if hash.is_empty() {
                data.remove(key);
            } else {
                let bytes = hash
                    .iter()
                    .map(|(field, value)| field.len() + value.len())
                    .sum::<usize>();
                let Ok(config) = self.collection_config.read() else {
                    internal_error(out);
                    return;
                };
                let encoded = encode_hash(hash, &config);
                let Some(payload) =
                    data.reserve(key, bytes, Some(previous), self.max_payload_bytes, now)
                else {
                    out.extend_from_slice(OOM);
                    return;
                };
                data.put_collection(key, encoded, expires, payload, now);
            }
        }
        write!(out, "*{numfields}\r\n").expect("Vec write");
        for value in values {
            encode_bulk(value.as_deref(), out);
        }
    }
}

impl Store {
    fn hash_expire(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let name = if args
            .first()
            .is_some_and(|c| c.eq_ignore_ascii_case(b"HPEXPIRE"))
        {
            "hpexpire"
        } else if args
            .first()
            .is_some_and(|c| c.eq_ignore_ascii_case(b"HEXPIRE"))
        {
            "hexpire"
        } else if args
            .first()
            .is_some_and(|c| c.eq_ignore_ascii_case(b"HPEXPIREAT"))
        {
            "hpexpireat"
        } else {
            "hexpireat"
        };
        let command = args.first().expect("nonempty command");
        let seconds =
            command.eq_ignore_ascii_case(b"HEXPIRE") || command.eq_ignore_ascii_case(b"HEXPIREAT");
        let absolute = command.eq_ignore_ascii_case(b"HPEXPIREAT")
            || command.eq_ignore_ascii_case(b"HEXPIREAT");
        if args.len() < 6 {
            arity(name, out);
            return;
        }
        let Some(value) = parse_i64(args[2]) else {
            out.extend_from_slice(NOT_INTEGER);
            return;
        };
        if value < 0 {
            out.extend_from_slice(b"-ERR invalid expire time, must be >= 0\r\n");
            return;
        }
        let condition = if args[3].eq_ignore_ascii_case(b"NX") {
            Some(b'N')
        } else if args[3].eq_ignore_ascii_case(b"XX") {
            Some(b'X')
        } else if args[3].eq_ignore_ascii_case(b"GT") {
            Some(b'G')
        } else if args[3].eq_ignore_ascii_case(b"LT") {
            Some(b'L')
        } else {
            None
        };
        let fields_index = if condition.is_some() { 4 } else { 3 };
        if !args
            .get(fields_index)
            .is_some_and(|token| token.eq_ignore_ascii_case(b"FIELDS"))
        {
            out.extend_from_slice(
                b"-ERR Mandatory argument FIELDS is missing or not at the right position\r\n",
            );
            return;
        }
        let Some(count) = parse_i64(args[fields_index + 1]).filter(|count| *count > 0) else {
            out.extend_from_slice(b"-ERR Parameter `numFields` should be greater than 0\r\n");
            return;
        };
        let Ok(count) = usize::try_from(count) else {
            out.extend_from_slice(b"-ERR Parameter `numFields` should be greater than 0\r\n");
            return;
        };
        let fields_start = fields_index + 2;
        if fields_start + count != args.len() {
            out.extend_from_slice(
                b"-ERR The `numfields` parameter must match the number of arguments\r\n",
            );
            return;
        }
        let fields = &args[fields_start..];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.clock.now_ms().max(0);
        data.expire_key(args[1], now);
        let Some(entry) = data.entries.get(args[1]) else {
            array(fields.len(), out);
            for _ in fields {
                integer(-2, out);
            }
            return;
        };
        if !entry.collection.as_ref().is_some_and(Collection::is_hash) {
            wrong_type(out);
            return;
        }
        let Some(scaled) = value.checked_mul(if seconds { 1000 } else { 1 }) else {
            out.extend_from_slice(b"-ERR invalid expire time, must be >= 0\r\n");
            return;
        };
        let Some(deadline) = (if absolute {
            Some(scaled)
        } else {
            now.checked_add(scaled)
        }) else {
            out.extend_from_slice(b"-ERR invalid expire time, must be >= 0\r\n");
            return;
        };
        array(fields.len(), out);
        let mut updates = Vec::new();
        for field in fields {
            let exists = data.entries[args[1]]
                .collection
                .as_ref()
                .and_then(|collection| collection.hash_get(field))
                .is_some_and(|_| true);
            if !exists {
                integer(-2, out);
                continue;
            }
            let current = data.entries[args[1]].field_ttl.get(*field).copied();
            let apply = match condition {
                Some(b'N') => current.is_none(),
                Some(b'X') => current.is_some(),
                Some(b'G') => current.is_some_and(|current| deadline > current),
                Some(b'L') => current.is_none_or(|current| deadline < current),
                _ => true,
            };
            if !apply {
                integer(0, out);
                continue;
            }
            updates.push((*field, deadline));
            integer(1, out);
        }
        let key = args[1];
        for (field, deadline) in updates {
            data.set_field_ttl(key, field, deadline);
        }
        data.touch(key);
    }

    fn hash_ttl(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let command = args.first().expect("nonempty command");
        let name = if command.eq_ignore_ascii_case(b"HPTTL") {
            "hpttl"
        } else if command.eq_ignore_ascii_case(b"HTTL") {
            "httl"
        } else if command.eq_ignore_ascii_case(b"HPEXPIRETIME") {
            "hpexpiretime"
        } else {
            "hexpiretime"
        };
        let seconds =
            command.eq_ignore_ascii_case(b"HTTL") || command.eq_ignore_ascii_case(b"HEXPIRETIME");
        let absolute = command.eq_ignore_ascii_case(b"HPEXPIRETIME")
            || command.eq_ignore_ascii_case(b"HEXPIRETIME");
        if args.len() < 5 || !args[2].eq_ignore_ascii_case(b"FIELDS") {
            arity(name, out);
            return;
        }
        let Some(count) = parse_i64(args[3]).filter(|count| *count > 0) else {
            out.extend_from_slice(b"-ERR Number of fields must be a positive integer\r\n");
            return;
        };
        let Ok(count) = usize::try_from(count) else {
            out.extend_from_slice(b"-ERR Number of fields must be a positive integer\r\n");
            return;
        };
        if 4 + count != args.len() {
            out.extend_from_slice(
                b"-ERR The `numfields` parameter must match the number of arguments\r\n",
            );
            return;
        }
        let fields = &args[4..];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.clock.now_ms().max(0);
        data.expire_key(args[1], now);
        let Some(entry) = data.entries.get(args[1]) else {
            array(fields.len(), out);
            for _ in fields {
                integer(-2, out);
            }
            return;
        };
        if !entry.collection.as_ref().is_some_and(Collection::is_hash) {
            wrong_type(out);
            return;
        }
        array(fields.len(), out);
        for field in fields {
            let exists = data.entries[args[1]]
                .collection
                .as_ref()
                .and_then(|collection| collection.hash_get(field))
                .is_some_and(|_| true);
            if !exists {
                integer(-2, out);
                continue;
            }
            let Some(deadline) = data.entries[args[1]].field_ttl.get(*field).copied() else {
                integer(-1, out);
                continue;
            };
            let value = if absolute {
                deadline
            } else {
                (deadline - now).max(0)
            };
            integer(
                if seconds {
                    value.saturating_add(999) / 1000
                } else {
                    value
                },
                out,
            );
        }
    }

    fn hash_persist(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if args.len() < 5 || !args[2].eq_ignore_ascii_case(b"FIELDS") {
            arity("hpersist", out);
            return;
        }
        let Some(count) = parse_i64(args[3]).filter(|count| *count > 0) else {
            out.extend_from_slice(b"-ERR Number of fields must be a positive integer\r\n");
            return;
        };
        let Ok(count) = usize::try_from(count) else {
            out.extend_from_slice(b"-ERR Number of fields must be a positive integer\r\n");
            return;
        };
        if 4 + count != args.len() {
            out.extend_from_slice(
                b"-ERR The `numfields` parameter must match the number of arguments\r\n",
            );
            return;
        }
        let fields = &args[4..];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.clock.now_ms().max(0);
        data.expire_key(args[1], now);
        let Some(entry) = data.entries.get(args[1]) else {
            array(fields.len(), out);
            for _ in fields {
                integer(-2, out);
            }
            return;
        };
        if !entry.collection.as_ref().is_some_and(Collection::is_hash) {
            wrong_type(out);
            return;
        }
        array(fields.len(), out);
        let mut removed = false;
        for field in fields {
            let exists = data.entries[args[1]]
                .collection
                .as_ref()
                .and_then(|collection| collection.hash_get(field))
                .is_some_and(|_| true);
            if !exists {
                integer(-2, out);
            } else if data.persist_field(args[1], field) {
                removed = true;
                integer(1, out);
            } else {
                integer(-1, out);
            }
        }
        if removed {
            data.touch(args[1]);
        }
    }

    fn hash_setex(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some(options) = parse_hash_field_options(args, true, true, false, out) else {
            return;
        };
        let Some((_count, start)) = parse_hash_fields_block(args, options.fields_index, true, out)
        else {
            return;
        };
        let key = args[1];
        let pairs = args[start..].as_chunks::<2>().0;
        let fields: Vec<&[u8]> = pairs.iter().map(|pair| pair[0]).collect();
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.clock.now_ms().max(0);
        data.expire_key(key, now);
        let deadline = if options.ttl.needs_value() {
            let Some(deadline) = resolve_field_deadline(
                options.ttl,
                options.ttl_value.expect("TTL option parsed with a value"),
                now,
            ) else {
                out.extend_from_slice(b"-ERR invalid expire time\r\n");
                return;
            };
            Some(deadline)
        } else {
            None
        };
        let current = data.entries.get(key);
        if current.is_some_and(|entry| !entry.collection.as_ref().is_some_and(Collection::is_hash))
        {
            wrong_type(out);
            return;
        }
        let mut hash: HashMap<Vec<u8>, Vec<u8>> = current
            .and_then(|entry| entry.collection.as_ref().and_then(Collection::hash_items))
            .unwrap_or_default()
            .into_iter()
            .map(|(field, value)| (field.to_vec(), value.to_vec()))
            .collect();
        let condition_met = match options.condition {
            None => true,
            Some(true) => !fields.iter().any(|field| hash.contains_key(*field)),
            Some(false) => fields.iter().all(|field| hash.contains_key(*field)),
        };
        if !condition_met {
            integer(0, out);
            return;
        }
        for pair in pairs {
            hash.insert(pair[0].to_vec(), pair[1].to_vec());
        }
        if hash.len() > MAX_ELEMENTS {
            out.extend_from_slice(OOM);
            return;
        }
        let expires = current.and_then(|entry| entry.expires);
        let bytes = hash
            .iter()
            .map(|(field, value)| field.len() + value.len())
            .sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_hash(hash, &config);
        let previous = current.map(Entry::payload_len);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        match options.ttl {
            FieldTtlMode::Default => data.remove_field_ttls(key, &fields),
            FieldTtlMode::Keep => {}
            _ => {
                let deadline = deadline.expect("deadline resolved for TTL option");
                for field in fields {
                    data.set_field_ttl(key, field, deadline);
                }
            }
        }
        integer(1, out);
    }

    fn hash_getex(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some(options) = parse_hash_field_options(args, false, false, true, out) else {
            return;
        };
        let Some((count, start)) = parse_hash_fields_block(args, options.fields_index, false, out)
        else {
            return;
        };
        let key = args[1];
        let fields = &args[start..];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.clock.now_ms().max(0);
        data.expire_key(key, now);
        let deadline = match options.ttl {
            FieldTtlMode::Ex | FieldTtlMode::Px | FieldTtlMode::ExAt | FieldTtlMode::PxAt => {
                let Some(deadline) = resolve_field_deadline(
                    options.ttl,
                    options.ttl_value.expect("TTL option parsed with a value"),
                    now,
                ) else {
                    out.extend_from_slice(b"-ERR invalid expire time\r\n");
                    return;
                };
                Some(deadline)
            }
            _ => None,
        };
        let Some(entry) = data.entries.get(key) else {
            array(count, out);
            for _ in 0..count {
                encode_bulk(None, out);
            }
            return;
        };
        if !entry.collection.as_ref().is_some_and(Collection::is_hash) {
            wrong_type(out);
            return;
        }
        let values: Vec<Option<Vec<u8>>> = fields
            .iter()
            .map(|field| {
                entry
                    .collection
                    .as_ref()
                    .and_then(|collection| collection.hash_get(field))
                    .map(<[u8]>::to_vec)
            })
            .collect();
        let mut changed = false;
        match options.ttl {
            FieldTtlMode::Persist => {
                for (index, field) in fields.iter().enumerate() {
                    if values[index].is_some() && data.persist_field(key, field) {
                        changed = true;
                    }
                }
            }
            FieldTtlMode::Default | FieldTtlMode::Keep => {}
            _ => {
                let deadline = deadline.expect("deadline resolved for TTL option");
                for (index, field) in fields.iter().enumerate() {
                    if values[index].is_some() {
                        data.set_field_ttl(key, field, deadline);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            data.touch(key);
        }
        array(count, out);
        for value in values {
            encode_bulk(value.as_deref(), out);
        }
    }

    fn hash_setnx(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        if args.len() != 4 {
            arity("hsetnx", out);
            return;
        }
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let current = data.entries.get(key);
        if current.is_some_and(|entry| !entry.collection.as_ref().is_some_and(Collection::is_hash))
        {
            wrong_type(out);
            return;
        }
        let mut hash: HashMap<Vec<u8>, Vec<u8>> = current
            .and_then(|entry| entry.collection.as_ref().and_then(Collection::hash_items))
            .unwrap_or_default()
            .into_iter()
            .map(|(field, value)| (field.to_vec(), value.to_vec()))
            .collect();
        if hash.contains_key(args[2]) {
            integer(0, out);
            return;
        }
        hash.insert(args[2].to_vec(), args[3].to_vec());
        if hash.len() > MAX_ELEMENTS {
            out.extend_from_slice(OOM);
            return;
        }
        let expires = current.and_then(|entry| entry.expires);
        let bytes = hash
            .iter()
            .map(|(field, value)| field.len() + value.len())
            .sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_hash(hash, &config);
        let previous = current.map(Entry::payload_len);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        integer(1, out);
    }

    fn hash_del(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            integer(0, out);
            return;
        };
        if !entry.collection.as_ref().is_some_and(Collection::is_hash) {
            wrong_type(out);
            return;
        }
        let mut hash: HashMap<Vec<u8>, Vec<u8>> = entry
            .collection
            .as_ref()
            .and_then(Collection::hash_items)
            .expect("hash type checked")
            .into_iter()
            .map(|(field, value)| (field.to_vec(), value.to_vec()))
            .collect();
        let expires = entry.expires;
        let previous = entry.payload_len();
        let mut removed = 0i64;
        for field in &args[2..] {
            if hash.remove(*field).is_some() {
                removed += 1;
            }
        }
        if removed == 0 {
            integer(0, out);
            return;
        }
        data.remove_field_ttls(key, &args[2..]);
        if hash.is_empty() {
            data.remove(key);
            integer(removed, out);
            return;
        }
        let bytes = hash
            .iter()
            .map(|(field, value)| field.len() + value.len())
            .sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_hash(hash, &config);
        let Some(payload) = data.reserve(key, bytes, Some(previous), self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        integer(removed, out);
    }

    fn hash_exists(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let exists = match data.entries.get(args[1]) {
            None => false,
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_hash) => entry
                .collection
                .as_ref()
                .and_then(|collection| collection.hash_get(args[2]))
                .is_some(),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        integer(i64::from(exists), out);
    }

    fn hash_incr(&self, args: &[&[u8]], float: bool, out: &mut Vec<u8>) {
        let key = args[1];
        let field = args[2];
        if float {
            let Some(text) = std::str::from_utf8(args[3]).ok() else {
                out.extend_from_slice(b"-ERR value is not a valid float\r\n");
                return;
            };
            let Ok(amount) = text.parse::<f64>() else {
                out.extend_from_slice(b"-ERR value is not a valid float\r\n");
                return;
            };
            if amount.is_nan() {
                out.extend_from_slice(b"-ERR value is not a valid float\r\n");
                return;
            }
            if !amount.is_finite() {
                out.extend_from_slice(b"-ERR value is NaN or Infinity\r\n");
                return;
            }
            self.hash_incr_float(key, field, amount, out);
        } else {
            let Some(amount) = parse_integer(args[3]) else {
                out.extend_from_slice(NOT_INTEGER);
                return;
            };
            self.hash_incr_integer(key, field, amount, out);
        }
    }

    fn hash_incr_integer(&self, key: &[u8], field: &[u8], amount: i64, out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let current = data.entries.get(key);
        if current.is_some_and(|entry| !entry.collection.as_ref().is_some_and(Collection::is_hash))
        {
            wrong_type(out);
            return;
        }
        let mut hash: HashMap<Vec<u8>, Vec<u8>> = current
            .and_then(|entry| entry.collection.as_ref().and_then(Collection::hash_items))
            .unwrap_or_default()
            .into_iter()
            .map(|(field, value)| (field.to_vec(), value.to_vec()))
            .collect();
        let value = match hash.get(field) {
            Some(value) => match parse_integer(value) {
                Some(value) => value,
                None => {
                    out.extend_from_slice(b"-ERR hash value is not an integer\r\n");
                    return;
                }
            },
            None => 0,
        };
        let Some(value) = value.checked_add(amount) else {
            out.extend_from_slice(b"-ERR increment or decrement would overflow\r\n");
            return;
        };
        hash.insert(field.to_vec(), value.to_string().into_bytes());
        let expires = current.and_then(|entry| entry.expires);
        let previous = current.map(Entry::payload_len);
        let bytes = hash
            .iter()
            .map(|(field, value)| field.len() + value.len())
            .sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_hash(hash, &config);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        integer(value, out);
    }

    fn hash_incr_float(&self, key: &[u8], field: &[u8], amount: f64, out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let current = data.entries.get(key);
        if current.is_some_and(|entry| !entry.collection.as_ref().is_some_and(Collection::is_hash))
        {
            wrong_type(out);
            return;
        }
        let mut hash: HashMap<Vec<u8>, Vec<u8>> = current
            .and_then(|entry| entry.collection.as_ref().and_then(Collection::hash_items))
            .unwrap_or_default()
            .into_iter()
            .map(|(field, value)| (field.to_vec(), value.to_vec()))
            .collect();
        let value = match hash.get(field) {
            Some(value) => match std::str::from_utf8(value)
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
            {
                Some(value) if value.is_finite() => value,
                _ => {
                    out.extend_from_slice(b"-ERR hash value is not a float\r\n");
                    return;
                }
            },
            None => 0.0,
        };
        let Some(value) = Some(value + amount).filter(|value| value.is_finite()) else {
            out.extend_from_slice(b"-ERR increment would produce NaN or Infinity\r\n");
            return;
        };
        let value = if value == 0.0 { 0.0 } else { value };
        hash.insert(field.to_vec(), value.to_string().into_bytes());
        let expires = current.and_then(|entry| entry.expires);
        let previous = current.map(Entry::payload_len);
        let bytes = hash
            .iter()
            .map(|(field, value)| field.len() + value.len())
            .sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_hash(hash, &config);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        encode_bulk(Some(value.to_string().as_bytes()), out);
    }

    fn hash_mget(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let collection = match data.entries.get(args[1]) {
            None => None,
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_hash) => {
                entry.collection.as_ref()
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        write!(out, "*{}\r\n", args.len() - 2).expect("Vec write");
        for field in &args[2..] {
            encode_bulk(
                collection.and_then(|collection| collection.hash_get(field)),
                out,
            );
        }
    }

    fn hash_keys_values(&self, args: &[&[u8]], fields: bool, out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let mut items = match data.entries.get(args[1]) {
            None => {
                out.extend_from_slice(b"*0\r\n");
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_hash) => entry
                .collection
                .as_ref()
                .and_then(Collection::hash_items)
                .expect("hash type checked"),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        items.sort_unstable_by(|a, b| a.0.cmp(b.0));
        write!(out, "*{}\r\n", items.len()).expect("Vec write");
        for (field, value) in items {
            encode_bulk(Some(if fields { field } else { value }), out);
        }
    }

    fn hash_strlen(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let length = match data.entries.get(args[1]) {
            None => 0,
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_hash) => entry
                .collection
                .as_ref()
                .and_then(|collection| collection.hash_get(args[2]))
                .map_or(0, |value| value.len()),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        integer(length as i64, out);
    }
}

impl Store {
    fn execute_list_extended(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let command = args.first().expect("nonempty command");
        if command.eq_ignore_ascii_case(b"LPOS") {
            if args.len() < 3 {
                arity("lpos", out);
            } else {
                self.list_position(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"LMPOP") {
            if args.len() < 4 {
                arity("lmpop", out);
            } else {
                self.list_mpop(args, 1, out);
            }
        } else if command.eq_ignore_ascii_case(b"BLMPOP") {
            if args.len() < 5 {
                arity("blmpop", out);
            } else {
                match parse_bzmpop_timeout(args[1]) {
                    Ok(_) => self.list_mpop(args, 2, out),
                    Err(error) => out.extend_from_slice(error),
                }
            }
        } else if command.eq_ignore_ascii_case(b"LMOVE") {
            if args.len() != 5 {
                arity("lmove", out);
            } else {
                self.list_move_command(args, args[3], args[4], out);
            }
        } else if command.eq_ignore_ascii_case(b"RPOPLPUSH") {
            if args.len() != 3 {
                arity("rpoplpush", out);
            } else {
                self.list_move_command(args, b"RIGHT", b"LEFT", out);
            }
        } else if command.eq_ignore_ascii_case(b"BLMOVE") {
            if args.len() != 6 {
                arity("blmove", out);
            } else {
                self.list_move_command(args, args[3], args[4], out);
            }
        } else if command.eq_ignore_ascii_case(b"BRPOPLPUSH") {
            if args.len() != 4 {
                arity("brpoplpush", out);
            } else {
                self.list_move_command(args, b"RIGHT", b"LEFT", out);
            }
        } else {
            return false;
        }
        true
    }

    fn list_position(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let mut rank = 1i64;
        let mut count = None;
        let mut maxlen = 0i64;
        let mut index = 3;
        while index < args.len() {
            if args[index].eq_ignore_ascii_case(b"RANK") && index + 1 < args.len() {
                let Some(value) = parse_i64(args[index + 1]) else {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                };
                if value == 0 {
                    out.extend_from_slice(
                        b"-ERR RANK can't be zero: use 1 to start from the first match, 2 from the second ... or use negative to start from the end of the list\r\n",
                    );
                    return;
                }
                rank = value;
                index += 2;
            } else if args[index].eq_ignore_ascii_case(b"COUNT") && index + 1 < args.len() {
                let Some(value) = parse_i64(args[index + 1]) else {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                };
                if value < 0 {
                    out.extend_from_slice(b"-ERR COUNT can't be negative\r\n");
                    return;
                }
                count = Some(usize::try_from(value).unwrap_or(usize::MAX));
                index += 2;
            } else if args[index].eq_ignore_ascii_case(b"MAXLEN") && index + 1 < args.len() {
                let Some(value) = parse_i64(args[index + 1]) else {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                };
                if value < 0 {
                    out.extend_from_slice(b"-ERR MAXLEN can't be negative\r\n");
                    return;
                }
                maxlen = value;
                index += 2;
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let values = match data.entries.get(args[1]) {
            None => None,
            Some(entry) => match entry.collection.as_ref().and_then(Collection::list_items) {
                Some(values) => Some(values),
                None => {
                    wrong_type(out);
                    return;
                }
            },
        };
        let Some(values) = values else {
            if count.is_some() {
                out.extend_from_slice(b"*0\r\n");
            } else {
                encode_bulk(None, out);
            }
            return;
        };
        let matches = if count == Some(0) {
            usize::MAX
        } else {
            count.unwrap_or(1)
        };
        let mut seen = 0i64;
        let mut result = Vec::new();
        if rank > 0 {
            for (position, value) in values.iter().enumerate() {
                if maxlen > 0 && position as i64 >= maxlen {
                    break;
                }
                if *value != args[2] {
                    continue;
                }
                seen += 1;
                if seen >= rank {
                    if count.is_none() {
                        integer(position as i64, out);
                        return;
                    }
                    result.push(position);
                    if result.len() >= matches {
                        break;
                    }
                }
            }
        } else {
            for (offset, position) in (0..values.len()).rev().enumerate() {
                if maxlen > 0 && offset as i64 >= maxlen {
                    break;
                }
                if values[position] != args[2] {
                    continue;
                }
                seen += 1;
                if -seen <= rank {
                    if count.is_none() {
                        integer(position as i64, out);
                        return;
                    }
                    result.push(position);
                    if result.len() >= matches {
                        break;
                    }
                }
            }
        }
        if count.is_none() {
            encode_bulk(None, out);
            return;
        }
        write!(out, "*{}\r\n", result.len()).expect("Vec write");
        for position in result {
            integer(position as i64, out);
        }
    }

    fn list_mpop(&self, args: &[&[u8]], numkeys_index: usize, out: &mut Vec<u8>) {
        let options = match parse_list_mpop(args, numkeys_index) {
            Ok(options) => options,
            Err(error) => {
                out.extend_from_slice(error);
                return;
            }
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let outcome =
            self.list_pop_in_data(&mut data, &options.keys, options.left, options.count, now);
        list_pop_reply(outcome, out);
    }

    pub(super) fn list_pop_in_data(
        &self,
        data: &mut Data,
        keys: &[&[u8]],
        left: bool,
        count: usize,
        now: i64,
    ) -> ListPopOutcome {
        for key in keys {
            let mut list = match owned_list(data, key, now) {
                Ok(Some(list)) => list,
                Ok(None) => continue,
                Err(()) => return ListPopOutcome::WrongType,
            };
            if list.is_empty() {
                continue;
            }
            let entry = data.entries.get(*key).expect("list exists");
            let expires = entry.expires;
            let previous = entry.payload_len();
            let take = count.min(list.len());
            let mut values = Vec::with_capacity(take);
            for _ in 0..take {
                let value = if left {
                    list.pop_front().expect("nonempty list")
                } else {
                    list.pop_back().expect("nonempty list")
                };
                values.push(value);
            }
            let encoded = if list.is_empty() {
                None
            } else {
                let Ok(config) = self.collection_config.read() else {
                    return ListPopOutcome::JournalError;
                };
                Some(encode_list(list, config.list_max_listpack_size))
            };
            if let Some(encoded) = &encoded {
                let Some(payload) = data.reserve(
                    key,
                    encoded.payload_len(),
                    Some(previous),
                    self.max_payload_bytes,
                    now,
                ) else {
                    return ListPopOutcome::Oom;
                };
                data.put_collection(key, encoded.clone(), expires, payload, now);
            } else {
                data.remove(key);
            }
            return ListPopOutcome::Popped {
                key: key.to_vec(),
                values,
            };
        }
        ListPopOutcome::Empty
    }

    fn list_move_command(&self, args: &[&[u8]], from_raw: &[u8], to_raw: &[u8], out: &mut Vec<u8>) {
        let (Some(from_left), Some(to_left)) = (parse_side(from_raw), parse_side(to_raw)) else {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let outcome = self.list_move_in_data(&mut data, args[1], args[2], from_left, to_left, now);
        list_move_reply(outcome, out);
    }

    pub(super) fn list_move_in_data(
        &self,
        data: &mut Data,
        source: &[u8],
        destination: &[u8],
        from_left: bool,
        to_left: bool,
        now: i64,
    ) -> ListMoveOutcome {
        data.expire_key(source, now);
        let Some(source_entry) = data.entries.get(source) else {
            return ListMoveOutcome::Empty;
        };
        let Some(mut source_list) = source_entry
            .collection
            .as_ref()
            .and_then(Collection::list_values)
        else {
            return ListMoveOutcome::WrongType;
        };
        if source_list.is_empty() {
            return ListMoveOutcome::Empty;
        }
        let source_expires = source_entry.expires;
        let source_previous = source_entry.payload_len();
        let same = source == destination;
        let mut destination_list = if same {
            VecDeque::new()
        } else {
            data.expire_key(destination, now);
            match data.entries.get(destination) {
                None => VecDeque::new(),
                Some(entry) => {
                    let Some(list) = entry.collection.as_ref().and_then(Collection::list_values)
                    else {
                        return ListMoveOutcome::WrongType;
                    };
                    list
                }
            }
        };
        let value = if from_left {
            source_list.pop_front().expect("nonempty list")
        } else {
            source_list.pop_back().expect("nonempty list")
        };
        let source_emptied = !same && source_list.is_empty();
        if same {
            if to_left {
                source_list.push_front(value.clone());
            } else {
                source_list.push_back(value.clone());
            }
        } else if to_left {
            destination_list.push_front(value.clone());
        } else {
            destination_list.push_back(value.clone());
        }
        let source_bytes = source_list.iter().map(Vec::len).sum::<usize>();
        let destination_bytes = destination_list.iter().map(Vec::len).sum::<usize>();
        let new_payload = if same {
            data.payload_bytes
                .checked_sub(source_previous)
                .and_then(|bytes| bytes.checked_add(source_bytes))
        } else {
            let destination_previous = data.entries.get(destination).map(|entry| entry.payload_len);
            data.payload_bytes
                .checked_sub(source_previous + source.len())
                .and_then(|bytes| match source_list.is_empty() {
                    true => Some(bytes),
                    false => bytes.checked_add(source_bytes + source.len()),
                })
                .and_then(|bytes| match destination_previous {
                    Some(previous) => bytes.checked_sub(previous),
                    None => bytes.checked_add(destination.len()),
                })
                .and_then(|bytes| bytes.checked_add(destination_bytes))
        };
        let Some(new_payload) = new_payload.filter(|bytes| *bytes <= self.max_payload_bytes) else {
            return ListMoveOutcome::Oom;
        };
        let Ok(config) = self.collection_config.read() else {
            return ListMoveOutcome::JournalError;
        };
        let destination_expires = if same {
            source_expires
        } else {
            data.entries
                .get(destination)
                .and_then(|entry| entry.expires)
        };
        data.remove(source);
        if same {
            data.put_collection(
                destination,
                encode_list(source_list, config.list_max_listpack_size),
                destination_expires,
                0,
                now,
            );
        } else {
            data.remove(destination);
            if !source_list.is_empty() {
                data.put_collection(
                    source,
                    encode_list(source_list, config.list_max_listpack_size),
                    source_expires,
                    0,
                    now,
                );
            }
            data.put_collection(
                destination,
                encode_list(destination_list, config.list_max_listpack_size),
                destination_expires,
                0,
                now,
            );
        }
        data.payload_bytes = new_payload;
        ListMoveOutcome::Moved {
            source: source.to_vec(),
            value,
            source_emptied,
        }
    }

    fn list_index(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let list = match data.entries.get(args[1]) {
            None => {
                encode_bulk(None, out);
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_list) => entry
                .collection
                .as_ref()
                .and_then(Collection::list_items)
                .expect("list type checked"),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let Some(index) = parse_i64(args[2]) else {
            out.extend_from_slice(NOT_INTEGER);
            return;
        };
        let value = normalize_index(index, list.len()).map(|index| list[index]);
        encode_bulk(value, out);
    }

    fn list_insert(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let before = if args[2].eq_ignore_ascii_case(b"BEFORE") {
            true
        } else if args[2].eq_ignore_ascii_case(b"AFTER") {
            false
        } else {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        };
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            integer(0, out);
            return;
        };
        let Some(mut list) = entry.collection.as_ref().and_then(Collection::list_values) else {
            wrong_type(out);
            return;
        };
        let Some(position) = list.iter().position(|value| value.as_slice() == args[3]) else {
            integer(-1, out);
            return;
        };
        let position = if before { position } else { position + 1 };
        if list.len() >= MAX_ELEMENTS {
            out.extend_from_slice(OOM);
            return;
        }
        list.insert(position, args[4].to_vec());
        let expires = entry.expires;
        let previous = entry.payload_len();
        let bytes = list.iter().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_list(list, config.list_max_listpack_size);
        let Some(payload) = data.reserve(key, bytes, Some(previous), self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        let length = encoded.list_items().expect("list encoding").len();
        data.put_collection(key, encoded, expires, payload, now);
        integer(length as i64, out);
    }

    fn list_remove(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some(count) = parse_i64(args[2]) else {
            out.extend_from_slice(NOT_INTEGER);
            return;
        };
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            integer(0, out);
            return;
        };
        let Some(list) = entry.collection.as_ref().and_then(Collection::list_values) else {
            wrong_type(out);
            return;
        };
        let limit = if count == 0 {
            usize::MAX
        } else {
            usize::try_from(count.unsigned_abs()).unwrap_or(usize::MAX)
        };
        let mut removed = 0usize;
        let mut result = VecDeque::with_capacity(list.len());
        if count >= 0 {
            for value in list {
                if removed < limit && value.as_slice() == args[3] {
                    removed += 1;
                } else {
                    result.push_back(value);
                }
            }
        } else {
            for value in list.into_iter().rev() {
                if removed < limit && value.as_slice() == args[3] {
                    removed += 1;
                } else {
                    result.push_front(value);
                }
            }
        }
        if removed == 0 {
            integer(0, out);
            return;
        }
        if result.is_empty() {
            data.remove(key);
            integer(removed as i64, out);
            return;
        }
        let expires = entry.expires;
        let previous = entry.payload_len();
        let bytes = result.iter().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_list(result, config.list_max_listpack_size);
        let Some(payload) = data.reserve(key, bytes, Some(previous), self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        integer(removed as i64, out);
    }

    fn list_set(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            out.extend_from_slice(b"-ERR no such key\r\n");
            return;
        };
        let Some(mut list) = entry.collection.as_ref().and_then(Collection::list_values) else {
            wrong_type(out);
            return;
        };
        let Some(index) = parse_i64(args[2]) else {
            out.extend_from_slice(NOT_INTEGER);
            return;
        };
        let Some(position) = normalize_index(index, list.len()) else {
            out.extend_from_slice(b"-ERR index out of range\r\n");
            return;
        };
        list[position] = args[3].to_vec();
        let expires = entry.expires;
        let previous = entry.payload_len();
        let bytes = list.iter().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_list(list, config.list_max_listpack_size);
        let Some(payload) = data.reserve(key, bytes, Some(previous), self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        out.extend_from_slice(b"+OK\r\n");
    }

    fn list_trim(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let (Some(start), Some(stop)) = (parse_i64(args[2]), parse_i64(args[3])) else {
            out.extend_from_slice(NOT_INTEGER);
            return;
        };
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            out.extend_from_slice(b"+OK\r\n");
            return;
        };
        let Some(list) = entry.collection.as_ref().and_then(Collection::list_values) else {
            wrong_type(out);
            return;
        };
        let len = list.len() as i128;
        let first = if start < 0 {
            (len + start as i128).max(0)
        } else {
            start as i128
        };
        let last = if stop < 0 {
            len + stop as i128
        } else {
            stop as i128
        };
        if first >= len || first > last {
            data.remove(key);
            out.extend_from_slice(b"+OK\r\n");
            return;
        }
        let begin = first as usize;
        let end = (last.min(len - 1) as usize) + 1;
        let result: VecDeque<Vec<u8>> = list.into_iter().skip(begin).take(end - begin).collect();
        if result.is_empty() {
            data.remove(key);
            out.extend_from_slice(b"+OK\r\n");
            return;
        }
        let expires = entry.expires;
        let previous = entry.payload_len();
        let bytes = result.iter().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_list(result, config.list_max_listpack_size);
        let Some(payload) = data.reserve(key, bytes, Some(previous), self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        out.extend_from_slice(b"+OK\r\n");
    }

    fn list_push_existing(&self, args: &[&[u8]], left: bool, out: &mut Vec<u8>) {
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let Some(entry) = data.entries.get(key) else {
            integer(0, out);
            return;
        };
        let Some(mut list) = entry.collection.as_ref().and_then(Collection::list_values) else {
            wrong_type(out);
            return;
        };
        if list
            .len()
            .checked_add(args.len() - 2)
            .is_none_or(|length| length > MAX_ELEMENTS)
        {
            out.extend_from_slice(OOM);
            return;
        }
        for value in &args[2..] {
            if left {
                list.push_front(value.to_vec());
            } else {
                list.push_back(value.to_vec());
            }
        }
        let expires = entry.expires;
        let previous = entry.payload_len();
        let bytes = list.iter().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_list(list, config.list_max_listpack_size);
        let Some(payload) = data.reserve(key, bytes, Some(previous), self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        let length = encoded.list_items().expect("list encoding").len();
        data.put_collection(key, encoded, expires, payload, now);
        integer(length as i64, out);
    }
}

fn owned_set(data: &mut Data, key: &[u8], now: i64) -> Result<Option<BTreeSet<Vec<u8>>>, ()> {
    data.expire_key(key, now);
    match data.entries.get(key) {
        None => Ok(None),
        Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_set) => Ok(Some(
            entry
                .collection
                .as_ref()
                .and_then(Collection::set_items)
                .expect("set type checked")
                .into_iter()
                .collect(),
        )),
        Some(_) => Err(()),
    }
}

impl Store {
    fn set_remove(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let set = match owned_set(&mut data, key, now) {
            Ok(Some(set)) => set,
            Ok(None) => {
                integer(0, out);
                return;
            }
            Err(()) => {
                wrong_type(out);
                return;
            }
        };
        let entry = data.entries.get(key).expect("set exists");
        let expires = entry.expires;
        let previous = entry.payload_len();
        let mut set = set;
        let mut removed = 0i64;
        for member in &args[2..] {
            if set.remove(*member) {
                removed += 1;
            }
        }
        if removed == 0 {
            integer(0, out);
            return;
        }
        if set.is_empty() {
            data.remove(key);
            integer(removed, out);
            return;
        }
        let bytes = set.iter().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_set(set, &config);
        let Some(payload) = data.reserve(key, bytes, Some(previous), self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        integer(removed, out);
    }

    fn set_mismember(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let members = match data.entries.get(args[1]) {
            None => None,
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_set) => Some(
                entry
                    .collection
                    .as_ref()
                    .and_then(Collection::set_items)
                    .expect("set type checked"),
            ),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        write!(out, "*{}\r\n", args.len() - 2).expect("Vec write");
        for member in &args[2..] {
            let exists = members
                .as_ref()
                .is_some_and(|members| members.iter().any(|item| item.as_slice() == *member));
            integer(i64::from(exists), out);
        }
    }

    fn set_move(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let (source, destination, member) = (args[1], args[2], args[3]);
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut source_set = match owned_set(&mut data, source, now) {
            Ok(Some(set)) => set,
            Ok(None) => {
                integer(0, out);
                return;
            }
            Err(()) => {
                wrong_type(out);
                return;
            }
        };
        if !source_set.contains(member) {
            integer(0, out);
            return;
        }
        if source == destination {
            integer(1, out);
            return;
        }
        let mut destination_set = match owned_set(&mut data, destination, now) {
            Ok(Some(set)) => set,
            Ok(None) => BTreeSet::new(),
            Err(()) => {
                wrong_type(out);
                return;
            }
        };
        let source_entry = data.entries.get(source).expect("source checked");
        let source_expires = source_entry.expires;
        let source_previous = source_entry.payload_len();
        let destination_expires = data
            .entries
            .get(destination)
            .and_then(|entry| entry.expires);
        let destination_previous = data.entries.get(destination).map(Entry::payload_len);
        source_set.remove(member);
        destination_set.insert(member.to_vec());
        let source_bytes = source_set.iter().map(Vec::len).sum::<usize>();
        let destination_bytes = destination_set.iter().map(Vec::len).sum::<usize>();
        let mut total = data.payload_bytes;
        for (key, previous) in [
            (source, Some(source_previous)),
            (destination, destination_previous),
        ] {
            if let Some(previous) = previous {
                let Some(next) = total.checked_sub(key.len() + previous) else {
                    out.extend_from_slice(OOM);
                    return;
                };
                total = next;
            }
        }
        if !source_set.is_empty() {
            let Some(next) = total.checked_add(source.len() + source_bytes) else {
                out.extend_from_slice(OOM);
                return;
            };
            total = next;
        }
        if !destination_set.is_empty() {
            let Some(next) = total.checked_add(destination.len() + destination_bytes) else {
                out.extend_from_slice(OOM);
                return;
            };
            total = next;
        }
        if total > self.max_payload_bytes {
            out.extend_from_slice(OOM);
            return;
        }
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        if source_set.is_empty() {
            data.remove(source);
        } else {
            let encoded = encode_set(source_set, &config);
            data.put_collection(source, encoded, source_expires, total, now);
        }
        if destination_set.is_empty() {
            data.remove(destination);
        } else {
            let encoded = encode_set(destination_set, &config);
            data.put_collection(destination, encoded, destination_expires, total, now);
        }
        integer(1, out);
    }

    fn set_algebra(&self, args: &[&[u8]], operation: &str, out: &mut Vec<u8>) {
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut sets = Vec::with_capacity(args.len() - 1);
        for key in &args[1..] {
            match owned_set(&mut data, key, now) {
                Ok(set) => sets.push(set.unwrap_or_default()),
                Err(()) => {
                    wrong_type(out);
                    return;
                }
            }
        }
        let mut result = sets.first().cloned().unwrap_or_default();
        for set in &sets[1..] {
            match operation {
                "sinter" => result.retain(|member| set.contains(member)),
                "sunion" => result.extend(set.iter().cloned()),
                "sdiff" => result.retain(|member| !set.contains(member)),
                _ => unreachable!(),
            }
        }
        write!(out, "*{}\r\n", result.len()).expect("Vec write");
        for member in result {
            encode_bulk(Some(&member), out);
        }
    }

    fn set_intercard(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some(count) =
            parse_i64(args[1]).filter(|count| *count > 0 && *count <= i32::MAX as i64)
        else {
            out.extend_from_slice(b"-ERR numkeys should be greater than 0\r\n");
            return;
        };
        let count = count as usize;
        if args.len() < 2 + count {
            out.extend_from_slice(b"-ERR Number of keys can't be greater than number of args\r\n");
            return;
        }
        let mut limit = 0usize;
        let mut index = 2 + count;
        while index < args.len() {
            if args[index].eq_ignore_ascii_case(b"LIMIT") && index + 1 < args.len() {
                let Some(value) = parse_i64(args[index + 1]).filter(|value| *value >= 0) else {
                    out.extend_from_slice(b"-ERR LIMIT can't be negative\r\n");
                    return;
                };
                limit = usize::try_from(value).unwrap_or(usize::MAX);
                index += 2;
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut sets = Vec::with_capacity(count);
        for key in &args[2..2 + count] {
            match owned_set(&mut data, key, now) {
                Ok(set) => sets.push(set.unwrap_or_default()),
                Err(()) => {
                    wrong_type(out);
                    return;
                }
            }
        }
        let mut result = sets.first().cloned().unwrap_or_default();
        for set in &sets[1..] {
            result.retain(|member| set.contains(member));
            if result.is_empty() {
                break;
            }
        }
        let cardinality = if limit == 0 {
            result.len()
        } else {
            result.len().min(limit)
        };
        integer(cardinality as i64, out);
    }

    fn set_store(&self, args: &[&[u8]], operation: &str, out: &mut Vec<u8>) {
        let destination = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut sets = Vec::with_capacity(args.len() - 2);
        for key in &args[2..] {
            match owned_set(&mut data, key, now) {
                Ok(set) => sets.push(set.unwrap_or_default()),
                Err(()) => {
                    wrong_type(out);
                    return;
                }
            }
        }
        let mut result = sets.first().cloned().unwrap_or_default();
        for set in &sets[1..] {
            match operation {
                "sinterstore" => result.retain(|member| set.contains(member)),
                "sunionstore" => result.extend(set.iter().cloned()),
                "sdiffstore" => result.retain(|member| !set.contains(member)),
                _ => unreachable!(),
            }
        }
        let destination_previous = data.entries.get(destination).map(Entry::payload_len);
        let Some(mut total) = data.payload_bytes.checked_sub(
            destination_previous
                .map(|previous| destination.len() + previous)
                .unwrap_or(0),
        ) else {
            out.extend_from_slice(OOM);
            return;
        };
        if !result.is_empty() {
            let bytes = result.iter().map(Vec::len).sum::<usize>();
            let Some(next) = total.checked_add(destination.len() + bytes) else {
                out.extend_from_slice(OOM);
                return;
            };
            total = next;
        }
        if total > self.max_payload_bytes {
            out.extend_from_slice(OOM);
            return;
        }
        if result.is_empty() {
            data.remove(destination);
            integer(0, out);
            return;
        }
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let length = result.len();
        let encoded = encode_set(result, &config);
        data.put_collection(destination, encoded, None, total, now);
        integer(length as i64, out);
    }

    fn set_pop(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let count = if args.len() == 3 {
            match parse_i64(args[2]) {
                Some(value) if value >= 0 => Some(usize::try_from(value).unwrap_or(usize::MAX)),
                Some(_) => {
                    out.extend_from_slice(b"-ERR value is out of range, must be positive\r\n");
                    return;
                }
                None => {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                }
            }
        } else {
            None
        };
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let set = match owned_set(&mut data, key, now) {
            Ok(Some(set)) => set,
            Ok(None) => {
                if count.is_some() {
                    out.extend_from_slice(b"*0\r\n");
                } else {
                    encode_bulk(None, out);
                }
                return;
            }
            Err(()) => {
                wrong_type(out);
                return;
            }
        };
        let entry = data.entries.get(key).expect("set exists");
        let expires = entry.expires;
        let previous = entry.payload_len();
        let mut set = set;
        let number = count.unwrap_or(1).min(set.len());
        if number == 0 {
            out.extend_from_slice(b"*0\r\n");
            return;
        }
        let mut members = Vec::with_capacity(number);
        for _ in 0..number {
            let index = random_index(set.len());
            let member = set
                .iter()
                .nth(index)
                .expect("index below set length")
                .clone();
            set.remove(&member);
            members.push(member);
        }
        if count.is_some() {
            write!(out, "*{}\r\n", members.len()).expect("Vec write");
            for member in &members {
                encode_bulk(Some(member), out);
            }
        } else {
            encode_bulk(members.first().map(Vec::as_slice), out);
        }
        if set.is_empty() {
            data.remove(key);
        } else {
            let bytes = set.iter().map(Vec::len).sum::<usize>();
            let payload = data.payload_bytes - previous + bytes;
            let Ok(config) = self.collection_config.read() else {
                internal_error(out);
                return;
            };
            let encoded = encode_set(set, &config);
            data.put_collection(key, encoded, expires, payload, now);
        }
    }

    fn set_random_member(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let count = if args.len() == 3 {
            match parse_i64(args[2]) {
                Some(value) => Some(value),
                None => {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                }
            }
        } else {
            None
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let set = match owned_set(&mut data, args[1], now) {
            Ok(Some(set)) => set,
            Ok(None) => {
                if count.is_some() {
                    out.extend_from_slice(b"*0\r\n");
                } else {
                    encode_bulk(None, out);
                }
                return;
            }
            Err(()) => {
                wrong_type(out);
                return;
            }
        };
        let members: Vec<Vec<u8>> = set.into_iter().collect();
        let Some(count) = count else {
            let index = random_index(members.len());
            encode_bulk(members.get(index).map(Vec::as_slice), out);
            return;
        };
        if count == 0 || members.is_empty() {
            out.extend_from_slice(b"*0\r\n");
            return;
        }
        let limit = usize::try_from(count.unsigned_abs()).unwrap_or(usize::MAX);
        if count > 0 {
            let mut pool = members;
            let take = limit.min(pool.len());
            for index in 0..take {
                let swap = index + random_index(pool.len() - index);
                pool.swap(index, swap);
            }
            write!(out, "*{}\r\n", take).expect("Vec write");
            for member in pool.iter().take(take) {
                encode_bulk(Some(member), out);
            }
        } else {
            let take = limit.min(MAX_ELEMENTS);
            write!(out, "*{take}\r\n").expect("Vec write");
            for _ in 0..take {
                let index = random_index(members.len());
                encode_bulk(members.get(index).map(Vec::as_slice), out);
            }
        }
    }
}

fn owned_list(
    data: &mut Data,
    key: &[u8],
    now: i64,
) -> Result<Option<std::collections::VecDeque<Vec<u8>>>, ()> {
    data.expire_key(key, now);
    match data.entries.get(key) {
        None => Ok(None),
        Some(entry) => entry
            .collection
            .as_ref()
            .and_then(Collection::list_values)
            .map(Some)
            .ok_or(()),
    }
}

fn owned_zset(data: &mut Data, key: &[u8], now: i64) -> Result<Option<HashMap<Vec<u8>, f64>>, ()> {
    data.expire_key(key, now);
    match data.entries.get(key) {
        None => Ok(None),
        Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_zset) => Ok(Some(
            entry
                .collection
                .as_ref()
                .and_then(Collection::zset_items)
                .expect("zset type checked")
                .into_iter()
                .map(|(member, score)| (member.to_vec(), score))
                .collect(),
        )),
        Some(_) => Err(()),
    }
}

impl Store {
    fn zset_add_extra(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let mut nx = false;
        let mut xx = false;
        let mut gt = false;
        let mut lt = false;
        let mut ch = false;
        let mut incr = false;
        let mut index = 2;
        while index < args.len() {
            let token = args[index];
            if token.eq_ignore_ascii_case(b"NX") {
                nx = true;
            } else if token.eq_ignore_ascii_case(b"XX") {
                xx = true;
            } else if token.eq_ignore_ascii_case(b"GT") {
                gt = true;
            } else if token.eq_ignore_ascii_case(b"LT") {
                lt = true;
            } else if token.eq_ignore_ascii_case(b"CH") {
                ch = true;
            } else if token.eq_ignore_ascii_case(b"INCR") {
                incr = true;
            } else {
                break;
            }
            index += 1;
        }
        if nx && xx {
            out.extend_from_slice(
                b"-ERR XX and NX options at the same time are not compatible\r\n",
            );
            return;
        }
        if gt && lt {
            out.extend_from_slice(
                b"-ERR GT, LT, and/or NX options at the same time are not compatible\r\n",
            );
            return;
        }
        if nx && (gt || lt) {
            out.extend_from_slice(
                b"-ERR GT, LT, and/or NX options at the same time are not compatible\r\n",
            );
            return;
        }
        let rest = &args[index..];
        if rest.is_empty() || !rest.len().is_multiple_of(2) {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        if incr && rest.len() != 2 {
            out.extend_from_slice(b"-ERR INCR option supports a single increment-element pair\r\n");
            return;
        }
        let mut pairs = Vec::with_capacity(rest.len() / 2);
        for pair in rest.as_chunks::<2>().0 {
            let Some(score) = parse_score(pair[0]) else {
                out.extend_from_slice(b"-ERR value is not a valid float\r\n");
                return;
            };
            pairs.push((normalize_score(score), pair[1]));
        }
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(key, now);
        let current = data.entries.get(key);
        if current.is_some_and(|entry| !entry.collection.as_ref().is_some_and(Collection::is_zset))
        {
            wrong_type(out);
            return;
        }
        let mut zset: HashMap<Vec<u8>, f64> = current
            .and_then(|entry| entry.collection.as_ref().and_then(Collection::zset_items))
            .unwrap_or_default()
            .into_iter()
            .map(|(member, score)| (member.to_vec(), score))
            .collect();
        let mut added = 0i64;
        let mut changed = 0i64;
        let mut increment_reply = None;
        for (amount, member) in pairs {
            let existing = zset.get(member).copied();
            let score = if incr {
                let Some(score) =
                    Some(existing.unwrap_or(0.0) + amount).filter(|score| !score.is_nan())
                else {
                    out.extend_from_slice(b"-ERR resulting score is not a number (NaN)\r\n");
                    return;
                };
                normalize_score(score)
            } else {
                amount
            };
            let blocked = (nx && existing.is_some())
                || (xx && existing.is_none())
                || existing.is_some_and(|old| (gt && score <= old) || (lt && score >= old));
            if blocked {
                continue;
            }
            if existing.is_none() {
                added += 1;
            }
            if existing != Some(score) {
                changed += 1;
            }
            zset.insert(member.to_vec(), score);
            if incr {
                increment_reply = Some(score);
            }
        }
        if incr && increment_reply.is_none() {
            encode_bulk(None, out);
            return;
        }
        if changed == 0 {
            if let Some(score) = increment_reply {
                encode_bulk(Some(&score_bytes(score)), out);
            } else {
                integer(if ch { 0 } else { added }, out);
            }
            return;
        }
        if zset.len() > MAX_ELEMENTS {
            out.extend_from_slice(OOM);
            return;
        }
        let expires = current.and_then(|entry| entry.expires);
        let previous = current.map(Entry::payload_len);
        let bytes = zset.keys().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_zset(zset, &config);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        if let Some(score) = increment_reply {
            encode_bulk(Some(&score_bytes(score)), out);
        } else {
            integer(if ch { changed } else { added }, out);
        }
    }

    fn zset_remove(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut zset = match owned_zset(&mut data, key, now) {
            Ok(Some(zset)) => zset,
            Ok(None) => {
                integer(0, out);
                return;
            }
            Err(()) => {
                wrong_type(out);
                return;
            }
        };
        let entry = data.entries.get(key).expect("zset exists");
        let expires = entry.expires;
        let previous = entry.payload_len();
        let mut removed = 0i64;
        for member in &args[2..] {
            if zset.remove(*member).is_some() {
                removed += 1;
            }
        }
        if removed == 0 {
            integer(0, out);
            return;
        }
        if zset.is_empty() {
            data.remove(key);
            integer(removed, out);
            return;
        }
        let bytes = zset.keys().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_zset(zset, &config);
        let Some(payload) = data.reserve(key, bytes, Some(previous), self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        integer(removed, out);
    }

    fn zset_incr(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some(increment) = parse_score(args[2]) else {
            out.extend_from_slice(b"-ERR value is not a valid float\r\n");
            return;
        };
        let key = args[1];
        let member = args[3];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut zset = match owned_zset(&mut data, key, now) {
            Ok(Some(zset)) => zset,
            Ok(None) => HashMap::new(),
            Err(()) => {
                wrong_type(out);
                return;
            }
        };
        let existing = zset.get(member).copied().unwrap_or(0.0);
        let Some(score) = Some(existing + increment).filter(|score| !score.is_nan()) else {
            out.extend_from_slice(b"-ERR resulting score is not a number (NaN)\r\n");
            return;
        };
        let score = normalize_score(score);
        let existing_entry = data.entries.get(key);
        let expires = existing_entry.and_then(|entry| entry.expires);
        let previous = existing_entry.map(Entry::payload_len);
        zset.insert(member.to_vec(), score);
        let bytes = zset.keys().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_zset(zset, &config);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        encode_bulk(Some(&score_bytes(score)), out);
    }

    fn zset_rank(&self, args: &[&[u8]], reverse: bool, out: &mut Vec<u8>) {
        let with_score = args.len() == 4;
        if with_score && !args[3].eq_ignore_ascii_case(b"WITHSCORE") {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let ordered = match data.entries.get(args[1]) {
            None => {
                if with_score {
                    out.extend_from_slice(b"*-1\r\n");
                } else {
                    encode_bulk(None, out);
                }
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_zset) => {
                ordered_zset(
                    entry
                        .collection
                        .as_ref()
                        .and_then(Collection::zset_items)
                        .expect("zset type checked"),
                    reverse,
                )
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let Some(rank) = ordered
            .iter()
            .position(|(member, _)| member.as_slice() == args[2])
        else {
            if with_score {
                out.extend_from_slice(b"*-1\r\n");
            } else {
                encode_bulk(None, out);
            }
            return;
        };
        if with_score {
            write!(out, "*2\r\n:{rank}\r\n").expect("Vec write");
            encode_bulk(Some(&score_bytes(ordered[rank].1)), out);
        } else {
            integer(rank as i64, out);
        }
    }

    fn zset_reverse_range(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let (Some(start), Some(stop)) = (parse_i64(args[2]), parse_i64(args[3])) else {
            out.extend_from_slice(NOT_INTEGER);
            return;
        };
        let with_scores = args.len() == 5;
        if with_scores && !args[4].eq_ignore_ascii_case(b"WITHSCORES") {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let ordered = match data.entries.get(args[1]) {
            None => {
                out.extend_from_slice(b"*0\r\n");
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_zset) => {
                ordered_zset(
                    entry
                        .collection
                        .as_ref()
                        .and_then(Collection::zset_items)
                        .expect("zset type checked"),
                    true,
                )
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let len = ordered.len() as i128;
        let first = if start < 0 {
            (len + start as i128).max(0)
        } else {
            start as i128
        };
        let last = if stop < 0 {
            len + stop as i128
        } else {
            stop as i128
        };
        if first >= len || first > last {
            out.extend_from_slice(b"*0\r\n");
            return;
        }
        let begin = first as usize;
        let end = (last.min(len - 1) as usize) + 1;
        write!(
            out,
            "*{}\r\n",
            (end - begin) * if with_scores { 2 } else { 1 }
        )
        .expect("Vec write");
        for (member, score) in &ordered[begin..end] {
            encode_bulk(Some(member), out);
            if with_scores {
                encode_bulk(Some(&score_bytes(*score)), out);
            }
        }
    }

    fn zset_count(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let (Some(min), Some(max)) = (parse_score_bound(args[2]), parse_score_bound(args[3]))
        else {
            out.extend_from_slice(b"-ERR min or max is not a float\r\n");
            return;
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let count = match data.entries.get(args[1]) {
            None => 0,
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_zset) => entry
                .collection
                .as_ref()
                .and_then(Collection::zset_items)
                .expect("zset type checked")
                .into_iter()
                .filter(|(_, score)| in_score_range(*score, min, max))
                .count(),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        integer(count as i64, out);
    }

    fn zset_range_by_score(&self, args: &[&[u8]], reverse: bool, out: &mut Vec<u8>) {
        let name = if reverse {
            "zrevrangebyscore"
        } else {
            "zrangebyscore"
        };
        if args.len() < 4 {
            arity(name, out);
            return;
        }
        let (min_arg, max_arg) = if reverse {
            (args[3], args[2])
        } else {
            (args[2], args[3])
        };
        let (Some(min), Some(max)) = (parse_score_bound(min_arg), parse_score_bound(max_arg))
        else {
            out.extend_from_slice(b"-ERR min or max is not a float\r\n");
            return;
        };
        let mut with_scores = false;
        let mut limit = None;
        let mut index = 4;
        while index < args.len() {
            if args[index].eq_ignore_ascii_case(b"WITHSCORES") {
                if with_scores {
                    out.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                with_scores = true;
                index += 1;
            } else if args[index].eq_ignore_ascii_case(b"LIMIT") && index + 2 < args.len() {
                let (Some(offset), Some(count)) =
                    (parse_i64(args[index + 1]), parse_i64(args[index + 2]))
                else {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                };
                limit = Some((offset, count));
                index += 3;
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let ordered = match data.entries.get(args[1]) {
            None => {
                out.extend_from_slice(b"*0\r\n");
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_zset) => {
                let mut ordered = ordered_zset(
                    entry
                        .collection
                        .as_ref()
                        .and_then(Collection::zset_items)
                        .expect("zset type checked"),
                    reverse,
                );
                ordered.retain(|(_, score)| in_score_range(*score, min, max));
                ordered
            }
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let (offset, count) = limit.unwrap_or((0, -1));
        if offset < 0 {
            out.extend_from_slice(b"*0\r\n");
            return;
        }
        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
        let take = if count < 0 {
            ordered.len().saturating_sub(offset)
        } else {
            usize::try_from(count).unwrap_or(usize::MAX)
        };
        let selected = ordered.iter().skip(offset).take(take);
        let selected: Vec<_> = selected.collect();
        write!(
            out,
            "*{}\r\n",
            selected.len() * if with_scores { 2 } else { 1 }
        )
        .expect("Vec write");
        for (member, score) in selected {
            encode_bulk(Some(member), out);
            if with_scores {
                encode_bulk(Some(&score_bytes(*score)), out);
            }
        }
    }

    fn execute_zset_extended(&self, args: &[&[u8]], out: &mut Vec<u8>) -> bool {
        let command = args.first().expect("nonempty command");
        if command.eq_ignore_ascii_case(b"ZRANGE") {
            if args.len() < 4 {
                arity("zrange", out);
            } else {
                self.zset_range(args, false, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZRANGESTORE") {
            if args.len() < 5 {
                arity("zrangestore", out);
            } else {
                self.zset_range(args, true, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZLEXCOUNT") {
            if args.len() != 4 {
                arity("zlexcount", out);
            } else {
                self.zset_lex_count(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZREMRANGEBYLEX") {
            if args.len() != 4 {
                arity("zremrangebylex", out);
            } else {
                self.zset_remove_range_by_lex(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZRANDMEMBER") {
            if !(2..=4).contains(&args.len()) {
                arity("zrandmember", out);
            } else {
                self.zset_random_member(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZUNION")
            || command.eq_ignore_ascii_case(b"ZUNIONSTORE")
            || command.eq_ignore_ascii_case(b"ZINTER")
            || command.eq_ignore_ascii_case(b"ZINTERSTORE")
            || command.eq_ignore_ascii_case(b"ZDIFF")
            || command.eq_ignore_ascii_case(b"ZDIFFSTORE")
        {
            let (name, operation, store) = if command.eq_ignore_ascii_case(b"ZUNION") {
                ("zunion", ZSetOp::Union, false)
            } else if command.eq_ignore_ascii_case(b"ZUNIONSTORE") {
                ("zunionstore", ZSetOp::Union, true)
            } else if command.eq_ignore_ascii_case(b"ZINTER") {
                ("zinter", ZSetOp::Inter, false)
            } else if command.eq_ignore_ascii_case(b"ZINTERSTORE") {
                ("zinterstore", ZSetOp::Inter, true)
            } else if command.eq_ignore_ascii_case(b"ZDIFF") {
                ("zdiff", ZSetOp::Diff, false)
            } else {
                ("zdiffstore", ZSetOp::Diff, true)
            };
            if args.len() < if store { 4 } else { 3 } {
                arity(name, out);
            } else {
                self.zset_algebra(args, name, operation, store, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZINTERCARD") {
            if args.len() < 3 {
                arity("zintercard", out);
            } else {
                self.zset_intercard(args, out);
            }
        } else if command.eq_ignore_ascii_case(b"ZMPOP") {
            if args.len() < 4 {
                arity("zmpop", out);
            } else {
                self.zset_mpop(args, 1, out);
            }
        } else if command.eq_ignore_ascii_case(b"BZMPOP") {
            if args.len() < 5 {
                arity("bzmpop", out);
            } else {
                match parse_bzmpop_timeout(args[1]) {
                    Ok(_) => self.zset_mpop(args, 2, out),
                    Err(error) => out.extend_from_slice(error),
                }
            }
        } else {
            return false;
        }
        true
    }

    fn zset_mpop(&self, args: &[&[u8]], numkeys_index: usize, out: &mut Vec<u8>) {
        let options = match parse_mpop(args, numkeys_index) {
            Ok(options) => options,
            Err(error) => {
                out.extend_from_slice(error);
                return;
            }
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let outcome =
            self.zset_pop_in_data(&mut data, &options.keys, options.max, options.count, now);
        zset_pop_reply(outcome, out);
    }

    pub(super) fn zset_pop_in_data(
        &self,
        data: &mut Data,
        keys: &[&[u8]],
        max: bool,
        count: usize,
        now: i64,
    ) -> ZSetPopOutcome {
        for key in keys {
            let mut zset = match owned_zset(data, key, now) {
                Ok(Some(zset)) => zset,
                Ok(None) => continue,
                Err(()) => return ZSetPopOutcome::WrongType,
            };
            if zset.is_empty() {
                continue;
            }
            let entry = data.entries.get(*key).expect("zset exists");
            let expires = entry.expires;
            let previous = entry.payload_len();
            let ordered = ordered_zset(
                zset.iter()
                    .map(|(member, score)| (member.as_slice(), *score))
                    .collect(),
                max,
            );
            let take = count.min(ordered.len());
            for (member, _) in ordered.iter().take(take) {
                zset.remove(member);
            }
            let encoded = if zset.is_empty() {
                None
            } else {
                let Ok(config) = self.collection_config.read() else {
                    return ZSetPopOutcome::JournalError;
                };
                Some(encode_zset(zset, &config))
            };
            match encoded {
                Some(encoded) => {
                    let Some(payload) = data.reserve(
                        key,
                        encoded.payload_len(),
                        Some(previous),
                        self.max_payload_bytes,
                        now,
                    ) else {
                        return ZSetPopOutcome::Oom;
                    };
                    data.put_collection(key, encoded, expires, payload, now);
                }
                None => {
                    data.remove(key);
                }
            }
            return ZSetPopOutcome::Popped {
                key: key.to_vec(),
                items: ordered.into_iter().take(take).collect(),
            };
        }
        ZSetPopOutcome::Empty
    }

    fn zset_range(&self, args: &[&[u8]], store: bool, out: &mut Vec<u8>) {
        let mut mode = RangeMode::Rank;
        let mut mode_set = false;
        let mut reverse = false;
        let mut with_scores = false;
        let mut limit = None;
        let mut index = if store { 5 } else { 4 };
        while index < args.len() {
            let token = args[index];
            if token.eq_ignore_ascii_case(b"BYSCORE") {
                if mode_set {
                    out.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                mode = RangeMode::Score;
                mode_set = true;
                index += 1;
            } else if token.eq_ignore_ascii_case(b"BYLEX") {
                if mode_set {
                    out.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                mode = RangeMode::Lex;
                mode_set = true;
                index += 1;
            } else if token.eq_ignore_ascii_case(b"REV") {
                reverse = true;
                index += 1;
            } else if token.eq_ignore_ascii_case(b"WITHSCORES") {
                if store {
                    out.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                with_scores = true;
                index += 1;
            } else if token.eq_ignore_ascii_case(b"LIMIT") && index + 2 < args.len() {
                let (Some(offset), Some(count)) =
                    (parse_i64(args[index + 1]), parse_i64(args[index + 2]))
                else {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                };
                limit = Some((offset, count));
                index += 3;
            } else {
                out.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        if mode == RangeMode::Rank && limit.is_some() {
            out.extend_from_slice(
                b"-ERR syntax error, LIMIT is only supported in combination with either BYSCORE or BYLEX\r\n",
            );
            return;
        }
        if mode == RangeMode::Lex && with_scores {
            out.extend_from_slice(
                b"-ERR syntax error, WITHSCORES not supported in combination with BYLEX\r\n",
            );
            return;
        }
        let (first, second) = if store {
            (args[3], args[4])
        } else {
            (args[2], args[3])
        };
        let selector = match parse_selector(mode, reverse, first, second) {
            Ok(selector) => selector,
            Err(error) => {
                out.extend_from_slice(error);
                return;
            }
        };
        let source = if store { args[2] } else { args[1] };
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(source, now);
        let items = match data.entries.get(source) {
            None => {
                if store {
                    data.expire_key(key, now);
                    data.remove(key);
                    integer(0, out);
                } else {
                    out.extend_from_slice(b"*0\r\n");
                }
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_zset) => entry
                .collection
                .as_ref()
                .and_then(Collection::zset_items)
                .expect("zset type checked"),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let selected = select_zset(items, selector, reverse, limit);
        if !store {
            write!(
                out,
                "*{}\r\n",
                selected.len() * if with_scores { 2 } else { 1 }
            )
            .expect("Vec write");
            for (member, score) in selected {
                encode_bulk(Some(&member), out);
                if with_scores {
                    encode_bulk(Some(&score_bytes(score)), out);
                }
            }
            return;
        }
        data.expire_key(key, now);
        if selected.is_empty() {
            data.remove(key);
            integer(0, out);
            return;
        }
        let mut zset = HashMap::with_capacity(selected.len());
        let mut bytes = 0usize;
        for (member, score) in selected {
            bytes = bytes.saturating_add(member.len());
            zset.insert(member, score);
        }
        let count = zset.len();
        let previous = data.entries.get(key).map(|entry| entry.payload_len);
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_zset(zset, &config);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, None, payload, now);
        integer(count as i64, out);
    }

    fn zset_algebra(
        &self,
        args: &[&[u8]],
        name: &str,
        operation: ZSetOp,
        store: bool,
        out: &mut Vec<u8>,
    ) {
        let (destination, numkeys_index) = if store { (Some(args[1]), 2) } else { (None, 1) };
        let Some(count) = parse_zset_numkeys(args[numkeys_index], name, out) else {
            return;
        };
        let keys_start = numkeys_index + 1;
        let Some(keys_end) = keys_start.checked_add(count) else {
            out.extend_from_slice(SYNTAX);
            return;
        };
        if keys_end > args.len() {
            out.extend_from_slice(SYNTAX);
            return;
        }
        let Some((weights, aggregate, with_scores)) =
            parse_zset_algebra_options(args, keys_end, count, operation, store, out)
        else {
            return;
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut sets = Vec::with_capacity(count);
        for key in &args[keys_start..keys_end] {
            match owned_zset(&mut data, key, now) {
                Ok(set) => sets.push(set.unwrap_or_default()),
                Err(()) => {
                    wrong_type(out);
                    return;
                }
            }
        }
        let result: HashMap<Vec<u8>, f64> = match operation {
            ZSetOp::Union => {
                let mut result: HashMap<Vec<u8>, f64> = HashMap::new();
                for (set, weight) in sets.iter().zip(weights.iter()) {
                    for (member, score) in set {
                        let weighted = nan_zero(score * weight);
                        match result.entry(member.clone()) {
                            std::collections::hash_map::Entry::Vacant(entry) => {
                                entry.insert(weighted);
                            }
                            std::collections::hash_map::Entry::Occupied(mut entry) => {
                                let value = *entry.get();
                                *entry.get_mut() = nan_zero(match aggregate {
                                    Aggregate::Sum => value + weighted,
                                    Aggregate::Min => f64::min(value, weighted),
                                    Aggregate::Max => f64::max(value, weighted),
                                });
                            }
                        }
                    }
                }
                result
            }
            ZSetOp::Inter => {
                let mut result: HashMap<Vec<u8>, f64> = HashMap::new();
                if let Some((first, rest)) = sets.split_first() {
                    for (member, score) in first {
                        result.insert(member.clone(), nan_zero(score * weights[0]));
                    }
                    for (set, weight) in rest.iter().zip(weights.iter().skip(1)) {
                        if result.is_empty() {
                            break;
                        }
                        result.retain(|member, current| {
                            let Some(score) = set.get(member) else {
                                return false;
                            };
                            let weighted = nan_zero(score * weight);
                            *current = nan_zero(match aggregate {
                                Aggregate::Sum => *current + weighted,
                                Aggregate::Min => f64::min(*current, weighted),
                                Aggregate::Max => f64::max(*current, weighted),
                            });
                            true
                        });
                    }
                }
                result
            }
            ZSetOp::Diff => {
                let mut result: HashMap<Vec<u8>, f64> = HashMap::new();
                if let Some((first, rest)) = sets.split_first() {
                    for (member, score) in first {
                        result.insert(member.clone(), *score);
                    }
                    for set in rest {
                        if result.is_empty() {
                            break;
                        }
                        result.retain(|member, _| !set.contains_key(member));
                    }
                }
                result
            }
        };
        let ordered = ordered_zset(
            result
                .iter()
                .map(|(member, score)| (member.as_slice(), *score))
                .collect(),
            false,
        );
        if !store {
            write!(
                out,
                "*{}\r\n",
                ordered.len() * (1 + usize::from(with_scores))
            )
            .expect("Vec write");
            for (member, score) in ordered {
                encode_bulk(Some(&member), out);
                if with_scores {
                    encode_bulk(Some(&score_bytes(score)), out);
                }
            }
            return;
        }
        let destination = destination.expect("store destination");
        data.expire_key(destination, now);
        if ordered.is_empty() {
            data.remove(destination);
            integer(0, out);
            return;
        }
        if ordered.len() > MAX_ELEMENTS {
            out.extend_from_slice(OOM);
            return;
        }
        let mut zset = HashMap::with_capacity(ordered.len());
        let mut bytes = 0usize;
        for (member, score) in ordered {
            bytes = bytes.saturating_add(member.len());
            zset.insert(member, score);
        }
        let count = zset.len();
        let previous = data.entries.get(destination).map(Entry::payload_len);
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_zset(zset, &config);
        let Some(payload) = data.reserve(destination, bytes, previous, self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(destination, encoded, None, payload, now);
        integer(count as i64, out);
    }

    fn zset_intercard(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some(count) = parse_zset_numkeys(args[1], "zintercard", out) else {
            return;
        };
        let keys_start = 2usize;
        let Some(keys_end) = keys_start.checked_add(count) else {
            out.extend_from_slice(SYNTAX);
            return;
        };
        if keys_end > args.len() {
            out.extend_from_slice(SYNTAX);
            return;
        }
        let mut limit = 0usize;
        let mut index = keys_end;
        while index < args.len() {
            if args[index].eq_ignore_ascii_case(b"LIMIT") && index + 1 < args.len() {
                let Some(value) = parse_i64(args[index + 1]) else {
                    out.extend_from_slice(b"-ERR LIMIT can't be negative\r\n");
                    return;
                };
                if value < 0 {
                    out.extend_from_slice(b"-ERR LIMIT can't be negative\r\n");
                    return;
                }
                limit = usize::try_from(value).unwrap_or(usize::MAX);
                index += 2;
            } else {
                out.extend_from_slice(SYNTAX);
                return;
            }
        }
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut sets = Vec::with_capacity(count);
        for key in &args[keys_start..keys_end] {
            match owned_zset(&mut data, key, now) {
                Ok(set) => sets.push(set.unwrap_or_default()),
                Err(()) => {
                    wrong_type(out);
                    return;
                }
            }
        }
        let mut members: BTreeSet<Vec<u8>> = sets
            .first()
            .map(|set| set.keys().cloned().collect())
            .unwrap_or_default();
        for set in &sets[1..] {
            if members.is_empty() {
                break;
            }
            members.retain(|member| set.contains_key(member));
        }
        let cardinality = if limit == 0 {
            members.len()
        } else {
            members.len().min(limit)
        };
        integer(cardinality as i64, out);
    }

    fn zset_lex_count(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some((min, max)) = parse_lex_bounds(args[2], args[3]) else {
            out.extend_from_slice(b"-ERR min or max not valid string range item\r\n");
            return;
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let count = match data.entries.get(args[1]) {
            None => 0,
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_zset) => entry
                .collection
                .as_ref()
                .and_then(Collection::zset_items)
                .expect("zset type checked")
                .iter()
                .filter(|(member, _)| in_lex_range(member, &min, &max))
                .count(),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        integer(count as i64, out);
    }

    fn zset_remove_range_by_lex(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let Some((min, max)) = parse_lex_bounds(args[2], args[3]) else {
            out.extend_from_slice(b"-ERR min or max not valid string range item\r\n");
            return;
        };
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut zset = match owned_zset(&mut data, key, now) {
            Ok(Some(zset)) => zset,
            Ok(None) => {
                integer(0, out);
                return;
            }
            Err(()) => {
                wrong_type(out);
                return;
            }
        };
        let before = zset.len();
        zset.retain(|member, _| !in_lex_range(member, &min, &max));
        let removed = before - zset.len();
        if removed == 0 {
            integer(0, out);
            return;
        }
        if zset.is_empty() {
            data.remove(key);
            integer(removed as i64, out);
            return;
        }
        let expires = data.entries.get(key).and_then(|entry| entry.expires);
        let previous = data.entries.get(key).map(|entry| entry.payload_len);
        let bytes = zset.keys().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_zset(zset, &config);
        let Some(payload) = data.reserve(key, bytes, previous, self.max_payload_bytes, now) else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        integer(removed as i64, out);
    }

    fn zset_random_member(&self, args: &[&[u8]], out: &mut Vec<u8>) {
        let with_scores = args.len() == 4 && args[3].eq_ignore_ascii_case(b"WITHSCORES");
        if args.len() == 4 && !with_scores {
            out.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        let count = if args.len() >= 3 {
            let Some(value) = parse_i64(args[2]) else {
                out.extend_from_slice(NOT_INTEGER);
                return;
            };
            Some(value)
        } else {
            None
        };
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        data.expire_key(args[1], now);
        let items = match data.entries.get(args[1]) {
            None => {
                if count.is_some() {
                    out.extend_from_slice(b"*0\r\n");
                } else {
                    encode_bulk(None, out);
                }
                return;
            }
            Some(entry) if entry.collection.as_ref().is_some_and(Collection::is_zset) => entry
                .collection
                .as_ref()
                .and_then(Collection::zset_items)
                .expect("zset type checked"),
            Some(_) => {
                wrong_type(out);
                return;
            }
        };
        let members: Vec<(Vec<u8>, f64)> = items
            .into_iter()
            .map(|(member, score)| (member.to_vec(), score))
            .collect();
        let Some(count) = count else {
            let index = random_index(members.len());
            encode_bulk(members.get(index).map(|(member, _)| member.as_slice()), out);
            return;
        };
        if count == 0 || members.is_empty() {
            out.extend_from_slice(b"*0\r\n");
            return;
        }
        let limit = usize::try_from(count.unsigned_abs()).unwrap_or(usize::MAX);
        if count > 0 {
            let mut pool = members;
            let take = limit.min(pool.len());
            for index in 0..take {
                let swap = index + random_index(pool.len() - index);
                pool.swap(index, swap);
            }
            write!(out, "*{}\r\n", take * if with_scores { 2 } else { 1 }).expect("Vec write");
            for (member, score) in pool.iter().take(take) {
                encode_bulk(Some(member), out);
                if with_scores {
                    encode_bulk(Some(&score_bytes(*score)), out);
                }
            }
        } else {
            let take = limit.min(MAX_ELEMENTS);
            write!(out, "*{}\r\n", take * if with_scores { 2 } else { 1 }).expect("Vec write");
            for _ in 0..take {
                let index = random_index(members.len());
                let (member, score) = &members[index];
                encode_bulk(Some(member), out);
                if with_scores {
                    encode_bulk(Some(&score_bytes(*score)), out);
                }
            }
        }
    }

    fn zset_remove_range(&self, args: &[&[u8]], by_score: bool, out: &mut Vec<u8>) {
        enum Range {
            Score((f64, bool), (f64, bool)),
            Rank(i64, i64),
        }
        let range = if by_score {
            let (Some(min), Some(max)) = (parse_score_bound(args[2]), parse_score_bound(args[3]))
            else {
                out.extend_from_slice(b"-ERR min or max is not a float\r\n");
                return;
            };
            Range::Score(min, max)
        } else {
            let (Some(start), Some(stop)) = (parse_i64(args[2]), parse_i64(args[3])) else {
                out.extend_from_slice(NOT_INTEGER);
                return;
            };
            Range::Rank(start, stop)
        };
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut zset = match owned_zset(&mut data, key, now) {
            Ok(Some(zset)) => zset,
            Ok(None) => {
                integer(0, out);
                return;
            }
            Err(()) => {
                wrong_type(out);
                return;
            }
        };
        let entry = data.entries.get(key).expect("zset exists");
        let expires = entry.expires;
        let previous = entry.payload_len();
        let members: Vec<Vec<u8>> = match range {
            Range::Score(min, max) => zset
                .iter()
                .filter(|(_, score)| in_score_range(**score, min, max))
                .map(|(member, _)| member.clone())
                .collect(),
            Range::Rank(start, stop) => {
                let ordered = ordered_zset(
                    zset.iter()
                        .map(|(member, score)| (member.as_slice(), *score))
                        .collect(),
                    false,
                );
                let len = ordered.len() as i128;
                let first = if start < 0 {
                    (len + start as i128).max(0)
                } else {
                    start as i128
                };
                let last = if stop < 0 {
                    len + stop as i128
                } else {
                    stop as i128
                };
                if first >= len || first > last {
                    integer(0, out);
                    return;
                }
                ordered[first as usize..=(last.min(len - 1) as usize)]
                    .iter()
                    .map(|(member, _)| member.clone())
                    .collect()
            }
        };
        if members.is_empty() {
            integer(0, out);
            return;
        }
        for member in &members {
            zset.remove(member);
        }
        if zset.is_empty() {
            data.remove(key);
            integer(members.len() as i64, out);
            return;
        }
        let bytes = zset.keys().map(Vec::len).sum::<usize>();
        let Ok(config) = self.collection_config.read() else {
            internal_error(out);
            return;
        };
        let encoded = encode_zset(zset, &config);
        let Some(payload) = data.reserve(key, bytes, Some(previous), self.max_payload_bytes, now)
        else {
            out.extend_from_slice(OOM);
            return;
        };
        data.put_collection(key, encoded, expires, payload, now);
        integer(members.len() as i64, out);
    }

    fn zset_pop(&self, args: &[&[u8]], max: bool, out: &mut Vec<u8>) {
        let count = if args.len() == 3 {
            match parse_i64(args[2]) {
                Some(value) if value >= 0 => usize::try_from(value).unwrap_or(usize::MAX),
                Some(_) => {
                    out.extend_from_slice(b"-ERR value is out of range, must be positive\r\n");
                    return;
                }
                None => {
                    out.extend_from_slice(NOT_INTEGER);
                    return;
                }
            }
        } else {
            1
        };
        let key = args[1];
        let Ok(mut data) = self.data.write() else {
            internal_error(out);
            return;
        };
        let now = self.now(&data);
        let mut zset = match owned_zset(&mut data, key, now) {
            Ok(Some(zset)) => zset,
            Ok(None) => {
                out.extend_from_slice(b"*0\r\n");
                return;
            }
            Err(()) => {
                wrong_type(out);
                return;
            }
        };
        let entry = data.entries.get(key).expect("zset exists");
        let expires = entry.expires;
        let previous = entry.payload_len();
        let ordered = ordered_zset(
            zset.iter()
                .map(|(member, score)| (member.as_slice(), *score))
                .collect(),
            max,
        );
        let take = count.min(ordered.len());
        if take == 0 {
            out.extend_from_slice(b"*0\r\n");
            return;
        }
        for (member, _) in ordered.iter().take(take) {
            zset.remove(member);
        }
        let encoded = if zset.is_empty() {
            None
        } else {
            let Ok(config) = self.collection_config.read() else {
                internal_error(out);
                return;
            };
            Some(encode_zset(zset, &config))
        };
        let payload = encoded.as_ref().map(|encoded| {
            data.payload_bytes
                .checked_sub(previous)
                .and_then(|value| value.checked_add(encoded.payload_len()))
        });
        if payload == Some(None) {
            internal_error(out);
            return;
        }
        write!(out, "*{}\r\n", take * 2).expect("Vec write");
        for (member, score) in ordered.iter().take(take) {
            encode_bulk(Some(member), out);
            encode_bulk(Some(&score_bytes(*score)), out);
        }
        match (encoded, payload) {
            (Some(encoded), Some(Some(payload))) => {
                data.put_collection(key, encoded, expires, payload, now)
            }
            (None, _) => {
                data.remove(key);
            }
            _ => unreachable!("payload checks parallel encoded value"),
        }
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
    fn combined_hash_commands_cover_conditions_and_ttls() {
        let store = Store::new(4096);
        assert_eq!(
            run(&store, &[b"HSETEX", b"h", b"FIELDS", b"1", b"a", b"1"]),
            b":1\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"HSETEX", b"h", b"FNX", b"FIELDS", b"2", b"a", b"x", b"b", b"2",
                ],
            ),
            b":0\r\n"
        );
        assert_eq!(run(&store, &[b"HGET", b"h", b"a"]), b"$1\r\n1\r\n");
        assert_eq!(
            run(
                &store,
                &[
                    b"HSETEX", b"h", b"FXX", b"FIELDS", b"2", b"a", b"3", b"b", b"4"
                ]
            ),
            b":0\r\n"
        );
        assert_eq!(
            run(&store, &[b"HSETEX", b"h", b"FIELDS", b"1", b"b", b"4"]),
            b":1\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"HSETEX", b"h", b"FXX", b"FIELDS", b"2", b"a", b"3", b"b", b"5"
                ]
            ),
            b":1\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"HGETEX", b"h", b"FIELDS", b"2", b"a", b"missing"]
            ),
            b"*2\r\n$1\r\n3\r\n$-1\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"HGETEX", b"h", b"PERSIST", b"FIELDS", b"1", b"a"]
            ),
            b"*1\r\n$1\r\n3\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[b"HSETEX", b"h", b"FNX", b"FXX", b"FIELDS", b"1", b"a", b"9"]
            ),
            b"-ERR Only one of FXX or FNX arguments can be specified\r\n"
        );
        assert_eq!(run(&store, &[b"HGET", b"h", b"a"]), b"$1\r\n3\r\n");
    }

    #[test]
    fn hash_mutations_cover_existence_counters_and_removal() {
        let store = Store::new(4096);
        assert_eq!(run(&store, &[b"HSETNX", b"h", b"f", b"1"]), b":1\r\n");
        assert_eq!(run(&store, &[b"HSETNX", b"h", b"f", b"2"]), b":0\r\n");
        assert_eq!(run(&store, &[b"HEXISTS", b"h", b"f"]), b":1\r\n");
        assert_eq!(run(&store, &[b"HEXISTS", b"h", b"missing"]), b":0\r\n");
        assert_eq!(run(&store, &[b"HINCRBY", b"h", b"f", b"4"]), b":5\r\n");
        assert_eq!(
            run(&store, &[b"HINCRBYFLOAT", b"h", b"f", b"0.5"]),
            b"$3\r\n5.5\r\n"
        );
        assert_eq!(
            run(&store, &[b"HMGET", b"h", b"f", b"missing", b"f"]),
            b"*3\r\n$3\r\n5.5\r\n$-1\r\n$3\r\n5.5\r\n"
        );
        assert_eq!(run(&store, &[b"HSTRLEN", b"h", b"f"]), b":3\r\n");
        assert_eq!(run(&store, &[b"HKEYS", b"h"]), b"*1\r\n$1\r\nf\r\n");
        assert_eq!(run(&store, &[b"HVALS", b"h"]), b"*1\r\n$3\r\n5.5\r\n");
        assert_eq!(run(&store, &[b"PEXPIRE", b"h", b"100"]), b":1\r\n");
        assert_eq!(run(&store, &[b"HDEL", b"h", b"f"]), b":1\r\n");
        assert_eq!(run(&store, &[b"TYPE", b"h"]), b"+none\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 0);
    }

    #[test]
    fn hash_integer_and_float_failures_are_atomic() {
        let store = Store::new(128);
        assert_eq!(run(&store, &[b"HSET", b"h", b"n", b"not"]), b":1\r\n");
        assert_eq!(
            run(&store, &[b"HINCRBY", b"h", b"n", b"1"]),
            b"-ERR hash value is not an integer\r\n"
        );
        assert_eq!(
            run(&store, &[b"HINCRBYFLOAT", b"h", b"n", b"1"]),
            b"-ERR hash value is not a float\r\n"
        );
        assert_eq!(
            run(&store, &[b"HSET", b"h", b"i", b"9223372036854775807"]),
            b":1\r\n"
        );
        assert_eq!(
            run(&store, &[b"HINCRBY", b"h", b"i", b"1"]),
            b"-ERR increment or decrement would overflow\r\n"
        );
        assert_eq!(run(&store, &[b"HGET", b"h", b"n"]), b"$3\r\nnot\r\n");
        assert_eq!(
            run(&store, &[b"HINCRBYFLOAT", b"h", b"f", b"inf"]),
            b"-ERR value is NaN or Infinity\r\n"
        );
        assert_eq!(
            run(&store, &[b"HINCRBYFLOAT", b"h", b"f", b"nan"]),
            b"-ERR value is not a valid float\r\n"
        );
    }

    #[test]
    fn list_mutations_preserve_order_and_ttl() {
        let store = Store::new(4096);
        assert_eq!(
            run(&store, &[b"RPUSH", b"l", b"a", b"b", b"a", b"c"]),
            b":4\r\n"
        );
        assert_eq!(run(&store, &[b"PEXPIRE", b"l", b"1000"]), b":1\r\n");
        assert_eq!(run(&store, &[b"LINDEX", b"l", b"-1"]), b"$1\r\nc\r\n");
        assert_eq!(run(&store, &[b"LINDEX", b"l", b"9"]), b"$-1\r\n");
        assert_eq!(
            run(&store, &[b"LINSERT", b"l", b"BEFORE", b"b", b"x"]),
            b":5\r\n"
        );
        assert_eq!(
            run(&store, &[b"LINSERT", b"l", b"AFTER", b"missing", b"x"]),
            b":-1\r\n"
        );
        assert_eq!(run(&store, &[b"LSET", b"l", b"0", b"z"]), b"+OK\r\n");
        assert_eq!(
            run(&store, &[b"LSET", b"l", b"99", b"z"]),
            b"-ERR index out of range\r\n"
        );
        assert_eq!(run(&store, &[b"LREM", b"l", b"1", b"a"]), b":1\r\n");
        assert_eq!(
            run(&store, &[b"LRANGE", b"l", b"0", b"-1"]),
            b"*4\r\n$1\r\nz\r\n$1\r\nx\r\n$1\r\nb\r\n$1\r\nc\r\n"
        );
        assert_eq!(run(&store, &[b"LTRIM", b"l", b"1", b"2"]), b"+OK\r\n");
        assert_eq!(
            run(&store, &[b"LRANGE", b"l", b"0", b"-1"]),
            b"*2\r\n$1\r\nx\r\n$1\r\nb\r\n"
        );
        let ttl = run(&store, &[b"PTTL", b"l"]);
        assert!(!ttl.starts_with(b":-2"));
        assert_eq!(run(&store, &[b"LPUSHX", b"missing", b"x"]), b":0\r\n");
        assert_eq!(run(&store, &[b"RPUSHX", b"l", b"tail"]), b":3\r\n");
    }

    #[test]
    fn list_empty_results_and_bad_indexes_do_not_corrupt_state() {
        let store = Store::new(1024);
        assert_eq!(
            run(&store, &[b"LTRIM", b"missing", b"0", b"-1"]),
            b"+OK\r\n"
        );
        assert_eq!(
            run(&store, &[b"LSET", b"missing", b"0", b"x"]),
            b"-ERR no such key\r\n"
        );
        assert_eq!(run(&store, &[b"LREM", b"missing", b"0", b"x"]), b":0\r\n");
        assert_eq!(run(&store, &[b"RPUSH", b"l", b"a"]), b":1\r\n");
        assert_eq!(run(&store, &[b"LTRIM", b"l", b"1", b"5"]), b"+OK\r\n");
        assert_eq!(run(&store, &[b"EXISTS", b"l"]), b":0\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 0);
    }

    #[test]
    fn set_mutations_algebra_and_cardinality_work() {
        let store = Store::new(4096);
        assert_eq!(run(&store, &[b"SADD", b"a", b"1", b"2", b"3"]), b":3\r\n");
        assert_eq!(run(&store, &[b"SADD", b"b", b"2", b"3", b"4"]), b":3\r\n");
        assert_eq!(
            run(&store, &[b"SMISMEMBER", b"a", b"1", b"9"]),
            b"*2\r\n:1\r\n:0\r\n"
        );
        assert_eq!(
            run(&store, &[b"SINTER", b"a", b"b"]),
            b"*2\r\n$1\r\n2\r\n$1\r\n3\r\n"
        );
        assert_eq!(
            run(&store, &[b"SUNION", b"a", b"b"]),
            b"*4\r\n$1\r\n1\r\n$1\r\n2\r\n$1\r\n3\r\n$1\r\n4\r\n"
        );
        assert_eq!(run(&store, &[b"SDIFF", b"a", b"b"]), b"*1\r\n$1\r\n1\r\n");
        assert_eq!(
            run(&store, &[b"SINTERCARD", b"2", b"a", b"b", b"LIMIT", b"1"]),
            b":1\r\n"
        );
        assert_eq!(run(&store, &[b"SMOVE", b"a", b"b", b"1"]), b":1\r\n");
        assert_eq!(run(&store, &[b"SMOVE", b"a", b"b", b"1"]), b":0\r\n");
        assert_eq!(run(&store, &[b"SREM", b"a", b"2"]), b":1\r\n");
        assert_eq!(run(&store, &[b"SINTERSTORE", b"i", b"a", b"b"]), b":1\r\n");
        assert_eq!(run(&store, &[b"SCARD", b"i"]), b":1\r\n");
        assert_eq!(run(&store, &[b"SUNIONSTORE", b"u", b"a", b"b"]), b":4\r\n");
        assert_eq!(run(&store, &[b"SCARD", b"u"]), b":4\r\n");
        assert_eq!(run(&store, &[b"SDIFFSTORE", b"d", b"u", b"b"]), b":0\r\n");
        assert_eq!(run(&store, &[b"EXISTS", b"d"]), b":0\r\n");
    }

    #[test]
    fn set_pop_and_random_member_are_bounded_and_remove_empties() {
        let store = Store::new(4096);
        assert_eq!(run(&store, &[b"SPOP", b"missing"]), b"$-1\r\n");
        assert_eq!(run(&store, &[b"SPOP", b"missing", b"2"]), b"*0\r\n");
        assert_eq!(run(&store, &[b"SADD", b"s", b"a", b"b"]), b":2\r\n");
        let random = run(&store, &[b"SRANDMEMBER", b"s"]);
        assert!(random == b"$1\r\na\r\n" || random == b"$1\r\nb\r\n");
        let many = run(&store, &[b"SRANDMEMBER", b"s", b"-3"]);
        assert!(many.starts_with(b"*3\r\n"));
        assert!(run(&store, &[b"SPOP", b"s", b"1"]).starts_with(b"*1\r\n"));
        assert!(run(&store, &[b"SPOP", b"s", b"1"]).starts_with(b"*1\r\n"));
        assert_eq!(run(&store, &[b"EXISTS", b"s"]), b":0\r\n");
    }

    #[test]
    fn sorted_set_mutations_options_ranges_and_pops_work() {
        let store = Store::new(8192);
        assert_eq!(
            run(&store, &[b"ZADD", b"z", b"1", b"a", b"2", b"b"]),
            b":2\r\n"
        );
        assert_eq!(run(&store, &[b"ZADD", b"z", b"NX", b"3", b"a"]), b":0\r\n");
        assert_eq!(
            run(&store, &[b"ZADD", b"z", b"XX", b"CH", b"3", b"a"]),
            b":1\r\n"
        );
        assert_eq!(run(&store, &[b"ZADD", b"z", b"GT", b"2", b"a"]), b":0\r\n");
        assert_eq!(run(&store, &[b"ZADD", b"z", b"GT", b"4", b"a"]), b":0\r\n");
        assert_eq!(
            run(&store, &[b"ZINCRBY", b"z", b"0.5", b"c"]),
            b"$3\r\n0.5\r\n"
        );
        assert_eq!(run(&store, &[b"ZRANK", b"z", b"c"]), b":0\r\n");
        assert_eq!(run(&store, &[b"ZREVRANK", b"z", b"a"]), b":0\r\n");
        assert_eq!(
            run(&store, &[b"ZRANK", b"z", b"a", b"WITHSCORE"]),
            b"*2\r\n:2\r\n$1\r\n4\r\n"
        );
        assert_eq!(run(&store, &[b"ZCOUNT", b"z", b"(0.5", b"+inf"]), b":2\r\n");
        assert_eq!(
            run(
                &store,
                &[b"ZRANGEBYSCORE", b"z", b"-inf", b"+inf", b"WITHSCORES"]
            ),
            b"*6\r\n$1\r\nc\r\n$3\r\n0.5\r\n$1\r\nb\r\n$1\r\n2\r\n$1\r\na\r\n$1\r\n4\r\n"
        );
        assert_eq!(
            run(
                &store,
                &[
                    b"ZREVRANGEBYSCORE",
                    b"z",
                    b"+inf",
                    b"-inf",
                    b"LIMIT",
                    b"1",
                    b"1"
                ]
            ),
            b"*1\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            run(&store, &[b"ZREVRANGE", b"z", b"0", b"-1"]),
            b"*3\r\n$1\r\na\r\n$1\r\nb\r\n$1\r\nc\r\n"
        );
        assert_eq!(run(&store, &[b"ZREM", b"z", b"b"]), b":1\r\n");
        assert_eq!(
            run(&store, &[b"ZREMRANGEBYSCORE", b"z", b"-inf", b"(1"]),
            b":1\r\n"
        );
        assert_eq!(
            run(&store, &[b"ZREMRANGEBYRANK", b"z", b"0", b"0"]),
            b":1\r\n"
        );
        assert_eq!(run(&store, &[b"EXISTS", b"z"]), b":0\r\n");
    }

    #[test]
    fn sorted_set_accepts_infinite_scores_but_rejects_nan() {
        let store = Store::new(4096);
        assert_eq!(
            run(&store, &[b"ZADD", b"z", b"inf", b"high", b"-inf", b"low"]),
            b":2\r\n"
        );
        assert_eq!(run(&store, &[b"ZSCORE", b"z", b"high"]), b"$3\r\ninf\r\n");
        assert_eq!(run(&store, &[b"ZSCORE", b"z", b"low"]), b"$4\r\n-inf\r\n");
        assert_eq!(
            run(&store, &[b"ZINCRBY", b"z", b"-inf", b"high"]),
            b"-ERR resulting score is not a number (NaN)\r\n"
        );
        assert_eq!(
            run(&store, &[b"ZADD", b"z", b"nan", b"bad"]),
            b"-ERR value is not a valid float\r\n"
        );
        assert_eq!(run(&store, &[b"ZADD", b"z", b"-0", b"zero"]), b":1\r\n");
        assert_eq!(run(&store, &[b"ZSCORE", b"z", b"zero"]), b"$1\r\n0\r\n");
        assert_eq!(run(&store, &[b"ZSCORE", b"z", b"high"]), b"$3\r\ninf\r\n");
    }

    #[test]
    fn sorted_set_zpop_and_zadd_incr_follow_reply_shapes() {
        let store = Store::new(4096);
        assert_eq!(run(&store, &[b"ZPOPMIN", b"missing"]), b"*0\r\n");
        assert_eq!(
            run(&store, &[b"ZADD", b"z", b"1", b"a", b"2", b"b"]),
            b":2\r\n"
        );
        assert_eq!(
            run(&store, &[b"ZADD", b"z", b"INCR", b"2", b"a"]),
            b"$1\r\n3\r\n"
        );
        assert_eq!(
            run(&store, &[b"ZADD", b"z", b"NX", b"INCR", b"9", b"a"]),
            b"$-1\r\n"
        );
        assert_eq!(
            run(&store, &[b"ZPOPMIN", b"z", b"1"]),
            b"*2\r\n$1\r\nb\r\n$1\r\n2\r\n"
        );
        assert_eq!(
            run(&store, &[b"ZPOPMAX", b"z"]),
            b"*2\r\n$1\r\na\r\n$1\r\n3\r\n"
        );
        assert_eq!(run(&store, &[b"EXISTS", b"z"]), b":0\r\n");
    }

    #[test]
    fn wrong_types_and_oom_are_atomic_for_new_mutations() {
        let store = Store::new(16);
        assert_eq!(run(&store, &[b"SET", b"s", b"x"]), b"+OK\r\n");
        for command in [
            &[b"HDEL".as_slice(), b"s", b"f"][..],
            &[b"HEXISTS".as_slice(), b"s", b"f"][..],
            &[b"HINCRBY".as_slice(), b"s", b"f", b"1"][..],
            &[b"HMGET".as_slice(), b"s", b"f"][..],
            &[b"LINDEX".as_slice(), b"s", b"0"][..],
            &[b"LREM".as_slice(), b"s", b"0", b"v"][..],
            &[b"LSET".as_slice(), b"s", b"0", b"v"][..],
            &[b"LTRIM".as_slice(), b"s", b"0", b"1"][..],
            &[b"SREM".as_slice(), b"s", b"v"][..],
            &[b"SMISMEMBER".as_slice(), b"s", b"v"][..],
            &[b"SINTER".as_slice(), b"s"][..],
            &[b"ZREM".as_slice(), b"s", b"v"][..],
            &[b"ZCOUNT".as_slice(), b"s", b"-inf", b"+inf"][..],
        ] {
            assert!(
                run(&store, command).starts_with(b"-WRONGTYPE"),
                "{command:?}"
            );
        }
        assert_eq!(run(&store, &[b"GET", b"s"]), b"$1\r\nx\r\n");
        let store = Store::new(18);
        assert_eq!(run(&store, &[b"SET", b"s", b"x"]), b"+OK\r\n");
        assert_eq!(
            run(&store, &[b"HSET", b"h", b"f", b"99999999999999"]),
            b":1\r\n"
        );
        let before = store.data.read().unwrap().payload_bytes;
        assert!(run(&store, &[b"HINCRBY", b"h", b"f", b"1"]).starts_with(b"-OOM"));
        assert_eq!(store.data.read().unwrap().payload_bytes, before);
        assert_eq!(
            run(&store, &[b"HGET", b"h", b"f"]),
            b"$14\r\n99999999999999\r\n"
        );
    }
}
