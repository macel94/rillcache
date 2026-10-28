//! Bounded per-source failed-authentication throttle.
//!
//! This complements the per-connection failed-auth budget. It tracks failures
//! by peer IP so reconnecting does not reset the source's budget. It is
//! deliberately process-local and bounded; it is not a distributed or
//! credential-aware lockout system.

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
    window: Duration,
    sources: BTreeMap<IpAddr, Source>,
}

impl AuthGuard {
    pub(super) fn new(limit: usize, window_ms: u64) -> Self {
        Self {
            limit,
            window: Duration::from_millis(window_ms),
            sources: BTreeMap::new(),
        }
    }

    pub(super) fn configure(&mut self, limit: usize, window_ms: u64) {
        self.limit = limit;
        self.window = Duration::from_millis(window_ms);
        if limit == 0 {
            self.sources.clear();
        }
    }

    pub(super) fn locked(&mut self, source: IpAddr) -> bool {
        if self.limit == 0 {
            return false;
        }
        let now = Instant::now();
        match self.sources.get(&source) {
            Some(entry) if entry.locked_until.is_some_and(|until| until > now) => true,
            Some(entry)
                if entry.locked_until.is_none()
                    && now.duration_since(entry.window_start) < self.window =>
            {
                false
            }
            Some(_) => {
                self.sources.remove(&source);
                false
            }
            None => false,
        }
    }

    pub(super) fn note_failure(&mut self, source: IpAddr) {
        if self.limit == 0 {
            return;
        }
        let now = Instant::now();
        self.prune(now);
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

    fn prune(&mut self, now: Instant) {
        self.sources.retain(|_, entry| {
            entry.locked_until.is_some_and(|until| until > now)
                || now.duration_since(entry.window_start) < self.window
        });
        if self.sources.len() >= MAX_SOURCES
            && let Some(oldest) = self.sources.keys().next().copied()
        {
            self.sources.remove(&oldest);
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
        let mut guard = AuthGuard::new(2, 60_000);
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
    fn zero_disables_and_entry_count_is_bounded() {
        let mut guard = AuthGuard::new(0, 60_000);
        for last in 0..8 {
            guard.note_failure(source(last));
        }
        assert!(!guard.locked(source(1)));

        let mut guard = AuthGuard::new(1, 60_000);
        for last in 0..(MAX_SOURCES + 10) {
            guard.note_failure(IpAddr::from([
                10,
                (last / 256) as u8,
                (last % 256) as u8,
                1,
            ]));
        }
        assert!(guard.sources.len() <= MAX_SOURCES);
    }
}
