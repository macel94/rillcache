//! `WATCH` snapshots and atomic `EXEC` execution.
//!
//! One owner executes each ordinary command or complete EXEC as a synchronous
//! turn. No other data operation can interleave with WATCH validation and the
//! queued batch; no shared transaction lock is needed. In multi-owner mode the
//! router records the owning worker and every participating owner's epoch in the
//! same token, so one token type serves both execution modes.
//!
//! An expiry is only flushed when it becomes logically observable; very short
//! expiries can therefore invalidate a watch through the ordinary presence and
//! version test.

use super::ShardExecutor;
use super::data::Data;

/// One watched key plus the generation observed on the owner that holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WatchToken {
    pub(crate) owner: usize,
    pub(crate) key: Vec<u8>,
    pub(crate) version: Option<u64>,
    pub(crate) present: bool,
    pub(crate) epoch: u64,
}

/// Opaque conflict snapshot for one connection's watched keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WatchState {
    tokens: Vec<WatchToken>,
}

impl WatchState {
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    pub(crate) fn tokens(&self) -> &[WatchToken] {
        &self.tokens
    }

    pub(crate) fn from_tokens(tokens: Vec<WatchToken>) -> Self {
        Self { tokens }
    }

    /// Add keys without losing earlier watches from the same connection.
    pub(crate) fn merge(&mut self, other: Self) {
        if self.tokens.is_empty() {
            *self = other;
            return;
        }
        // A later snapshot of the same key supersedes the earlier observation.
        for token in other.tokens {
            if let Some(existing) = self
                .tokens
                .iter_mut()
                .find(|candidate| candidate.owner == token.owner && candidate.key == token.key)
            {
                *existing = token;
            } else {
                self.tokens.push(token);
            }
        }
    }

    /// Single-owner validation: every token was recorded on owner 0. A flush
    /// between snapshots advances the epoch and conservatively aborts.
    pub(super) fn changed(&self, data: &Data, now: i64) -> bool {
        self.tokens.iter().any(|token| {
            data.epoch() != token.epoch
                || data.version(&token.key) != token.version
                || data
                    .entries
                    .get(&token.key)
                    .is_some_and(|entry| entry.alive(now))
                    != token.present
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

impl ShardExecutor {
    /// Capture every requested key in one non-interleaved owner turn.
    pub fn watch_snapshot(&self, keys: &[&[u8]]) -> Option<WatchState> {
        let mut data = self.data.try_borrow_mut().ok()?;
        let now = self.clock.now_ms().max(0);
        for key in keys {
            data.expire_key(key, now);
        }
        let epoch = data.epoch();
        Some(WatchState::from_tokens(
            keys.iter()
                .map(|key| WatchToken {
                    owner: 0,
                    key: key.to_vec(),
                    version: data.version(key),
                    present: data.entries.get(*key).is_some_and(|entry| entry.alive(now)),
                    epoch,
                })
                .collect(),
        ))
    }

    /// Execute a queued transaction atomically. Replies are appended as a RESP
    /// array; a watched-key conflict emits a null array and changes no data.
    pub fn execute_transaction(
        &self,
        commands: &[Vec<Vec<u8>>],
        watches: &WatchState,
        output: &mut Vec<u8>,
    ) -> TransactionOutcome {
        let start = output.len();
        let Ok(data) = self.data.try_borrow() else {
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
            action = self.execute_queued(&arguments, output);
            if action == super::Action::Close {
                break;
            }
        }
        TransactionOutcome::Completed(action)
    }

    /// Run queued commands without the WATCH re-check. The multi-owner router
    /// validates every participant's watches under one reservation and then
    /// replays the batch on this executor.
    #[allow(dead_code)] // test-only today
    pub(crate) fn execute_batch_without_watch(
        &self,
        commands: &[Vec<Vec<u8>>],
        output: &mut Vec<u8>,
    ) -> super::Action {
        let _ = std::io::Write::write_fmt(output, format_args!("*{}\r\n", commands.len()));
        let mut action = super::Action::Continue;
        for command in commands {
            let arguments: Vec<&[u8]> = command.iter().map(Vec::as_slice).collect();
            action = self.execute_queued(&arguments, output);
            if action == super::Action::Close {
                break;
            }
        }
        action
    }

    /// Execute one queued command. Blocking commands are queued and executed by
    /// their non-blocking equivalent, as Redis does inside `MULTI`: they never
    /// wait, they return the immediate reply (nil when nothing is available),
    /// and the journal records the effective mutation, never the blocking form.
    fn execute_queued(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> super::Action {
        let Some(name) = arguments.first() else {
            return super::Action::Continue;
        };
        if name.eq_ignore_ascii_case(b"BLPOP") || name.eq_ignore_ascii_case(b"BRPOP") {
            return self.queued_blocking_pop(arguments, output);
        }
        if let Some(effective) = rewrite_blocking(arguments) {
            let marks = self.journal_marks();
            let start = output.len();
            let action = self.execute_command(&effective, output);
            self.commit_journal(marks, &effective, output, start);
            return action;
        }
        let marks = self.journal_marks();
        let start = output.len();
        let action = self.execute_command(arguments, output);
        self.commit_journal(marks, arguments, output, start);
        action
    }

    fn queued_blocking_pop(&self, arguments: &[&[u8]], output: &mut Vec<u8>) -> super::Action {
        let left = arguments[0].eq_ignore_ascii_case(b"BLPOP");
        if arguments.len() < 3 {
            super::arity("blpop", output);
            return super::Action::Continue;
        }
        let (timeout, keys) = arguments[1..].split_last().expect("nonempty");
        if let Err(reply) = queued_timeout(timeout) {
            output.extend_from_slice(reply);
            return super::Action::Continue;
        }
        let command: &[u8] = if left { b"LPOP" } else { b"RPOP" };
        for key in keys {
            let effective = [command, *key];
            let marks = self.journal_marks();
            let mut scratch = Vec::new();
            let action = self.execute_command(&effective, &mut scratch);
            self.commit_journal(marks, &effective, &mut scratch, 0);
            if action != super::Action::Continue {
                output.extend_from_slice(&scratch);
                return action;
            }
            if scratch.starts_with(b"$-1") || scratch.starts_with(b"*-1") {
                continue;
            }
            if !scratch.starts_with(b"$") {
                output.extend_from_slice(&scratch);
                return super::Action::Continue;
            }
            let _ = std::io::Write::write_fmt(output, format_args!("*2\r\n${}\r\n", key.len()));
            output.extend_from_slice(key);
            output.extend_from_slice(b"\r\n");
            output.extend_from_slice(&scratch);
            return super::Action::Continue;
        }
        output.extend_from_slice(b"*-1\r\n");
        super::Action::Continue
    }
}

/// The documented timeout rejection classes shared by the blocking commands.
fn queued_timeout(raw: &[u8]) -> Result<f64, &'static [u8]> {
    let Ok(text) = std::str::from_utf8(raw) else {
        return Err(b"-ERR timeout is not a float or out of range\r\n");
    };
    let Ok(value) = text.parse::<f64>() else {
        return Err(b"-ERR timeout is not a float or out of range\r\n");
    };
    if value < 0.0 {
        return Err(b"-ERR timeout is negative\r\n");
    }
    if !value.is_finite() || std::time::Duration::try_from_secs_f64(value).is_err() {
        return Err(b"-ERR timeout is out of range\r\n");
    }
    Ok(value)
}

/// Rewrite a blocking command to its non-blocking equivalent. The timeout is
/// dropped and the effective command is what the journal records.
fn rewrite_blocking<'a>(arguments: &[&'a [u8]]) -> Option<Vec<&'a [u8]>> {
    let name = *arguments.first()?;
    let rewritten = if name.eq_ignore_ascii_case(b"BLMPOP") {
        // BLMPOP timeout numkeys key [key ...] LEFT|RIGHT [COUNT count]
        if arguments.len() < 4 {
            return None;
        }
        let mut args = Vec::with_capacity(arguments.len() - 1);
        args.push(b"LMPOP".as_slice());
        args.extend_from_slice(&arguments[2..]);
        args
    } else if name.eq_ignore_ascii_case(b"BLMOVE") {
        // BLMOVE source destination LEFT|RIGHT LEFT|RIGHT timeout
        if arguments.len() != 6 {
            return None;
        }
        let mut args = Vec::with_capacity(5);
        args.push(b"LMOVE".as_slice());
        args.extend_from_slice(&arguments[1..5]);
        args
    } else if name.eq_ignore_ascii_case(b"BRPOPLPUSH") {
        // BRPOPLPUSH source destination timeout
        if arguments.len() != 4 {
            return None;
        }
        let mut args = Vec::with_capacity(3);
        args.push(b"RPOPLPUSH".as_slice());
        args.extend_from_slice(&arguments[1..3]);
        args
    } else {
        return None;
    };
    Some(rewritten)
}
