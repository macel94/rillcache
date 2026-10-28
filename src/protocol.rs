//! Bounded, binary-safe RESP2 and Redis inline request dispatch. No payload is interpreted as UTF-8.

use std::fmt;

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
    InvalidInlineCommand,
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
            Self::InvalidInlineCommand => "invalid inline command",
        })
    }
}

impl std::error::Error for ProtocolError {}

/// Parse one RESP array command, borrowing its arguments from `input`.
///
/// `Ok(None)` means more bytes are needed. `consumed` excludes subsequent
/// pipelined commands. Advertised lengths are checked before allocating payloads.
/// Inline lines are handled by [`dispatch_request`], because unescaping may own bytes.
pub fn parse_request(input: &[u8], limits: Limits) -> Result<Option<Request<'_>>, ProtocolError> {
    decode::<Vec<&[u8]>, _>(input, limits, |arguments, consumed| Request {
        arguments,
        consumed,
    })
}

/// Invoke `execute` exactly once for a complete valid command, never for a
/// partial/error frame. RESP commands borrow up to eight stack-held argument
/// references; larger RESP commands use a bounded-growing vector. Inline command
/// tokens are owned only while the callback runs. The callback cannot retain a
/// reference to the temporary argument container.
pub fn dispatch_request<'a, T>(
    input: &'a [u8],
    limits: Limits,
    execute: impl FnOnce(&[&[u8]]) -> T,
) -> Result<Option<(T, usize)>, ProtocolError> {
    // The canonical one-argument PING frame is the hottest request in every
    // benchmark and health probe. Skip generic array/length parsing when the
    // complete exact frame is already available; fragmented or non-canonical
    // frames still take the full bounded parser.
    if input.len() >= 14
        && limits.max_arguments >= 1
        && limits.max_bulk_bytes >= 4
        && limits.max_frame_bytes >= 14
        && &input[..8] == b"*1\r\n$4\r\n"
        && input[8..12].eq_ignore_ascii_case(b"PING")
        && &input[12..14] == b"\r\n"
    {
        return Ok(Some((execute(&[b"PING"]), 14)));
    }
    if input.first().is_some_and(|&byte| byte != b'*') {
        let Some((owned, consumed)) = parse_inline(input, limits)? else {
            return Ok(None);
        };
        let arguments: Vec<&[u8]> = owned.iter().map(Vec::as_slice).collect();
        return Ok(Some((execute(&arguments), consumed)));
    }
    decode::<StackArguments<'a>, _>(input, limits, |arguments, consumed| {
        (execute(arguments.as_slice()), consumed)
    })
}

/// Parse a single inline command line using Redis's quote and escape rules.
/// The returned arguments own decoded escape bytes; raw RESP remains borrowed.
type InlineRequest = (Vec<Vec<u8>>, usize);

fn parse_inline(input: &[u8], limits: Limits) -> Result<Option<InlineRequest>, ProtocolError> {
    let Some(newline) = input.iter().position(|&byte| byte == b'\n') else {
        return if input.len() >= limits.max_frame_bytes {
            Err(ProtocolError::FrameTooLarge)
        } else {
            Ok(None)
        };
    };
    let consumed = newline + 1;
    if consumed > limits.max_frame_bytes {
        return Err(ProtocolError::FrameTooLarge);
    }
    let end = if newline > 0 && input[newline - 1] == b'\r' {
        newline - 1
    } else {
        newline
    };
    let line = &input[..end];
    let mut arguments = Vec::new();
    let mut index = 0;
    while index < line.len() {
        while index < line.len() && line[index].is_ascii_whitespace() {
            index += 1;
        }
        if index == line.len() {
            break;
        }
        if arguments.len() == limits.max_arguments {
            return Err(ProtocolError::TooManyArguments);
        }
        let mut token = Vec::new();
        let quote = match line[index] {
            b'"' => Some(b'"'),
            b'\'' => Some(b'\''),
            _ => None,
        };
        if let Some(quote) = quote {
            index += 1;
            let mut closed = false;
            while index < line.len() {
                let byte = line[index];
                if byte == quote {
                    index += 1;
                    closed = true;
                    if index < line.len() && !line[index].is_ascii_whitespace() {
                        return Err(ProtocolError::InvalidInlineCommand);
                    }
                    break;
                }
                if byte == b'\\' && index + 1 < line.len() {
                    let next = line[index + 1];
                    if quote == b'"'
                        && next == b'x'
                        && index + 3 < line.len()
                        && hex(line[index + 2]).is_some()
                        && hex(line[index + 3]).is_some()
                    {
                        token.push(
                            (hex(line[index + 2]).unwrap() << 4) | hex(line[index + 3]).unwrap(),
                        );
                        if token.len() > limits.max_bulk_bytes {
                            return Err(ProtocolError::BulkTooLarge);
                        }
                        index += 4;
                        continue;
                    }
                    if quote == b'"' || next == b'\'' {
                        token.push(if quote == b'"' {
                            match next {
                                b'n' => b'\n',
                                b'r' => b'\r',
                                b't' => b'\t',
                                b'b' => 8,
                                b'a' => 7,
                                other => other,
                            }
                        } else {
                            next
                        });
                        if token.len() > limits.max_bulk_bytes {
                            return Err(ProtocolError::BulkTooLarge);
                        }
                        index += 2;
                        continue;
                    }
                }
                token.push(byte);
                index += 1;
                if token.len() > limits.max_bulk_bytes {
                    return Err(ProtocolError::BulkTooLarge);
                }
            }
            if !closed {
                return Err(ProtocolError::InvalidInlineCommand);
            }
        } else {
            while index < line.len() && !line[index].is_ascii_whitespace() {
                token.push(line[index]);
                index += 1;
                if token.len() > limits.max_bulk_bytes {
                    return Err(ProtocolError::BulkTooLarge);
                }
            }
        }
        arguments.push(token);
    }
    Ok(Some((arguments, consumed)))
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

// Both APIs share one validator. The owned API returns its vector directly.
#[inline(always)]
fn decode<'a, S: ArgumentStorage<'a>, T>(
    input: &'a [u8],
    limits: Limits,
    finish: impl FnOnce(S, usize) -> T,
) -> Result<Option<T>, ProtocolError> {
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
    let mut arguments = S::new(count);
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
    Ok(Some(finish(arguments, offset)))
}

trait ArgumentStorage<'a>: Sized {
    fn new(count: usize) -> Self;
    fn push(&mut self, value: &'a [u8]);
}
impl<'a> ArgumentStorage<'a> for Vec<&'a [u8]> {
    #[inline]
    fn new(count: usize) -> Self {
        Vec::with_capacity(count.min(16))
    }
    #[inline]
    fn push(&mut self, value: &'a [u8]) {
        Vec::push(self, value);
    }
}
enum StackArguments<'a> {
    Inline { values: [&'a [u8]; 8], len: usize },
    Overflow(Vec<&'a [u8]>),
}
impl<'a> ArgumentStorage<'a> for StackArguments<'a> {
    #[inline]
    fn new(count: usize) -> Self {
        if count <= 8 {
            Self::Inline {
                values: [&[]; 8],
                len: 0,
            }
        }
        // Never allocate an untrusted advertised argument count eagerly.
        else {
            Self::Overflow(Vec::with_capacity(count.min(16)))
        }
    }
    #[inline]
    fn push(&mut self, value: &'a [u8]) {
        match self {
            Self::Inline { values, len } => {
                values[*len] = value;
                *len += 1;
            }
            Self::Overflow(values) => values.push(value),
        }
    }
}
impl<'a> StackArguments<'a> {
    #[inline]
    fn as_slice(&self) -> &[&'a [u8]] {
        match self {
            Self::Inline { values, len } => &values[..*len],
            Self::Overflow(values) => values,
        }
    }
}

fn incomplete<T>(input: &[u8], limits: Limits) -> Result<Option<T>, ProtocolError> {
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
    encode_length(b'*', arguments.len(), output);
    for argument in arguments {
        encode_bulk(Some(argument), output);
    }
}

pub fn encode_bulk(value: Option<&[u8]>, output: &mut Vec<u8>) {
    match value {
        Some(value) => {
            encode_length(b'$', value.len(), output);
            output.extend_from_slice(value);
            output.extend_from_slice(b"\r\n");
        }
        None => output.extend_from_slice(b"$-1\r\n"),
    }
}

// At most three decimal digits per byte of usize, plus prefix and CRLF.
// Build the entire header on the stack and append it in one bounded copy.
fn encode_length(prefix: u8, mut value: usize, output: &mut Vec<u8>) {
    let mut header = [0u8; 3 * std::mem::size_of::<usize>() + 3];
    let end = header.len();
    header[end - 2..].copy_from_slice(b"\r\n");
    let mut start = end - 2;
    loop {
        start -= 1;
        header[start] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    start -= 1;
    header[start] = prefix;
    output.extend_from_slice(&header[start..]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_commands_decode_redis_quotes_escapes_and_binary_bytes() {
        let mut wire = br#"  SET key "space\n\x00\xff" 'it\'s' raw"#.to_vec();
        wire.extend_from_slice(b"\r\n");
        let mut calls = 0;
        let parsed = dispatch_request(&wire, Limits::default(), |arguments| {
            calls += 1;
            assert_eq!(
                arguments,
                [b"SET".as_slice(), b"key", b"space\n\0\xff", b"it's", b"raw"]
            );
        })
        .unwrap();
        assert_eq!(parsed, Some(((), wire.len())));
        assert_eq!(calls, 1);
    }

    #[test]
    fn inline_fragmentation_pipeline_boundaries_and_blank_lines() {
        let wire = b"\r\nPING\n*1\r\n$4\r\nPING\r\n";
        let mut offset = 0;
        let mut requests = Vec::new();
        loop {
            match dispatch_request(&wire[offset..], Limits::default(), |args| {
                args.iter().map(|arg| arg.to_vec()).collect::<Vec<_>>()
            })
            .unwrap()
            {
                Some((args, consumed)) => {
                    requests.push(args);
                    offset += consumed;
                    if offset == wire.len() {
                        break;
                    }
                }
                None => panic!("complete pipelined command was incomplete"),
            }
        }
        assert_eq!(
            requests,
            [vec![], vec![b"PING".to_vec()], vec![b"PING".to_vec()]]
        );
        let fragmented = b"SET key value\r\n";
        for split in 0..fragmented.len() {
            assert_eq!(
                dispatch_request(&fragmented[..split], Limits::default(), |_| panic!(
                    "fragment executed"
                ))
                .unwrap(),
                None
            );
        }
    }

    #[test]
    fn malformed_or_oversized_inline_input_is_rejected_within_limits() {
        for line in [
            b"SET \"open\r\n".as_slice(),
            b"SET \"closed\"suffix\r\n",
            b"SET 'open\r\n",
        ] {
            assert_eq!(
                dispatch_request(line, Limits::default(), |_| panic!(
                    "invalid inline executed"
                )),
                Err(ProtocolError::InvalidInlineCommand)
            );
        }
        let limits = Limits {
            max_frame_bytes: 8,
            max_bulk_bytes: 3,
            max_arguments: 2,
        };
        assert_eq!(
            dispatch_request(b"PINGPING", limits, |_| panic!("oversize executed")),
            Err(ProtocolError::FrameTooLarge)
        );
        let line_limits = Limits {
            max_frame_bytes: 64,
            max_arguments: 4,
            ..limits
        };
        assert_eq!(
            dispatch_request(b"SET key value\n", line_limits, |_| panic!(
                "oversize token executed"
            )),
            Err(ProtocolError::BulkTooLarge)
        );
        let argument_limits = Limits {
            max_arguments: 2,
            ..line_limits
        };
        assert_eq!(
            dispatch_request(b"A B C\n", argument_limits, |_| panic!(
                "too many args executed"
            )),
            Err(ProtocolError::TooManyArguments)
        );
    }

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
    fn canonical_ping_fast_path_handles_fragmentation_and_case() {
        let frame = b"*1\r\n$4\r\nPING\r\n";
        for split in 0..frame.len() {
            assert_eq!(
                dispatch_request(&frame[..split], Limits::default(), |_| ()),
                Ok(None),
                "split {split}"
            );
        }
        let mut calls = 0;
        assert_eq!(
            dispatch_request(frame, Limits::default(), |arguments| {
                calls += 1;
                assert_eq!(arguments, &[b"PING"]);
            }),
            Ok(Some(((), frame.len())))
        );
        assert_eq!(calls, 1);
        assert_eq!(
            dispatch_request(b"*1\r\n$4\r\nping\r\n", Limits::default(), |arguments| {
                assert_eq!(arguments, &[b"PING"]);
            }),
            Ok(Some(((), frame.len())))
        );
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
    fn dispatch_handles_inline_and_heap_boundaries_without_changing_owned_requests() {
        for count in [1, 7, 8, 9, 16, 17, 1024] {
            let args: Vec<&[u8]> = (0..count).map(|_| b"\0\xff\r\n".as_slice()).collect();
            let mut bytes = Vec::new();
            encode_request(&args, &mut bytes);
            let size = bytes.len();
            let mut calls = 0;
            for split in [0, 1, size / 2, size - 1] {
                assert_eq!(
                    dispatch_request(&bytes[..split], Limits::default(), |_| {
                        calls += 1;
                    }),
                    Ok(None)
                );
            }
            assert_eq!(calls, 0);
            bytes.extend_from_slice(b"*1\r\n$4\r\nPING\r\n");
            assert_eq!(
                dispatch_request(&bytes, Limits::default(), |actual| {
                    calls += 1;
                    assert_eq!(actual, args);
                }),
                Ok(Some(((), size)))
            );
            assert_eq!(calls, 1);
            let parsed = parse_request(&bytes, Limits::default()).unwrap().unwrap();
            assert_eq!(parsed.arguments, args);
            assert_eq!(parsed.consumed, size);
        }
    }

    #[test]
    fn dispatch_never_executes_a_partial_or_malformed_late_argument() {
        for count in [3, 8, 9, 17] {
            let args: Vec<&[u8]> = (0..count).map(|_| b"binary\0\xff".as_slice()).collect();
            let mut bytes = Vec::new();
            encode_request(&args, &mut bytes);
            for split in 0..bytes.len() {
                assert_eq!(
                    dispatch_request(&bytes[..split], Limits::default(), |_| panic!(
                        "partial execution"
                    )),
                    Ok(None)
                );
            }
            *bytes.last_mut().unwrap() = b'x';
            assert_eq!(
                dispatch_request(&bytes, Limits::default(), |_| panic!("malformed execution")),
                Err(ProtocolError::InvalidTerminator)
            );
        }
        let unlimited = Limits {
            max_arguments: usize::MAX,
            ..Limits::default()
        };
        assert_eq!(
            dispatch_request(
                format!("*{}\r\n", usize::MAX).as_bytes(),
                unlimited,
                |_| panic!("advertised count")
            ),
            Ok(None)
        );
    }

    #[test]
    fn specialized_length_headers_match_formatting_at_all_digit_boundaries() {
        let mut values = vec![0, 1, 9, 10, 11, 63, 64, 99, 100, 255, 1024, usize::MAX];
        let mut power = 1usize;
        while let Some(next) = power.checked_mul(10) {
            values.extend([next - 1, next, next + 1]);
            power = next;
        }
        let mut seed = 0x4589_32ab_usize;
        for _ in 0..1000 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            values.push(seed);
        }
        for value in values {
            for prefix in *b"$*" {
                let mut out = b"already-buffered:".to_vec();
                encode_length(prefix, value, &mut out);
                assert_eq!(
                    out,
                    format!("already-buffered:{}{value}\r\n", char::from(prefix)).as_bytes()
                );
            }
        }
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
