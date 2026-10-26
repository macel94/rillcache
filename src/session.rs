//! Per-connection protocol negotiation and bounded client metadata.
//!
//! RESP3 support is deliberately limited to the reply types emitted by the
//! current command set: strings, integers, errors, arrays and nulls.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::engine::{Action, Store};
use crate::protocol::encode_bulk;

const MAX_NAME_BYTES: usize = 256;
const MAX_LIBRARY_FIELD_BYTES: usize = 128;
static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

/// State belonging to one socket. Move this together with any borrowed client
/// lease; it contains no socket handles and is never shared across clients.
#[derive(Debug)]
pub struct Session {
    protocol: Protocol,
    id: u64,
    name: Option<Vec<u8>>,
    library_name: Option<Vec<u8>>,
    library_version: Option<Vec<u8>>,
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
        }
    }

    /// Dispatch connection commands locally and all data commands through the
    /// stable public RESP2 Store API, adapting only this session's reply.
    pub fn execute(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) -> Action {
        let Some(command) = arguments.first() else {
            output.extend_from_slice(b"-ERR empty command\r\n");
            return Action::Continue;
        };
        if command.eq_ignore_ascii_case(b"HELLO") {
            self.hello(&arguments[1..], output);
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
            output.extend_from_slice(b"-ERR AUTH is not supported\r\n");
            return Action::Continue;
        }

        let start = output.len();
        let action = store.execute(arguments, output);
        if self.protocol == Protocol::Resp3 {
            let converted = convert_resp2_reply(&output[start..]);
            output.truncate(start);
            output.extend_from_slice(&converted);
        }
        action
    }

    fn hello(&mut self, arguments: &[&[u8]], output: &mut Vec<u8>) {
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

        // Parse and validate every option before changing either protocol or metadata.
        let mut name = None;
        let mut index = 0;
        while index < options.len() {
            if options[index].eq_ignore_ascii_case(b"AUTH") {
                output.extend_from_slice(b"-ERR AUTH is not supported\r\n");
                return;
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
            b"-ERR AUTH is not supported\r\n"
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
}
