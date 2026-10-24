use super::*;

fn execute(store: &Store, arguments: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::new();
    assert_eq!(store.execute(arguments, &mut output), Action::Continue);
    output
}

#[test]
fn binary_values_overwrite_and_missing_values_are_distinct() {
    let store = Store::new(100);
    assert_eq!(execute(&store, &[b"gEt", b"\xff"]), b"$-1\r\n");
    assert_eq!(execute(&store, &[b"sEt", b"\xff", b"\0\r\n"]), b"+OK\r\n");
    assert_eq!(execute(&store, &[b"GET", b"\xff"]), b"$3\r\n\0\r\n\r\n");
    assert_eq!(execute(&store, &[b"SET", b"\xff", b""]), b"+OK\r\n");
    assert_eq!(execute(&store, &[b"GET", b"\xff"]), b"$0\r\n\r\n");
}

#[test]
fn duplicates_match_redis_semantics() {
    let store = Store::new(100);
    execute(&store, &[b"SET", b"key", b"value"]);
    assert_eq!(
        execute(&store, &[b"EXISTS", b"key", b"key", b"missing"]),
        b":2\r\n"
    );
    assert_eq!(
        execute(&store, &[b"DEL", b"key", b"key", b"missing"]),
        b":1\r\n"
    );
    assert_eq!(execute(&store, &[b"EXISTS", b"key"]), b":0\r\n");
}

#[test]
fn payload_budget_is_atomic_and_reclaimed_on_overwrite_and_delete() {
    let store = Store::new(5);
    assert_eq!(execute(&store, &[b"SET", b"k", b"1234"]), b"+OK\r\n");
    assert!(execute(&store, &[b"SET", b"k", b"12345"]).starts_with(b"-OOM"));
    assert_eq!(execute(&store, &[b"GET", b"k"]), b"$4\r\n1234\r\n");
    assert!(execute(&store, &[b"SET", b"x", b""]).starts_with(b"-OOM"));
    assert_eq!(execute(&store, &[b"SET", b"k", b"1"]), b"+OK\r\n");
    assert_eq!(execute(&store, &[b"SET", b"x", b"12"]), b"+OK\r\n");
    assert_eq!(execute(&store, &[b"DEL", b"k", b"x"]), b":2\r\n");
    assert_eq!(store.data.read().unwrap().payload_bytes, 0);
    assert_eq!(execute(&store, &[b"SET", b"new", b"12"]), b"+OK\r\n");
}

#[test]
fn errors_do_not_mutate_data_or_inject_resp_frames() {
    let store = Store::new(100);
    assert_eq!(
        execute(&store, &[b"SET", b"k", b"v", b"UNSUPPORTED", b"1"]),
        b"-ERR syntax error\r\n"
    );
    assert_eq!(execute(&store, &[b"GET", b"k"]), b"$-1\r\n");
    assert_eq!(
        execute(&store, &[b"\r\n+INJECTED\r\n"]),
        b"-ERR unknown command\r\n"
    );
    for (arguments, command) in [
        (vec![b"PING".as_slice(), b"a", b"b"], "ping"),
        (vec![b"ECHO".as_slice()], "echo"),
        (vec![b"GET".as_slice()], "get"),
        (vec![b"SET".as_slice(), b"key"], "set"),
        (vec![b"DEL".as_slice()], "del"),
        (vec![b"EXISTS".as_slice()], "exists"),
        (vec![b"QUIT".as_slice(), b"extra"], "quit"),
    ] {
        assert_eq!(
            execute(&store, &arguments),
            format!("-ERR wrong number of arguments for '{command}' command\r\n").as_bytes()
        );
    }
}

#[test]
fn conditional_sets_distinguish_missing_existing_and_empty_values() {
    for key in [b"".as_slice(), b"\0\xff"] {
        let store = Store::new(100);
        assert_eq!(execute(&store, &[b"SET", key, b"no", b"XX"]), b"$-1\r\n");
        assert_eq!(execute(&store, &[b"GET", key]), b"$-1\r\n");
        assert_eq!(execute(&store, &[b"SET", key, b"", b"nX"]), b"+OK\r\n");
        assert_eq!(execute(&store, &[b"SET", key, b"no", b"NX"]), b"$-1\r\n");
        assert_eq!(execute(&store, &[b"GET", key]), b"$0\r\n\r\n");
        assert_eq!(
            execute(&store, &[b"SET", key, b"\0\xff", b"xX"]),
            b"+OK\r\n"
        );
        assert_eq!(execute(&store, &[b"GET", key]), b"$2\r\n\0\xff\r\n");
        assert_eq!(execute(&store, &[b"DEL", key]), b":1\r\n");
        assert_eq!(execute(&store, &[b"SET", key, b"no", b"XX"]), b"$-1\r\n");
        assert_eq!(execute(&store, &[b"SET", key, b"new", b"NX"]), b"+OK\r\n");
    }
}

#[test]
fn identical_conditional_flags_may_repeat() {
    let store = Store::new(100);
    assert_eq!(
        execute(&store, &[b"SET", b"k", b"first", b"NX", b"nx"]),
        b"+OK\r\n"
    );
    assert_eq!(
        execute(&store, &[b"SET", b"k", b"no", b"nx", b"NX"]),
        b"$-1\r\n"
    );
    assert_eq!(
        execute(&store, &[b"SET", b"k", b"last", b"XX", b"xx"]),
        b"+OK\r\n"
    );
    assert_eq!(execute(&store, &[b"GET", b"k"]), b"$4\r\nlast\r\n");
}

#[test]
fn all_set_options_are_validated_even_when_a_condition_would_reject() {
    let invalid: &[&[&[u8]]] = &[
        &[b"NX", b"XX"],
        &[b"XX", b"NX"],
        &[b"NX", b"nx", b"XX"],
        &[b"NX", b"BAD"],
        &[b"XX", b"EX"],
        &[b""],
        &[b"NXX"],
        &[b"NX "],
        &[b"NX\0suffix"],
        &[b"XX\0suffix"],
        &[b"\xff"],
        &[b"PX"],
        &[b"KEEPTTL", b"EX", b"1"],
    ];
    for present in [false, true] {
        let store = Store::new(100);
        if present {
            execute(&store, &[b"SET", b"k", b"old"]);
        }
        for flags in invalid {
            let mut arguments: Vec<&[u8]> = vec![b"SET", b"k", b"new"];
            arguments.extend_from_slice(flags);
            assert_eq!(
                execute(&store, &arguments),
                b"-ERR syntax error\r\n",
                "{flags:?}"
            );
            assert_eq!(
                execute(&store, &[b"GET", b"k"]),
                if present {
                    b"$3\r\nold\r\n".as_slice()
                } else {
                    b"$-1\r\n"
                }
            );
        }
    }
}

#[test]
fn conditional_rejection_does_not_consume_payload_budget() {
    let store = Store::new(5);
    assert_eq!(execute(&store, &[b"SET", b"k", b"1234", b"NX"]), b"+OK\r\n");
    for arguments in [
        [b"SET".as_slice(), b"k", b"123456", b"NX"],
        [b"SET".as_slice(), b"missing", b"123456", b"XX"],
    ] {
        assert_eq!(execute(&store, &arguments), b"$-1\r\n");
        assert_eq!(store.data.read().unwrap().payload_bytes, 5);
    }
    assert!(execute(&store, &[b"SET", b"k", b"12345", b"XX"]).starts_with(b"-OOM"));
    assert_eq!(execute(&store, &[b"GET", b"k"]), b"$4\r\n1234\r\n");
    assert_eq!(execute(&store, &[b"SET", b"k", b"1", b"XX"]), b"+OK\r\n");
    assert_eq!(execute(&store, &[b"SET", b"x", b"12", b"NX"]), b"+OK\r\n");
    assert_eq!(store.data.read().unwrap().payload_bytes, 5);
    assert_eq!(execute(&store, &[b"DEL", b"k", b"x"]), b":2\r\n");
    assert_eq!(store.data.read().unwrap().payload_bytes, 0);
}

#[test]
fn ping_echo_and_quit() {
    let store = Store::new(100);
    assert_eq!(execute(&store, &[b"PING"]), b"+PONG\r\n");
    assert_eq!(execute(&store, &[b"PING", b""]), b"$0\r\n\r\n");
    assert_eq!(execute(&store, &[b"ECHO", b"a\0b"]), b"$3\r\na\0b\r\n");
    let mut output = Vec::new();
    assert_eq!(store.execute(&[b"quit"], &mut output), Action::Close);
    assert_eq!(output, b"+OK\r\n");
}
