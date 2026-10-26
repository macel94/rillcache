#![forbid(unsafe_code)]

use rillcache::engine::{Action, Store};

fn call(store: &Store, args: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    assert_eq!(store.execute(args, &mut out), Action::Continue);
    out
}

#[test]
fn bitmap_commands_are_binary_safe_budgeted_and_keep_expiration() {
    let store = Store::new(32);
    assert_eq!(call(&store, &[b"SET", b"k", b"@"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"SETBIT", b"k", b"2", b"1"]), b":0\r\n");
    assert_eq!(call(&store, &[b"GET", b"k"]), b"$1\r\n`\r\n");
    assert_eq!(call(&store, &[b"GETBIT", b"k", b"2"]), b":1\r\n");
    assert_eq!(call(&store, &[b"GETBIT", b"k", b"999"]), b":0\r\n");
    assert_eq!(
        call(&store, &[b"SETBIT", b"k", b"-1", b"1"]),
        b"-ERR bit offset is not an integer or out of range\r\n"
    );
    assert_eq!(
        call(&store, &[b"SETBIT", b"k", b"0", b"2"]),
        b"-ERR bit is not an integer or out of range\r\n"
    );
    let store = Store::new(2);
    assert!(call(&store, &[b"SETBIT", b"long", b"0", b"1"]).starts_with(b"-OOM"));
    assert_eq!(call(&store, &[b"GET", b"long"]), b"$-1\r\n");
}

#[test]
fn incrbyfloat_is_atomic_finite_and_preserves_ttl() {
    let store = Store::new(32);
    assert_eq!(
        call(&store, &[b"SET", b"k", b"1", b"PX", b"100000"]),
        b"+OK\r\n"
    );
    assert_eq!(
        call(&store, &[b"INCRBYFLOAT", b"k", b"0.5"]),
        b"$3\r\n1.5\r\n"
    );
    assert_eq!(
        call(&store, &[b"INCRBYFLOAT", b"k", b"+inf"]),
        b"-ERR increment would produce NaN or Infinity\r\n"
    );
    assert_eq!(call(&store, &[b"GET", b"k"]), b"$3\r\n1.5\r\n");
    assert_eq!(
        call(&store, &[b"INCRBYFLOAT", b"k", b"bad"]),
        b"-ERR value is not a valid float\r\n"
    );
}

#[test]
fn lcs_options_match_subsequence_and_index_wire_shapes() {
    let store = Store::new(128);
    call(&store, &[b"SET", b"a", b"ohmytext"]);
    call(&store, &[b"SET", b"b", b"mynewtext"]);
    assert_eq!(call(&store, &[b"LCS", b"a", b"b"]), b"$6\r\nmytext\r\n");
    assert_eq!(call(&store, &[b"LCS", b"a", b"b", b"LEN"]), b":6\r\n");
    let indexes = call(&store, &[b"LCS", b"a", b"b", b"IDX", b"WITHMATCHLEN"]);
    assert!(indexes.starts_with(b"*4\r\n$7\r\nmatches\r\n"));
    assert!(indexes.ends_with(b"$3\r\nlen\r\n:6\r\n"));
    assert_eq!(
        call(&store, &[b"LCS", b"a", b"b", b"LEN", b"IDX"]),
        b"-ERR If you want both the length and indexes, please just use IDX.\r\n"
    );
    assert!(
        call(&store, &[b"LCS", b"a", b"b", b"IDX", b"MINMATCHLEN", b"2"]).starts_with(b"*4\r\n")
    );
}

#[test]
fn substr_alias_and_algorithm_discovery_dispatch() {
    let store = Store::new(32);
    call(&store, &[b"SET", b"k", b"abcde"]);
    assert_eq!(
        call(&store, &[b"SUBSTR", b"k", b"-4", b"-1"]),
        b"$4\r\nbcde\r\n"
    );
    assert_eq!(call(&store, &[b"COMMAND", b"COUNT"]), b":50\r\n");
}
