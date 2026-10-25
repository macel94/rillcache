#![forbid(unsafe_code)]

use std::hint::black_box;
use std::time::Instant;

use rillcache::engine::Store;
use rillcache::protocol::{Limits, dispatch_request, encode_bulk, encode_request, parse_request};

fn measure(name: &str, mut operation: impl FnMut()) {
    const ITERATIONS: usize = 50_000;
    let mut samples = Vec::new();
    for _ in 0..1000 {
        operation();
    }
    for _ in 0..7 {
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            operation();
        }
        samples.push(start.elapsed().as_nanos() as f64 / ITERATIONS as f64);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{{\"benchmark\":\"{name}\",\"iterations\":{ITERATIONS},\"samples\":7,\"median_ns_per_op\":{:.2}}}",
        samples[3]
    );
}

fn measure_store(name: &str, store: &Store, arguments: &[&[u8]], expected: &[u8]) {
    let mut output = Vec::new();
    assert_eq!(
        store.execute(arguments, &mut output),
        rillcache::engine::Action::Continue
    );
    assert_eq!(output, expected, "{name}: invalid benchmark setup");
    measure(name, || {
        output.clear();
        black_box(store.execute(black_box(arguments), &mut output));
        black_box(output.as_slice());
    });
    assert_eq!(
        output, expected,
        "{name}: benchmark returned an incorrect reply"
    );
}

fn main() {
    // Cargo may invoke a harness-free bench in test mode: keep the fast test loop fast.
    if std::env::args().any(|argument| argument == "--test") {
        return;
    }
    let mut request = Vec::new();
    encode_request(&[b"SET", b"bench:key", &[b'x'; 64]], &mut request);
    measure("resp2_parse_set_64b", || {
        black_box(
            parse_request(black_box(&request), Limits::default())
                .unwrap()
                .unwrap(),
        );
    });
    measure("resp2_dispatch_set_64b", || {
        black_box(
            dispatch_request(black_box(&request), Limits::default(), |args| {
                black_box(args);
            })
            .unwrap()
            .unwrap(),
        );
    });
    let store = Store::new(1024);
    let mut output = Vec::new();
    store.execute(&[b"SET", b"bench:key", &[b'x'; 64]], &mut output);
    assert_eq!(output, b"+OK\r\n");
    let mut bulk = Vec::new();
    rillcache::protocol::encode_bulk(Some(&[b'x'; 64]), &mut bulk);
    measure_store("store_ping", &store, &[b"PING"], b"+PONG\r\n");
    measure("resp2_encode_bulk_64b", || {
        output.clear();
        encode_bulk(Some(black_box(&[b'x'; 64])), &mut output);
        black_box(output.as_slice());
    });
    assert_eq!(output, bulk);
    measure_store("store_get_64b", &store, &[b"GET", b"bench:key"], &bulk);
    measure_store(
        "store_set_64b",
        &store,
        &[b"SET", b"bench:key", &[b'x'; 64]],
        b"+OK\r\n",
    );
    measure_store(
        "store_set_nx_existing_64b",
        &store,
        &[b"SET", b"bench:key", &[b'x'; 64], b"NX"],
        b"$-1\r\n",
    );
    measure_store(
        "store_set_xx_existing_64b",
        &store,
        &[b"SET", b"bench:key", &[b'x'; 64], b"XX"],
        b"+OK\r\n",
    );
    measure_store(
        "store_set_xx_missing_64b",
        &store,
        &[b"SET", b"missing", &[b'x'; 64], b"XX"],
        b"$-1\r\n",
    );
}
