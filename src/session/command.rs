//! Shared command classification, command-name formatting and arity replies.

use std::io::Write;

pub(super) fn arity(command: &str, output: &mut Vec<u8>) {
    let _ = write!(
        output,
        "-ERR wrong number of arguments for '{command}' command\r\n"
    );
}

pub(super) fn is_auth_exempt(command: &[u8]) -> bool {
    [b"AUTH".as_slice(), b"HELLO", b"QUIT", b"RESET"]
        .iter()
        .any(|name| command.eq_ignore_ascii_case(name))
}

pub(super) fn is_session_command(command: &[u8]) -> bool {
    [
        b"ACL".as_slice(),
        b"AUTH",
        b"BLPOP",
        b"BRPOP",
        b"CLIENT",
        b"HELLO",
        b"PSUBSCRIBE",
        b"PUBSUB",
        b"PUNSUBSCRIBE",
        b"SELECT",
        b"SUBSCRIBE",
        b"UNSUBSCRIBE",
    ]
    .iter()
    .any(|name| command.eq_ignore_ascii_case(name))
}

pub(super) fn allows_subscriber_command(command: &[u8]) -> bool {
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

pub(super) fn command_name(command: &[u8]) -> String {
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
