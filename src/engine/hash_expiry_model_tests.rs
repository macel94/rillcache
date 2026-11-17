use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

struct ModelClock(AtomicI64);

impl Clock for ModelClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

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

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

#[derive(Clone)]
struct Field {
    value: String,
    deadline: Option<i64>,
}

fn exec(store: &ShardExecutor, args: &[Vec<u8>]) -> Vec<u8> {
    let borrowed: Vec<&[u8]> = args.iter().map(Vec::as_slice).collect();
    let mut output = Vec::new();
    assert_eq!(store.execute(&borrowed, &mut output), Action::Continue);
    output
}

fn integer_reply(value: i64) -> Vec<u8> {
    format!(":{value}\r\n").into_bytes()
}

fn bulk_reply(value: Option<&str>) -> Vec<u8> {
    match value {
        Some(value) => format!("${}\r\n{value}\r\n", value.len()).into_bytes(),
        None => b"$-1\r\n".to_vec(),
    }
}

// The reference is intentionally a plain map with absolute deadlines; it does
// not inspect Data, collections, TTL maps, or command implementation helpers.
#[test]
fn seeded_hash_field_ttl_sequences_match_independent_map_model() {
    const SEED: u64 = 0x6f31_aa02_7c49_d815;
    const FIELDS: [&[u8]; 5] = [b"a", b"b", b"c", b"d", b"e"];
    let clock = Arc::new(ModelClock(AtomicI64::new(10_000)));
    let store = ShardExecutor::with_clock(4096, clock.clone());
    let mut model: HashMap<String, Field> = HashMap::new();
    let mut rng = Rng(SEED);
    let mut operation_counts = [0usize; 5];

    for step in 0..600 {
        let advance = if step == 8 {
            10
        } else if step < 8 {
            0
        } else {
            rng.below(4) as i64
        };
        let now = clock.0.fetch_add(advance, Ordering::SeqCst) + advance;
        let field_index = match step {
            0 | 2 | 4 | 5 => 0,
            1 | 3 | 6 | 7 | 8 => 1,
            _ => rng.below(FIELDS.len() as u64) as usize,
        };
        let field = FIELDS[field_index];
        let field_name = std::str::from_utf8(field).unwrap().to_owned();
        // All operations observe and lazily remove every field whose deadline
        // has arrived, matching the command-visible hash state.
        model.retain(|_, entry| entry.deadline.is_none_or(|deadline| deadline > now));
        let operation = match step {
            0 | 1 | 4 => 0, // Create two fields, then overwrite only the first.
            2 | 3 => 1,     // Give them distinct deadlines.
            5..=7 => 3,     // Verify the first is persistent and second still expires.
            8 => 2,         // Let the still-expiring second field pass its deadline.
            _ => rng.below(5) as usize,
        };
        operation_counts[operation] += 1;
        let (args, expected) = match operation {
            0 => {
                let value = format!("v{step}");
                let added = i64::from(!model.contains_key(&field_name));
                model.insert(
                    field_name,
                    Field {
                        value: value.clone(),
                        deadline: None,
                    },
                );
                // Keep the owned HSET value alive while forming argument slices.
                (
                    vec![
                        b"HSET".to_vec(),
                        b"h".to_vec(),
                        field.to_vec(),
                        value.into_bytes(),
                    ],
                    integer_reply(added),
                )
            }
            1 => {
                let duration = match step {
                    2 => 5,
                    3 => 10,
                    _ => rng.below(18) as i64,
                };
                let duration_text = duration.to_string();
                let status = if model.contains_key(&field_name) {
                    1
                } else {
                    -2
                };
                if status == 1 {
                    model.get_mut(&field_name).unwrap().deadline = Some(now + duration);
                }
                (
                    vec![
                        b"HPEXPIRE".to_vec(),
                        b"h".to_vec(),
                        duration_text.into_bytes(),
                        b"FIELDS".to_vec(),
                        b"1".to_vec(),
                        field.to_vec(),
                    ],
                    format!("*1\r\n:{status}\r\n").into_bytes(),
                )
            }
            2 => {
                let value = model.get(&field_name).map(|entry| entry.value.as_str());
                (
                    vec![b"HGET".to_vec(), b"h".to_vec(), field.to_vec()],
                    bulk_reply(value),
                )
            }
            3 => {
                let ttl = match model.get(&field_name) {
                    None => -2,
                    Some(entry) => entry
                        .deadline
                        .map_or(-1, |deadline| (deadline - now).max(0)),
                };
                (
                    vec![
                        b"HPTTL".to_vec(),
                        b"h".to_vec(),
                        b"FIELDS".to_vec(),
                        b"1".to_vec(),
                        field.to_vec(),
                    ],
                    format!("*1\r\n:{ttl}\r\n").into_bytes(),
                )
            }
            _ => {
                let removed = i64::from(model.remove(&field_name).is_some());
                (
                    vec![b"HDEL".to_vec(), b"h".to_vec(), field.to_vec()],
                    integer_reply(removed),
                )
            }
        };
        assert_eq!(
            exec(&store, &args),
            expected,
            "seed {SEED:#x}, step {step}, now {now}, operation {operation}"
        );
    }
    assert!(operation_counts.iter().all(|count| *count > 0));
}
