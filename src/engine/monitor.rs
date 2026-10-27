//! Bounded MONITOR command stream formatting and secret redaction.
//!
//! Monitor lines are simple strings in Redis' documented shape. Arguments are
//! quoted and escaped so the line cannot contain RESP delimiters, capped per
//! argument and per line, and credentials are redacted before formatting.

use std::io::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LINE_BYTES: usize = 4096;
const MAX_ARG_BYTES: usize = 128;

pub(super) fn line(address: &str, arguments: &[&[u8]]) -> Vec<u8> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let mut out = Vec::new();
    let _ = write!(
        out,
        "+{}.{:06} [0 {}]",
        now.as_secs(),
        now.subsec_micros(),
        address
    );
    let redacted = redact(arguments);
    for argument in redacted {
        if out.len() + MAX_ARG_BYTES + 8 > MAX_LINE_BYTES {
            out.extend_from_slice(b" ...");
            break;
        }
        out.push(b' ');
        match argument {
            Some(argument) => repr(argument, &mut out),
            None => out.extend_from_slice(b"(redacted)"),
        }
    }
    if out.len() + 2 > MAX_LINE_BYTES {
        out.truncate(MAX_LINE_BYTES - 2);
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// `None` marks a redacted argument. Unknown malformed credential layouts are
/// redacted conservatively instead of leaking the remaining arguments.
fn redact<'a>(arguments: &'a [&'a [u8]]) -> Vec<Option<&'a [u8]>> {
    let mut output: Vec<Option<&[u8]>> = arguments.iter().copied().map(Some).collect();
    let Some(command) = arguments.first() else {
        return output;
    };
    if command.eq_ignore_ascii_case(b"AUTH") {
        output.truncate(1);
        output.push(None);
        return output;
    }
    if command.eq_ignore_ascii_case(b"HELLO") {
        if let Some(position) = arguments
            .iter()
            .position(|argument| argument.eq_ignore_ascii_case(b"AUTH"))
        {
            let password = position.saturating_add(2);
            if password < output.len() {
                output[password] = None;
            } else {
                for slot in output.iter_mut().skip(position) {
                    *slot = None;
                }
            }
        }
    } else if command.eq_ignore_ascii_case(b"ACL")
        && arguments
            .get(1)
            .is_some_and(|sub| sub.eq_ignore_ascii_case(b"SETUSER"))
    {
        for slot in output.iter_mut().skip(3) {
            if slot
                .is_some_and(|argument| matches!(argument.first(), Some(b'>' | b'<' | b'#' | b'!')))
            {
                *slot = None;
            }
        }
    } else if command.eq_ignore_ascii_case(b"CONFIG")
        && arguments
            .get(1)
            .is_some_and(|sub| sub.eq_ignore_ascii_case(b"SET"))
    {
        let mut index = 2;
        while index + 1 < output.len() {
            if arguments[index].eq_ignore_ascii_case(b"requirepass")
                || arguments[index].eq_ignore_ascii_case(b"masterauth")
                || arguments[index].eq_ignore_ascii_case(b"masteruser")
            {
                output[index + 1] = None;
            }
            index += 2;
        }
    }
    output
}

fn repr(argument: &[u8], out: &mut Vec<u8>) {
    out.push(b'"');
    for &byte in argument.iter().take(MAX_ARG_BYTES) {
        match byte {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'"' => out.extend_from_slice(b"\\\""),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            0x20..=0x7e => out.push(byte),
            other => {
                let _ = write!(out, "\\x{other:02x}");
            }
        }
    }
    if argument.len() > MAX_ARG_BYTES {
        out.extend_from_slice(b"...");
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_bounded_escaped_and_redact_credentials() {
        let args: &[&[u8]] = &[b"SET", b"k\n", b"v\"\\"];
        let frame = line("127.0.0.1:1", args);
        let text = String::from_utf8(frame).unwrap();
        assert!(text.starts_with('+'));
        assert!(text.ends_with("\r\n"));
        assert!(text.contains("\"SET\" \"k\\n\" \"v\\\"\\\\\""), "{text}");

        let args: &[&[u8]] = &[b"AUTH", b"alice", b"secret"];
        let auth = line("addr", args);
        let text = String::from_utf8(auth).unwrap();
        assert!(!text.contains("alice"), "{text}");
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("\"AUTH\" (redacted)"), "{text}");

        let args: &[&[u8]] = &[b"HELLO", b"3", b"AUTH", b"alice", b"secret"];
        let hello = line("addr", args);
        let text = String::from_utf8(hello).unwrap();
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("(redacted)"), "{text}");

        let args: &[&[u8]] = &[b"ACL", b"SETUSER", b"bob", b">secret", b"+@all"];
        let acl = line("addr", args);
        let text = String::from_utf8(acl).unwrap();
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("(redacted)"), "{text}");

        let args: &[&[u8]] = &[b"CONFIG", b"SET", b"requirepass", b"secret"];
        let config = line("addr", args);
        let text = String::from_utf8(config).unwrap();
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("(redacted)"), "{text}");

        let long = line("addr", &[b"SET", b"k", &[b'x'; 1024]]);
        assert!(long.len() <= MAX_LINE_BYTES, "{}", long.len());
        assert!(long.ends_with(b"\r\n"));
    }
}
