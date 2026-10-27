#![forbid(unsafe_code)]

use rillcache::engine::Store;
use rillcache::session::Session;

#[test]
fn exec_enabling_statistics_is_counted_without_individual_queued_calls() {
    let store = Store::new(4096);
    let mut session = Session::new();
    let mut call = |args: &[&[u8]]| {
        let mut output = Vec::new();
        session.execute(&store, args, &mut output);
        output
    };
    assert_eq!(
        call(&[b"CONFIG", b"SET", b"slowlog-log-slower-than", b"-1"]),
        b"+OK\r\n"
    );
    assert_eq!(call(&[b"MULTI"]), b"+OK\r\n");
    assert_eq!(
        call(&[b"CONFIG", b"SET", b"commandstats-enabled", b"yes"]),
        b"+QUEUED\r\n"
    );
    assert_eq!(call(&[b"PING"]), b"+QUEUED\r\n");
    assert!(!store.commandstats_enabled());
    assert_eq!(call(&[b"EXEC"]), b"*2\r\n+OK\r\n+PONG\r\n");
    let info = String::from_utf8(call(&[b"INFO", b"commandstats"])).unwrap();
    assert!(info.contains("cmdstat_exec:calls=1,"), "{info}");
    assert!(!info.contains("cmdstat_config:"), "{info}");
    assert!(!info.contains("cmdstat_ping:"), "{info}");
}
