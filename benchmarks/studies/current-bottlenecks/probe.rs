#![forbid(unsafe_code)]
//! In-process diagnostics only: not TCP throughput or merge-gate evidence.
use rillcache::engine::{Action, Store};
use std::hint::black_box;
use std::time::Instant;

const WORKLOADS: &[&str] = &[
    "ping",
    "hget",
    "hset",
    "hlen",
    "scard",
    "info-server",
    "memory-stats",
];

fn call(store: &Store, args: &[&[u8]]) -> Vec<u8> {
    let mut reply = Vec::new();
    assert_eq!(store.execute(args, &mut reply), Action::Continue);
    reply
}

fn bulk(reply: &[u8]) -> &[u8] {
    assert_eq!(reply.first(), Some(&b'$'));
    let end = reply.windows(2).position(|p| p == b"\r\n").unwrap();
    let length: usize = std::str::from_utf8(&reply[1..end])
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(reply.len(), end + 2 + length + 2);
    assert!(reply.ends_with(b"\r\n"));
    &reply[end + 2..end + 2 + length]
}

fn prepare(workload: &str, size: usize) -> Store {
    let store = Store::new(64 * 1024 * 1024);
    let names: Vec<_> = (0..size)
        .map(|i| format!("member:{i:08}").into_bytes())
        .collect();
    let value = [b'x'; 64];
    if matches!(workload, "hget" | "hset" | "hlen") {
        for chunk in names.chunks(256) {
            let mut args: Vec<&[u8]> = vec![b"HSET", b"hash"];
            for name in chunk {
                args.extend([name.as_slice(), value.as_slice()]);
            }
            assert_eq!(
                call(&store, &args),
                format!(":{}\r\n", chunk.len()).as_bytes()
            );
        }
        assert_eq!(
            call(&store, &[b"HLEN", b"hash"]),
            format!(":{size}\r\n").as_bytes()
        );
        assert_eq!(
            bulk(&call(&store, &[b"HGET", b"hash", b"member:00000000"])),
            value
        );
    } else if workload == "scard" {
        for chunk in names.chunks(512) {
            let mut args: Vec<&[u8]> = vec![b"SADD", b"set"];
            args.extend(chunk.iter().map(Vec::as_slice));
            assert_eq!(
                call(&store, &args),
                format!(":{}\r\n", chunk.len()).as_bytes()
            );
        }
        assert_eq!(
            call(&store, &[b"SCARD", b"set"]),
            format!(":{size}\r\n").as_bytes()
        );
    } else {
        for name in names {
            assert_eq!(call(&store, &[b"SET", &name, &value]), b"+OK\r\n");
        }
        assert_eq!(
            call(&store, &[b"DBSIZE"]),
            format!(":{size}\r\n").as_bytes()
        );
    }
    store
}

fn arguments<'a>(workload: &str, value: &'a [u8]) -> Vec<&'a [u8]> {
    match workload {
        "ping" => vec![b"PING"],
        "hget" => vec![b"HGET", b"hash", b"member:00000000"],
        "hset" => vec![b"HSET", b"hash", b"member:00000000", value],
        "hlen" => vec![b"HLEN", b"hash"],
        "scard" => vec![b"SCARD", b"set"],
        "info-server" => vec![b"INFO", b"server"],
        "memory-stats" => vec![b"MEMORY", b"STATS"],
        _ => panic!("unsupported workload"),
    }
}

fn validate(workload: &str, size: usize, reply: &[u8], value: &[u8]) {
    match workload {
        "ping" => assert_eq!(reply, b"+PONG\r\n"),
        "hget" => assert_eq!(bulk(reply), value),
        "hset" => assert_eq!(reply, b":0\r\n"),
        "hlen" | "scard" => assert_eq!(reply, format!(":{size}\r\n").as_bytes()),
        "info-server" => {
            let text = bulk(reply);
            assert!(text.starts_with(b"# Server\r\n"));
            assert!(
                text.windows(b"rillcache_version:".len())
                    .any(|p| p == b"rillcache_version:")
            );
            assert!(!text.windows(10).any(|p| p == b"# Keyspace"));
        }
        "memory-stats" => {
            assert!(reply.starts_with(b"*14\r\n"));
            let expected = format!("$10\r\nkeys.count\r\n:{size}\r\n");
            assert!(
                reply
                    .windows(expected.len())
                    .any(|p| p == expected.as_bytes())
            );
            assert!(
                reply
                    .windows(b"key_and_value_bytes_not_RSS".len())
                    .any(|p| p == b"key_and_value_bytes_not_RSS")
            );
            assert!(reply.ends_with(b"\r\n"));
        }
        _ => panic!("unsupported workload"),
    }
}

fn percentile(samples: &[u128], percent: usize) -> u128 {
    samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)]
}

fn run(workload: &str, size: usize, operations: usize) {
    let store = prepare(workload, size);
    let x = [b'x'; 64];
    let y = [b'y'; 64];
    let args_x = arguments(workload, &x);
    let args_y = arguments(workload, &y);
    let mut reply = Vec::with_capacity(1024);
    // Warmup has the same operation/reply checks but is outside measurement.
    for _ in 0..16 {
        reply.clear();
        store.execute(&args_x, &mut reply);
        validate(workload, size, &reply, &x);
    }
    let mut samples = Vec::with_capacity(operations);
    let begin = Instant::now();
    for index in 0..operations {
        // HSET alternates equal-width DIFFERENT bytes; it is not a no-op probe.
        let (args, expected) = if workload == "hset" && index % 2 == 0 {
            (&args_y, &y)
        } else {
            (&args_x, &x)
        };
        reply.clear();
        let started = Instant::now();
        assert_eq!(store.execute(black_box(args), &mut reply), Action::Continue);
        samples.push(started.elapsed().as_nanos());
        validate(workload, size, black_box(&reply), expected);
    }
    let elapsed = begin.elapsed().as_nanos();
    if matches!(workload, "hget" | "hset" | "hlen") {
        assert_eq!(
            call(&store, &[b"HLEN", b"hash"]),
            format!(":{size}\r\n").as_bytes()
        );
    }
    if workload == "hset" {
        let expected = if operations % 2 == 1 { y } else { x };
        assert_eq!(
            bulk(&call(&store, &[b"HGET", b"hash", b"member:00000000"])),
            expected
        );
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"in-process Store; not TCP/gate\",\"workload\":\"{workload}\",\"size\":{size},\"operations\":{operations},\"elapsed_ns\":{elapsed},\"handler_p50_ns\":{},\"handler_p99_ns\":{},\"handler_max_ns\":{},\"replies_validated\":true}}",
        percentile(&samples, 50),
        percentile(&samples, 99),
        samples.last().unwrap()
    );
}

fn parse(args: &[String]) -> Result<(&str, usize, usize), &'static str> {
    if args.len() != 3 || !WORKLOADS.contains(&args[0].as_str()) {
        return Err("expected WORKLOAD SIZE OPERATIONS; see probe.rs WORKLOADS");
    }
    let size = args[1].parse().map_err(|_| "invalid size")?;
    let operations = args[2].parse().map_err(|_| "invalid operation count")?;
    if !(1..=32768).contains(&size) || !(1..=1000000).contains(&operations) {
        return Err("size must be 1..32768; operations 1..1000000");
    }
    Ok((&args[0], size, operations))
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match parse(&args) {
        Ok((workload, size, operations)) => run(workload, size, operations),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_zero_and_unbounded_inputs() {
        for args in [
            vec!["unknown", "1", "1"],
            vec!["ping", "0", "1"],
            vec!["ping", "32769", "1"],
            vec!["ping", "1", "0"],
            vec!["ping", "1", "1000001"],
            vec!["ping", "1"],
        ] {
            assert!(parse(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).is_err());
        }
    }

    #[test]
    fn every_probe_validates_real_replies_and_constant_cardinality() {
        for workload in WORKLOADS {
            run(workload, 4, 3);
        }
    }

    #[test]
    fn handler_percentiles_use_nearest_rank() {
        assert_eq!(percentile(&[1, 2, 3, 4], 50), 2);
        assert_eq!(percentile(&[1, 2, 3, 4], 99), 4);
    }

    #[test]
    #[should_panic]
    fn malformed_bulk_reply_cannot_be_timing_evidence() {
        bulk(b"$2\r\nx\r\n");
    }
}
