use super::*;
use std::sync::atomic::{AtomicI64, Ordering};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn setup(global: u32, per_user: u32) -> (Store, Arc<TestClock>) {
    let clock = Arc::new(TestClock(AtomicI64::new(1_000)));
    let store = Store::with_clock(1024, clock.clone());
    store.set_command_budget(global, per_user);
    (store, clock)
}

#[test]
fn disabled_budgets_never_reject_and_never_read_the_clock() {
    let (store, clock) = setup(0, 0);
    clock.0.store(0, Ordering::SeqCst);
    for _ in 0..10_000 {
        assert_eq!(store.admit_command(None), CommandAdmission::Allowed);
        assert_eq!(
            store.admit_command(Some(b"alice")),
            CommandAdmission::Allowed
        );
    }
}

#[test]
fn global_budget_sheds_within_the_window_and_recovers_at_the_next_second() {
    let (store, clock) = setup(2, 0);
    assert_eq!(store.admit_command(None), CommandAdmission::Allowed);
    assert_eq!(store.admit_command(None), CommandAdmission::Allowed);
    assert_eq!(store.admit_command(None), CommandAdmission::GlobalExceeded);
    // Still the same wall-clock second: the window does not slide on demand.
    clock.0.store(1_999, Ordering::SeqCst);
    assert_eq!(store.admit_command(None), CommandAdmission::GlobalExceeded);
    clock.0.store(2_000, Ordering::SeqCst);
    assert_eq!(store.admit_command(None), CommandAdmission::Allowed);
    assert_eq!(store.admit_command(None), CommandAdmission::Allowed);
    assert_eq!(store.admit_command(None), CommandAdmission::GlobalExceeded);
}

#[test]
fn per_user_budget_isolates_identities_and_shares_the_global_window() {
    let (store, _) = setup(3, 2);
    assert_eq!(
        store.admit_command(Some(b"alice")),
        CommandAdmission::Allowed
    );
    assert_eq!(
        store.admit_command(Some(b"alice")),
        CommandAdmission::Allowed
    );
    // Alice's own allowance is gone; the rejected call costs no global slot.
    assert_eq!(
        store.admit_command(Some(b"alice")),
        CommandAdmission::UserExceeded
    );
    // Another user's identity has an independent allowance in the same second.
    assert_eq!(store.admit_command(Some(b"bob")), CommandAdmission::Allowed);
    // The global window now holds the three admitted commands.
    assert_eq!(
        store.admit_command(None),
        CommandAdmission::GlobalExceeded,
        "the global window counts every admitted identity"
    );
}

#[test]
fn global_exhaustion_is_reported_before_a_fresh_user_allowance() {
    let (store, _) = setup(1, 1);
    assert_eq!(
        store.admit_command(Some(b"alice")),
        CommandAdmission::Allowed
    );
    assert_eq!(
        store.admit_command(Some(b"alice")),
        CommandAdmission::UserExceeded
    );
    // Bob has user headroom but the global window is already full.
    assert_eq!(
        store.admit_command(Some(b"bob")),
        CommandAdmission::GlobalExceeded
    );
}

#[test]
fn rejection_counter_is_monotonic_and_resettable() {
    let (store, _) = setup(1, 0);
    assert_eq!(store.admit_command(None), CommandAdmission::Allowed);
    assert_eq!(store.admit_command(None), CommandAdmission::GlobalExceeded);
    store.record_command_budget_rejection();
    store.record_command_budget_rejection();
    assert_eq!(store.stats().command_budget_rejections, 2);
    assert!(store.reset_stats());
    assert_eq!(store.stats().command_budget_rejections, 0);
}
