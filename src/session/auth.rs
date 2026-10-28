//! AUTH, ACL subcommands and denial accounting for one connection.

use std::io::Write;

use crate::engine::{Access, AclError, AuthError, Store};
use crate::protocol::encode_bulk;

use super::Session;
use super::client::parse_client_id;
use super::command::{arity, command_name};

impl Session {
    pub(super) fn auth(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
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

    pub(super) fn acl(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
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
                write!(output, "*{}\r\n", details.selectors.len()).expect("Vec write");
                for (commands, keys, channels) in details.selectors {
                    output.extend_from_slice(b"*6\r\n");
                    encode_bulk(Some(b"commands"), output);
                    encode_bulk(Some(commands.as_bytes()), output);
                    encode_bulk(Some(b"keys"), output);
                    encode_bulk(Some(keys.as_bytes()), output);
                    encode_bulk(Some(b"channels"), output);
                    encode_bulk(Some(channels.as_bytes()), output);
                }
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
}
impl Session {
    pub(super) fn log_denial(&self, store: &Store, reason: &str, context: &str, object: &[u8]) {
        let user = self
            .authenticated_user
            .clone()
            .unwrap_or_else(|| b"default".to_vec());
        store.record_acl_denial(reason, context, object, &user);
    }
}
pub(super) fn acl_error(error: AclError, output: &mut Vec<u8>) {
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
