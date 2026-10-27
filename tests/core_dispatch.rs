#![forbid(unsafe_code)]

use rillcache::engine::Store;
use rillcache::session::Session;

fn call(session: &mut Session, store: &Store, args: &[&[u8]]) -> Vec<u8> {
    let mut output = Vec::new();
    session.execute(store, args, &mut output);
    output
}

#[test]
fn core_dispatch_preserves_authentication_and_acl_before_storage() {
    let store = Store::new(4096);
    assert!(store.set_requirepass(Some(b"secret")));
    let mut admin = Session::new();
    for args in [
        vec![b"PING".as_slice()],
        vec![b"GET".as_slice(), b"k"],
        vec![b"SET".as_slice(), b"k", b"v"],
        vec![b"ECHO".as_slice(), b"v"],
    ] {
        assert_eq!(
            call(&mut admin, &store, &args),
            b"-NOAUTH Authentication required.\r\n"
        );
    }
    assert_eq!(call(&mut admin, &store, &[b"AUTH", b"secret"]), b"+OK\r\n");
    assert_eq!(call(&mut admin, &store, &[b"GET", b"k"]), b"$-1\r\n");
    assert_eq!(call(&mut admin, &store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
    assert_eq!(call(&mut admin, &store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
    assert_eq!(call(&mut admin, &store, &[b"ECHO", b"v"]), b"$1\r\nv\r\n");
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[
                b"ACL",
                b"SETUSER",
                b"reader",
                b"on",
                b">pw",
                b"-@all",
                b"+ping",
                b"+get",
                b"~allowed:*"
            ]
        ),
        b"+OK\r\n"
    );
    let mut reader = Session::new();
    assert_eq!(
        call(&mut reader, &store, &[b"AUTH", b"reader", b"pw"]),
        b"+OK\r\n"
    );
    assert_eq!(call(&mut reader, &store, &[b"PING"]), b"+PONG\r\n");
    assert_eq!(
        call(&mut reader, &store, &[b"GET", b"allowed:missing"]),
        b"$-1\r\n"
    );
    assert!(call(&mut reader, &store, &[b"GET", b"k"]).starts_with(b"-NOPERM"));
    assert!(call(&mut reader, &store, &[b"SET", b"allowed:k", b"bad"]).starts_with(b"-NOPERM"));
    assert!(call(&mut reader, &store, &[b"ECHO", b"bad"]).starts_with(b"-NOPERM"));
}

#[test]
fn core_dispatch_preserves_queueing_and_resp3_null_and_binary_framing() {
    let store = Store::new(4096);
    let mut session = Session::new();
    assert!(call(&mut session, &store, &[b"HELLO", b"3"]).starts_with(b"%5\r\n"));
    assert_eq!(call(&mut session, &store, &[b"GET", b"missing"]), b"_\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"ECHO", b"\0\r\n\xff"]),
        b"$4\r\n\0\r\n\xff\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    for args in [
        vec![b"SET".as_slice(), b"k", b"v"],
        vec![b"GET".as_slice(), b"k"],
        vec![b"PING".as_slice()],
    ] {
        assert_eq!(call(&mut session, &store, &args), b"+QUEUED\r\n");
    }
    let mut observer = Session::new();
    assert_eq!(call(&mut observer, &store, &[b"GET", b"k"]), b"$-1\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"EXEC"]),
        b"*3\r\n+OK\r\n$1\r\nv\r\n+PONG\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"missing", b"v", b"XX"]),
        b"_\r\n"
    );
}

#[test]
fn resp2_subscriber_ping_and_resp3_subscriber_data_keep_distinct_paths() {
    let store = Store::new(4096);
    let mut resp2 = Session::new();
    let (sender, _messages) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    resp2.set_pubsub_sender(sender);
    assert_eq!(
        call(&mut resp2, &store, &[b"SUBSCRIBE", b"c"]),
        b"*3\r\n$9\r\nsubscribe\r\n$1\r\nc\r\n:1\r\n"
    );
    assert_eq!(
        call(&mut resp2, &store, &[b"PING", b"b\0"]),
        b"*2\r\n$4\r\npong\r\n$2\r\nb\0\r\n"
    );
    assert!(call(&mut resp2, &store, &[b"GET", b"k"]).starts_with(b"-ERR Can't execute"));
    assert!(call(&mut resp2, &store, &[b"SET", b"k", b"bad"]).starts_with(b"-ERR Can't execute"));
    let mut resp3 = Session::new();
    let (sender, _messages3) = std::sync::mpsc::sync_channel(Store::pubsub_queue_capacity());
    resp3.set_pubsub_sender(sender);
    assert!(call(&mut resp3, &store, &[b"HELLO", b"3"]).starts_with(b"%5\r\n"));
    assert!(call(&mut resp3, &store, &[b"SUBSCRIBE", b"c"]).starts_with(b">3\r\n"));
    assert_eq!(call(&mut resp3, &store, &[b"PING"]), b"+PONG\r\n");
    assert_eq!(call(&mut resp3, &store, &[b"GET", b"k"]), b"_\r\n");
    assert_eq!(call(&mut resp3, &store, &[b"SET", b"k", b"ok"]), b"+OK\r\n");
    assert_eq!(call(&mut resp3, &store, &[b"GET", b"k"]), b"$2\r\nok\r\n");
}
