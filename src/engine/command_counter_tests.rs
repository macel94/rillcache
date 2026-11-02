use super::*;

#[test]
fn command_counter_slots_are_separate_in_the_actual_services_allocation() {
    let store = Store::new(1024);
    let counters = &store.commands_processed;
    assert_eq!(std::mem::align_of::<CommandCounter>(), 128);
    assert_eq!(std::mem::size_of::<CommandCounter>(), 128);
    for counter in counters {
        // Address inspection only: no raw pointer dereference or unsafe code.
        let address = std::ptr::from_ref(&counter.0) as usize;
        assert_eq!(address % 128, 0);
    }
    for pair in counters.windows(2) {
        let first = std::ptr::from_ref(&pair[0].0) as usize;
        let second = std::ptr::from_ref(&pair[1].0) as usize;
        assert_eq!(second - first, 128);
    }
}

#[test]
fn concurrent_distinct_and_colliding_clients_preserve_exact_command_totals() {
    let store = Store::new(1024);
    let services = store.services_handle();
    let services = &services;
    const THREADS: usize = 8;
    const COMMANDS: usize = 4096;
    std::thread::scope(|scope| {
        for worker in 0..THREADS {
            scope.spawn(move || {
                for command in 0..COMMANDS {
                    // Exercise every shard as well as collisions between threads
                    // and IDs that wrap around the fixed shard array.
                    let id = (worker + command % COMMAND_COUNTER_SHARDS) as u64;
                    services.record_command(id);
                    services.record_command(id + COMMAND_COUNTER_SHARDS as u64);
                }
            });
        }
    });
    assert_eq!(
        store.stats().total_commands_processed,
        (THREADS * COMMANDS * 2) as u64
    );
    let expected_per_slot = (THREADS * COMMANDS * 2 / COMMAND_COUNTER_SHARDS) as u64;
    for counter in &services.commands_processed {
        assert_eq!(counter.0.load(Ordering::Relaxed), expected_per_slot);
    }
}

#[test]
fn reset_clears_every_command_counter_and_slots_remain_usable() {
    let store = Store::new(1024);
    for id in 0..COMMAND_COUNTER_SHARDS as u64 * 3 {
        store.record_command(id);
    }
    store.record_command(u64::MAX);
    assert_eq!(
        store.stats().total_commands_processed,
        COMMAND_COUNTER_SHARDS as u64 * 3 + 1
    );
    assert!(store.reset_stats());
    assert_eq!(store.stats().total_commands_processed, 0);
    for counter in &store.commands_processed {
        assert_eq!(counter.0.load(Ordering::Relaxed), 0);
    }
    for id in 0..COMMAND_COUNTER_SHARDS as u64 {
        store.record_command(id);
    }
    assert_eq!(
        store.stats().total_commands_processed,
        COMMAND_COUNTER_SHARDS as u64
    );
}
