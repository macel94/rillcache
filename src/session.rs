//! Per-connection protocol negotiation and bounded client metadata.
//!
//! RESP3 support is deliberately limited to the reply types emitted by the
//! current command set: strings, integers, errors, arrays and nulls.

use std::collections::BTreeSet;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::time::{Duration, Instant};

use crate::engine::{
    Access, AclError, Action, AuthError, ListMoveOutcome, ListPopOutcome, PopOutcome, Store,
    StreamId, TransactionOutcome, WatchState, XReadGroupOptions, XReadGroupOutcome,
    XReadGroupStart, XReadOutcome, ZSetPopOutcome, blocking_lmpop_reply, blocking_reply,
    blocking_zpop_reply, is_write_command, list_move_reply, parse_bzmpop_timeout, parse_list_mpop,
    parse_mpop, parse_side, parse_xread, parse_xreadgroup, xread_reply, xreadgroup_reply,
};
use crate::protocol::encode_bulk;

const MAX_NAME_BYTES: usize = 256;
const MAX_LIBRARY_FIELD_BYTES: usize = 128;
const MAX_QUEUED_COMMANDS: usize = 10_000;
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

/// Commands queued by MULTI, plus whether a queue-time error forces EXECABORT.
#[derive(Debug)]
struct Multi {
    queue: Vec<Vec<Vec<u8>>>,
    dirty: bool,
}

/// State belonging to one socket. Move this together with any borrowed client
/// lease; it contains no socket handles and is never shared across clients.
#[derive(Debug)]
pub struct Session {
    protocol: Protocol,
    id: u64,
    name: Option<Vec<u8>>,
    library_name: Option<Vec<u8>>,
    library_version: Option<Vec<u8>>,
    multi: Option<Multi>,
    watched: WatchState,
    authenticated_user: Option<Vec<u8>>,
    pubsub_sender: Option<SyncSender<Vec<u8>>>,
    pubsub_channels: BTreeSet<Vec<u8>>,
    pubsub_patterns: BTreeSet<Vec<u8>>,
    pubsub_shard_channels: BTreeSet<Vec<u8>>,
    blocked: Option<Blocked>,
    paused: Option<PausedCommand>,
}

#[derive(Debug)]
struct PausedCommand {
    arguments: Vec<Vec<u8>>,
}

#[derive(Debug)]
struct Blocked {
    keys: Vec<Vec<u8>>,
    deadline: Option<Instant>,
    kind: BlockedKind,
    slowlog: Option<(Instant, Vec<Vec<u8>>)>,
}

#[derive(Clone, Debug)]
enum BlockedKind {
    List {
        left: bool,
    },
    ListMulti {
        left: bool,
        count: usize,
    },
    ListMove {
        source: Vec<u8>,
        destination: Vec<u8>,
        from_left: bool,
        to_left: bool,
    },
    ZSet {
        max: bool,
        count: usize,
    },
    XRead {
        starts: Vec<StreamId>,
        count: usize,
    },
    XReadGroup {
        group: Vec<u8>,
        consumer: Vec<u8>,
        count: usize,
        noack: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Protocol {
    Resp2,
    Resp3,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            protocol: Protocol::Resp2,
            id: NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed),
            name: None,
            library_name: None,
            library_version: None,
            multi: None,
            watched: WatchState::default(),
            authenticated_user: None,
            pubsub_sender: None,
            pubsub_channels: BTreeSet::new(),
            pubsub_patterns: BTreeSet::new(),
            pubsub_shard_channels: BTreeSet::new(),
            blocked: None,
            paused: None,
        }
    }

    /// Dispatch connection commands locally and all data commands through the
    /// stable public RESP2 Store API, adapting only this session's reply.
    pub fn execute(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        if self.paused.is_none() && self.command_paused(store, arguments) {
            self.paused = Some(PausedCommand {
                arguments: arguments.iter().map(|argument| argument.to_vec()).collect(),
            });
            return Action::Continue;
        }
        let (slowlog, latency) = arguments.first().map_or((false, false), |command| {
            let queued = self.multi.is_some() && !command.eq_ignore_ascii_case(b"EXEC");
            (
                !queued && store.slowlog_enabled(),
                !queued && store.latency_enabled(),
            )
        });
        let started = (slowlog || latency).then(Instant::now);
        let action = self.execute_inner(store, arguments, output);
        if let Some(started) = started {
            if latency {
                store.record_command_latency(started.elapsed());
            }
            if slowlog {
                if self.blocked.is_some() {
                    if let Some(blocked) = self.blocked.as_mut() {
                        blocked.slowlog = Some((
                            started,
                            arguments.iter().map(|argument| argument.to_vec()).collect(),
                        ));
                    }
                } else {
                    self.record_slowlog(store, arguments, started);
                }
            }
        }
        action
    }

    fn sync_blocked(&self, store: &Store) {
        store.set_client_blocked(self.id, self.blocked.is_some());
    }

    fn record_slowlog(&self, store: &Store, arguments: &[&[u8]], started: Instant) {
        if !store.slowlog_enabled() {
            return;
        }
        store.slowlog_push(
            arguments,
            started.elapsed(),
            &store.client_address(self.id).unwrap_or_default(),
            self.name.as_deref().unwrap_or_default(),
        );
    }

    pub fn is_paused(&self) -> bool {
        self.paused.is_some()
    }

    /// Execute the single command deferred by CLIENT PAUSE once the pause no
    /// longer applies. Returns `None` while the connection must stay paused.
    pub fn poll_paused(&mut self, store: &Store) -> Option<(Vec<u8>, Action)> {
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
        let action = self.execute(store, &arguments, &mut output);
        Some((output, action))
    }

    fn command_paused(&self, store: &Store, arguments: &[&[u8]]) -> bool {
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

    fn execute_inner(
        &mut self,
        store: &Store,
        arguments: &[&[u8]],
        output: &mut Vec<u8>,
    ) -> Action {
        let Some(command) = arguments.first() else {
            output.extend_from_slice(b"-ERR empty command\r\n");
            return Action::Continue;
        };
        store.record_command();
        if command.eq_ignore_ascii_case(b"RESET") {
            return self.reset(store, arguments, output);
        }
        if !is_auth_exempt(command) {
            let authentication_required = store.authentication_required();
            if authentication_required && self.authenticated_user.is_none() {
                self.log_denial(store, "noauth", "toplevel", command);
                output.extend_from_slice(b"-NOAUTH Authentication required.\r\n");
                return Action::Continue;
            }
            if authentication_required
                || store.acl_restricted()
                || self.authenticated_user.is_some()
            {
                match store.authorize(self.authenticated_user.as_deref(), arguments) {
                    Access::Allowed => {}
                    Access::NoAuth => {
                        self.log_denial(store, "noauth", "toplevel", command);
                        output.extend_from_slice(b"-NOAUTH Authentication required.\r\n");
                        return Action::Continue;
                    }
                    Access::DeniedCommand => {
                        let name = command_name(command);
                        self.log_denial(store, "command", "toplevel", name.as_bytes());
                        let _ = write!(
                            output,
                            "-NOPERM this user has no permissions to run the '{name}' command\r\n"
                        );
                        if let Some(multi) = self.multi.as_mut() {
                            multi.dirty = true;
                        }
                        return Action::Continue;
                    }
                    Access::DeniedKey(key) => {
                        self.log_denial(store, "key", "toplevel", &key);
                        output.extend_from_slice(
                            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
                        );
                        if let Some(multi) = self.multi.as_mut() {
                            multi.dirty = true;
                        }
                        return Action::Continue;
                    }
                    Access::DeniedChannel(channel) => {
                        self.log_denial(store, "channel", "toplevel", &channel);
                        output.extend_from_slice(
                            b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n",
                        );
                        if let Some(multi) = self.multi.as_mut() {
                            multi.dirty = true;
                        }
                        return Action::Continue;
                    }
                }
            }
        }
        if self.is_subscribed()
            && self.protocol == Protocol::Resp2
            && !allows_subscriber_command(command)
        {
            let name = command_name(command);
            let _ = write!(
                output,
                "-ERR Can't execute '{name}': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n"
            );
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"MULTI") {
            self.multi(arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"EXEC") {
            let start = output.len();
            let action = self.exec(store, arguments, output);
            self.convert_reply(start, output);
            return action;
        }
        if command.eq_ignore_ascii_case(b"DISCARD") {
            self.discard(arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"WATCH") {
            self.watch(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"UNWATCH") {
            self.unwatch(arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"QUIT") {
            // QUIT executes immediately even inside MULTI, as in Redis.
            self.multi = None;
            self.watched = WatchState::default();
            self.pubsub_channels.clear();
            self.pubsub_patterns.clear();
            self.pubsub_shard_channels.clear();
            self.blocked = None;
            self.sync_blocked(store);
        } else if self.multi.is_some() {
            self.queue(arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SHUTDOWN") {
            store.shutdown_command(arguments, output);
            return if store.shutdown_requested() {
                Action::Close
            } else {
                Action::Continue
            };
        }
        if command.eq_ignore_ascii_case(b"SUBSCRIBE") {
            self.subscribe(store, &arguments[1..], false, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"PSUBSCRIBE") {
            self.subscribe(store, &arguments[1..], true, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"UNSUBSCRIBE") {
            self.unsubscribe(store, &arguments[1..], false, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"PUNSUBSCRIBE") {
            self.unsubscribe(store, &arguments[1..], true, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SSUBSCRIBE") {
            self.subscribe_shard(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SUNSUBSCRIBE") {
            self.unsubscribe_shard(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"BLPOP") || command.eq_ignore_ascii_case(b"BRPOP") {
            let left = command.eq_ignore_ascii_case(b"BLPOP");
            self.blocking(store, &arguments[1..], left, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"BLMPOP") {
            self.blocking_lmpop(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"BLMOVE") || command.eq_ignore_ascii_case(b"BRPOPLPUSH") {
            self.blocking_list_move(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"BZMPOP") {
            self.blocking_zpop(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"XREAD") {
            self.xread(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"XREADGROUP")
            && let Ok(options) = parse_xreadgroup(arguments)
            && options.block.is_some()
            && options
                .starts
                .iter()
                .all(|start| matches!(start, XReadGroupStart::New))
        {
            self.blocking_xreadgroup(store, &options, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"PUBLISH") {
            self.publish(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SPUBLISH") {
            self.spublish(store, arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"PUBSUB") {
            self.pubsub(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"PING")
            && self.is_subscribed()
            && self.protocol == Protocol::Resp2
        {
            self.subscriber_ping(&arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"HELLO") {
            self.hello(store, &arguments[1..], output);
            self.refresh_pubsub_protocol(store);
            self.sync_client_metadata(store);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"CLIENT") {
            self.client(store, &arguments[1..], output);
            self.sync_client_metadata(store);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SELECT") {
            select(&arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"AUTH") {
            self.auth(store, &arguments[1..], output);
            self.sync_client_metadata(store);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"ACL") {
            self.acl(store, &arguments[1..], output);
            return Action::Continue;
        }

        let start = output.len();
        let action = store.execute(arguments, output);
        self.convert_reply(start, output);
        action
    }

    fn convert_reply(&self, start: usize, output: &mut Vec<u8>) {
        if self.protocol == Protocol::Resp3 {
            let converted = convert_resp2_reply(&output[start..]);
            output.truncate(start);
            output.extend_from_slice(&converted);
        }
    }

    fn multi(&mut self, arguments: &[&[u8]], output: &mut Vec<u8>) {
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

    fn exec(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
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

    fn discard(&mut self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 1 {
            arity("discard", output);
        } else if self.multi.take().is_none() {
            output.extend_from_slice(b"-ERR DISCARD without MULTI\r\n");
        } else {
            self.watched = WatchState::default();
            output.extend_from_slice(b"+OK\r\n");
        }
    }

    fn watch(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
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

    fn unwatch(&mut self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 1 {
            arity("unwatch", output);
        } else {
            self.watched = WatchState::default();
            output.extend_from_slice(b"+OK\r\n");
        }
    }

    /// Queue one command, validating support and table arity before EXEC.
    fn queue(&mut self, arguments: &[&[u8]], output: &mut Vec<u8>) {
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

    fn auth(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let (username, password) = match arguments {
            [password] => (None, *password),
            [username, password] => (Some(*username), *password),
            _ => {
                arity("auth", output);
                return;
            }
        };
        match store.authenticate_user(username, password) {
            Ok(name) => {
                self.authenticated_user = Some(name);
                output.extend_from_slice(b"+OK\r\n");
            }
            Err(AuthError::NoPassword) => output.extend_from_slice(
                b"-ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?\r\n",
            ),
            Err(AuthError::WrongPassword) => output.extend_from_slice(
                b"-WRONGPASS invalid username-password pair or user is disabled.\r\n",
            ),
            Err(AuthError::Internal) => {
                output.extend_from_slice(b"-ERR internal authentication error\r\n")
            }
        }
    }

    fn acl(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let Some(subcommand) = arguments.first() else {
            arity("acl", output);
            return;
        };
        if subcommand.eq_ignore_ascii_case(b"WHOAMI") {
            if arguments.len() != 1 {
                arity("acl|whoami", output);
            } else if let Some(name) = store.whoami(self.authenticated_user.as_deref()) {
                encode_bulk(Some(&name), output);
            } else {
                output.extend_from_slice(b"-NOAUTH Authentication required.\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"USERS") {
            if arguments.len() != 1 {
                arity("acl|users", output);
            } else if let Some(names) = store.acl_user_names() {
                write!(output, "*{}\r\n", names.len()).expect("Vec write");
                for name in names {
                    encode_bulk(Some(&name), output);
                }
            } else {
                output.extend_from_slice(b"-ERR internal authentication error\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"LIST") {
            if arguments.len() != 1 {
                arity("acl|list", output);
            } else if let Some(names) = store.acl_user_names() {
                let lines: Vec<String> = names
                    .iter()
                    .filter_map(|name| store.acl_line(name))
                    .collect();
                write!(output, "*{}\r\n", lines.len()).expect("Vec write");
                for line in lines {
                    encode_bulk(Some(line.as_bytes()), output);
                }
            } else {
                output.extend_from_slice(b"-ERR internal authentication error\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"CAT") {
            if arguments.len() > 2 {
                arity("acl|cat", output);
            } else if arguments.len() == 1 {
                write!(output, "*{}\r\n", Store::acl_categories().len()).expect("Vec write");
                for category in Store::acl_categories() {
                    encode_bulk(Some(category.as_bytes()), output);
                }
            } else if !Store::acl_categories()
                .iter()
                .any(|category| arguments[1].eq_ignore_ascii_case(category.as_bytes()))
            {
                output.extend_from_slice(b"-ERR Unknown category name in ACL\r\n");
            } else {
                let commands = Store::acl_commands(arguments[1]);
                write!(output, "*{}\r\n", commands.len()).expect("Vec write");
                for command in commands {
                    encode_bulk(Some(command.as_bytes()), output);
                }
            }
        } else if subcommand.eq_ignore_ascii_case(b"GETUSER") {
            if arguments.len() != 2 {
                arity("acl|getuser", output);
            } else if let Some(details) = store.acl_details(arguments[1]) {
                write!(output, "*12\r\n").expect("Vec write");
                encode_bulk(Some(b"flags"), output);
                write!(output, "*{}\r\n", details.flags.len()).expect("Vec write");
                for flag in details.flags {
                    encode_bulk(Some(flag.as_bytes()), output);
                }
                encode_bulk(Some(b"passwords"), output);
                write!(output, "*{}\r\n", details.passwords.len()).expect("Vec write");
                for password in details.passwords {
                    encode_bulk(Some(password.as_bytes()), output);
                }
                encode_bulk(Some(b"commands"), output);
                encode_bulk(Some(details.commands.as_bytes()), output);
                encode_bulk(Some(b"keys"), output);
                encode_bulk(Some(details.keys.as_bytes()), output);
                encode_bulk(Some(b"channels"), output);
                encode_bulk(Some(details.channels.as_bytes()), output);
                encode_bulk(Some(b"selectors"), output);
                output.extend_from_slice(b"*0\r\n");
            } else {
                output.extend_from_slice(b"$-1\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"SETUSER") {
            if arguments.len() < 2 {
                arity("acl|setuser", output);
            } else {
                match store.acl_setuser(arguments[1], &arguments[2..]) {
                    Ok(()) => output.extend_from_slice(b"+OK\r\n"),
                    Err(error) => acl_error(error, output),
                }
            }
        } else if subcommand.eq_ignore_ascii_case(b"DELUSER") {
            if arguments.len() < 2 {
                arity("acl|deluser", output);
            } else {
                match store.acl_deluser(&arguments[1..]) {
                    Ok(removed) => {
                        let _ = write!(output, ":{removed}\r\n");
                    }
                    Err(error) => acl_error(error, output),
                }
            }
        } else if subcommand.eq_ignore_ascii_case(b"SAVE") {
            if arguments.len() != 1 {
                arity("acl|save", output);
            } else {
                match store.acl_save() {
                    Ok(()) => output.extend_from_slice(b"+OK\r\n"),
                    Err(error) => acl_error(error, output),
                }
            }
        } else if subcommand.eq_ignore_ascii_case(b"GENPASS") {
            if arguments.len() > 2 {
                arity("acl|genpass", output);
            } else {
                let bits = if arguments.len() == 1 {
                    256
                } else {
                    match parse_client_id(arguments[1]).filter(|bits| (1..=1024).contains(bits)) {
                        Some(bits) => bits as usize,
                        None => {
                            output.extend_from_slice(b"-ERR GENPASS bits must be 1..=1024\r\n");
                            return;
                        }
                    }
                };
                match store.generate_password(bits) {
                    Some(password) => encode_bulk(Some(password.as_bytes()), output),
                    None => output.extend_from_slice(b"-ERR secure random source unavailable\r\n"),
                }
            }
        } else if subcommand.eq_ignore_ascii_case(b"DRYRUN") {
            if arguments.len() < 3 {
                arity("acl|dryrun", output);
            } else {
                match store.acl_dryrun(arguments[1], &arguments[2..]) {
                    Some(Access::Allowed) => output.extend_from_slice(b"+OK\r\n"),
                    Some(Access::DeniedCommand) => {
                        let name = command_name(arguments[2]);
                        let _ = write!(
                            output,
                            "-NOPERM this user has no permissions to run the '{name}' command\r\n"
                        );
                    }
                    Some(Access::DeniedKey(_)) => output.extend_from_slice(
                        b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
                    ),
                    Some(Access::DeniedChannel(_)) => output.extend_from_slice(
                        b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n",
                    ),
                    Some(Access::NoAuth) => {
                        output.extend_from_slice(b"-NOAUTH Authentication required.\r\n")
                    }
                    None => output.extend_from_slice(b"-ERR User not found\r\n"),
                }
            }
        } else if subcommand.eq_ignore_ascii_case(b"LOG") {
            if arguments.len() == 2 && arguments[1].eq_ignore_ascii_case(b"RESET") {
                if store.reset_acl_log() {
                    output.extend_from_slice(b"+OK\r\n");
                } else {
                    output.extend_from_slice(b"-ERR internal authentication error\r\n");
                }
            } else {
                let count = if arguments.len() == 1 {
                    10
                } else if arguments.len() == 2 {
                    match parse_client_id(arguments[1]).filter(|count| *count > 0) {
                        Some(count) => count as usize,
                        None => {
                            output.extend_from_slice(b"-ERR invalid ACL LOG count\r\n");
                            return;
                        }
                    }
                } else {
                    arity("acl|log", output);
                    return;
                };
                let entries = store.acl_log(count);
                let _ = write!(output, "*{}\r\n", entries.len());
                for entry in entries {
                    output.extend_from_slice(b"*6\r\n");
                    let _ = write!(output, ":{}", entry.count);
                    output.extend_from_slice(b"\r\n");
                    for value in [
                        &entry.reason,
                        &entry.context,
                        &entry.object,
                        &entry.username,
                    ] {
                        encode_bulk(Some(value), output);
                    }
                    let _ = write!(output, ":{}", entry.age_seconds());
                    output.extend_from_slice(b"\r\n");
                }
            }
        } else if subcommand.eq_ignore_ascii_case(b"LOAD") {
            if arguments.len() != 1 {
                arity("acl|load", output);
            } else {
                match store.acl_load() {
                    Ok(()) => output.extend_from_slice(b"+OK\r\n"),
                    Err(error) => acl_error(error, output),
                }
            }
        } else {
            output.extend_from_slice(b"-ERR unsupported ACL subcommand or invalid arguments\r\n");
        }
    }

    /// Attach the bounded message channel used by this connection's pub/sub hub.
    pub fn set_pubsub_sender(&mut self, sender: SyncSender<Vec<u8>>) {
        self.pubsub_sender = Some(sender);
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn is_subscribed(&self) -> bool {
        !self.pubsub_channels.is_empty()
            || !self.pubsub_patterns.is_empty()
            || !self.pubsub_shard_channels.is_empty()
    }

    fn subscription_count(&self) -> usize {
        self.pubsub_channels.len() + self.pubsub_patterns.len()
    }

    fn shard_subscription_count(&self) -> usize {
        self.pubsub_shard_channels.len()
    }

    fn refresh_pubsub_protocol(&self, store: &Store) {
        if self.is_subscribed() {
            store.pubsub_set_resp3(self.id, self.protocol == Protocol::Resp3);
        }
    }

    fn subscribe(&mut self, store: &Store, names: &[&[u8]], pattern: bool, output: &mut Vec<u8>) {
        let command = if pattern { "psubscribe" } else { "subscribe" };
        if names.is_empty() {
            arity(command, output);
            return;
        }
        let Some(sender) = self.pubsub_sender.clone() else {
            output.extend_from_slice(b"-ERR pub/sub is unavailable on this connection\r\n");
            return;
        };
        for name in names {
            if pattern {
                self.pubsub_patterns.insert(name.to_vec());
            } else {
                self.pubsub_channels.insert(name.to_vec());
            }
            store.pubsub_subscribe(
                pattern,
                name,
                self.id,
                sender.clone(),
                self.protocol == Protocol::Resp3,
            );
            self.subscription_reply(command, Some(name), output);
        }
    }

    fn subscribe_shard(&mut self, store: &Store, names: &[&[u8]], output: &mut Vec<u8>) {
        if names.is_empty() {
            arity("ssubscribe", output);
            return;
        }
        let Some(sender) = self.pubsub_sender.clone() else {
            output.extend_from_slice(b"-ERR pub/sub is unavailable on this connection\r\n");
            return;
        };
        for name in names {
            self.pubsub_shard_channels.insert(name.to_vec());
            store.pubsub_subscribe_shard(
                name,
                self.id,
                sender.clone(),
                self.protocol == Protocol::Resp3,
            );
            self.shard_subscription_reply("ssubscribe", Some(name), output);
        }
    }

    fn unsubscribe_shard(&mut self, store: &Store, names: &[&[u8]], output: &mut Vec<u8>) {
        let local: Vec<Vec<u8>> = if names.is_empty() {
            self.pubsub_shard_channels.iter().cloned().collect()
        } else {
            names.iter().map(|name| name.to_vec()).collect()
        };
        if local.is_empty() {
            self.shard_subscription_reply("sunsubscribe", None, output);
            return;
        }
        for name in &local {
            self.pubsub_shard_channels.remove(name);
            store.pubsub_unsubscribe_shard(name, self.id);
            self.shard_subscription_reply("sunsubscribe", Some(name), output);
        }
    }

    fn shard_subscription_reply(&self, kind: &str, channel: Option<&[u8]>, output: &mut Vec<u8>) {
        output.push(if self.protocol == Protocol::Resp3 {
            b'>'
        } else {
            b'*'
        });
        output.extend_from_slice(b"3\r\n");
        encode_bulk(Some(kind.as_bytes()), output);
        encode_bulk(channel, output);
        let _ = write!(output, ":{}", self.shard_subscription_count());
        output.extend_from_slice(b"\r\n");
    }

    fn unsubscribe(&mut self, store: &Store, names: &[&[u8]], pattern: bool, output: &mut Vec<u8>) {
        let command = if pattern {
            "punsubscribe"
        } else {
            "unsubscribe"
        };
        let local: Vec<Vec<u8>> = if names.is_empty() {
            if pattern {
                self.pubsub_patterns.iter().cloned().collect()
            } else {
                self.pubsub_channels.iter().cloned().collect()
            }
        } else {
            names.iter().map(|name| name.to_vec()).collect()
        };
        if local.is_empty() {
            self.subscription_reply(command, None, output);
            return;
        }
        for name in &local {
            if pattern {
                self.pubsub_patterns.remove(name);
            } else {
                self.pubsub_channels.remove(name);
            }
            store.pubsub_unsubscribe(pattern, name, self.id);
            self.subscription_reply(command, Some(name), output);
        }
    }

    fn subscription_reply(&self, kind: &str, channel: Option<&[u8]>, output: &mut Vec<u8>) {
        output.push(if self.protocol == Protocol::Resp3 {
            b'>'
        } else {
            b'*'
        });
        output.extend_from_slice(b"3\r\n");
        encode_bulk(Some(kind.as_bytes()), output);
        encode_bulk(channel, output);
        let _ = write!(output, ":{}", self.subscription_count());
        output.extend_from_slice(b"\r\n");
    }

    fn publish(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 3 {
            arity("publish", output);
            return;
        }
        let delivered = store.publish(arguments[1], arguments[2]);
        let _ = write!(output, ":{delivered}\r\n");
    }

    fn spublish(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 3 {
            arity("spublish", output);
            return;
        }
        let delivered = store.publish_shard(arguments[1], arguments[2]);
        let _ = write!(output, ":{delivered}\r\n");
    }

    fn pubsub(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let Some(subcommand) = arguments.first() else {
            arity("pubsub", output);
            return;
        };
        if subcommand.eq_ignore_ascii_case(b"CHANNELS") {
            if arguments.len() > 2 {
                arity("pubsub|channels", output);
                return;
            }
            let names = store.pubsub_channels(arguments.get(1).copied());
            let _ = write!(output, "*{}\r\n", names.len());
            for name in names {
                encode_bulk(Some(&name), output);
            }
        } else if subcommand.eq_ignore_ascii_case(b"NUMSUB") {
            let counts = store.pubsub_numsub(&arguments[1..]);
            let _ = write!(output, "*{}\r\n", counts.len() * 2);
            for (channel, count) in counts {
                encode_bulk(Some(&channel), output);
                let _ = write!(output, ":{count}\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"NUMPAT") {
            if arguments.len() != 1 {
                arity("pubsub|numpat", output);
            } else {
                let _ = write!(output, ":{}", store.pubsub_numpat());
                output.extend_from_slice(b"\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"SHARDCHANNELS") {
            if arguments.len() > 2 {
                output.extend_from_slice(
                    b"-ERR unknown subcommand or wrong number of arguments for 'shardchannels'. Try PUBSUB HELP.\r\n",
                );
                return;
            }
            let names = store.pubsub_shard_channels(arguments.get(1).copied());
            let _ = write!(output, "*{}\r\n", names.len());
            for name in names {
                encode_bulk(Some(&name), output);
            }
        } else if subcommand.eq_ignore_ascii_case(b"SHARDNUMSUB") {
            let counts = store.pubsub_shard_numsub(&arguments[1..]);
            let _ = write!(output, "*{}\r\n", counts.len() * 2);
            for (channel, count) in counts {
                encode_bulk(Some(&channel), output);
                let _ = write!(output, ":{count}\r\n");
            }
        } else {
            output.extend_from_slice(b"-ERR unsupported PUBSUB subcommand\r\n");
        }
    }

    fn blocking(&mut self, store: &Store, arguments: &[&[u8]], left: bool, output: &mut Vec<u8>) {
        let name = if left { "blpop" } else { "brpop" };
        if arguments.len() < 2 {
            arity(name, output);
            return;
        }
        let (timeout, keys) = arguments.split_last().expect("nonempty");
        let Some(timeout) = std::str::from_utf8(timeout)
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        else {
            output.extend_from_slice(b"-ERR timeout is not a float or out of range\r\n");
            return;
        };
        let keys: Vec<Vec<u8>> = keys.iter().map(|key| key.to_vec()).collect();
        match store.blocking_pop(&keys.iter().map(Vec::as_slice).collect::<Vec<_>>(), left) {
            PopOutcome::Empty => {}
            outcome => {
                blocking_reply(outcome, output);
                return;
            }
        }
        let deadline = if timeout == 0.0 {
            None
        } else {
            let Some(duration) = Duration::try_from_secs_f64(timeout).ok() else {
                output.extend_from_slice(b"-ERR timeout is out of range\r\n");
                return;
            };
            Instant::now().checked_add(duration)
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys,
            deadline,
            kind: BlockedKind::List { left },
        });
        self.sync_blocked(store);
    }

    fn blocking_lmpop(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() < 4 {
            arity("blmpop", output);
            return;
        }
        let timeout = match parse_bzmpop_timeout(arguments[0]) {
            Ok(timeout) => timeout,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        let options = match parse_list_mpop(arguments, 1) {
            Ok(options) => options,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        match store.blocking_lmpop(&options.keys, options.left, options.count) {
            ListPopOutcome::Empty => {}
            outcome => {
                blocking_lmpop_reply(outcome, output);
                return;
            }
        }
        let deadline = if timeout == 0.0 {
            None
        } else {
            let Some(duration) = Duration::try_from_secs_f64(timeout).ok() else {
                output.extend_from_slice(b"-ERR timeout is out of range\r\n");
                return;
            };
            Instant::now().checked_add(duration)
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: options.keys.iter().map(|key| key.to_vec()).collect(),
            deadline,
            kind: BlockedKind::ListMulti {
                left: options.left,
                count: options.count,
            },
        });
        self.sync_blocked(store);
    }

    fn blocking_list_move(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let blmove = arguments[0].eq_ignore_ascii_case(b"BLMOVE");
        let (source, destination, from_left, to_left, timeout_raw) = if blmove {
            if arguments.len() != 6 {
                arity("blmove", output);
                return;
            }
            let (Some(from_left), Some(to_left)) =
                (parse_side(arguments[3]), parse_side(arguments[4]))
            else {
                output.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            };
            (arguments[1], arguments[2], from_left, to_left, arguments[5])
        } else {
            if arguments.len() != 4 {
                arity("brpoplpush", output);
                return;
            }
            (arguments[1], arguments[2], false, true, arguments[3])
        };
        let Some(timeout) = std::str::from_utf8(timeout_raw)
            .ok()
            .and_then(|text| text.parse::<f64>().ok())
            .filter(|seconds| seconds.is_finite())
        else {
            output.extend_from_slice(b"-ERR timeout is not a float or out of range\r\n");
            return;
        };
        if timeout < 0.0 {
            output.extend_from_slice(b"-ERR timeout is negative\r\n");
            return;
        }
        match store.blocking_list_move(source, destination, from_left, to_left) {
            ListMoveOutcome::Empty => {}
            outcome => {
                list_move_reply(outcome, output);
                return;
            }
        }
        let deadline = if timeout == 0.0 {
            None
        } else {
            let Some(duration) = Duration::try_from_secs_f64(timeout).ok() else {
                output.extend_from_slice(b"-ERR timeout is out of range\r\n");
                return;
            };
            Instant::now().checked_add(duration)
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: vec![source.to_vec(), destination.to_vec()],
            deadline,
            kind: BlockedKind::ListMove {
                source: source.to_vec(),
                destination: destination.to_vec(),
                from_left,
                to_left,
            },
        });
        self.sync_blocked(store);
    }

    fn blocking_zpop(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() < 4 {
            arity("bzmpop", output);
            return;
        }
        let timeout = match parse_bzmpop_timeout(arguments[0]) {
            Ok(timeout) => timeout,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        let options = match parse_mpop(arguments, 1) {
            Ok(options) => options,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        match store.blocking_zpop(&options.keys, options.max, options.count) {
            ZSetPopOutcome::Empty => {}
            outcome => {
                blocking_zpop_reply(outcome, output);
                return;
            }
        }
        let deadline = if timeout == 0.0 {
            None
        } else {
            let Some(duration) = Duration::try_from_secs_f64(timeout).ok() else {
                output.extend_from_slice(b"-ERR timeout is out of range\r\n");
                return;
            };
            Instant::now().checked_add(duration)
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: options.keys.iter().map(|key| key.to_vec()).collect(),
            deadline,
            kind: BlockedKind::ZSet {
                max: options.max,
                count: options.count,
            },
        });
        self.sync_blocked(store);
    }

    fn xread(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let options = match parse_xread(arguments) {
            Ok(options) => options,
            Err(error) => {
                output.extend_from_slice(error);
                return;
            }
        };
        let Some((starts, outcome)) = store.xread_start(&options) else {
            output.extend_from_slice(b"-ERR internal store error\r\n");
            return;
        };
        let Some(block) = options.block else {
            xread_reply(outcome, output);
            return;
        };
        match outcome {
            XReadOutcome::Empty => {}
            outcome => {
                xread_reply(outcome, output);
                return;
            }
        }
        let deadline = if block == 0 {
            None
        } else {
            Instant::now().checked_add(Duration::from_millis(block))
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: options.keys.iter().map(|key| key.to_vec()).collect(),
            deadline,
            kind: BlockedKind::XRead {
                starts,
                count: options.count,
            },
        });
        self.sync_blocked(store);
    }

    fn blocking_xreadgroup(
        &mut self,
        store: &Store,
        options: &XReadGroupOptions<'_>,
        output: &mut Vec<u8>,
    ) {
        match store.blocking_xreadgroup(options) {
            XReadGroupOutcome::Empty => {}
            outcome => {
                xreadgroup_reply(outcome, options.group, output);
                return;
            }
        }
        let block = options.block.unwrap_or(0);
        let deadline = if block == 0 {
            None
        } else {
            Instant::now().checked_add(Duration::from_millis(block))
        };
        self.blocked = Some(Blocked {
            slowlog: None,
            keys: options.keys.iter().map(|key| key.to_vec()).collect(),
            deadline,
            kind: BlockedKind::XReadGroup {
                group: options.group.to_vec(),
                consumer: options.consumer.to_vec(),
                count: options.count,
                noack: options.noack,
            },
        });
        self.sync_blocked(store);
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked.is_some()
    }

    /// Poll a blocked pop. Returns one complete reply when it succeeded, failed or
    /// timed out, or `None` while the connection must remain blocked.
    pub fn poll_blocked(&mut self, store: &Store) -> Option<Vec<u8>> {
        let blocked = self.blocked.as_ref()?;
        let keys: Vec<&[u8]> = blocked.keys.iter().map(Vec::as_slice).collect();
        let kind = blocked.kind.clone();
        let expired = blocked
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline);
        let mut reply = Vec::new();
        let ready = match kind {
            BlockedKind::List { left } => match store.blocking_pop(&keys, left) {
                PopOutcome::Empty => false,
                outcome => {
                    blocking_reply(outcome, &mut reply);
                    true
                }
            },
            BlockedKind::ListMulti { left, count } => {
                match store.blocking_lmpop(&keys, left, count) {
                    ListPopOutcome::Empty => false,
                    outcome => {
                        blocking_lmpop_reply(outcome, &mut reply);
                        true
                    }
                }
            }
            BlockedKind::ListMove {
                source,
                destination,
                from_left,
                to_left,
            } => match store.blocking_list_move(&source, &destination, from_left, to_left) {
                ListMoveOutcome::Empty => false,
                outcome => {
                    list_move_reply(outcome, &mut reply);
                    true
                }
            },
            BlockedKind::ZSet { max, count } => match store.blocking_zpop(&keys, max, count) {
                ZSetPopOutcome::Empty => false,
                outcome => {
                    blocking_zpop_reply(outcome, &mut reply);
                    true
                }
            },
            BlockedKind::XRead { starts, count } => match store.xread_once(&keys, &starts, count) {
                Some(XReadOutcome::Empty) | None => false,
                Some(outcome) => {
                    xread_reply(outcome, &mut reply);
                    true
                }
            },
            BlockedKind::XReadGroup {
                group,
                consumer,
                count,
                noack,
            } => {
                let options = XReadGroupOptions {
                    count,
                    block: None,
                    noack,
                    group: &group,
                    consumer: &consumer,
                    keys: keys.clone(),
                    starts: vec![XReadGroupStart::New; keys.len()],
                };
                match store.blocking_xreadgroup(&options) {
                    XReadGroupOutcome::Empty => false,
                    outcome => {
                        xreadgroup_reply(outcome, &group, &mut reply);
                        true
                    }
                }
            }
        };
        if !ready && !expired {
            return None;
        }
        let slowlog = self
            .blocked
            .as_mut()
            .and_then(|blocked| blocked.slowlog.take());
        self.blocked = None;
        self.sync_blocked(store);
        if !ready {
            reply.extend_from_slice(b"*-1\r\n");
        }
        if self.protocol == Protocol::Resp3 {
            reply = convert_resp2_reply(&reply);
        }
        if let Some((started, arguments)) = slowlog {
            let borrowed: Vec<&[u8]> = arguments.iter().map(Vec::as_slice).collect();
            self.record_slowlog(store, &borrowed, started);
        }
        Some(reply)
    }

    fn subscriber_ping(&self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() > 1 {
            arity("ping", output);
            return;
        }
        output.extend_from_slice(b"*2\r\n$4\r\npong\r\n");
        encode_bulk(Some(arguments.first().copied().unwrap_or(b"")), output);
    }

    fn hello(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let (requested, options) = match arguments.split_first() {
            None => (self.protocol, &[][..]),
            Some((&version, rest)) => {
                let protocol = match version {
                    b"2" => Protocol::Resp2,
                    b"3" => Protocol::Resp3,
                    _ => {
                        output.extend_from_slice(b"-NOPROTO unsupported protocol version\r\n");
                        return;
                    }
                };
                (protocol, rest)
            }
        };
        if arguments.is_empty() {
            self.hello_reply(requested, output);
            return;
        }

        // Parse and validate every option before changing either protocol,
        // metadata or authentication state.
        let mut name = None;
        let mut credentials = None;
        let mut index = 0;
        while index < options.len() {
            if options[index].eq_ignore_ascii_case(b"AUTH") {
                if credentials.is_some() || index + 2 >= options.len() {
                    output.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                credentials = Some((options[index + 1], options[index + 2]));
                index += 3;
                continue;
            }
            if options[index].eq_ignore_ascii_case(b"SETNAME") {
                if name.is_some()
                    || index + 1 >= options.len()
                    || !valid_metadata(options[index + 1], MAX_NAME_BYTES)
                {
                    output.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                name = Some(options[index + 1]);
                index += 2;
                continue;
            }
            output.extend_from_slice(b"-ERR syntax error\r\n");
            return;
        }
        if let Some((username, password)) = credentials {
            match store.authenticate_user(Some(username), password) {
                Ok(name) => self.authenticated_user = Some(name),
                Err(AuthError::NoPassword) => {
                    output.extend_from_slice(
                        b"-ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?\r\n",
                    );
                    return;
                }
                Err(AuthError::WrongPassword) => {
                    output.extend_from_slice(
                        b"-WRONGPASS invalid username-password pair or user is disabled.\r\n",
                    );
                    return;
                }
                Err(AuthError::Internal) => {
                    output.extend_from_slice(b"-ERR internal authentication error\r\n");
                    return;
                }
            }
        }
        self.protocol = requested;
        if let Some(name) = name {
            self.set_name(name);
        }
        self.hello_reply(requested, output);
    }

    fn hello_reply(&self, protocol: Protocol, output: &mut Vec<u8>) {
        // These fields identify this server without claiming a Redis version,
        // modules, replication role, or unsupported capability.
        enum Value<'a> {
            Bulk(&'a [u8]),
            Integer(u64),
        }
        let fields = [
            (b"server".as_slice(), Value::Bulk(b"rillcache")),
            (
                b"version".as_slice(),
                Value::Bulk(env!("CARGO_PKG_VERSION").as_bytes()),
            ),
            (
                b"proto".as_slice(),
                Value::Integer(if protocol == Protocol::Resp3 { 3 } else { 2 }),
            ),
            (b"id".as_slice(), Value::Integer(self.id)),
            (b"mode".as_slice(), Value::Bulk(b"standalone")),
        ];
        if protocol == Protocol::Resp3 {
            output.extend_from_slice(b"%5\r\n");
        } else {
            output.extend_from_slice(b"*10\r\n");
        }
        for (key, value) in fields {
            encode_bulk(Some(key), output);
            match value {
                Value::Bulk(value) => encode_bulk(Some(value), output),
                Value::Integer(value) => {
                    let _ = write!(output, ":{value}\r\n");
                }
            }
        }
    }

    fn log_denial(&self, store: &Store, reason: &str, context: &str, object: &[u8]) {
        let user = self
            .authenticated_user
            .clone()
            .unwrap_or_else(|| b"default".to_vec());
        store.record_acl_denial(reason, context, object, &user);
    }

    fn sync_client_metadata(&self, store: &Store) {
        let user = store.whoami(self.authenticated_user.as_deref());
        store.update_client_metadata(
            self.id,
            self.name.as_deref(),
            self.library_name.as_deref(),
            self.library_version.as_deref(),
            self.protocol == Protocol::Resp3,
            user.as_deref(),
        );
    }

    fn client(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let Some((subcommand, rest)) = arguments.split_first() else {
            output.extend_from_slice(b"-ERR wrong number of arguments for 'client' command\r\n");
            return;
        };
        if subcommand.eq_ignore_ascii_case(b"ID") && rest.is_empty() {
            let _ = write!(output, ":{}\r\n", self.id);
        } else if subcommand.eq_ignore_ascii_case(b"SETNAME") && rest.len() == 1 {
            if !valid_metadata(rest[0], MAX_NAME_BYTES) {
                output.extend_from_slice(b"-ERR invalid client name\r\n");
            } else {
                self.set_name(rest[0]);
                output.extend_from_slice(b"+OK\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"GETNAME") && rest.is_empty() {
            if self.protocol == Protocol::Resp3 && self.name.is_none() {
                output.extend_from_slice(b"_\r\n");
            } else {
                encode_bulk(self.name.as_deref(), output);
            }
        } else if subcommand.eq_ignore_ascii_case(b"SETINFO") && rest.len() == 2 {
            let target = if rest[0].eq_ignore_ascii_case(b"LIB-NAME") {
                &mut self.library_name
            } else if rest[0].eq_ignore_ascii_case(b"LIB-VER") {
                &mut self.library_version
            } else {
                output.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            };
            if !valid_metadata(rest[1], MAX_LIBRARY_FIELD_BYTES) {
                output.extend_from_slice(b"-ERR invalid client metadata\r\n");
            } else {
                *target = Some(rest[1].to_vec());
                output.extend_from_slice(b"+OK\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"PAUSE") {
            if rest.is_empty() || rest.len() > 2 {
                arity("client|pause", output);
                return;
            }
            let Some(timeout) = std::str::from_utf8(rest[0])
                .ok()
                .and_then(|text| text.parse::<i64>().ok())
            else {
                output.extend_from_slice(b"-ERR timeout is not an integer or out of range\r\n");
                return;
            };
            if timeout < 0 {
                output.extend_from_slice(b"-ERR timeout is negative\r\n");
                return;
            }
            let writes_only = match rest.get(1) {
                None => false,
                Some(mode) if mode.eq_ignore_ascii_case(b"WRITE") => true,
                Some(mode) if mode.eq_ignore_ascii_case(b"ALL") => false,
                Some(_) => {
                    output.extend_from_slice(b"-ERR CLIENT PAUSE mode must be WRITE or ALL\r\n");
                    return;
                }
            };
            store.pause_clients(timeout, writes_only);
            output.extend_from_slice(b"+OK\r\n");
        } else if subcommand.eq_ignore_ascii_case(b"UNPAUSE") {
            if !rest.is_empty() {
                arity("client|unpause", output);
                return;
            }
            store.unpause_clients();
            output.extend_from_slice(b"+OK\r\n");
        } else if subcommand.eq_ignore_ascii_case(b"LIST") {
            self.client_list(store, rest, output);
        } else if subcommand.eq_ignore_ascii_case(b"KILL") {
            self.client_kill(store, rest, output);
        } else if subcommand.eq_ignore_ascii_case(b"INFO") && rest.is_empty() {
            let mut info = format!("id={} db=0", self.id).into_bytes();
            append_info(&mut info, b"name", self.name.as_deref());
            append_info(&mut info, b"lib-name", self.library_name.as_deref());
            append_info(&mut info, b"lib-ver", self.library_version.as_deref());
            if let Some(snapshot) = store.client_snapshot(self.id) {
                append_info(&mut info, b"addr", Some(snapshot.addr.as_bytes()));
                append_info(
                    &mut info,
                    b"resp",
                    Some(if snapshot.resp3 { b"3" } else { b"2" }),
                );
                append_info(&mut info, b"user", Some(&snapshot.user));
            }
            encode_bulk(Some(&info), output);
        } else {
            output
                .extend_from_slice(b"-ERR unsupported CLIENT subcommand or invalid arguments\r\n");
        }
    }

    fn client_list(&self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let mut type_filter = None;
        let mut ids: Option<Vec<u64>> = None;
        let mut index = 0;
        while index < arguments.len() {
            if arguments[index].eq_ignore_ascii_case(b"TYPE") {
                let Some(value) = arguments.get(index + 1) else {
                    output.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                };
                if value.eq_ignore_ascii_case(b"normal") {
                    type_filter = Some(false);
                } else if value.eq_ignore_ascii_case(b"pubsub") {
                    type_filter = Some(true);
                } else {
                    output.extend_from_slice(b"-ERR unknown client type\r\n");
                    return;
                }
                index += 2;
            } else if arguments[index].eq_ignore_ascii_case(b"ID") {
                let mut parsed = Vec::new();
                for raw in &arguments[index + 1..] {
                    let Some(id) = parse_client_id(raw) else {
                        output.extend_from_slice(b"-ERR invalid client ID\r\n");
                        return;
                    };
                    parsed.push(id);
                }
                ids = Some(parsed);
                index = arguments.len();
            } else {
                output.extend_from_slice(b"-ERR syntax error\r\n");
                return;
            }
        }
        let snapshots = store.client_snapshots(type_filter);
        let mut text = String::new();
        for snapshot in snapshots {
            if ids.as_ref().is_some_and(|ids| !ids.contains(&snapshot.id)) {
                continue;
            }
            let name = snapshot
                .name
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            let lib_name = snapshot
                .library_name
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            let lib_version = snapshot
                .library_version
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            let user = String::from_utf8_lossy(&snapshot.user);
            let mut flags = String::new();
            if snapshot.blocked {
                flags.push('b');
            }
            flags.push(if snapshot.sub + snapshot.psub > 0 {
                'P'
            } else {
                'N'
            });
            use std::fmt::Write as _;
            let _ = writeln!(
                text,
                "id={} addr={} name={} age={} flags={} db=0 sub={} psub={} resp={} user={} lib-name={} lib-ver={}",
                snapshot.id,
                snapshot.addr,
                name,
                snapshot.age_seconds,
                flags,
                snapshot.sub,
                snapshot.psub,
                if snapshot.resp3 { 3 } else { 2 },
                user,
                lib_name,
                lib_version
            );
        }
        encode_bulk(Some(text.as_bytes()), output);
    }

    fn client_kill(&self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.is_empty() {
            arity("client|kill", output);
            return;
        }
        let legacy = arguments.len() == 1;
        let mut addr_filter: Option<Vec<u8>> = None;
        let mut id_filter = None;
        let mut type_filter = None;
        let mut user_filter: Option<Vec<u8>> = None;
        let mut max_age = None;
        if legacy {
            addr_filter = Some(arguments[0].to_vec());
        } else {
            let mut index = 0;
            while index < arguments.len() {
                let Some(value) = arguments.get(index + 1) else {
                    output.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                };
                if arguments[index].eq_ignore_ascii_case(b"ID") {
                    let Some(id) = parse_client_id(value) else {
                        output.extend_from_slice(b"-ERR invalid client ID\r\n");
                        return;
                    };
                    id_filter = Some(id);
                } else if arguments[index].eq_ignore_ascii_case(b"ADDR") {
                    addr_filter = Some(value.to_vec());
                } else if arguments[index].eq_ignore_ascii_case(b"TYPE") {
                    if value.eq_ignore_ascii_case(b"normal") {
                        type_filter = Some(false);
                    } else if value.eq_ignore_ascii_case(b"pubsub") {
                        type_filter = Some(true);
                    } else {
                        output.extend_from_slice(b"-ERR unknown client type\r\n");
                        return;
                    }
                } else if arguments[index].eq_ignore_ascii_case(b"USER") {
                    user_filter = Some(value.to_vec());
                } else if arguments[index].eq_ignore_ascii_case(b"MAXAGE") {
                    let Some(age) = std::str::from_utf8(value)
                        .ok()
                        .and_then(|value| value.parse::<u64>().ok())
                    else {
                        output.extend_from_slice(b"-ERR invalid client MAXAGE\r\n");
                        return;
                    };
                    max_age = Some(age);
                } else if arguments[index].eq_ignore_ascii_case(b"SKIPME") {
                    if !value.eq_ignore_ascii_case(b"yes") {
                        output
                            .extend_from_slice(b"-ERR CLIENT KILL SKIPME no is not supported\r\n");
                        return;
                    }
                } else {
                    output.extend_from_slice(b"-ERR syntax error\r\n");
                    return;
                }
                index += 2;
            }
        }
        let mut ids = Vec::new();
        for snapshot in store.client_snapshots(type_filter) {
            if id_filter.is_some_and(|id| snapshot.id != id) {
                continue;
            }
            if addr_filter
                .as_deref()
                .is_some_and(|addr| snapshot.addr.as_bytes() != addr)
            {
                continue;
            }
            if user_filter
                .as_deref()
                .is_some_and(|user| snapshot.user.as_slice() != user)
            {
                continue;
            }
            if max_age.is_some_and(|age| snapshot.age_ms < age) {
                continue;
            }
            ids.push(snapshot.id);
        }
        let killed = store.client_kill_ids(&ids, self.id);
        if legacy {
            if killed == 0 {
                output.extend_from_slice(b"-ERR No such client\r\n");
            } else {
                output.extend_from_slice(b"+OK\r\n");
            }
        } else {
            let _ = write!(output, ":{killed}\r\n");
        }
    }

    /// Disconnect-equivalent server-side context reset (Redis RESET).
    fn reset(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        if arguments.len() != 1 {
            arity("reset", output);
            return Action::Continue;
        }
        self.multi = None;
        self.watched = WatchState::default();
        self.blocked = None;
        self.sync_blocked(store);
        for channel in std::mem::take(&mut self.pubsub_channels) {
            store.pubsub_unsubscribe(false, &channel, self.id);
        }
        for pattern in std::mem::take(&mut self.pubsub_patterns) {
            store.pubsub_unsubscribe(true, &pattern, self.id);
        }
        for channel in std::mem::take(&mut self.pubsub_shard_channels) {
            store.pubsub_unsubscribe_shard(&channel, self.id);
        }
        self.protocol = Protocol::Resp2;
        self.authenticated_user = None;
        self.sync_client_metadata(store);
        output.extend_from_slice(b"+RESET\r\n");
        Action::Continue
    }

    fn set_name(&mut self, name: &[u8]) {
        self.name = (!name.is_empty()).then(|| name.to_vec());
    }
}

fn arity(command: &str, output: &mut Vec<u8>) {
    let _ = write!(
        output,
        "-ERR wrong number of arguments for '{command}' command\r\n"
    );
}

fn is_auth_exempt(command: &[u8]) -> bool {
    [b"AUTH".as_slice(), b"HELLO", b"QUIT", b"RESET"]
        .iter()
        .any(|name| command.eq_ignore_ascii_case(name))
}

fn is_session_command(command: &[u8]) -> bool {
    [
        b"ACL".as_slice(),
        b"AUTH",
        b"BLPOP",
        b"BRPOP",
        b"CLIENT",
        b"HELLO",
        b"PSUBSCRIBE",
        b"PUBLISH",
        b"PUBSUB",
        b"PUNSUBSCRIBE",
        b"SELECT",
        b"SUBSCRIBE",
        b"UNSUBSCRIBE",
    ]
    .iter()
    .any(|name| command.eq_ignore_ascii_case(name))
}

fn allows_subscriber_command(command: &[u8]) -> bool {
    [
        b"PING".as_slice(),
        b"PSUBSCRIBE",
        b"PUNSUBSCRIBE",
        b"QUIT",
        b"RESET",
        b"SSUBSCRIBE",
        b"SUBSCRIBE",
        b"SUNSUBSCRIBE",
        b"UNSUBSCRIBE",
    ]
    .iter()
    .any(|name| command.eq_ignore_ascii_case(name))
}

fn acl_error(error: AclError, output: &mut Vec<u8>) {
    match error {
        AclError::InvalidName => output.extend_from_slice(b"-ERR Invalid ACL user name\r\n"),
        AclError::InvalidPassword => {
            output.extend_from_slice(b"-ERR Invalid password or password hash\r\n")
        }
        AclError::UnknownCommand | AclError::UnknownCategory => {
            output.extend_from_slice(b"-ERR Unknown command or category name in ACL\r\n")
        }
        AclError::ProtectedDefault => {
            output.extend_from_slice(b"-ERR The 'default' user cannot be removed\r\n")
        }
        AclError::NoAclFile => {
            output.extend_from_slice(b"-ERR This instance has no ACL file configured\r\n")
        }
        AclError::MissingDefault => {
            output.extend_from_slice(b"-ERR ACL file must define an enabled default user\r\n")
        }
        AclError::Syntax | AclError::UnknownRule => {
            output.extend_from_slice(b"-ERR Error in ACL SETUSER modifier\r\n")
        }
        AclError::TooLarge => {
            output.extend_from_slice(b"-ERR ACL file or rule set exceeds configured bounds\r\n")
        }
        AclError::Io(_) => output.extend_from_slice(b"-ERR ACL file I/O error\r\n"),
    }
}

fn parse_client_id(raw: &[u8]) -> Option<u64> {
    if raw.is_empty() || !raw.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(raw).ok()?.parse().ok()
}

fn command_name(command: &[u8]) -> String {
    std::str::from_utf8(command)
        .ok()
        .filter(|name| {
            name.len() <= 64
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
        .map_or_else(|| "unknown".to_string(), str::to_ascii_lowercase)
}

fn valid_metadata(bytes: &[u8], maximum: usize) -> bool {
    bytes.len() <= maximum && bytes.iter().all(|byte| (b'!'..=b'~').contains(byte))
}

fn append_info(output: &mut Vec<u8>, key: &[u8], value: Option<&[u8]>) {
    if let Some(value) = value {
        output.push(b' ');
        output.extend_from_slice(key);
        output.push(b'=');
        output.extend_from_slice(value);
    }
}

fn select(arguments: &[&[u8]], output: &mut Vec<u8>) {
    if arguments.len() != 1
        || arguments[0].is_empty()
        || !arguments[0].iter().all(u8::is_ascii_digit)
    {
        output.extend_from_slice(b"-ERR invalid DB index\r\n");
    } else if arguments[0].iter().all(|byte| *byte == b'0') {
        output.extend_from_slice(b"+OK\r\n");
    } else {
        output.extend_from_slice(b"-ERR DB index is out of range\r\n");
    }
}

// Convert RESP2 values by parsing frame boundaries, never by searching payload
// bytes. The command surface emits flat scalar replies; arrays are handled for
// future aggregate children and malformed internal output is left untouched.
fn convert_resp2_reply(reply: &[u8]) -> Vec<u8> {
    let mut converted = Vec::with_capacity(reply.len());
    let mut offset = 0;
    if parse_resp_value(reply, &mut offset, &mut converted) && offset == reply.len() {
        converted
    } else {
        reply.to_vec()
    }
}

fn parse_resp_value(input: &[u8], offset: &mut usize, output: &mut Vec<u8>) -> bool {
    let Some(&prefix) = input.get(*offset) else {
        return false;
    };
    match prefix {
        b'$' => {
            let Some((length, body)) = parse_line_number(input, *offset + 1) else {
                return false;
            };
            if length == -1 {
                output.extend_from_slice(b"_\r\n");
                *offset = body;
                return true;
            }
            if length < 0 {
                return false;
            }
            let Ok(length) = usize::try_from(length) else {
                return false;
            };
            let Some(end) = body.checked_add(length).and_then(|n| n.checked_add(2)) else {
                return false;
            };
            if end > input.len() || &input[end - 2..end] != b"\r\n" {
                return false;
            }
            output.extend_from_slice(&input[*offset..end]);
            *offset = end;
            true
        }
        b'*' => {
            let Some((count, body)) = parse_line_number(input, *offset + 1) else {
                return false;
            };
            if count == -1 {
                output.extend_from_slice(b"_\r\n");
                *offset = body;
                return true;
            }
            if count < 0 {
                return false;
            }
            let Ok(count) = usize::try_from(count) else {
                return false;
            };
            output.extend_from_slice(&input[*offset..body]);
            *offset = body;
            for _ in 0..count {
                if !parse_resp_value(input, offset, output) {
                    return false;
                }
            }
            true
        }
        b'+' | b'-' | b':' => {
            let Some(end) = input[*offset..].windows(2).position(|w| w == b"\r\n") else {
                return false;
            };
            let end = *offset + end + 2;
            output.extend_from_slice(&input[*offset..end]);
            *offset = end;
            true
        }
        _ => false,
    }
}

fn parse_line_number(input: &[u8], start: usize) -> Option<(i64, usize)> {
    let relative = input.get(start..)?.windows(2).position(|w| w == b"\r\n")?;
    let end = start + relative;
    let number = std::str::from_utf8(&input[start..end]).ok()?.parse().ok()?;
    Some((number, end + 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(session: &mut Session, store: &Store, args: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        session.execute(store, args, &mut out);
        out
    }

    #[test]
    fn hello_protocol_switch_and_errors_are_atomic() {
        let store = Store::new(1024);
        let mut session = Session::new();
        let hello = call(&mut session, &store, &[b"HELLO", b"3"]);
        assert!(hello.starts_with(b"%5\r\n"));
        assert!(hello.windows(9).any(|v| v == b"rillcache"));
        assert_eq!(call(&mut session, &store, &[b"GET", b"missing"]), b"_\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"CLIENT", b"GETNAME"]),
            b"_\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"HELLO", b"9"]),
            b"-NOPROTO unsupported protocol version\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"GET", b"missing"]), b"_\r\n");
        assert_eq!(
            call(
                &mut session,
                &store,
                &[b"HELLO", b"2", b"AUTH", b"default", b"pw"]
            ),
            b"-ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"GET", b"missing"]), b"_\r\n");
    }

    #[test]
    fn hello_no_argument_reports_current_protocol_and_setname_is_atomic() {
        let store = Store::new(1024);
        let mut session = Session::new();
        assert!(call(&mut session, &store, &[b"HELLO"]).starts_with(b"*10\r\n"));
        assert!(
            call(
                &mut session,
                &store,
                &[b"HELLO", b"3", b"SETNAME", b"client"]
            )
            .starts_with(b"%5\r\n")
        );
        assert_eq!(
            call(&mut session, &store, &[b"CLIENT", b"GETNAME"]),
            b"$6\r\nclient\r\n"
        );
        assert!(call(&mut session, &store, &[b"HELLO"]).starts_with(b"%5\r\n"));
        assert_eq!(
            call(
                &mut session,
                &store,
                &[b"HELLO", b"2", b"SETNAME", b"bad name"]
            ),
            b"-ERR syntax error\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"GET", b"missing"]),
            b"_\r\n",
            "invalid negotiation must preserve RESP3"
        );
    }

    #[test]
    fn metadata_validation_is_bounded_atomic_and_observable() {
        let store = Store::new(1024);
        let mut session = Session::new();
        assert_eq!(
            call(&mut session, &store, &[b"CLIENT", b"SETNAME", b"client-a"]),
            b"+OK\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"CLIENT", b"SETNAME", b"bad name"]),
            b"-ERR invalid client name\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"CLIENT", b"GETNAME"]),
            b"$8\r\nclient-a\r\n"
        );
        assert_eq!(
            call(
                &mut session,
                &store,
                &[b"CLIENT", b"SETINFO", b"LIB-NAME", b"lib"]
            ),
            b"+OK\r\n"
        );
        assert_eq!(
            call(
                &mut session,
                &store,
                &[b"CLIENT", b"SETINFO", b"LIB-VER", b"1.2"]
            ),
            b"+OK\r\n"
        );
        let info = call(&mut session, &store, &[b"CLIENT", b"INFO"]);
        assert!(info.windows(13).any(|v| v == b"name=client-a"));
        assert!(info.windows(12).any(|v| v == b"lib-name=lib"));
        assert_eq!(
            call(
                &mut session,
                &store,
                &[
                    b"CLIENT",
                    b"SETINFO",
                    b"LIB-VER",
                    &[b'x'; MAX_LIBRARY_FIELD_BYTES + 1]
                ]
            ),
            b"-ERR invalid client metadata\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"CLIENT", b"GETNAME"]),
            b"$8\r\nclient-a\r\n"
        );
    }

    #[test]
    fn resp3_null_conversion_is_framing_aware_for_binary_payloads_and_arrays() {
        let binary = b"$7\r\n$-1\r\n!\r\n";
        let mut reply = Vec::new();
        encode_bulk(Some(binary), &mut reply);
        let original = reply.clone();
        let mut converted = Vec::new();
        assert!(parse_resp_value(&reply, &mut 0, &mut converted));
        assert_eq!(converted, original);
        let mut array = b"*2\r\n$-1\r\n$1\r
x\r\n"
            .to_vec();
        let mut converted = Vec::new();
        assert!(parse_resp_value(&array, &mut 0, &mut converted));
        assert_eq!(
            converted,
            b"*2\r\n_\r\n$1\r
x\r\n"
        );
        array = b"$-1\r\n".to_vec();
        let mut converted = Vec::new();
        assert!(parse_resp_value(&array, &mut 0, &mut converted));
        assert_eq!(converted, b"_\r\n");

        assert_eq!(convert_resp2_reply(b"*-1\r\n"), b"_\r\n");
        let nested = b"*2\r\n*-1\r\n$-1\r\n";
        let mut converted = Vec::new();
        assert!(parse_resp_value(nested, &mut 0, &mut converted));
        assert_eq!(converted, b"*2\r\n_\r\n_\r\n");
    }

    #[test]
    fn select_only_accepts_database_zero_and_ids_are_distinct() {
        let store = Store::new(1024);
        let mut first = Session::new();
        let mut second = Session::new();
        let one = call(&mut first, &store, &[b"CLIENT", b"ID"]);
        let two = call(&mut second, &store, &[b"CLIENT", b"ID"]);
        assert_ne!(one, two);
        assert_eq!(call(&mut first, &store, &[b"SELECT", b"00"]), b"+OK\r\n");
        assert_eq!(
            call(&mut first, &store, &[b"SELECT", b"1"]),
            b"-ERR DB index is out of range\r\n"
        );
    }

    #[test]
    fn multi_queues_then_executes_one_ordered_array_and_clears_state() {
        let store = Store::new(1024);
        let mut session = Session::new();
        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"a", b"1"]),
            b"+QUEUED\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"INCR", b"a"]), b"+QUEUED\r\n");
        assert_eq!(call(&mut session, &store, &[b"GET", b"a"]), b"+QUEUED\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"EXEC"]),
            b"*3\r\n+OK\r\n:2\r\n$1\r\n2\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"GET", b"a"]), b"$1\r\n2\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"EXEC"]),
            b"-ERR EXEC without MULTI\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"DISCARD"]),
            b"-ERR DISCARD without MULTI\r\n"
        );
    }

    #[test]
    fn discard_drops_the_queue_and_watches_without_executing() {
        let store = Store::new(1024);
        let mut session = Session::new();
        let mut other = Session::new();
        assert_eq!(call(&mut session, &store, &[b"WATCH", b"k"]), b"+OK\r\n");
        assert_eq!(
            call(&mut other, &store, &[b"SET", b"k", b"other"]),
            b"+OK\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"k", b"queued"]),
            b"+QUEUED\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"DISCARD"]), b"+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"EXEC"]),
            b"-ERR EXEC without MULTI\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"GET", b"k"]),
            b"$5\r\nother\r\n"
        );
    }

    #[test]
    fn queue_time_arity_and_unknown_errors_discard_the_whole_transaction() {
        for bad in [
            &[b"SET".as_slice(), b"k"][..],
            &[b"NO-SUCH".as_slice(), b"k"][..],
            &[b"SELECT".as_slice(), b"0"][..],
        ] {
            let store = Store::new(1024);
            let mut session = Session::new();
            assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
            let error = call(&mut session, &store, bad);
            assert!(error.starts_with(b"-ERR"), "{error:?}");
            assert_eq!(
                call(&mut session, &store, &[b"SET", b"k", b"v"]),
                b"+QUEUED\r\n"
            );
            assert_eq!(
                call(&mut session, &store, &[b"EXEC"]),
                b"-EXECABORT Transaction discarded because of previous errors.\r\n"
            );
            assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"$-1\r\n");
        }
    }

    #[test]
    fn watch_aborts_on_another_client_and_unwatch_releases_it() {
        let store = Store::new(1024);
        let mut session = Session::new();
        let mut other = Session::new();
        assert_eq!(call(&mut session, &store, &[b"WATCH", b"k"]), b"+OK\r\n");
        assert_eq!(
            call(&mut other, &store, &[b"SET", b"k", b"other"]),
            b"+OK\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"k", b"queued"]),
            b"+QUEUED\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"EXEC"]), b"*-1\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"GET", b"k"]),
            b"$5\r\nother\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"UNWATCH"]), b"+OK\r\n");
        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"k", b"after"]),
            b"+QUEUED\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"EXEC"]), b"*1\r\n+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"GET", b"k"]),
            b"$5\r\nafter\r\n"
        );
    }

    #[test]
    fn watched_expiry_aborts_exec_without_sleeping() {
        struct TestClock(std::sync::atomic::AtomicI64);
        impl crate::engine::Clock for TestClock {
            fn now_ms(&self) -> i64 {
                self.0.load(std::sync::atomic::Ordering::SeqCst)
            }
        }
        let clock = std::sync::Arc::new(TestClock(std::sync::atomic::AtomicI64::new(1000)));
        let store = Store::with_clock(1024, clock.clone());
        let mut session = Session::new();
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"k", b"v", b"PX", b"100"]),
            b"+OK\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"WATCH", b"k"]), b"+OK\r\n");
        clock.0.store(1101, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"+QUEUED\r\n");
        assert_eq!(call(&mut session, &store, &[b"EXEC"]), b"*-1\r\n");
        assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"$-1\r\n");
    }

    #[test]
    fn nested_multi_watch_and_empty_exec_follow_redis_control_flow() {
        let store = Store::new(1024);
        let mut session = Session::new();
        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"MULTI"]),
            b"-ERR MULTI calls can not be nested\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"WATCH", b"k"]),
            b"-ERR WATCH inside MULTI is not allowed\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"EXEC"]), b"*0\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"WATCH"]),
            b"-ERR wrong number of arguments for 'watch' command\r\n"
        );
    }

    #[test]
    fn runtime_errors_are_reported_inside_exec_without_rolling_back() {
        let store = Store::new(1024);
        let mut session = Session::new();
        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"a", b"abc"]),
            b"+QUEUED\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"INCR", b"a"]), b"+QUEUED\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"b", b"ok"]),
            b"+QUEUED\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"EXEC"]),
            b"*3\r\n+OK\r\n-ERR value is not an integer or out of range\r\n+OK\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"GET", b"a"]),
            b"$3\r\nabc\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"GET", b"b"]), b"$2\r\nok\r\n");
    }

    #[test]
    fn password_auth_blocks_data_access_until_valid_and_quit_is_allowed() {
        let store = Store::with_auth(1024, crate::engine::Auth::from_password(Some(b"pw")));
        let mut session = Session::new();
        for command in [
            &[b"GET".as_slice(), b"k"][..],
            &[b"PING".as_slice()][..],
            &[b"MULTI".as_slice()][..],
            &[b"CONFIG".as_slice(), b"GET", b"*"][..],
        ] {
            assert_eq!(
                call(&mut session, &store, command),
                b"-NOAUTH Authentication required.\r\n"
            );
        }
        assert_eq!(
            call(&mut session, &store, &[b"AUTH", b"wrong"]),
            b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"AUTH", b"default", b"pw"]),
            b"+OK\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"k", b"v"]),
            b"+OK\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"$1\r\nv\r\n");
        // A failed re-authentication must not silently deauthenticate a live
        // administrative connection.
        assert_eq!(
            call(&mut session, &store, &[b"AUTH", b"nope"]),
            b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"PING"]), b"+PONG\r\n");

        let mut visitor = Session::new();
        let mut output = Vec::new();
        assert_eq!(
            visitor.execute(&store, &[b"QUIT"], &mut output),
            Action::Close
        );
        assert_eq!(output, b"+OK\r\n");
    }

    #[test]
    fn auth_arity_usernames_and_no_password_errors_are_explicit() {
        let open = Store::new(1024);
        let mut session = Session::new();
        assert_eq!(
            call(&mut session, &open, &[b"AUTH", b"anything"]),
            b"-ERR AUTH <password> called without any password configured for the default user. Are you sure your configuration is correct?\r\n"
        );
        assert_eq!(call(&mut session, &open, &[b"PING"]), b"+PONG\r\n");

        let store = Store::with_auth(1024, crate::engine::Auth::from_password(Some(b"pw")));
        let mut session = Session::new();
        assert_eq!(
            call(&mut session, &store, &[b"AUTH", b"other", b"pw"]),
            b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"AUTH", b"a", b"b", b"c"]),
            b"-ERR wrong number of arguments for 'auth' command\r\n"
        );
        assert_eq!(
            call(
                &mut session,
                &store,
                &[b"HELLO", b"3", b"AUTH", b"default", b"bad"]
            ),
            b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"GET", b"k"]),
            b"-NOAUTH Authentication required.\r\n"
        );
        assert!(
            call(
                &mut session,
                &store,
                &[b"HELLO", b"3", b"AUTH", b"default", b"pw"]
            )
            .starts_with(b"%5\r\n")
        );
        assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"_\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"HELLO", b"2", b"AUTH", b"default"]),
            b"-ERR syntax error\r\n"
        );
        assert_eq!(
            call(
                &mut session,
                &store,
                &[b"HELLO", b"AUTH", b"default", b"pw"]
            ),
            b"-NOPROTO unsupported protocol version\r\n"
        );
    }

    #[test]
    fn hello_auth_is_atomic_with_protocol_and_name_negotiation() {
        let store = Store::with_auth(1024, crate::engine::Auth::from_password(Some(b"pw")));
        let mut session = Session::new();
        let mut output = Vec::new();
        assert_eq!(
            session.execute(
                &store,
                &[
                    b"HELLO", b"3", b"AUTH", b"default", b"bad", b"SETNAME", b"name"
                ],
                &mut output
            ),
            Action::Continue
        );
        assert_eq!(
            output,
            b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"GET", b"k"]),
            b"-NOAUTH Authentication required.\r\n"
        );

        let mut output = Vec::new();
        assert_eq!(
            session.execute(
                &store,
                &[
                    b"HELLO", b"3", b"AUTH", b"default", b"pw", b"SETNAME", b"client"
                ],
                &mut output
            ),
            Action::Continue
        );
        assert!(output.starts_with(b"%5\r\n"));
        assert_eq!(
            call(&mut session, &store, &[b"CLIENT", b"GETNAME"]),
            b"$6\r\nclient\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"HELLO", b"2", b"AUTH", b"default"]),
            b"-ERR syntax error\r\n"
        );
    }

    #[test]
    fn config_requirepass_rotates_disables_and_never_echoes_the_secret() {
        let store = Store::new(1024);
        let mut admin = Session::new();
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[b"CONFIG", b"SET", b"requirepass", b"first"]
            ),
            b"+OK\r\n"
        );
        assert_eq!(
            call(&mut admin, &store, &[b"GET", b"k"]),
            b"-NOAUTH Authentication required.\r\n"
        );
        assert_eq!(call(&mut admin, &store, &[b"AUTH", b"first"]), b"+OK\r\n");
        assert_eq!(
            call(&mut admin, &store, &[b"CONFIG", b"GET", b"requirepass"]),
            b"*2\r\n$11\r\nrequirepass\r\n$10\r\n<redacted>\r\n"
        );
        assert!(
            !call(&mut admin, &store, &[b"CONFIG", b"GET", b"*"])
                .windows(5)
                .any(|window| window == b"first")
        );
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[b"CONFIG", b"SET", b"requirepass", b"second"]
            ),
            b"+OK\r\n"
        );
        let mut other = Session::new();
        assert_eq!(
            call(&mut other, &store, &[b"AUTH", b"first"]),
            b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
        );
        assert_eq!(call(&mut other, &store, &[b"AUTH", b"second"]), b"+OK\r\n");
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[b"CONFIG", b"SET", b"requirepass", b"too-long"]
            ),
            b"+OK\r\n"
        );
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[
                    b"CONFIG",
                    b"SET",
                    b"maxmemory",
                    b"0",
                    b"requirepass",
                    b"third"
                ]
            ),
            b"-ERR CONFIG SET option is unsupported or invalid\r\n"
        );
        assert_eq!(
            call(&mut admin, &store, &[b"AUTH", b"third"]),
            b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"
        );
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[b"CONFIG", b"SET", b"requirepass", b""]
            ),
            b"+OK\r\n"
        );
        let mut fresh = Session::new();
        assert_eq!(call(&mut fresh, &store, &[b"PING"]), b"+PONG\r\n");
        assert_eq!(
            call(&mut fresh, &store, &[b"CONFIG", b"GET", b"requirepass"]),
            b"*2\r\n$11\r\nrequirepass\r\n$0\r\n\r\n"
        );
    }

    #[test]
    fn acl_users_restrict_commands_keys_and_whoami() {
        let store = Store::new(1024);
        let mut admin = Session::new();
        assert_eq!(
            call(&mut admin, &store, &[b"ACL", b"WHOAMI"]),
            b"$7\r\ndefault\r\n"
        );
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[
                    b"ACL",
                    b"SETUSER",
                    b"alice",
                    b"on",
                    b">pw",
                    b"resetkeys",
                    b"~app:*",
                    b"resetchannels",
                    b"&events:*",
                    b"-@all",
                    b"+@read",
                    b"+@write"
                ]
            ),
            b"+OK\r\n"
        );
        assert!(call(&mut admin, &store, &[b"ACL", b"USERS"]).starts_with(b"*2\r\n"));
        assert!(
            call(&mut admin, &store, &[b"ACL", b"LIST"])
                .windows(9)
                .any(|window| window == b"user alic")
        );
        assert!(
            call(&mut admin, &store, &[b"ACL", b"CAT", b"read"])
                .windows(3)
                .any(|window| window == b"get")
        );
        let details = call(&mut admin, &store, &[b"ACL", b"GETUSER", b"alice"]);
        assert!(details.starts_with(b"*12\r\n"));
        assert!(details.windows(7).any(|window| window == b"flags\r\n"));
        assert!(details.windows(6).any(|window| window == b"~app:*"));
        assert_eq!(
            call(&mut admin, &store, &[b"ACL", b"GETUSER", b"missing"]),
            b"$-1\r\n"
        );

        let mut alice = Session::new();
        assert_eq!(
            call(&mut alice, &store, &[b"AUTH", b"alice", b"pw"]),
            b"+OK\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"ACL", b"WHOAMI"]),
            b"$5\r\nalice\r\n"
        );
        assert_eq!(call(&mut alice, &store, &[b"GET", b"app:k"]), b"$-1\r\n");
        assert_eq!(
            call(&mut alice, &store, &[b"SET", b"app:k", b"v"]),
            b"+OK\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"GET", b"app:k"]),
            b"$1\r\nv\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"GET", b"other"]),
            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"PING"]),
            b"-NOPERM this user has no permissions to run the 'ping' command\r\n"
        );
        assert_eq!(call(&mut alice, &store, &[b"DEL", b"app:k"]), b":1\r\n");

        assert_eq!(
            call(&mut admin, &store, &[b"ACL", b"DELUSER", b"default"]),
            b"-ERR The 'default' user cannot be removed\r\n"
        );
        assert_eq!(
            call(&mut admin, &store, &[b"ACL", b"DELUSER", b"alice"]),
            b":1\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"GET", b"app:k"]),
            b"-NOAUTH Authentication required.\r\n"
        );
        assert_eq!(
            call(&mut admin, &store, &[b"ACL", b"SAVE"]),
            b"-ERR This instance has no ACL file configured\r\n"
        );
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[b"ACL", b"SETUSER", b"bob", b"on", b"+nope"]
            ),
            b"-ERR Unknown command or category name in ACL\r\n"
        );
        assert_eq!(
            call(&mut admin, &store, &[b"ACL", b"GETUSER", b"bob"]),
            b"$-1\r\n"
        );
    }

    #[test]
    fn denied_acl_command_marks_an_active_transaction_dirty() {
        let store = Store::new(1024);
        let mut admin = Session::new();
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[
                    b"ACL",
                    b"SETUSER",
                    b"reader",
                    b"on",
                    b">pw",
                    b"allkeys",
                    b"-@all",
                    b"+@read",
                    b"+@transaction"
                ]
            ),
            b"+OK\r\n"
        );
        let mut reader = Session::new();
        assert_eq!(
            call(&mut reader, &store, &[b"AUTH", b"reader", b"pw"]),
            b"+OK\r\n"
        );
        assert_eq!(call(&mut reader, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(call(&mut reader, &store, &[b"GET", b"k"]), b"+QUEUED\r\n");
        assert_eq!(
            call(&mut reader, &store, &[b"SET", b"k", b"v"]),
            b"-NOPERM this user has no permissions to run the 'set' command\r\n"
        );
        assert_eq!(
            call(&mut reader, &store, &[b"EXEC"]),
            b"-EXECABORT Transaction discarded because of previous errors.\r\n"
        );
        assert_eq!(call(&mut reader, &store, &[b"GET", b"k"]), b"$-1\r\n");
    }

    #[test]
    fn exec_rechecks_acls_so_revocation_cannot_commit_queued_writes() {
        let store = Store::new(1024);
        let mut admin = Session::new();
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[
                    b"ACL",
                    b"SETUSER",
                    b"alice",
                    b"on",
                    b">pw",
                    b"allkeys",
                    b"allchannels",
                    b"+@all"
                ]
            ),
            b"+OK\r\n"
        );
        let mut alice = Session::new();
        assert_eq!(
            call(&mut alice, &store, &[b"AUTH", b"alice", b"pw"]),
            b"+OK\r\n"
        );
        assert_eq!(call(&mut alice, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(
            call(&mut alice, &store, &[b"SET", b"k", b"v"]),
            b"+QUEUED\r\n"
        );
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[b"ACL", b"SETUSER", b"alice", b"-@write"]
            ),
            b"+OK\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"EXEC"]),
            b"-NOPERM this user has no permissions to run the 'set' command\r\n"
        );
        assert_eq!(call(&mut admin, &store, &[b"GET", b"k"]), b"$-1\r\n");
    }

    #[test]
    fn pubsub_subscribe_publish_patterns_and_unsubscribe_are_ordered() {
        let store = Store::new(1024);
        let mut subscriber = Session::new();
        let (sender, receiver) = std::sync::mpsc::sync_channel(16);
        subscriber.set_pubsub_sender(sender);
        assert!(!subscriber.is_subscribed());
        assert_eq!(
            call(&mut subscriber, &store, &[b"SUBSCRIBE", b"a", b"b"]),
            b"*3\r\n$9\r\nsubscribe\r\n$1\r\na\r\n:1\r\n*3\r\n$9\r\nsubscribe\r\n$1\r\nb\r\n:2\r\n"
        );
        assert!(subscriber.is_subscribed());
        assert_eq!(
            call(&mut subscriber, &store, &[b"PSUBSCRIBE", b"news.*"]),
            b"*3\r\n$10\r\npsubscribe\r\n$6\r\nnews.*\r\n:3\r\n"
        );

        let mut publisher = Session::new();
        assert_eq!(
            call(&mut publisher, &store, &[b"PUBLISH", b"a", b"hello"]),
            b":1\r\n"
        );
        assert_eq!(
            receiver.recv().unwrap(),
            b"*3\r\n$7\r\nmessage\r\n$1\r\na\r\n$5\r\nhello\r\n"
        );
        assert_eq!(
            call(&mut publisher, &store, &[b"PUBLISH", b"news.tech", b"x"]),
            b":1\r\n"
        );
        assert_eq!(
            receiver.recv().unwrap(),
            b"*4\r\n$8\r\npmessage\r\n$6\r\nnews.*\r\n$9\r\nnews.tech\r\n$1\r\nx\r\n"
        );
        assert_eq!(
            call(
                &mut publisher,
                &store,
                &[b"PUBSUB", b"NUMSUB", b"a", b"missing"]
            ),
            b"*4\r\n$1\r\na\r\n:1\r\n$7\r\nmissing\r\n:0\r\n"
        );
        assert_eq!(
            call(&mut publisher, &store, &[b"PUBSUB", b"CHANNELS"]),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            call(&mut publisher, &store, &[b"PUBSUB", b"NUMPAT"]),
            b":1\r\n"
        );

        assert_eq!(
            call(&mut subscriber, &store, &[b"PING", b"hi"]),
            b"*2\r\n$4\r\npong\r\n$2\r\nhi\r\n"
        );
        assert_eq!(
            call(&mut subscriber, &store, &[b"GET", b"k"]),
            b"-ERR Can't execute 'get': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n"
        );
        assert_eq!(
            call(&mut subscriber, &store, &[b"UNSUBSCRIBE", b"a"]),
            b"*3\r\n$11\r\nunsubscribe\r\n$1\r\na\r\n:2\r\n"
        );
        assert_eq!(
            call(&mut publisher, &store, &[b"PUBLISH", b"a", b"gone"]),
            b":0\r\n"
        );
        assert_eq!(
            call(&mut subscriber, &store, &[b"UNSUBSCRIBE"]),
            b"*3\r\n$11\r\nunsubscribe\r\n$1\r\nb\r\n:1\r\n"
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            call(&mut subscriber, &store, &[b"PUNSUBSCRIBE"]),
            b"*3\r\n$12\r\npunsubscribe\r\n$6\r\nnews.*\r\n:0\r\n"
        );
        assert!(!subscriber.is_subscribed());
        store.pubsub_remove_client(subscriber.id());
    }

    #[test]
    fn pubsub_resp3_uses_push_frames_and_allows_normal_commands() {
        let store = Store::new(1024);
        let mut subscriber = Session::new();
        let (sender, receiver) = std::sync::mpsc::sync_channel(4);
        subscriber.set_pubsub_sender(sender);
        assert!(call(&mut subscriber, &store, &[b"HELLO", b"3"]).starts_with(b"%5\r\n"));
        assert!(
            call(&mut subscriber, &store, &[b"SUBSCRIBE", b"c"])
                .starts_with(b">3\r\n$9\r\nsubscribe\r\n")
        );
        let mut publisher = Session::new();
        assert_eq!(
            call(&mut publisher, &store, &[b"PUBLISH", b"c", b"v"]),
            b":1\r\n"
        );
        assert_eq!(
            receiver.recv().unwrap(),
            b">3\r\n$7\r\nmessage\r\n$1\r\nc\r\n$1\r\nv\r\n"
        );
        assert_eq!(
            call(&mut subscriber, &store, &[b"GET", b"missing"]),
            b"_\r\n"
        );
        store.pubsub_remove_client(subscriber.id());
    }

    #[test]
    fn acl_channel_patterns_gate_publish_subscribe_and_pattern_subscribe() {
        let store = Store::new(1024);
        let mut admin = Session::new();
        assert_eq!(
            call(
                &mut admin,
                &store,
                &[
                    b"ACL",
                    b"SETUSER",
                    b"alice",
                    b"on",
                    b">pw",
                    b"allkeys",
                    b"allchannels",
                    b"-@all",
                    b"+@pubsub",
                    b"resetchannels",
                    b"&allowed:*"
                ]
            ),
            b"+OK\r\n"
        );
        let mut alice = Session::new();
        alice.set_pubsub_sender(std::sync::mpsc::sync_channel(4).0);
        assert_eq!(
            call(&mut alice, &store, &[b"AUTH", b"alice", b"pw"]),
            b"+OK\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"PUBLISH", b"allowed:news", b"v"]),
            b":0\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"PUBLISH", b"other", b"v"]),
            b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"SUBSCRIBE", b"other"]),
            b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"SPUBLISH", b"allowed:news", b"v"]),
            b":0\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"SPUBLISH", b"other", b"v"]),
            b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"SSUBSCRIBE", b"other"]),
            b"-NOPERM this user has no permissions to access one of the channels used as arguments\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"SSUBSCRIBE", b"allowed:news"]),
            b"*3\r\n$10\r\nssubscribe\r\n$12\r\nallowed:news\r\n:1\r\n"
        );
        assert_eq!(
            call(&mut alice, &store, &[b"PSUBSCRIBE", b"allowed:*"]),
            b"*3\r\n$10\r\npsubscribe\r\n$9\r\nallowed:*\r\n:1\r\n"
        );
        store.pubsub_remove_client(alice.id());
    }

    #[test]
    fn reset_discards_transactions_watches_subscriptions_and_resp3() {
        let store = Store::new(1024);
        let mut session = Session::new();
        let (sender, _receiver) = std::sync::mpsc::sync_channel(4);
        session.set_pubsub_sender(sender);
        assert!(call(&mut session, &store, &[b"HELLO", b"3"]).starts_with(b"%5\r\n"));
        assert_eq!(call(&mut session, &store, &[b"WATCH", b"k"]), b"+OK\r\n");
        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"k", b"v"]),
            b"+QUEUED\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"RESET"]), b"+RESET\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"EXEC"]),
            b"-ERR EXEC without MULTI\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"GET", b"missing"]),
            b"$-1\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"PING"]), b"+PONG\r\n");

        assert!(!call(&mut session, &store, &[b"SUBSCRIBE", b"ch"]).is_empty());
        assert!(session.is_subscribed());
        assert_eq!(call(&mut session, &store, &[b"RESET"]), b"+RESET\r\n");
        assert!(!session.is_subscribed());
        assert_eq!(
            call(&mut session, &store, &[b"GET", b"missing"]),
            b"$-1\r\n"
        );
        assert_eq!(
            call(&mut session, &store, &[b"RESET", b"extra"]),
            b"-ERR wrong number of arguments for 'reset' command\r\n"
        );
    }

    #[test]
    fn reset_deauthenticates_an_authenticated_connection() {
        let store = Store::with_auth(1024, crate::engine::Auth::from_password(Some(b"pw")));
        let mut session = Session::new();
        assert_eq!(call(&mut session, &store, &[b"AUTH", b"pw"]), b"+OK\r\n");
        assert_eq!(call(&mut session, &store, &[b"PING"]), b"+PONG\r\n");
        assert_eq!(call(&mut session, &store, &[b"RESET"]), b"+RESET\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"PING"]),
            b"-NOAUTH Authentication required.\r\n"
        );
        assert_eq!(call(&mut session, &store, &[b"AUTH", b"pw"]), b"+OK\r\n");
        assert_eq!(call(&mut session, &store, &[b"PING"]), b"+PONG\r\n");
    }

    #[test]
    fn resp3_exec_abort_is_a_null_value_and_quit_discards_the_queue() {
        let store = Store::new(1024);
        let mut session = Session::new();
        let mut other = Session::new();
        assert!(call(&mut session, &store, &[b"HELLO", b"3"]).starts_with(b"%5\r\n"));
        assert_eq!(call(&mut session, &store, &[b"WATCH", b"k"]), b"+OK\r\n");
        assert_eq!(call(&mut other, &store, &[b"SET", b"k", b"v"]), b"+OK\r\n");
        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(call(&mut session, &store, &[b"GET", b"k"]), b"+QUEUED\r\n");
        assert_eq!(call(&mut session, &store, &[b"EXEC"]), b"_\r\n");

        assert_eq!(call(&mut session, &store, &[b"MULTI"]), b"+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"SET", b"discarded", b"v"]),
            b"+QUEUED\r\n"
        );
        let mut output = Vec::new();
        assert_eq!(
            session.execute(&store, &[b"QUIT"], &mut output),
            Action::Close
        );
        assert_eq!(output, b"+OK\r\n");
        assert_eq!(
            call(&mut session, &store, &[b"GET", b"discarded"]),
            b"_\r\n"
        );
    }
    #[test]
    fn sharded_pubsub_is_separate_bounded_and_reset_cleans_it() {
        let store = Store::new(1024);
        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        let mut subscriber = Session::new();
        subscriber.set_pubsub_sender(sender);
        assert_eq!(
            call(&mut subscriber, &store, &[b"SSUBSCRIBE", b"a", b"b"]),
            b"*3\r\n$10\r\nssubscribe\r\n$1\r\na\r\n:1\r\n*3\r\n$10\r\nssubscribe\r\n$1\r\nb\r\n:2\r\n"
        );
        assert!(subscriber.is_subscribed());
        assert_eq!(
            call(&mut subscriber, &store, &[b"GET", b"k"]),
            b"-ERR Can't execute 'get': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context\r\n"
        );

        let mut publisher = Session::new();
        assert_eq!(
            call(&mut publisher, &store, &[b"SPUBLISH", b"a", b"hi"]),
            b":1\r\n"
        );
        assert_eq!(
            receiver.try_recv().unwrap(),
            b"*3\r\n$8\r\nsmessage\r\n$1\r\na\r\n$2\r\nhi\r\n"
        );
        assert_eq!(
            call(&mut publisher, &store, &[b"PUBLISH", b"a", b"normal"]),
            b":0\r\n"
        );
        assert_eq!(
            call(&mut publisher, &store, &[b"PUBSUB", b"SHARDCHANNELS"]),
            b"*2\r\n$1\r\na\r\n$1\r\nb\r\n"
        );
        assert_eq!(
            call(
                &mut publisher,
                &store,
                &[b"PUBSUB", b"SHARDCHANNELS", b"a*"]
            ),
            b"*1\r\n$1\r\na\r\n"
        );
        assert_eq!(
            call(
                &mut publisher,
                &store,
                &[b"PUBSUB", b"SHARDNUMSUB", b"a", b"b", b"c"]
            ),
            b"*6\r\n$1\r\na\r\n:1\r\n$1\r\nb\r\n:1\r\n$1\r\nc\r\n:0\r\n"
        );
        assert_eq!(
            call(&mut publisher, &store, &[b"PUBSUB", b"CHANNELS"]),
            b"*0\r\n"
        );

        let mut normal = Session::new();
        let (normal_sender, normal_receiver) = std::sync::mpsc::sync_channel(8);
        normal.set_pubsub_sender(normal_sender);
        assert_eq!(
            call(&mut normal, &store, &[b"SUBSCRIBE", b"a"]),
            b"*3\r\n$9\r\nsubscribe\r\n$1\r\na\r\n:1\r\n"
        );
        assert_eq!(
            call(&mut publisher, &store, &[b"SPUBLISH", b"a", b"shard"]),
            b":1\r\n"
        );
        assert_eq!(
            call(&mut publisher, &store, &[b"PUBLISH", b"a", b"normal"]),
            b":1\r\n"
        );
        assert_eq!(
            normal_receiver.try_recv().unwrap(),
            b"*3\r\n$7\r\nmessage\r\n$1\r\na\r\n$6\r\nnormal\r\n"
        );

        assert_eq!(
            call(&mut subscriber, &store, &[b"SUNSUBSCRIBE", b"a"]),
            b"*3\r\n$12\r\nsunsubscribe\r\n$1\r\na\r\n:1\r\n"
        );
        assert_eq!(
            call(&mut subscriber, &store, &[b"SUNSUBSCRIBE"]),
            b"*3\r\n$12\r\nsunsubscribe\r\n$1\r\nb\r\n:0\r\n"
        );
        assert_eq!(call(&mut subscriber, &store, &[b"RESET"]), b"+RESET\r\n");
        assert!(!subscriber.is_subscribed());
        assert_eq!(
            call(
                &mut publisher,
                &store,
                &[b"PUBSUB", b"SHARDNUMSUB", b"a", b"b"]
            ),
            b"*4\r\n$1\r\na\r\n:0\r\n$1\r\nb\r\n:0\r\n"
        );
    }
    #[test]
    fn sharded_pubsub_resp3_uses_push_frames() {
        let store = Store::new(1024);
        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        let mut subscriber = Session::new();
        subscriber.set_pubsub_sender(sender);
        assert!(call(&mut subscriber, &store, &[b"HELLO", b"3"]).starts_with(b"%5\r\n"));
        assert_eq!(
            call(&mut subscriber, &store, &[b"SSUBSCRIBE", b"a"]),
            b">3\r\n$10\r\nssubscribe\r\n$1\r\na\r\n:1\r\n"
        );
        let mut publisher = Session::new();
        assert_eq!(
            call(&mut publisher, &store, &[b"SPUBLISH", b"a", b"m"]),
            b":1\r\n"
        );
        assert_eq!(
            receiver.try_recv().unwrap(),
            b">3\r\n$8\r\nsmessage\r\n$1\r\na\r\n$1\r\nm\r\n"
        );
    }
    #[test]
    fn expired_and_evicted_notifications_use_injected_clock_and_eviction_policy() {
        struct TestClock(std::sync::atomic::AtomicI64);
        impl crate::engine::Clock for TestClock {
            fn now_ms(&self) -> i64 {
                self.0.load(std::sync::atomic::Ordering::SeqCst)
            }
        }
        let clock = std::sync::Arc::new(TestClock(std::sync::atomic::AtomicI64::new(1000)));
        let store = Store::with_clock(10, clock.clone());
        store.set_notify_flags(crate::engine::NotifyFlags::parse(b"KEA").unwrap());
        let (sender, receiver) = std::sync::mpsc::sync_channel(16);
        let mut subscriber = Session::new();
        subscriber.set_pubsub_sender(sender);
        assert_eq!(
            call(
                &mut subscriber,
                &store,
                &[b"PSUBSCRIBE", b"__keyevent@0__:*"]
            ),
            b"*3\r\n$10\r\npsubscribe\r\n$16\r\n__keyevent@0__:*\r\n:1\r\n"
        );
        let mut writer = Session::new();
        assert_eq!(
            call(&mut writer, &store, &[b"SET", b"k", b"v", b"PX", b"10"]),
            b"+OK\r\n"
        );
        assert!(receiver.try_recv().is_ok(), "SET event");
        clock.0.store(1011, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(call(&mut writer, &store, &[b"GET", b"k"]), b"$-1\r\n");
        assert_eq!(
            receiver.try_recv().unwrap(),
            b"*4\r\n$8\r\npmessage\r\n$16\r\n__keyevent@0__:*\r\n$22\r\n__keyevent@0__:expired\r\n$1\r\nk\r\n"
        );

        store.set_eviction_policy(crate::engine::EvictionPolicy::AllKeysRandom);
        assert_eq!(
            call(&mut writer, &store, &[b"SET", b"victim", b"123"]),
            b"+OK\r\n"
        );
        assert!(receiver.try_recv().is_ok(), "victim SET event");
        assert_eq!(
            call(&mut writer, &store, &[b"SET", b"new", b"123"]),
            b"+OK\r\n"
        );
        assert_eq!(
            receiver.try_recv().unwrap(),
            b"*4\r\n$8\r\npmessage\r\n$16\r\n__keyevent@0__:*\r\n$22\r\n__keyevent@0__:evicted\r\n$6\r\nvictim\r\n"
        );
        assert_eq!(
            receiver.try_recv().unwrap(),
            b"*4\r\n$8\r\npmessage\r\n$16\r\n__keyevent@0__:*\r\n$18\r\n__keyevent@0__:set\r\n$3\r\nnew\r\n"
        );

        assert_eq!(
            call(&mut writer, &store, &[b"SET", b"p", b"v", b"PX", b"10"]),
            b"+OK\r\n"
        );
        assert!(receiver.try_recv().is_ok(), "p SET event");
        clock.0.store(1022, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(store.purge_expired(16), 1);
        assert_eq!(
            receiver.try_recv().unwrap(),
            b"*4\r\n$8\r\npmessage\r\n$16\r\n__keyevent@0__:*\r\n$22\r\n__keyevent@0__:expired\r\n$1\r\np\r\n"
        );

        store.set_eviction_policy(crate::engine::EvictionPolicy::NoEviction);
        let reply = call(
            &mut writer,
            &store,
            &[b"SET", b"big", b"01234567890123456789"],
        );
        assert!(reply.starts_with(b"-OOM"), "{reply:?}");
        assert!(receiver.try_recv().is_err(), "no event for rejected write");
    }
}
