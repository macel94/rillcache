//! Session unit tests. They exercise the assembled dispatcher and module state.

use super::blocking::{Blocked, BlockedKind};
use super::reply::{convert_resp2_reply, parse_resp_value};
use super::*;

use crate::protocol::encode_bulk;

fn call(session: &mut Session, store: &Store, args: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    session.execute(store, args, &mut out);
    out
}

#[test]
fn hello_protocol_switch_and_errors_are_atomic() {
    let store = Store::new(1024);
    let mut session = Session::new();
    let hello = call(&mut session, &store, &[b"HELLO", b"3"]);
    assert!(hello.starts_with(b"%5\r\n"));
    assert!(hello.windows(9).any(|v| v == b"rillcache"));
    assert_eq!(call(&mut session, &store, &[b"GET", b"missing"]), b"_\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"CLIENT", b"GETNAME"]),
        b"_\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"HELLO", b"9"]),
        b"-NOPROTO unsupported protocol version\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"GET", b"missing"]), b"_\r\n");
    assert_eq!(
            call(
                &mut session,
                &store,
                &[b"HELLO", b"2", b"AUTH", b"default", b"pw"]
            ),
            b"-ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?\r\n"
        );
    assert_eq!(call(&mut session, &store, &[b"GET", b"missing"]), b"_\r\n");
}

#[test]
fn hello_no_argument_reports_current_protocol_and_setname_is_atomic() {
    let store = Store::new(1024);
    let mut session = Session::new();
    assert!(call(&mut session, &store, &[b"HELLO"]).starts_with(b"*10\r\n"));
    assert!(
        call(
            &mut session,
            &store,
            &[b"HELLO", b"3", b"SETNAME", b"client"]
        )
        .starts_with(b"%5\r\n")
    );
    assert_eq!(
        call(&mut session, &store, &[b"CLIENT", b"GETNAME"]),
        b"$6\r\nclient\r\n"
    );
    assert!(call(&mut session, &store, &[b"HELLO"]).starts_with(b"%5\r\n"));
    assert_eq!(
        call(
            &mut session,
            &store,
            &[b"HELLO", b"2", b"SETNAME", b"bad name"]
        ),
        b"-ERR syntax error\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"GET", b"missing"]),
        b"_\r\n",
        "invalid negotiation must preserve RESP3"
    );
}

#[test]
fn metadata_validation_is_bounded_atomic_and_observable() {
    let store = Store::new(1024);
    let mut session = Session::new();
    assert_eq!(
        call(&mut session, &store, &[b"CLIENT", b"SETNAME", b"client-a"]),
        b"+OK\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"CLIENT", b"SETNAME", b"bad name"]),
        b"-ERR invalid client name\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"CLIENT", b"GETNAME"]),
        b"$8\r\nclient-a\r\n"
    );
    assert_eq!(
        call(
            &mut session,
            &store,
            &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"lib"]
        ),
        b"+OK\r\n"
    );
    assert_eq!(
        call(
            &mut session,
            &store,
            &[b"CLIENT", b"SETINFO", b"LIB-VER", b"1.2"]
        ),
        b"+OK\r\n"
    );
    let info = call(&mut session, &store, &[b"CLIENT", b"INFO"]);
    assert!(info.windows(13).any(|v| v == b"name=client-a"));
    assert!(info.windows(12).any(|v| v == b"lib-name=lib"));
    assert_eq!(
        call(
            &mut session,
            &store,
            &[
                b"CLIENT",
                b"SETINFO",
                b"LIB-VER",
                &[b'x'; MAX_LIBRARY_FIELD_BYTES + 1]
            ]
        ),
        b"-ERR invalid client metadata\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"CLIENT", b"GETNAME"]),
        b"$8\r\nclient-a\r\n"
    );
}

#[test]
fn resp3_null_conversion_is_framing_aware_for_binary_payloads_and_arrays() {
    let binary = b"$7\r\n$-1\r\n!\r\n";
    let mut reply = Vec::new();
    encode_bulk(Some(binary), &mut reply);
    let original = reply.clone();
    let mut converted = Vec::new();
    assert!(parse_resp_value(&reply, &mut 0, &mut converted));
    assert_eq!(converted, original);
    let mut array = b"*2\r\n$-1\r\n$1\r
x\r\n"
        .to_vec();
    let mut converted = Vec::new();
    assert!(parse_resp_value(&array, &mut 0, &mut converted));
    assert_eq!(
        converted,
        b"*2\r\n_\r\n$1\r
x\r\n"
    );
    array = b"$-1\r\n".to_vec();
    let mut converted = Vec::new();
    assert!(parse_resp_value(&array, &mut 0, &mut converted));
    assert_eq!(converted, b"_\r\n");

    assert_eq!(convert_resp2_reply(b"*-1\r\n"), b"_\r\n");
    let nested = b"*2\r\n*-1\r\n$-1\r\n";
    let mut converted = Vec::new();
    assert!(parse_resp_value(nested, &mut 0, &mut converted));
    assert_eq!(converted, b"*2\r\n_\r\n_\r\n");
}

#[test]
fn select_only_accepts_database_zero_and_ids_are_distinct() {
    let store = Store::new(1024);
    let mut first = Session::new();
    let mut second = Session::new();
    let one = call(&mut first, &store, &[b"CLIENT", b"ID"]);
    let two = call(&mut second, &store, &[b"CLIENT", b"ID"]);
    assert_ne!(one, two);
    assert_eq!(call(&mut first, &store, &[b"SELECT", b"00"]), b"+OK\r\n");
    assert_eq!(
        call(&mut first, &store, &[b"SELECT", b"1"]),
        b"-ERR DB index is out of range\r\n"
    );
}

#[test]
fn multi_queues_then_executes_one_ordered_array_and_clears_state() {
    let store = Store::new(1024);
    let mut session = Session::new();
    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"a", b"1"]),
        b"+QUEUED\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"INCR", b"a"]), b"+QUEUED\r\n");
    assert_eq!(call(&mut session, &store, &[b"GET", b"a"]), b"+QUEUED\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"EXEC"]),
        b"*3\r\n+OK\r\n:2\r\n$1\r\n2\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"GET", b"a"]), b"$1\r\n2\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"EXEC"]),
        b"-ERR EXEC without MULTI\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"DISCARD"]),
        b"-ERR DISCARD without MULTI\r\n"
    );
}

#[test]
fn discard_drops_the_queue_and_watches_without_executing() {
    let store = Store::new(1024);
    let mut session = Session::new();
    let mut other = Session::new();
    assert_eq!(call(&mut session, &store, &[b"WATCH", b"k"]), b"+OK\r\n");
    assert_eq!(
        call(&mut other, &store, &[b"SET", b"k", b"other"]),
        b"+OK\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"k", b"queued"]),
        b"+QUEUED\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"DISCARD"]), b"+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"EXEC"]),
        b"-ERR EXEC without MULTI\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"GET", b"k"]),
        b"$5\r\nother\r\n"
    );
}

#[test]
fn queue_time_arity_and_unknown_errors_discard_the_whole_transaction() {
    for bad in [
        &[b"SET".as_slice(), b"k"][..],
        &[b"NO-SUCH".as_slice(), b"k"][..],
        &[b"SELECT".as_slice(), b"0"][..],
    ] {
        let store = Store::new(1024);
        let mut session = Session::new();
        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        let error = call(&mut session, &store, bad);
        assert!(error.starts_with(b"-ERR"), "{error:?}");
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"k", b"v"]),
            b"+QUEUED\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"EXEC"]),
            b"-EXECABORT Transaction discarded because of previous errors.\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"$-1\r\n");
    }
}

#[test]
fn watch_aborts_on_another_client_and_unwatch_releases_it() {
    let store = Store::new(1024);
    let mut session = Session::new();
    let mut other = Session::new();
    assert_eq!(call(&mut session, &store, &[b"WATCH", b"k"]), b"+OK\r\n");
    assert_eq!(
        call(&mut other, &store, &[b"SET", b"k", b"other"]),
        b"+OK\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"k", b"queued"]),
        b"+QUEUED\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"EXEC"]), b"*-1\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"GET", b"k"]),
        b"$5\r\nother\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"UNWATCH"]), b"+OK\r\n");
    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"k", b"after"]),
        b"+QUEUED\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"EXEC"]), b"*1\r\n+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"GET", b"k"]),
        b"$5\r\nafter\r\n"
    );
}

#[test]
fn watched_expiry_aborts_exec_without_sleeping() {
    struct TestClock(std::sync::atomic::AtomicI64);
    impl crate::engine::Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
    }
    let clock = std::sync::Arc::new(TestClock(std::sync::atomic::AtomicI64::new(1000)));
    let store = Store::with_clock(1024, clock.clone());
    let mut session = Session::new();
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"k", b"v", b"PX", b"100"]),
        b"+OK\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"WATCH", b"k"]), b"+OK\r\n");
    clock.0.store(1101, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"+QUEUED\r\n");
    assert_eq!(call(&mut session, &store, &[b"EXEC"]), b"*-1\r\n");
    assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"$-1\r\n");
}

#[test]
fn nested_multi_watch_and_empty_exec_follow_redis_control_flow() {
    let store = Store::new(1024);
    let mut session = Session::new();
    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"MULTI"]),
        b"-ERR MULTI calls can not be nested\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"WATCH", b"k"]),
        b"-ERR WATCH inside MULTI is not allowed\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"EXEC"]), b"*0\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"WATCH"]),
        b"-ERR wrong number of arguments for 'watch' command\r\n"
    );
}

#[test]
fn runtime_errors_are_reported_inside_exec_without_rolling_back() {
    let store = Store::new(1024);
    let mut session = Session::new();
    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"a", b"abc"]),
        b"+QUEUED\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"INCR", b"a"]), b"+QUEUED\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"b", b"ok"]),
        b"+QUEUED\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"EXEC"]),
        b"*3\r\n+OK\r\n-ERR value is not an integer or out of range\r\n+OK\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"GET", b"a"]),
        b"$3\r\nabc\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"GET", b"b"]), b"$2\r\nok\r\n");
}

#[test]
fn password_auth_blocks_data_access_until_valid_and_quit_is_allowed() {
    let store = Store::with_auth(1024, crate::engine::Auth::from_password(Some(b"pw")));
    let mut session = Session::new();
    for command in [
        &[b"GET".as_slice(), b"k"][..],
        &[b"PING".as_slice()][..],
        &[b"MULTI".as_slice()][..],
        &[b"CONFIG".as_slice(), b"GET", b"*"][..],
    ] {
        assert_eq!(
            call(&mut session, &store, command),
            b"-NOAUTH Authentication required.\r\n"
        );
    }
    assert_eq!(
        call(&mut session, &store, &[b"AUTH", b"wrong"]),
        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"AUTH", b"default", b"pw"]),
        b"+OK\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"k", b"v"]),
        b"+OK\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
    // A failed re-authentication must not silently deauthenticate a live
    // administrative connection.
    assert_eq!(
        call(&mut session, &store, &[b"AUTH", b"nope"]),
        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"PING"]), b"+PONG\r\n");

    let mut visitor = Session::new();
    let mut output = Vec::new();
    assert_eq!(
        visitor.execute(&store, &[b"QUIT"], &mut output),
        Action::Close
    );
    assert_eq!(output, b"+OK\r\n");
}

#[test]
fn auth_arity_usernames_and_no_password_errors_are_explicit() {
    let open = Store::new(1024);
    let mut session = Session::new();
    assert_eq!(
            call(&mut session, &open, &[b"AUTH", b"anything"]),
            b"-ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?\r\n"
        );
    assert_eq!(call(&mut session, &open, &[b"PING"]), b"+PONG\r\n");

    let store = Store::with_auth(1024, crate::engine::Auth::from_password(Some(b"pw")));
    let mut session = Session::new();
    assert_eq!(
        call(&mut session, &store, &[b"AUTH", b"other", b"pw"]),
        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"AUTH", b"a", b"b", b"c"]),
        b"-ERR wrong number of arguments for 'auth' command\r\n"
    );
    assert_eq!(
        call(
            &mut session,
            &store,
            &[b"HELLO", b"3", b"AUTH", b"default", b"bad"]
        ),
        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"GET", b"k"]),
        b"-NOAUTH Authentication required.\r\n"
    );
    assert!(
        call(
            &mut session,
            &store,
            &[b"HELLO", b"3", b"AUTH", b"default", b"pw"]
        )
        .starts_with(b"%5\r\n")
    );
    assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"_\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"HELLO", b"2", b"AUTH", b"default"]),
        b"-ERR syntax error\r\n"
    );
    assert_eq!(
        call(
            &mut session,
            &store,
            &[b"HELLO", b"AUTH", b"default", b"pw"]
        ),
        b"-NOPROTO unsupported protocol version\r\n"
    );
}

#[test]
fn hello_auth_is_atomic_with_protocol_and_name_negotiation() {
    let store = Store::with_auth(1024, crate::engine::Auth::from_password(Some(b"pw")));
    let mut session = Session::new();
    let mut output = Vec::new();
    assert_eq!(
        session.execute(
            &store,
            &[
                b"HELLO", b"3", b"AUTH", b"default", b"bad", b"SETNAME", b"name"
            ],
            &mut output
        ),
        Action::Continue
    );
    assert_eq!(
        output,
        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"GET", b"k"]),
        b"-NOAUTH Authentication required.\r\n"
    );

    let mut output = Vec::new();
    assert_eq!(
        session.execute(
            &store,
            &[
                b"HELLO", b"3", b"AUTH", b"default", b"pw", b"SETNAME", b"client"
            ],
            &mut output
        ),
        Action::Continue
    );
    assert!(output.starts_with(b"%5\r\n"));
    assert_eq!(
        call(&mut session, &store, &[b"CLIENT", b"GETNAME"]),
        b"$6\r\nclient\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"HELLO", b"2", b"AUTH", b"default"]),
        b"-ERR syntax error\r\n"
    );
}

#[test]
fn config_requirepass_rotates_disables_and_never_echoes_the_secret() {
    let store = Store::new(1024);
    let mut admin = Session::new();
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[b"CONFIG", b"SET", b"requirepass", b"first"]
        ),
        b"+OK\r\n"
    );
    assert_eq!(
        call(&mut admin, &store, &[b"GET", b"k"]),
        b"-NOAUTH Authentication required.\r\n"
    );
    assert_eq!(call(&mut admin, &store, &[b"AUTH", b"first"]), b"+OK\r\n");
    assert_eq!(
        call(&mut admin, &store, &[b"CONFIG", b"GET", b"requirepass"]),
        b"*2\r\n$11\r\nrequirepass\r\n$10\r\n<redacted>\r\n"
    );
    assert!(
        !call(&mut admin, &store, &[b"CONFIG", b"GET", b"*"])
            .windows(5)
            .any(|window| window == b"first")
    );
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[b"CONFIG", b"SET", b"requirepass", b"second"]
        ),
        b"+OK\r\n"
    );
    let mut other = Session::new();
    assert_eq!(
        call(&mut other, &store, &[b"AUTH", b"first"]),
        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
    );
    assert_eq!(call(&mut other, &store, &[b"AUTH", b"second"]), b"+OK\r\n");
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[b"CONFIG", b"SET", b"requirepass", b"too-long"]
        ),
        b"+OK\r\n"
    );
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[
                b"CONFIG",
                b"SET",
                b"maxmemory",
                b"0",
                b"requirepass",
                b"third"
            ]
        ),
        b"-ERR maxmemory must be a positive payload byte budget\r\n"
    );
    assert_eq!(
        call(&mut admin, &store, &[b"AUTH", b"third"]),
        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
    );
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[b"CONFIG", b"SET", b"requirepass", b""]
        ),
        b"+OK\r\n"
    );
    let mut fresh = Session::new();
    assert_eq!(call(&mut fresh, &store, &[b"PING"]), b"+PONG\r\n");
    assert_eq!(
        call(&mut fresh, &store, &[b"CONFIG", b"GET", b"requirepass"]),
        b"*2\r\n$11\r\nrequirepass\r\n$0\r\n\r\n"
    );
}

#[test]
fn acl_users_restrict_commands_keys_and_whoami() {
    let store = Store::new(1024);
    let mut admin = Session::new();
    assert_eq!(
        call(&mut admin, &store, &[b"ACL", b"WHOAMI"]),
        b"$7\r\ndefault\r\n"
    );
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[
                b"ACL",
                b"SETUSER",
                b"alice",
                b"on",
                b">pw",
                b"resetkeys",
                b"~app:*",
                b"resetchannels",
                b"&events:*",
                b"-@all",
                b"+@read",
                b"+@write"
            ]
        ),
        b"+OK\r\n"
    );
    assert!(call(&mut admin, &store, &[b"ACL", b"USERS"]).starts_with(b"*2\r\n"));
    assert!(
        call(&mut admin, &store, &[b"ACL", b"LIST"])
            .windows(9)
            .any(|window| window == b"user alic")
    );
    assert!(
        call(&mut admin, &store, &[b"ACL", b"CAT", b"read"])
            .windows(3)
            .any(|window| window == b"get")
    );
    let details = call(&mut admin, &store, &[b"ACL", b"GETUSER", b"alice"]);
    assert!(details.starts_with(b"*12\r\n"));
    assert!(details.windows(7).any(|window| window == b"flags\r\n"));
    assert!(details.windows(6).any(|window| window == b"~app:*"));
    assert_eq!(
        call(&mut admin, &store, &[b"ACL", b"GETUSER", b"missing"]),
        b"$-1\r\n"
    );

    let mut alice = Session::new();
    assert_eq!(
        call(&mut alice, &store, &[b"AUTH", b"alice", b"pw"]),
        b"+OK\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"ACL", b"WHOAMI"]),
        b"$5\r\nalice\r\n"
    );
    assert_eq!(call(&mut alice, &store, &[b"GET", b"app:k"]), b"$-1\r\n");
    assert_eq!(
        call(&mut alice, &store, &[b"SET", b"app:k", b"v"]),
        b"+OK\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"GET", b"app:k"]),
        b"$1\r\nv\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"GET", b"other"]),
        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"PING"]),
        b"-NOPERM this user has no permissions to run the 'ping' command\r\n"
    );
    assert_eq!(call(&mut alice, &store, &[b"DEL", b"app:k"]), b":1\r\n");

    assert_eq!(
        call(&mut admin, &store, &[b"ACL", b"DELUSER", b"default"]),
        b"-ERR The 'default' user cannot be removed\r\n"
    );
    assert_eq!(
        call(&mut admin, &store, &[b"ACL", b"DELUSER", b"alice"]),
        b":1\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"GET", b"app:k"]),
        b"-NOAUTH Authentication required.\r\n"
    );
    assert_eq!(
        call(&mut admin, &store, &[b"ACL", b"SAVE"]),
        b"-ERR This instance has no ACL file configured\r\n"
    );
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[b"ACL", b"SETUSER", b"bob", b"on", b"+nope"]
        ),
        b"-ERR Unknown command or category name in ACL\r\n"
    );
    assert_eq!(
        call(&mut admin, &store, &[b"ACL", b"GETUSER", b"bob"]),
        b"$-1\r\n"
    );
}

#[test]
fn denied_acl_command_marks_an_active_transaction_dirty() {
    let store = Store::new(1024);
    let mut admin = Session::new();
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
                b"allkeys",
                b"-@all",
                b"+@read",
                b"+@transaction"
            ]
        ),
        b"+OK\r\n"
    );
    let mut reader = Session::new();
    assert_eq!(
        call(&mut reader, &store, &[b"AUTH", b"reader", b"pw"]),
        b"+OK\r\n"
    );
    assert_eq!(call(&mut reader, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(call(&mut reader, &store, &[b"GET", b"k"]), b"+QUEUED\r\n");
    assert_eq!(
        call(&mut reader, &store, &[b"SET", b"k", b"v"]),
        b"-NOPERM this user has no permissions to run the 'set' command\r\n"
    );
    assert_eq!(
        call(&mut reader, &store, &[b"EXEC"]),
        b"-EXECABORT Transaction discarded because of previous errors.\r\n"
    );
    assert_eq!(call(&mut reader, &store, &[b"GET", b"k"]), b"$-1\r\n");
}

#[test]
fn exec_rechecks_acls_so_revocation_cannot_commit_queued_writes() {
    let store = Store::new(1024);
    let mut admin = Session::new();
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[
                b"ACL",
                b"SETUSER",
                b"alice",
                b"on",
                b">pw",
                b"allkeys",
                b"allchannels",
                b"+@all"
            ]
        ),
        b"+OK\r\n"
    );
    let mut alice = Session::new();
    assert_eq!(
        call(&mut alice, &store, &[b"AUTH", b"alice", b"pw"]),
        b"+OK\r\n"
    );
    assert_eq!(call(&mut alice, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&mut alice, &store, &[b"SET", b"k", b"v"]),
        b"+QUEUED\r\n"
    );
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[b"ACL", b"SETUSER", b"alice", b"-@write"]
        ),
        b"+OK\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"EXEC"]),
        b"-NOPERM this user has no permissions to run the 'set' command\r\n"
    );
    assert_eq!(call(&mut admin, &store, &[b"GET", b"k"]), b"$-1\r\n");
}

#[test]
fn pubsub_subscribe_publish_patterns_and_unsubscribe_are_ordered() {
    let store = Store::new(1024);
    let mut subscriber = Session::new();
    let (sender, receiver) = std::sync::mpsc::sync_channel(16);
    subscriber.set_pubsub_sender(sender);
    assert!(!subscriber.is_subscribed());
    assert_eq!(
        call(&mut subscriber, &store, &[b"SUBSCRIBE", b"a", b"b"]),
        b"*3\r\n$9\r\nsubscribe\r\n$1\r\na\r\n:1\r\n*3\r\n$9\r\nsubscribe\r\n$1\r\nb\r\n:2\r\n"
    );
    assert!(subscriber.is_subscribed());
    assert_eq!(
        call(&mut subscriber, &store, &[b"PSUBSCRIBE", b"news.*"]),
        b"*3\r\n$10\r\npsubscribe\r\n$6\r\nnews.*\r\n:3\r\n"
    );

    let mut publisher = Session::new();
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBLISH", b"a", b"hello"]),
        b":1\r\n"
    );
    assert_eq!(
        receiver.recv().unwrap(),
        b"*3\r\n$7\r\nmessage\r\n$1\r\na\r\n$5\r\nhello\r\n"
    );
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBLISH", b"news.tech", b"x"]),
        b":1\r\n"
    );
    assert_eq!(
        receiver.recv().unwrap(),
        b"*4\r\n$8\r\npmessage\r\n$6\r\nnews.*\r\n$9\r\nnews.tech\r\n$1\r\nx\r\n"
    );
    assert_eq!(
        call(
            &mut publisher,
            &store,
            &[b"PUBSUB", b"NUMSUB", b"a", b"missing"]
        ),
        b"*4\r\n$1\r\na\r\n:1\r\n$7\r\nmissing\r\n:0\r\n"
    );
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBSUB", b"CHANNELS"]),
        b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
    );
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBSUB", b"NUMPAT"]),
        b":1\r\n"
    );

    assert_eq!(
        call(&mut subscriber, &store, &[b"PING", b"hi"]),
        b"*2\r\n$4\r\npong\r\n$2\r\nhi\r\n"
    );
    assert_eq!(
            call(&mut subscriber, &store, &[b"GET", b"k"]),
            b"-ERR Can't execute 'get': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n"
        );
    assert_eq!(
        call(&mut subscriber, &store, &[b"UNSUBSCRIBE", b"a"]),
        b"*3\r\n$11\r\nunsubscribe\r\n$1\r\na\r\n:2\r\n"
    );
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBLISH", b"a", b"gone"]),
        b":0\r\n"
    );
    assert_eq!(
        call(&mut subscriber, &store, &[b"UNSUBSCRIBE"]),
        b"*3\r\n$11\r\nunsubscribe\r\n$1\r\nb\r\n:1\r\n"
    );
    assert!(receiver.try_recv().is_err());
    assert_eq!(
        call(&mut subscriber, &store, &[b"PUNSUBSCRIBE"]),
        b"*3\r\n$12\r\npunsubscribe\r\n$6\r\nnews.*\r\n:0\r\n"
    );
    assert!(!subscriber.is_subscribed());
    store.pubsub_remove_client(subscriber.id());
}

#[test]
fn publish_and_spublish_are_queued_and_delivered_at_exec() {
    let store = Store::new(1024);
    let mut normal = Session::new();
    let (normal_sender, normal_receiver) = std::sync::mpsc::sync_channel(8);
    normal.set_pubsub_sender(normal_sender);
    assert_eq!(
        call(&mut normal, &store, &[b"SUBSCRIBE", b"normal"]),
        b"*3\r\n$9\r\nsubscribe\r\n$6\r\nnormal\r\n:1\r\n"
    );
    let mut shard = Session::new();
    let (shard_sender, shard_receiver) = std::sync::mpsc::sync_channel(8);
    shard.set_pubsub_sender(shard_sender);
    assert_eq!(
        call(&mut shard, &store, &[b"SSUBSCRIBE", b"shard"]),
        b"*3\r\n$10\r\nssubscribe\r\n$5\r\nshard\r\n:1\r\n"
    );

    let mut publisher = Session::new();
    assert_eq!(call(&mut publisher, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBLISH", b"normal", b"hello"]),
        b"+QUEUED\r\n"
    );
    assert_eq!(
        call(&mut publisher, &store, &[b"SPUBLISH", b"shard", b"hello"]),
        b"+QUEUED\r\n"
    );
    assert!(normal_receiver.try_recv().is_err());
    assert!(shard_receiver.try_recv().is_err());
    assert_eq!(
        call(&mut publisher, &store, &[b"EXEC"]),
        b"*2\r\n:1\r\n:1\r\n"
    );
    assert_eq!(
        normal_receiver.try_recv().unwrap(),
        b"*3\r\n$7\r\nmessage\r\n$6\r\nnormal\r\n$5\r\nhello\r\n"
    );
    assert_eq!(
        shard_receiver.try_recv().unwrap(),
        b"*3\r\n$8\r\nsmessage\r\n$5\r\nshard\r\n$5\r\nhello\r\n"
    );
    store.pubsub_remove_client(normal.id());
    store.pubsub_remove_client(shard.id());
}

#[test]
fn pubsub_resp3_uses_push_frames_and_allows_normal_commands() {
    let store = Store::new(1024);
    let mut subscriber = Session::new();
    let (sender, receiver) = std::sync::mpsc::sync_channel(4);
    subscriber.set_pubsub_sender(sender);
    assert!(call(&mut subscriber, &store, &[b"HELLO", b"3"]).starts_with(b"%5\r\n"));
    assert!(
        call(&mut subscriber, &store, &[b"SUBSCRIBE", b"c"])
            .starts_with(b">3\r\n$9\r\nsubscribe\r\n")
    );
    let mut publisher = Session::new();
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBLISH", b"c", b"v"]),
        b":1\r\n"
    );
    assert_eq!(
        receiver.recv().unwrap(),
        b">3\r\n$7\r\nmessage\r\n$1\r\nc\r\n$1\r\nv\r\n"
    );
    assert_eq!(
        call(&mut subscriber, &store, &[b"GET", b"missing"]),
        b"_\r\n"
    );
    store.pubsub_remove_client(subscriber.id());
}

#[test]
fn acl_channel_patterns_gate_publish_subscribe_and_pattern_subscribe() {
    let store = Store::new(1024);
    let mut admin = Session::new();
    assert_eq!(
        call(
            &mut admin,
            &store,
            &[
                b"ACL",
                b"SETUSER",
                b"alice",
                b"on",
                b">pw",
                b"allkeys",
                b"allchannels",
                b"-@all",
                b"+@pubsub",
                b"resetchannels",
                b"&allowed:*"
            ]
        ),
        b"+OK\r\n"
    );
    let mut alice = Session::new();
    alice.set_pubsub_sender(std::sync::mpsc::sync_channel(4).0);
    assert_eq!(
        call(&mut alice, &store, &[b"AUTH", b"alice", b"pw"]),
        b"+OK\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"PUBLISH", b"allowed:news", b"v"]),
        b":0\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"PUBLISH", b"other", b"v"]),
        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"SUBSCRIBE", b"other"]),
        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"SPUBLISH", b"allowed:news", b"v"]),
        b":0\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"SPUBLISH", b"other", b"v"]),
        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"SSUBSCRIBE", b"other"]),
        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"SSUBSCRIBE", b"allowed:news"]),
        b"*3\r\n$10\r\nssubscribe\r\n$12\r\nallowed:news\r\n:1\r\n"
    );
    assert_eq!(
        call(&mut alice, &store, &[b"PSUBSCRIBE", b"allowed:*"]),
        b"*3\r\n$10\r\npsubscribe\r\n$9\r\nallowed:*\r\n:1\r\n"
    );
    store.pubsub_remove_client(alice.id());
}

#[test]
fn reset_discards_transactions_watches_subscriptions_and_resp3() {
    let store = Store::new(1024);
    let mut session = Session::new();
    let (sender, _receiver) = std::sync::mpsc::sync_channel(4);
    session.set_pubsub_sender(sender);
    assert!(call(&mut session, &store, &[b"HELLO", b"3"]).starts_with(b"%5\r\n"));
    assert_eq!(call(&mut session, &store, &[b"WATCH", b"k"]), b"+OK\r\n");
    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"k", b"v"]),
        b"+QUEUED\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"RESET"]), b"+RESET\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"EXEC"]),
        b"-ERR EXEC without MULTI\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"GET", b"missing"]),
        b"$-1\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"PING"]), b"+PONG\r\n");

    assert!(!call(&mut session, &store, &[b"SUBSCRIBE", b"ch"]).is_empty());
    assert!(session.is_subscribed());
    assert_eq!(call(&mut session, &store, &[b"RESET"]), b"+RESET\r\n");
    assert!(!session.is_subscribed());
    assert_eq!(
        call(&mut session, &store, &[b"GET", b"missing"]),
        b"$-1\r\n"
    );
    assert_eq!(
        call(&mut session, &store, &[b"RESET", b"extra"]),
        b"-ERR wrong number of arguments for 'reset' command\r\n"
    );
}

#[test]
fn reset_clears_the_registered_blocked_flag() {
    let store = Store::new(1024);
    let mut session = Session::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (control, _) = listener.accept().unwrap();
    let peer_addr = control.peer_addr().unwrap();
    let local_addr = control.local_addr().unwrap();
    store.register_client(session.id(), peer_addr, local_addr, control);
    drop(client);
    session.blocked = Some(Blocked {
        keys: vec![b"queue".to_vec()],
        deadline: None,
        kind: BlockedKind::List { left: true },
        slowlog: None,
    });
    session.sync_blocked(&store);
    assert_eq!(store.blocked_clients(), 1);

    assert_eq!(call(&mut session, &store, &[b"RESET"]), b"+RESET\r\n");
    assert_eq!(store.blocked_clients(), 0);
    assert!(!store.client_snapshot(session.id()).unwrap().blocked);
    store.unregister_client(session.id());
}

#[test]
fn reset_deauthenticates_an_authenticated_connection() {
    let store = Store::with_auth(1024, crate::engine::Auth::from_password(Some(b"pw")));
    let mut session = Session::new();
    assert_eq!(call(&mut session, &store, &[b"AUTH", b"pw"]), b"+OK\r\n");
    assert_eq!(call(&mut session, &store, &[b"PING"]), b"+PONG\r\n");
    assert_eq!(call(&mut session, &store, &[b"RESET"]), b"+RESET\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"PING"]),
        b"-NOAUTH Authentication required.\r\n"
    );
    assert_eq!(call(&mut session, &store, &[b"AUTH", b"pw"]), b"+OK\r\n");
    assert_eq!(call(&mut session, &store, &[b"PING"]), b"+PONG\r\n");
}

#[test]
fn resp3_exec_abort_is_a_null_value_and_quit_discards_the_queue() {
    let store = Store::new(1024);
    let mut session = Session::new();
    let mut other = Session::new();
    assert!(call(&mut session, &store, &[b"HELLO", b"3"]).starts_with(b"%5\r\n"));
    assert_eq!(call(&mut session, &store, &[b"WATCH", b"k"]), b"+OK\r\n");
    assert_eq!(call(&mut other, &store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"+QUEUED\r\n");
    assert_eq!(call(&mut session, &store, &[b"EXEC"]), b"_\r\n");

    assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"SET", b"discarded", b"v"]),
        b"+QUEUED\r\n"
    );
    let mut output = Vec::new();
    assert_eq!(
        session.execute(&store, &[b"QUIT"], &mut output),
        Action::Close
    );
    assert_eq!(output, b"+OK\r\n");
    assert_eq!(
        call(&mut session, &store, &[b"GET", b"discarded"]),
        b"_\r\n"
    );
}
#[test]
fn sharded_pubsub_is_separate_bounded_and_reset_cleans_it() {
    let store = Store::new(1024);
    let (sender, receiver) = std::sync::mpsc::sync_channel(8);
    let mut subscriber = Session::new();
    subscriber.set_pubsub_sender(sender);
    assert_eq!(
        call(&mut subscriber, &store, &[b"SSUBSCRIBE", b"a", b"b"]),
        b"*3\r\n$10\r\nssubscribe\r\n$1\r\na\r\n:1\r\n*3\r\n$10\r\nssubscribe\r\n$1\r\nb\r\n:2\r\n"
    );
    assert!(subscriber.is_subscribed());
    assert_eq!(
            call(&mut subscriber, &store, &[b"GET", b"k"]),
            b"-ERR Can't execute 'get': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n"
        );

    let mut publisher = Session::new();
    assert_eq!(
        call(&mut publisher, &store, &[b"SPUBLISH", b"a", b"hi"]),
        b":1\r\n"
    );
    assert_eq!(
        receiver.try_recv().unwrap(),
        b"*3\r\n$8\r\nsmessage\r\n$1\r\na\r\n$2\r\nhi\r\n"
    );
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBLISH", b"a", b"normal"]),
        b":0\r\n"
    );
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBSUB", b"SHARDCHANNELS"]),
        b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
    );
    assert_eq!(
        call(
            &mut publisher,
            &store,
            &[b"PUBSUB", b"SHARDCHANNELS", b"a*"]
        ),
        b"*1\r\n$1\r\na\r\n"
    );
    assert_eq!(
        call(
            &mut publisher,
            &store,
            &[b"PUBSUB", b"SHARDNUMSUB", b"a", b"b", b"c"]
        ),
        b"*6\r\n$1\r\na\r\n:1\r\n$1\r\nb\r\n:1\r\n$1\r\nc\r\n:0\r\n"
    );
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBSUB", b"CHANNELS"]),
        b"*0\r\n"
    );

    let mut normal = Session::new();
    let (normal_sender, normal_receiver) = std::sync::mpsc::sync_channel(8);
    normal.set_pubsub_sender(normal_sender);
    assert_eq!(
        call(&mut normal, &store, &[b"SUBSCRIBE", b"a"]),
        b"*3\r\n$9\r\nsubscribe\r\n$1\r\na\r\n:1\r\n"
    );
    assert_eq!(
        call(&mut publisher, &store, &[b"SPUBLISH", b"a", b"shard"]),
        b":1\r\n"
    );
    assert_eq!(
        call(&mut publisher, &store, &[b"PUBLISH", b"a", b"normal"]),
        b":1\r\n"
    );
    assert_eq!(
        normal_receiver.try_recv().unwrap(),
        b"*3\r\n$7\r\nmessage\r\n$1\r\na\r\n$6\r\nnormal\r\n"
    );

    assert_eq!(
        call(&mut subscriber, &store, &[b"SUNSUBSCRIBE", b"a"]),
        b"*3\r\n$12\r\nsunsubscribe\r\n$1\r\na\r\n:1\r\n"
    );
    assert_eq!(
        call(&mut subscriber, &store, &[b"SUNSUBSCRIBE"]),
        b"*3\r\n$12\r\nsunsubscribe\r\n$1\r\nb\r\n:0\r\n"
    );
    assert_eq!(call(&mut subscriber, &store, &[b"RESET"]), b"+RESET\r\n");
    assert!(!subscriber.is_subscribed());
    assert_eq!(
        call(
            &mut publisher,
            &store,
            &[b"PUBSUB", b"SHARDNUMSUB", b"a", b"b"]
        ),
        b"*4\r\n$1\r\na\r\n:0\r\n$1\r\nb\r\n:0\r\n"
    );
}
#[test]
fn sharded_pubsub_resp3_uses_push_frames() {
    let store = Store::new(1024);
    let (sender, receiver) = std::sync::mpsc::sync_channel(8);
    let mut subscriber = Session::new();
    subscriber.set_pubsub_sender(sender);
    assert!(call(&mut subscriber, &store, &[b"HELLO", b"3"]).starts_with(b"%5\r\n"));
    assert_eq!(
        call(&mut subscriber, &store, &[b"SSUBSCRIBE", b"a"]),
        b">3\r\n$10\r\nssubscribe\r\n$1\r\na\r\n:1\r\n"
    );
    let mut publisher = Session::new();
    assert_eq!(
        call(&mut publisher, &store, &[b"SPUBLISH", b"a", b"m"]),
        b":1\r\n"
    );
    assert_eq!(
        receiver.try_recv().unwrap(),
        b">3\r\n$8\r\nsmessage\r\n$1\r\na\r\n$1\r\nm\r\n"
    );
}
#[test]
fn expired_and_evicted_notifications_use_injected_clock_and_eviction_policy() {
    struct TestClock(std::sync::atomic::AtomicI64);
    impl crate::engine::Clock for TestClock {
        fn now_ms(&self) -> i64 {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
    }
    let clock = std::sync::Arc::new(TestClock(std::sync::atomic::AtomicI64::new(1000)));
    let store = Store::with_clock(10, clock.clone());
    store.set_notify_flags(crate::engine::NotifyFlags::parse(b"KEA").unwrap());
    let (sender, receiver) = std::sync::mpsc::sync_channel(16);
    let mut subscriber = Session::new();
    subscriber.set_pubsub_sender(sender);
    assert_eq!(
        call(
            &mut subscriber,
            &store,
            &[b"PSUBSCRIBE", b"__keyevent@0__:*"]
        ),
        b"*3\r\n$10\r\npsubscribe\r\n$16\r\n__keyevent@0__:*\r\n:1\r\n"
    );
    let mut writer = Session::new();
    assert_eq!(
        call(&mut writer, &store, &[b"SET", b"k", b"v", b"PX", b"10"]),
        b"+OK\r\n"
    );
    assert!(receiver.try_recv().is_ok(), "SET event");
    clock.0.store(1011, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(call(&mut writer, &store, &[b"GET", b"k"]), b"$-1\r\n");
    assert_eq!(
            receiver.try_recv().unwrap(),
            b"*4\r\n$8\r\npmessage\r\n$16\r\n__keyevent@0__:*\r\n$22\r\n__keyevent@0__:expired\r\n$1\r\nk\r\n"
        );

    store.set_eviction_policy(crate::engine::EvictionPolicy::AllKeysRandom);
    assert_eq!(
        call(&mut writer, &store, &[b"SET", b"victim", b"123"]),
        b"+OK\r\n"
    );
    assert!(receiver.try_recv().is_ok(), "victim SET event");
    assert_eq!(
        call(&mut writer, &store, &[b"SET", b"new", b"123"]),
        b"+OK\r\n"
    );
    assert_eq!(
            receiver.try_recv().unwrap(),
            b"*4\r\n$8\r\npmessage\r\n$16\r\n__keyevent@0__:*\r\n$22\r\n__keyevent@0__:evicted\r\n$6\r\nvictim\r\n"
        );
    assert_eq!(
            receiver.try_recv().unwrap(),
            b"*4\r\n$8\r\npmessage\r\n$16\r\n__keyevent@0__:*\r\n$18\r\n__keyevent@0__:set\r\n$3\r\nnew\r\n"
        );

    assert_eq!(
        call(&mut writer, &store, &[b"SET", b"p", b"v", b"PX", b"10"]),
        b"+OK\r\n"
    );
    assert!(receiver.try_recv().is_ok(), "p SET event");
    clock.0.store(1022, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(store.purge_expired(16), 1);
    assert_eq!(
            receiver.try_recv().unwrap(),
            b"*4\r\n$8\r\npmessage\r\n$16\r\n__keyevent@0__:*\r\n$22\r\n__keyevent@0__:expired\r\n$1\r\np\r\n"
        );

    store.set_eviction_policy(crate::engine::EvictionPolicy::NoEviction);
    let reply = call(
        &mut writer,
        &store,
        &[b"SET", b"big", b"01234567890123456789"],
    );
    assert!(reply.starts_with(b"-OOM"), "{reply:?}");
    assert!(receiver.try_recv().is_err(), "no event for rejected write");
}
