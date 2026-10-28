//! Framing-aware RESP2 to RESP3 reply conversion.

// Convert RESP2 values by parsing frame boundaries, never by searching payload
// bytes. The command surface emits flat scalar replies; arrays are handled for
// future aggregate children and malformed internal output is left untouched.
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
