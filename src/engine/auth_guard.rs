//! Bounded process-local failed-authentication throttles.
//!
//! The per-source budget tracks peer IPs so reconnecting does not reset a
//! source's budget; the optional aggregate budget spans peers in this process.
//! Neither is distributed or credential-aware. Once the per-IP table is full,
//! new sources share one overflow budget rather than evicting tracked peers.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

const MAX_SOURCES: usize = 4096;

#[derive(Debug)]
struct Source {
    failures: usize,
    window_start: Instant,
    locked_until: Option<Instant>,
}

pub(super) struct AuthGuard {
    limit: usize,
    global_limit: usize,
    window: Duration,
    global: Option<Source>,
    sources: BTreeMap<IpAddr, Source>,
    overflow: Option<Source>,
}

impl AuthGuard {
    pub(super) fn new(limit: usize, global_limit: usize, window_ms: u64) -> Self {
        Self {
            limit,
            global_limit,
            window: Duration::from_millis(window_ms),
            global: None,
            sources: BTreeMap::new(),
            overflow: None,
        }
    }

    pub(super) fn configure(&mut self, limit: usize, global_limit: usize, window_ms: u64) {
        self.limit = limit;
        self.global_limit = global_limit;
        self.window = Duration::from_millis(window_ms);
        if limit == 0 {
            self.sources.clear();
            self.overflow = None;
        }
        if global_limit == 0 {
            self.global = None;
        }
    }

    pub(super) fn locked(&mut self, source: IpAddr) -> bool {
        if self.limit == 0 && self.global_limit == 0 {
            return false;
        }
        let now = Instant::now();
        if self.global_limit > 0
            && self
                .global
                .as_ref()
                .is_some_and(|entry| entry.locked_until.is_some_and(|until| until > now))
        {
            return true;
        }
        if self.limit == 0 {
            return false;
        }
        if let Some(entry) = self.sources.get(&source) {
            if entry.locked_until.is_some_and(|until| until > now) {
                return true;
            }
            if entry.locked_until.is_none() && now.duration_since(entry.window_start) < self.window
            {
                return false;
            }
            self.sources.remove(&source);
            return false;
        }
        self.overflow
            .as_ref()
            .is_some_and(|entry| entry.locked_until.is_some_and(|until| until > now))
    }

    pub(super) fn note_failure(&mut self, source: IpAddr) {
        if self.limit == 0 && self.global_limit == 0 {
            return;
        }
        let now = Instant::now();
        self.prune(now);
        if self.global_limit > 0 {
            self.note_global_failure(now);
        }
        if self.limit == 0 {
            return;
        }
        if !self.sources.contains_key(&source) && self.sources.len() >= MAX_SOURCES {
            self.note_overflow_failure(now);
            return;
        }
        let entry = self.sources.entry(source).or_insert(Source {
            failures: 0,
            window_start: now,
            locked_until: None,
        });
        if now.duration_since(entry.window_start) >= self.window {
            entry.failures = 0;
            entry.window_start = now;
            entry.locked_until = None;
        }
        entry.failures = entry.failures.saturating_add(1);
        if entry.failures >= self.limit {
            entry.locked_until = Some(now + self.window);
        }
    }

    pub(super) fn reset(&mut self, source: IpAddr) {
        self.sources.remove(&source);
    }

    fn note_global_failure(&mut self, now: Instant) {
        let entry = self.global.get_or_insert(Source {
            failures: 0,
            window_start: now,
            locked_until: None,
        });
        if now.duration_since(entry.window_start) >= self.window {
            entry.failures = 0;
            entry.window_start = now;
            entry.locked_until = None;
        }
        entry.failures = entry.failures.saturating_add(1);
        if entry.failures >= self.global_limit {
            entry.locked_until = Some(now + self.window);
        }
    }

    fn note_overflow_failure(&mut self, now: Instant) {
        let entry = self.overflow.get_or_insert(Source {
            failures: 0,
            window_start: now,
            locked_until: None,
        });
        if now.duration_since(entry.window_start) >= self.window {
            entry.failures = 0;
            entry.window_start = now;
            entry.locked_until = None;
        }
        entry.failures = entry.failures.saturating_add(1);
        if entry.failures >= self.limit {
            entry.locked_until = Some(now + self.window);
        }
    }

    fn prune(&mut self, now: Instant) {
        self.sources.retain(|_, entry| {
            entry.locked_until.is_some_and(|until| until > now)
                || now.duration_since(entry.window_start) < self.window
        });
        if self.overflow.as_ref().is_some_and(|entry| {
            !entry.locked_until.is_some_and(|until| until > now)
                && now.duration_since(entry.window_start) >= self.window
        }) {
            self.overflow = None;
        }
        if self.global.as_ref().is_some_and(|entry| {
            !entry.locked_until.is_some_and(|until| until > now)
                && now.duration_since(entry.window_start) >= self.window
        }) {
            self.global = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(last: u8) -> IpAddr {
        IpAddr::from([127, 0, 0, last])
    }

    #[test]
    fn budget_locks_one_source_and_resets_on_success() {
        let mut guard = AuthGuard::new(2, 0, 60_000);
        assert!(!guard.locked(source(1)));
        guard.note_failure(source(1));
        assert!(!guard.locked(source(1)));
        guard.note_failure(source(1));
        assert!(guard.locked(source(1)));
        assert!(!guard.locked(source(2)));
        guard.reset(source(1));
        assert!(!guard.locked(source(1)));
    }

    #[test]
    fn process_global_budget_spans_sources_and_is_not_reset_per_source() {
        let mut guard = AuthGuard::new(0, 2, 60_000);
        let first = IpAddr::from([192, 0, 2, 1]);
        let second = IpAddr::from([192, 0, 2, 2]);
        let third = IpAddr::from([192, 0, 2, 3]);
        guard.note_failure(first);
        assert!(!guard.locked(second));
        guard.note_failure(second);
        assert!(guard.locked(third));
        guard.reset(first);
        assert!(guard.locked(third));
    }

    #[test]
    fn expired_global_lockout_starts_a_fresh_window() {
        let mut guard = AuthGuard::new(0, 2, 60_000);
        let now = Instant::now();
        guard.global = Some(Source {
            failures: 2,
            window_start: now - Duration::from_secs(61),
            locked_until: Some(now - Duration::from_secs(1)),
        });
        assert!(!guard.locked(source(1)));
        guard.note_failure(source(1));
        assert_eq!(guard.global.as_ref().unwrap().failures, 1);
        assert!(!guard.locked(source(2)));
    }

    #[test]
    fn zero_disables_and_entry_count_is_bounded() {
        let mut guard = AuthGuard::new(0, 0, 60_000);
        for last in 0..8 {
            guard.note_failure(source(last));
        }
        assert!(!guard.locked(source(1)));

        let mut guard = AuthGuard::new(1, 0, 60_000);
        for index in 0..(MAX_SOURCES + 10) {
            guard.note_failure(IpAddr::from([
                10,
                (index / 256) as u8,
                (index % 256) as u8,
                1,
            ]));
        }
        assert_eq!(guard.sources.len(), MAX_SOURCES);
        assert!(guard.overflow.is_some());
    }

    #[test]
    fn saturation_preserves_tracked_peers_and_throttles_overflow_sources_together() {
        let mut guard = AuthGuard::new(2, 0, 60_000);
        let tracked = IpAddr::from([1, 2, 3, 4]);
        guard.note_failure(tracked);
        guard.note_failure(tracked);
        assert!(guard.locked(tracked));

        for index in 0..(MAX_SOURCES - 1) {
            guard.note_failure(IpAddr::from([
                10,
                (index / 256) as u8,
                (index % 256) as u8,
                1,
            ]));
        }
        assert_eq!(guard.sources.len(), MAX_SOURCES);
        assert!(guard.locked(tracked));

        let overflow_a = IpAddr::from([203, 0, 113, 1]);
        let overflow_b = IpAddr::from([203, 0, 113, 2]);
        guard.note_failure(overflow_a);
        assert!(!guard.locked(overflow_b));
        guard.note_failure(overflow_b);
        assert!(guard.locked(overflow_a));
        assert!(guard.locked(overflow_b));
        guard.reset(overflow_a);
        assert!(guard.locked(overflow_b));
        assert!(guard.locked(tracked));
    }
}
