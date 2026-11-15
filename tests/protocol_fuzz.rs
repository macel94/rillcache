#![forbid(unsafe_code)]

//! Deterministic RESP parser/property corpus. No external fuzzing dependency:
//! a small xorshift generator reproduces every case from the seed printed in a
//! failure message.

use rillcache::protocol::{Limits, dispatch_request, encode_request, parse_request};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, bound: usize) -> usize {
        assert!(bound > 0);
        (self.next() % bound as u64) as usize
    }

    fn bytes(&mut self, length: usize) -> Vec<u8> {
        (0..length).map(|_| (self.next() & 0xff) as u8).collect()
    }
}

fn limits_for(case: usize) -> Limits {
    const FRAMES: [usize; 5] = [14, 64, 256, 2048, 2 * 1024 * 1024];
    const BULKS: [usize; 5] = [4, 16, 64, 512, 1024 * 1024];
    const ARGUMENTS: [usize; 5] = [1, 2, 8, 64, 1024];
    Limits {
        max_frame_bytes: FRAMES[case % FRAMES.len()],
        max_bulk_bytes: BULKS[case % BULKS.len()],
        max_arguments: ARGUMENTS[case % ARGUMENTS.len()],
    }
}

#[test]
fn valid_binary_frames_round_trip_at_every_fragmentation_boundary() {
    let mut rng = Rng(0x5eed_1a2b_3c4d_5e6f);
    for case in 0..250 {
        let count = 1 + rng.below(8);
        let mut arguments = Vec::new();
        for index in 0..count {
            let length = if index == 0 {
                1 + rng.below(16)
            } else {
                rng.below(64)
            };
            arguments.push(rng.bytes(length));
        }
        let slices: Vec<&[u8]> = arguments.iter().map(Vec::as_slice).collect();
        let mut frame = Vec::new();
        encode_request(&slices, &mut frame);

        let parsed = parse_request(&frame, Limits::default())
            .unwrap_or_else(|error| panic!("seed {:#x} case {case}: {error}", rng.0))
            .unwrap_or_else(|| {
                panic!(
                    "seed {:#x} case {case}: complete frame was incomplete",
                    rng.0
                )
            });
        assert_eq!(parsed.arguments, slices, "case {case}");
        assert_eq!(parsed.consumed, frame.len(), "case {case}");

        for split in 0..frame.len() {
            assert!(
                parse_request(&frame[..split], Limits::default())
                    .expect("prefix must parse")
                    .is_none(),
                "seed {:#x} case {case}: prefix {split} executed or errored",
                rng.0
            );
        }

        let mut calls = 0;
        let (executed, consumed) = dispatch_request(&frame, Limits::default(), |received| {
            calls += 1;
            received
                .iter()
                .map(|argument| argument.to_vec())
                .collect::<Vec<_>>()
        })
        .expect("complete dispatch")
        .expect("complete frame must dispatch");
        assert_eq!(calls, 1, "case {case}");
        assert_eq!(executed, arguments, "case {case}");
        assert_eq!(consumed, frame.len(), "case {case}");

        for split in [0, frame.len() / 2, frame.len() - 1] {
            let mut calls = 0;
            assert!(
                dispatch_request(&frame[..split], Limits::default(), |_| calls += 1)
                    .expect("prefix dispatch")
                    .is_none(),
                "case {case} split {split}"
            );
            assert_eq!(calls, 0, "case {case} split {split}");
        }
    }
}

#[test]
fn random_bytes_and_tight_limits_never_panic_or_overrun() {
    let mut rng = Rng(0x00c0_ffee_1234_5678);
    for case in 0..8000 {
        let length = rng.below(257);
        let bytes = rng.bytes(length);
        let limits = limits_for(case);

        let parsed = parse_request(&bytes, limits);
        if let Ok(Some(request)) = parsed {
            assert!(request.consumed > 0, "seed {:#x} case {case}", rng.0);
            assert!(
                request.consumed <= bytes.len(),
                "seed {:#x} case {case}",
                rng.0
            );
            assert!(
                request.arguments.len() <= limits.max_arguments,
                "seed {:#x} case {case}",
                rng.0
            );
            for argument in &request.arguments {
                assert!(
                    argument.len() <= limits.max_bulk_bytes,
                    "seed {:#x} case {case}",
                    rng.0
                );
            }
        }

        let mut calls = 0;
        let dispatched = dispatch_request(&bytes, limits, |arguments| {
            calls += 1;
            assert!(
                arguments.len() <= limits.max_arguments,
                "seed {:#x} case {case}",
                rng.0
            );
            for argument in arguments {
                assert!(
                    argument.len() <= limits.max_bulk_bytes,
                    "seed {:#x} case {case}",
                    rng.0
                );
            }
        });
        assert!(calls <= 1, "seed {:#x} case {case}", rng.0);
        if let Ok(Some(((), consumed))) = dispatched {
            assert!(consumed > 0, "seed {:#x} case {case}", rng.0);
            assert!(consumed <= bytes.len(), "seed {:#x} case {case}", rng.0);
        }
    }
}

#[test]
fn random_inline_bytes_are_bounded_and_execute_at_most_once() {
    let mut rng = Rng(0xdead_beef_0bad_f00d);
    for case in 0..3000 {
        let length = rng.below(128);
        let mut bytes = rng.bytes(length);
        if rng.next() & 1 == 0 {
            bytes.push(b'\n');
        }
        let limits = Limits {
            max_frame_bytes: 256,
            max_bulk_bytes: 64,
            max_arguments: 8,
        };
        let mut calls = 0;
        let result = dispatch_request(&bytes, limits, |arguments| {
            calls += 1;
            assert!(
                arguments.len() <= limits.max_arguments,
                "seed {:#x} case {case}",
                rng.0
            );
        });
        assert!(calls <= 1, "seed {:#x} case {case}", rng.0);
        if let Ok(Some(((), consumed))) = result {
            assert!(consumed > 0 && consumed <= bytes.len(), "case {case}");
        }
    }
}

#[test]
fn pipelined_frames_consume_exactly_one_command_at_a_time() {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    for case in 0..100 {
        let mut expected: Vec<Vec<Vec<u8>>> = Vec::new();
        let mut wire = Vec::new();
        for _ in 0..1 + rng.below(4) {
            let count = 1 + rng.below(6);
            let mut arguments = Vec::new();
            for index in 0..count {
                let length = if index == 0 {
                    1 + rng.below(8)
                } else {
                    rng.below(32)
                };
                arguments.push(rng.bytes(length));
            }
            let slices: Vec<&[u8]> = arguments.iter().map(Vec::as_slice).collect();
            encode_request(&slices, &mut wire);
            expected.push(arguments);
        }

        let mut offset = 0;
        let mut parsed_arguments = 0;
        while offset < wire.len() {
            let request = parse_request(&wire[offset..], Limits::default())
                .expect("pipeline parse")
                .unwrap_or_else(|| panic!("seed {:#x} case {case}: incomplete pipeline", rng.0));
            let decoded: Vec<Vec<u8>> = request
                .arguments
                .iter()
                .map(|argument| argument.to_vec())
                .collect();
            assert_eq!(
                decoded, expected[parsed_arguments],
                "seed {:#x} case {case}",
                rng.0
            );
            parsed_arguments += 1;
            offset += request.consumed;
            assert!(offset <= wire.len(), "case {case}");
        }
        assert_eq!(offset, wire.len(), "case {case}");
    }
}
