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
    assert_eq!(call(&store, &[b"COMMAND", b"COUNT"]), b":203\r\n");
}

#[test]
fn bitcount_and_bitpos_ranges_match_reference_boundaries() {
    let store = Store::new(1024);
    assert_eq!(call(&store, &[b"SET", b"k", b"\xff\x00\x0f"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"BITCOUNT", b"k"]), b":12\r\n");
    assert_eq!(call(&store, &[b"BITCOUNT", b"k", b"0", b"0"]), b":8\r\n");
    assert_eq!(call(&store, &[b"BITCOUNT", b"k", b"-1", b"-1"]), b":4\r\n");
    assert_eq!(
        call(&store, &[b"BITCOUNT", b"k", b"0", b"7", b"BIT"]),
        b":8\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITCOUNT", b"k", b"0", b"7", b"BYTE"]),
        b":12\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITCOUNT", b"k", b"-2", b"-1", b"BIT"]),
        b":2\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITCOUNT", b"k", b"100", b"200", b"BIT"]),
        b":0\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITCOUNT", b"missing", b"0", b"1"]),
        b":0\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITCOUNT", b"k", b"id", b"1"]),
        b"-ERR value is not an integer or out of range\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITCOUNT", b"k", b"0", b"1", b"BOGUS"]),
        b"-ERR syntax error\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITCOUNT", b"k", b"0"]),
        b"-ERR syntax error\r\n"
    );

    assert_eq!(call(&store, &[b"BITPOS", b"k", b"0"]), b":8\r\n");
    assert_eq!(call(&store, &[b"BITPOS", b"k", b"1"]), b":0\r\n");
    assert_eq!(
        call(&store, &[b"BITPOS", b"k", b"0", b"0", b"0"]),
        b":-1\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITPOS", b"k", b"0", b"0", b"1"]),
        b":8\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITPOS", b"k", b"1", b"0", b"0", b"BIT"]),
        b":0\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITPOS", b"k", b"0", b"-5", b"-1"]),
        b":8\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITPOS", b"k", b"0", b"-5", b"-1", b"BIT"]),
        b":19\r\n"
    );
    assert_eq!(call(&store, &[b"BITPOS", b"missing", b"0"]), b":0\r\n");
    assert_eq!(call(&store, &[b"BITPOS", b"missing", b"1"]), b":-1\r\n");
    assert_eq!(call(&store, &[b"SET", b"ones", b"\xff\xff"]), b"+OK\r\n");
    assert_eq!(call(&store, &[b"BITPOS", b"ones", b"0"]), b":16\r\n");
    assert_eq!(
        call(&store, &[b"BITPOS", b"ones", b"0", b"0", b"0"]),
        b":-1\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITPOS", b"k", b"2"]),
        b"-ERR The bit argument must be 1 or 0.\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITPOS", b"k", b"x"]),
        b"-ERR value is not an integer or out of range\r\n"
    );
}

#[test]
fn bitop_is_binary_atomic_alias_safe_and_clears_destination_ttl() {
    let store = Store::new(1024);
    call(&store, &[b"SET", b"a", b"\xf0\x0f"]);
    call(&store, &[b"SET", b"b", b"\x0f\xf0\xff"]);
    assert_eq!(
        call(&store, &[b"BITOP", b"AND", b"d", b"a", b"b"]),
        b":3\r\n"
    );
    assert_eq!(call(&store, &[b"GET", b"d"]), b"$3\r\n\0\0\0\r\n");
    assert_eq!(
        call(&store, &[b"BITOP", b"OR", b"d", b"a", b"b"]),
        b":3\r\n"
    );
    assert_eq!(call(&store, &[b"GET", b"d"]), b"$3\r\n\xff\xff\xff\r\n");
    assert_eq!(
        call(&store, &[b"BITOP", b"XOR", b"d", b"a", b"b"]),
        b":3\r\n"
    );
    assert_eq!(call(&store, &[b"GET", b"d"]), b"$3\r\n\xff\xff\xff\r\n");
    assert_eq!(call(&store, &[b"BITOP", b"NOT", b"d", b"a"]), b":2\r\n");
    assert_eq!(call(&store, &[b"GET", b"d"]), b"$2\r\n\x0f\xf0\r\n");
    assert_eq!(
        call(&store, &[b"BITOP", b"AND", b"d", b"missing"]),
        b":0\r\n"
    );
    assert_eq!(call(&store, &[b"EXISTS", b"d"]), b":0\r\n");

    assert_eq!(call(&store, &[b"BITOP", b"OR", b"s", b"a"]), b":2\r\n");
    assert_eq!(call(&store, &[b"PEXPIRE", b"s", b"10000"]), b":1\r\n");
    assert_eq!(call(&store, &[b"BITOP", b"NOT", b"s", b"s"]), b":2\r\n");
    assert_eq!(call(&store, &[b"GET", b"s"]), b"$2\r\n\x0f\xf0\r\n");
    assert_eq!(call(&store, &[b"PTTL", b"s"]), b":-1\r\n");

    call(&store, &[b"RPUSH", b"list", b"x"]);
    assert_eq!(
        call(&store, &[b"BITOP", b"OR", b"d", b"a", b"list"]),
        b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
    );
    assert_eq!(call(&store, &[b"EXISTS", b"d"]), b":0\r\n");
    assert_eq!(
        call(&store, &[b"BITOP", b"NOT", b"d", b"a", b"b"]),
        b"-ERR BITOP NOT must be called with a single source key.\r\n"
    );
    assert_eq!(
        call(&store, &[b"BITOP", b"BOGUS", b"d", b"a"]),
        b"-ERR syntax error\r\n"
    );
}
