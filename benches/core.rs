#![forbid(unsafe_code)]

use std::hint::black_box;
use std::time::Instant;

use rillcache::engine::Store;
use rillcache::protocol::{Limits, encode_request, parse_request};

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
    let store = Store::new(1024);
    let mut output = Vec::new();
    store.execute(&[b"SET", b"bench:key", &[b'x'; 64]], &mut output);
    measure("store_get_64b", || {
        output.clear();
        black_box(store.execute(black_box(&[b"GET", b"bench:key"]), &mut output));
        black_box(output.as_slice());
    });
    measure("store_set_64b", || {
        output.clear();
        black_box(store.execute(black_box(&[b"SET", b"bench:key", &[b'x'; 64]]), &mut output));
        black_box(output.as_slice());
    });
}
