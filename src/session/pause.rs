//! CLIENT PAUSE command deferral for one connection.

use crate::engine::Execution;
use crate::engine::{Action, is_write_command};

use super::Session;

#[derive(Debug)]
pub(super) struct PausedCommand {
    arguments: Vec<Vec<u8>>,
}

impl Session {
    pub fn is_paused(&self) -> bool {
        self.paused.is_some()
    }

    /// Execute the single command deferred by CLIENT PAUSE once the pause no
    /// longer applies. Returns `None` while the connection must stay paused.
    pub(crate) fn poll_paused_on<E: Execution>(&mut self, store: &E) -> Option<(Vec<u8>, Action)> {
        self.paused.as_ref()?;
        let waiting = {
            let paused = self.paused.as_ref()?;
            let arguments: Vec<&[u8]> = paused.arguments.iter().map(Vec::as_slice).collect();
            self.command_paused(store, &arguments)
        };
        if waiting {
            return None;
        }
        let paused = self.paused.take()?;
        let arguments: Vec<&[u8]> = paused.arguments.iter().map(Vec::as_slice).collect();
        let mut output = Vec::new();
        let action = self.execute_on(store, &arguments, &mut output);
        Some((output, action))
    }

    pub(super) fn command_paused<E: Execution>(&self, store: &E, arguments: &[&[u8]]) -> bool {
        let Some(writes_only) = store.client_pause_mode() else {
            return false;
        };
        let Some(command) = arguments.first() else {
            return false;
        };
        // Management commands must remain able to end or replace a pause.
        if command.eq_ignore_ascii_case(b"CLIENT")
            && arguments.get(1).is_some_and(|subcommand| {
                subcommand.eq_ignore_ascii_case(b"PAUSE")
                    || subcommand.eq_ignore_ascii_case(b"UNPAUSE")
            })
        {
            return false;
        }
        !writes_only || command.eq_ignore_ascii_case(b"EXEC") || is_write_command(command)
    }

    /// Defer this command while a CLIENT PAUSE applies. Returns `true` when the
    /// command was stored instead of executed.
    pub(super) fn pause_command<E: Execution>(&mut self, store: &E, arguments: &[&[u8]]) -> bool {
        if self.paused.is_some() || !self.command_paused(store, arguments) {
            return false;
        }
        self.paused = Some(PausedCommand {
            arguments: arguments.iter().map(|argument| argument.to_vec()).collect(),
        });
        true
    }
}
