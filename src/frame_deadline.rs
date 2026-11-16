//! Per-connection monotonic deadline for an incomplete plaintext request frame.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrameDeadline(Option<Instant>);

impl FrameDeadline {
    /// Starts a frame's timer once. Further observations of the same partial frame
    /// retain the original timestamp; consuming/completing it clears the timer.
    pub(crate) fn observe_incomplete(&mut self, incomplete: bool, now: Instant) {
        if incomplete {
            self.0.get_or_insert(now);
        } else {
            self.clear();
        }
    }

    pub(crate) fn clear(&mut self) {
        self.0 = None;
    }

    pub(crate) fn deadline(&self, timeout: Option<Duration>) -> Option<Instant> {
        self.0?.checked_add(timeout?)
    }

    pub(crate) fn expired_now(&self, timeout: Option<Duration>) -> bool {
        let (Some(timeout), Some(started)) = (timeout, self.0) else {
            return false;
        };
        Instant::now().saturating_duration_since(started) >= timeout
    }

    pub(crate) fn remaining_now(&self, timeout: Option<Duration>) -> Option<Duration> {
        let timeout = timeout?;
        let started = self.0?;
        Some(
            timeout.saturating_sub(
                Instant::now()
                    .saturating_duration_since(started)
                    .min(timeout),
            ),
        )
    }

    pub(crate) fn expired_at(&self, timeout: Option<Duration>, now: Instant) -> bool {
        timeout.is_some_and(|timeout| {
            self.0
                .is_some_and(|started| now.saturating_duration_since(started) >= timeout)
        })
    }

    #[cfg(test)]
    fn expired(&self, timeout: Option<Duration>, now: Instant) -> bool {
        self.expired_at(timeout, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_do_not_extend_frame_and_consumption_restarts_it() {
        let start = Instant::now();
        let mut deadline = FrameDeadline::default();
        deadline.observe_incomplete(true, start);
        deadline.observe_incomplete(true, start + Duration::from_millis(9));
        assert!(!deadline.expired(
            Some(Duration::from_millis(10)),
            start + Duration::from_millis(9)
        ));
        assert!(deadline.expired(
            Some(Duration::from_millis(10)),
            start + Duration::from_millis(10)
        ));
        deadline.observe_incomplete(false, start + Duration::from_millis(10));
        assert!(!deadline.expired(
            Some(Duration::from_millis(10)),
            start + Duration::from_secs(1)
        ));
        deadline.observe_incomplete(true, start + Duration::from_secs(1));
        assert!(!deadline.expired(
            Some(Duration::from_millis(10)),
            start + Duration::from_secs(1)
        ));
    }

    #[test]
    fn disabled_deadline_never_expires() {
        let start = Instant::now();
        let mut deadline = FrameDeadline::default();
        deadline.observe_incomplete(true, start);
        assert!(!deadline.expired(None, start + Duration::from_secs(60)));
    }
}
