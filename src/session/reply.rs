//! Framing-aware RESP2 to RESP3 reply conversion.

use std::io::Write;

pub(super) fn uses_hash_pairs<B: AsRef<[u8]>>(arguments: &[B]) -> bool {
    arguments.len() == 4
        && arguments[0].as_ref().eq_ignore_ascii_case(b"HRANDFIELD")
        && arguments[3].as_ref().eq_ignore_ascii_case(b"WITHVALUES")
}

pub(super) fn convert_hash_pairs(reply: &[u8]) -> Vec<u8> {
    let mut converted = Vec::with_capacity(reply.len());
    let mut offset = 0;
    if parse_hash_pairs(reply, &mut offset, &mut converted) && offset == reply.len() {
        converted
    } else {
        convert_resp2_reply(reply)
    }
}

pub(super) fn convert_exec_reply(reply: &[u8], commands: &[Vec<Vec<u8>>]) -> Vec<u8> {
    if !reply.starts_with(b"*") {
        return convert_resp2_reply(reply);
    }
    let Some((count, body)) = parse_line_number(reply, 1) else {
        return convert_resp2_reply(reply);
    };
    if usize::try_from(count).ok() != Some(commands.len()) {
        return convert_resp2_reply(reply);
    }
    let mut converted = Vec::with_capacity(reply.len());
    converted.extend_from_slice(&reply[..body]);
    let mut offset = body;
    for command in commands {
        let parsed = if uses_hash_pairs(command) {
            parse_hash_pairs(reply, &mut offset, &mut converted)
        } else {
            parse_resp_value(reply, &mut offset, &mut converted)
        };
        if !parsed {
            return convert_resp2_reply(reply);
        }
    }
    if offset != reply.len() {
        return convert_resp2_reply(reply);
    }
    converted
}

fn parse_hash_pairs(input: &[u8], offset: &mut usize, output: &mut Vec<u8>) -> bool {
    if input.get(*offset) != Some(&b'*') {
        // Errors are scalar replies even with WITHVALUES.
        return parse_resp_value(input, offset, output);
    }
    let Some((count, body)) = parse_line_number(input, *offset + 1) else {
        return false;
    };
    if count < 0 || count % 2 != 0 {
        return false;
    }
    write!(output, "*{}\r\n", count / 2).expect("Vec write");
    *offset = body;
    for _ in 0..count / 2 {
        output.extend_from_slice(b"*2\r\n");
        for _ in 0..2 {
            if input.get(*offset) != Some(&b'$') || !parse_resp_value(input, offset, output) {
                return false;
            }
        }
    }
    true
}

// Convert RESP2 values by parsing frame boundaries, never by searching payload
// bytes. Arrays are handled recursively, and malformed internal output is
// left untouched.
pub(super) fn convert_resp2_reply(reply: &[u8]) -> Vec<u8> {
    let mut converted = Vec::with_capacity(reply.len());
    let mut offset = 0;
    if parse_resp_value(reply, &mut offset, &mut converted) && offset == reply.len() {
        converted
    } else {
        reply.to_vec()
    }
}

pub(super) fn parse_resp_value(input: &[u8], offset: &mut usize, output: &mut Vec<u8>) -> bool {
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

pub(super) fn parse_line_number(input: &[u8], start: usize) -> Option<(i64, usize)> {
    let relative = input.get(start..)?.windows(2).position(|w| w == b"\r\n")?;
    let end = start + relative;
    let number = std::str::from_utf8(&input[start..end]).ok()?.parse().ok()?;
    Some((number, end + 2))
}

#[cfg(test)]
mod tests {
    use super::convert_hash_pairs;

    #[test]
    fn malformed_hash_pair_frames_are_not_partially_rewritten() {
        for input in [
            b"*1\r\n$1\r\nf\r\n".as_slice(),
            b"*2\r\n$1\r\nf\r\n$2\r\nx",
            b"*2\r\n$1\r\nf\r\n:1\r\n",
        ] {
            assert_eq!(convert_hash_pairs(input), input);
        }
    }
}
