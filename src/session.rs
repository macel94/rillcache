//! Per-connection protocol negotiation and bounded client metadata.
//!
//! RESP3 support is deliberately limited to the reply types emitted by the
//! current command set: strings, integers, errors, arrays and nulls.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::engine::{Access, AclError, Action, AuthError, Store, TransactionOutcome, WatchState};
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
        }
    }

    /// Dispatch connection commands locally and all data commands through the
    /// stable public RESP2 Store API, adapting only this session's reply.
    pub fn execute(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        let Some(command) = arguments.first() else {
            output.extend_from_slice(b"-ERR empty command\r\n");
            return Action::Continue;
        };
        if !is_auth_exempt(command) {
            let authentication_required = store.authentication_required();
            if authentication_required && self.authenticated_user.is_none() {
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
                        output.extend_from_slice(b"-NOAUTH Authentication required.\r\n");
                        return Action::Continue;
                    }
                    Access::DeniedCommand => {
                        let name = command_name(command);
                        let _ = write!(
                            output,
                            "-NOPERM this user has no permissions to run the '{name}' command\r\n"
                        );
                        if let Some(multi) = self.multi.as_mut() {
                            multi.dirty = true;
                        }
                        return Action::Continue;
                    }
                    Access::DeniedKey => {
                        output.extend_from_slice(
                            b"-NOPERM this user has no permissions to access one of the keys used as arguments\r\n",
                        );
                        if let Some(multi) = self.multi.as_mut() {
                            multi.dirty = true;
                        }
                        return Action::Continue;
                    }
                }
            }
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
        } else if self.multi.is_some() {
            self.queue(arguments, output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"HELLO") {
            self.hello(store, &arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"CLIENT") {
            self.client(&arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"SELECT") {
            select(&arguments[1..], output);
            return Action::Continue;
        }
        if command.eq_ignore_ascii_case(b"AUTH") {
            self.auth(store, &arguments[1..], output);
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

    fn client(&mut self, arguments: &[&[u8]], output: &mut Vec<u8>) {
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
        } else if subcommand.eq_ignore_ascii_case(b"INFO") && rest.is_empty() {
            let mut info = format!("id={} db=0", self.id).into_bytes();
            append_info(&mut info, b"name", self.name.as_deref());
            append_info(&mut info, b"lib-name", self.library_name.as_deref());
            append_info(&mut info, b"lib-ver", self.library_version.as_deref());
            encode_bulk(Some(&info), output);
        } else {
            output
                .extend_from_slice(b"-ERR unsupported CLIENT subcommand or invalid arguments\r\n");
        }
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
    [b"AUTH".as_slice(), b"HELLO", b"QUIT"]
        .iter()
        .any(|name| command.eq_ignore_ascii_case(name))
}

fn is_session_command(command: &[u8]) -> bool {
    [b"ACL".as_slice(), b"AUTH", b"CLIENT", b"HELLO", b"SELECT"]
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
        AclError::Io(_) => output.extend_from_slice(b"-ERR ACL file I/O error\r\n"),
    }
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
}
