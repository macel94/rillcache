//! Bounded, binary-safe RESP2 command parsing. No payload is interpreted as UTF-8.

use std::fmt;
use std::io::Write;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_frame_bytes: usize,
    pub max_bulk_bytes: usize,
    pub max_arguments: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 2 * 1024 * 1024,
            max_bulk_bytes: 1024 * 1024,
            max_arguments: 1024,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Request<'a> {
    pub arguments: Vec<&'a [u8]>,
    pub consumed: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolError {
    ExpectedArray,
    ExpectedBulkString,
    InvalidLength,
    InvalidTerminator,
    EmptyCommand,
    TooManyArguments,
    BulkTooLarge,
    FrameTooLarge,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ExpectedArray => "expected an array of bulk strings",
            Self::ExpectedBulkString => "expected a non-null bulk string",
            Self::InvalidLength => "invalid length",
            Self::InvalidTerminator => "invalid CRLF terminator",
            Self::EmptyCommand => "empty command array is not supported",
            Self::TooManyArguments => "too many arguments",
            Self::BulkTooLarge => "bulk string limit exceeded",
            Self::FrameTooLarge => "request frame limit exceeded",
        })
    }
}

impl std::error::Error for ProtocolError {}

/// Parse one command, borrowing its arguments from `input`.
///
/// `Ok(None)` means more bytes are needed. `consumed` excludes subsequent
/// pipelined commands. Advertised lengths are checked before allocating payloads.
pub fn parse_request(input: &[u8], limits: Limits) -> Result<Option<Request<'_>>, ProtocolError> {
    let Some((count, mut offset)) = length(
        input,
        0,
        b'*',
        limits.max_arguments,
        ProtocolError::TooManyArguments,
    )?
    else {
        return incomplete(input, limits);
    };
    if count == 0 {
        return Err(ProtocolError::EmptyCommand);
    }
    let mut arguments = Vec::with_capacity(count.min(16));
    for _ in 0..count {
        if offset >= limits.max_frame_bytes {
            return Err(ProtocolError::FrameTooLarge);
        }
        let Some((size, start)) = length(
            input,
            offset,
            b'$',
            limits.max_bulk_bytes,
            ProtocolError::BulkTooLarge,
        )?
        else {
            return incomplete(input, limits);
        };
        let end = start
            .checked_add(size)
            .ok_or(ProtocolError::FrameTooLarge)?;
        let next = end.checked_add(2).ok_or(ProtocolError::FrameTooLarge)?;
        if next > limits.max_frame_bytes {
            return Err(ProtocolError::FrameTooLarge);
        }
        if input.len() < next {
            if input.get(end).is_some_and(|&byte| byte != b'\r') {
                return Err(ProtocolError::InvalidTerminator);
            }
            return incomplete(input, limits);
        }
        if &input[end..next] != b"\r\n" {
            return Err(ProtocolError::InvalidTerminator);
        }
        arguments.push(&input[start..end]);
        offset = next;
    }
    Ok(Some(Request {
        arguments,
        consumed: offset,
    }))
}

fn incomplete(input: &[u8], limits: Limits) -> Result<Option<Request<'_>>, ProtocolError> {
    if input.len() >= limits.max_frame_bytes {
        Err(ProtocolError::FrameTooLarge)
    } else {
        Ok(None)
    }
}

fn length(
    input: &[u8],
    start: usize,
    prefix: u8,
    maximum: usize,
    limit_error: ProtocolError,
) -> Result<Option<(usize, usize)>, ProtocolError> {
    let Some(&first) = input.get(start) else {
        return Ok(None);
    };
    if first != prefix {
        return Err(if prefix == b'*' {
            ProtocolError::ExpectedArray
        } else {
            ProtocolError::ExpectedBulkString
        });
    }
    let mut value = 0usize;
    for (digits, &byte) in input[start + 1..].iter().enumerate() {
        let index = start + 1 + digits;
        if byte == b'\r' {
            if digits == 0 {
                return Err(ProtocolError::InvalidLength);
            }
            return match input.get(index + 1) {
                None => Ok(None),
                Some(b'\n') => Ok(Some((value, index + 2))),
                Some(_) => Err(ProtocolError::InvalidTerminator),
            };
        }
        if !byte.is_ascii_digit() || digits == 20 {
            return Err(ProtocolError::InvalidLength);
        }
        value = value
            .checked_mul(10)
            .and_then(|n| n.checked_add(usize::from(byte - b'0')))
            .ok_or(ProtocolError::InvalidLength)?;
        if value > maximum {
            return Err(limit_error);
        }
    }
    Ok(None)
}

pub fn encode_request(arguments: &[&[u8]], output: &mut Vec<u8>) {
    write!(output, "*{}\r\n", arguments.len()).expect("writing to a Vec cannot fail");
    for argument in arguments {
        encode_bulk(Some(argument), output);
    }
}

pub fn encode_bulk(value: Option<&[u8]>, output: &mut Vec<u8>) {
    match value {
        Some(value) => {
            write!(output, "${}\r\n", value.len()).expect("writing to a Vec cannot fail");
            output.extend_from_slice(value);
            output.extend_from_slice(b"\r\n");
        }
        None => output.extend_from_slice(b"$-1\r\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fragment_of_a_binary_command_is_incomplete_until_the_end() {
        let arguments: &[&[u8]] = &[b"SET", b"\0\xff\r\n", b"value\0\r\n\xff"];
        let mut bytes = Vec::new();
        encode_request(arguments, &mut bytes);
        for split in 0..bytes.len() {
            assert_eq!(parse_request(&bytes[..split], Limits::default()), Ok(None));
        }
        let parsed = parse_request(&bytes, Limits::default()).unwrap().unwrap();
        assert_eq!(parsed.arguments, arguments);
        assert_eq!(parsed.consumed, bytes.len());
    }

    #[test]
    fn consumes_only_one_command_even_when_pipeline_exceeds_frame_limit() {
        let bytes = b"*1\r\n$4\r\nPING\r\n".repeat(20);
        let limits = Limits {
            max_frame_bytes: 14,
            ..Limits::default()
        };
        let parsed = parse_request(&bytes, limits).unwrap().unwrap();
        assert_eq!(parsed.arguments, [b"PING".as_slice()]);
        assert_eq!(parsed.consumed, 14);
    }

    #[test]
    fn empty_bulk_is_distinct_from_a_null_bulk() {
        let parsed = parse_request(b"*2\r\n$4\r\nECHO\r\n$0\r\n\r\n", Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(parsed.arguments[1], b"");
        assert_eq!(
            parse_request(b"*1\r\n$-1\r\n", Limits::default()),
            Err(ProtocolError::InvalidLength)
        );
        let mut output = Vec::new();
        encode_bulk(None, &mut output);
        encode_bulk(Some(b""), &mut output);
        assert_eq!(output, b"$-1\r\n$0\r\n\r\n");
    }

    #[test]
    fn rejects_invalid_frames_without_waiting_for_a_payload() {
        for (input, expected) in [
            (b"PING\r\n".as_slice(), ProtocolError::ExpectedArray),
            (b"*0\r\n", ProtocolError::EmptyCommand),
            (b"*-1\r\n", ProtocolError::InvalidLength),
            (b"*+1\r\n", ProtocolError::InvalidLength),
            (b"*\r\n", ProtocolError::InvalidLength),
            (b"*1\nx", ProtocolError::InvalidLength),
            (b"*1\rx", ProtocolError::InvalidTerminator),
            (b"*1\r\n+PING\r\n", ProtocolError::ExpectedBulkString),
            (b"*1\r\n$4\r\nPINGxx", ProtocolError::InvalidTerminator),
            (b"*1\r\n$4\r\nPINGx", ProtocolError::InvalidTerminator),
            (b"*1025", ProtocolError::TooManyArguments),
            (b"*1\r\n$1048577", ProtocolError::BulkTooLarge),
            (b"*000000000000000000000", ProtocolError::InvalidLength),
        ] {
            assert_eq!(
                parse_request(input, Limits::default()),
                Err(expected),
                "{input:?}"
            );
        }
    }

    #[test]
    fn checks_overflow_and_frame_limits() {
        let unlimited = Limits {
            max_arguments: usize::MAX,
            max_bulk_bytes: usize::MAX,
            max_frame_bytes: usize::MAX,
        };
        assert!(parse_request(b"*184467440737095516160\r\n", unlimited).is_err());
        assert_eq!(
            parse_request(b"*1\r\n$18446744073709551615\r\n", unlimited),
            Err(ProtocolError::FrameTooLarge)
        );
        let limits = Limits {
            max_frame_bytes: 13,
            ..Limits::default()
        };
        assert_eq!(
            parse_request(b"*1\r\n$4\r\n", limits),
            Err(ProtocolError::FrameTooLarge)
        );
    }

    #[test]
    fn deterministic_arbitrary_input_does_not_panic() {
        let limits = Limits {
            max_frame_bytes: 128,
            max_bulk_bytes: 64,
            max_arguments: 8,
        };
        let mut seed = 0x1234_5678_u64;
        for size in 0..256 {
            let mut input = Vec::new();
            for _ in 0..size {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                input.push((seed >> 32) as u8);
            }
            let _ = parse_request(&input, limits);
            for prefix in [b"*1\r\n$".as_slice(), b"*2\r\n$4\r\nPING\r\n$"] {
                let mut framed = prefix.to_vec();
                framed.extend_from_slice(&input);
                let _ = parse_request(&framed, limits);
            }
        }
    }
}
