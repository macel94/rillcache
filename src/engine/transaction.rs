//! `WATCH` snapshots and atomic `EXEC` execution.
//!
//! A store-wide read/write gate separates ordinary commands from a transaction:
//! every normal command holds the read side, while `EXEC` holds the write side
//! for the whole queued batch. `WATCH` compares bounded per-key mutation versions
//! captured under the same gate, so a check cannot race the execution it guards.

use super::Store;
use super::data::Data;
use std::sync::RwLockReadGuard;

/// Opaque conflict snapshot for one connection's watched keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WatchState {
    epoch: u64,
    keys: Vec<(Vec<u8>, Option<u64>, bool)>,
}

impl WatchState {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Add keys without losing earlier watches from the same connection.
    pub(crate) fn merge(&mut self, other: Self) {
        if self.keys.is_empty() {
            *self = other;
            return;
        }
        if self.epoch != other.epoch {
            // A flush happened between snapshots: invalidate the earlier set.
            self.epoch = other.epoch;
            self.keys = other.keys;
            return;
        }
        for (key, version, present) in other.keys {
            if let Some(existing) = self.keys.iter_mut().find(|(name, _, _)| *name == key) {
                existing.1 = version;
                existing.2 = present;
            } else {
                self.keys.push((key, version, present));
            }
        }
    }

    pub(super) fn changed(&self, data: &Data, now: i64) -> bool {
        data.epoch() != self.epoch
            || self.keys.iter().any(|(key, version, present)| {
                data.version(key) != *version
                    || data.entries.get(key).is_some_and(|entry| entry.alive(now)) != *present
            })
    }
}

/// Result of an atomic `EXEC`: either replies were written or the transaction
/// was aborted because a watched key changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionOutcome {
    Completed(super::Action),
    Aborted,
    InternalError,
}

impl Store {
    /// Capture every requested key under the same gate used by `EXEC`.
    pub fn watch_snapshot(&self, keys: &[&[u8]]) -> Option<WatchState> {
        let _gate: RwLockReadGuard<'_, ()> = self.transaction_gate.read().ok()?;
        let mut data = self.data.write().ok()?;
        let now = self.clock.now_ms().max(0);
        for key in keys {
            data.expire_key(key, now);
        }
        Some(WatchState {
            epoch: data.epoch(),
            keys: keys
                .iter()
                .map(|key| {
                    (
                        key.to_vec(),
                        data.version(key),
                        data.entries.get(*key).is_some_and(|entry| entry.alive(now)),
                    )
                })
                .collect(),
        })
    }

    /// Execute a queued transaction atomically. Replies are appended as a RESP
    /// array; a watched-key conflict emits a null array and changes no data.
    pub fn execute_transaction(
        &self,
        commands: &[Vec<Vec<u8>>],
        watches: &WatchState,
        output: &mut Vec<u8>,
    ) -> TransactionOutcome {
        let Ok(_gate) = self.transaction_gate.write() else {
            super::internal_error(output);
            return TransactionOutcome::InternalError;
        };
        let start = output.len();
        let Ok(data) = self.data.read() else {
            super::internal_error(output);
            return TransactionOutcome::InternalError;
        };
        let changed = watches.changed(&data, self.clock.now_ms().max(0));
        drop(data);
        if changed {
            output.truncate(start);
            output.extend_from_slice(b"*-1\r\n");
            return TransactionOutcome::Aborted;
        }
        let _ = std::io::Write::write_fmt(output, format_args!("*{}\r\n", commands.len()));
        let mut action = super::Action::Continue;
        for command in commands {
            let arguments: Vec<&[u8]> = command.iter().map(Vec::as_slice).collect();
            let marks = self.journal_marks();
            let command_start = output.len();
            action = self.execute_locked(&arguments, output);
            self.commit_journal(marks, &arguments, output, command_start);
            if action == super::Action::Close {
                break;
            }
        }
        TransactionOutcome::Completed(action)
    }
}
