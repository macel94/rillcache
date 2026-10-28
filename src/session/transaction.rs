//! MULTI/EXEC/DISCARD/WATCH queueing and transaction state.

use std::io::Write;

use crate::engine::{Access, Action, Store, TransactionOutcome, WatchState};

use super::Session;
use super::command::{arity, command_name, is_session_command};

const MAX_QUEUED_COMMANDS: usize = 10_000;

/// Commands queued by MULTI, plus whether a queue-time error forces EXECABORT.
#[derive(Debug)]
pub(super) struct Multi {
    pub(super) queue: Vec<Vec<Vec<u8>>>,
    pub(super) dirty: bool,
}

impl Session {
    pub(super) fn multi(&mut self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 1 {
            arity("multi", output);
        } else if self.multi.is_some() {
            output.extend_from_slice(b"-ERR MULTI calls can not be nested\r\n");
        } else {
            self.multi = Some(Multi {
                queue: Vec::new(),
                dirty: false,
            });
            output.extend_from_slice(b"+OK\r\n");
        }
    }

    pub(super) fn exec(
        &mut self,
        store: &Store,
        arguments: &[&[u8]],
        output: &mut Vec<u8>,
    ) -> Action {
        if arguments.len() != 1 {
            arity("exec", output);
            return Action::Continue;
        }
        let Some(multi) = self.multi.take() else {
            output.extend_from_slice(b"-ERR EXEC without MULTI\r\n");
            return Action::Continue;
        };
        let watched = std::mem::take(&mut self.watched);
        if multi.dirty {
            output.extend_from_slice(
                b"-EXECABORT Transaction discarded because of previous errors.\r\n",
            );
            return Action::Continue;
        }
        // Re-check every queued command: ACL edits between queueing and EXEC
        // must not let a revoked permission commit.
        for command in &multi.queue {
            let arguments: Vec<&[u8]> = command.iter().map(Vec::as_slice).collect();
            match store.authorize(self.authenticated_user.as_deref(), &arguments) {
                Access::Allowed => {}
                Access::NoAuth => {
                    self.log_denial(store, "noauth", "multi", arguments[0]);
                    output.extend_from_slice(b"-NOAUTH Authentication required.\r\n");
                    return Action::Continue;
                }
                Access::DeniedCommand => {
                    let name = command_name(arguments[0]);
                    self.log_denial(store, "command", "multi", name.as_bytes());
                    let _ = write!(
                        output,
                        "-NOPERM this user has no permissions to run the '{name}' command\r\n"
                    );
                    return Action::Continue;
                }
                Access::DeniedKey(key) => {
                    self.log_denial(store, "key", "multi", &key);
                    output.extend_from_slice(
                        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
                    );
                    return Action::Continue;
                }
                Access::DeniedChannel(channel) => {
                    self.log_denial(store, "channel", "multi", &channel);
                    output.extend_from_slice(
                        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n",
                    );
                    return Action::Continue;
                }
            }
        }
        match store.execute_transaction(&multi.queue, &watched, output) {
            TransactionOutcome::Completed(action) => action,
            TransactionOutcome::Aborted | TransactionOutcome::InternalError => Action::Continue,
        }
    }

    pub(super) fn discard(&mut self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 1 {
            arity("discard", output);
        } else if self.multi.take().is_none() {
            output.extend_from_slice(b"-ERR DISCARD without MULTI\r\n");
        } else {
            self.watched = WatchState::default();
            output.extend_from_slice(b"+OK\r\n");
        }
    }

    pub(super) fn watch(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() < 2 {
            arity("watch", output);
        } else if self.multi.is_some() {
            output.extend_from_slice(b"-ERR WATCH inside MULTI is not allowed\r\n");
        } else {
            match store.watch_snapshot(&arguments[1..]) {
                Some(state) => {
                    self.watched.merge(state);
                    output.extend_from_slice(b"+OK\r\n");
                }
                None => output.extend_from_slice(b"-ERR internal store error\r\n"),
            }
        }
    }

    pub(super) fn unwatch(&mut self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 1 {
            arity("unwatch", output);
        } else {
            self.watched = WatchState::default();
            output.extend_from_slice(b"+OK\r\n");
        }
    }

    /// Queue one command, validating support and table arity before EXEC.
    pub(super) fn queue(&mut self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let Some(multi) = self.multi.as_mut() else {
            return;
        };
        let command = arguments[0];
        if command.eq_ignore_ascii_case(b"SHUTDOWN") {
            multi.dirty = true;
            output.extend_from_slice(b"-ERR SHUTDOWN is not allowed in transactions\r\n");
            return;
        }
        if is_session_command(command) {
            multi.dirty = true;
            output.extend_from_slice(b"-ERR session commands are not supported inside MULTI\r\n");
            return;
        }
        let Some(arity_value) = Store::command_arity(command) else {
            multi.dirty = true;
            output.extend_from_slice(b"-ERR unknown command\r\n");
            return;
        };
        let count = arguments.len() as i64;
        if (arity_value > 0 && count != arity_value) || (arity_value < 0 && count < -arity_value) {
            multi.dirty = true;
            let name = String::from_utf8_lossy(command).to_ascii_lowercase();
            let _ = write!(
                output,
                "-ERR wrong number of arguments for '{name}' command\r\n"
            );
            return;
        }
        if multi.queue.len() >= MAX_QUEUED_COMMANDS {
            multi.dirty = true;
            output.extend_from_slice(b"-ERR transaction queue limit exceeded\r\n");
            return;
        }
        multi
            .queue
            .push(arguments.iter().map(|argument| argument.to_vec()).collect());
        output.extend_from_slice(b"+QUEUED\r\n");
    }
}
