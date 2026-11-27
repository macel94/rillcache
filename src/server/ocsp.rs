//! Semantic validation of stapled OCSP responses (readiness S5, native path).
//!
//! rustls treats the stapled bytes as opaque, so a malformed, revoked, unknown or
//! stale response would otherwise be served as-is and silently downgrade the
//! client's revocation check. This module parses the bounded DER subset needed to
//! enforce the server side of RFC 6960:
//!
//! * `OCSPResponse.responseStatus` must be `successful(0)`,
//! * the response type must be `id-pkix-ocsp-basic`,
//! * every `SingleResponse` must carry status `good` (revoked/unknown rejected),
//! * `producedAt` and every `thisUpdate` must not be in the future beyond a small
//!   clock skew, and
//! * every `thisUpdate` must carry a `nextUpdate` that has not expired; a missing
//!   `nextUpdate` is rejected so the TLS generation fails closed instead of
//!   serving a staple whose freshness cannot be established.
//!
//! What this deliberately does not do: `certID` binding (issuer name/key hash and
//! serial) needs an X.509 subject/public-key parser, which is outside the reviewed
//! dependency set. RFC 6960 requires the client to verify that binding, and the
//! limitation is documented in `docs/tls.md` rather than hidden.

/// Allowed clock skew for `producedAt`/`thisUpdate` in the future.
const CLOCK_SKEW_SECONDS: u64 = 300;
/// Upper bound on `SingleResponse` entries we are willing to scan.
const MAX_SINGLE_RESPONSES: usize = 256;

const TAG_INTEGER: u8 = 0x02;
const TAG_OID: u8 = 0x06;
const TAG_OCTET_STRING: u8 = 0x04;
const TAG_GENERALIZED_TIME: u8 = 0x18;
const TAG_ENUMERATED: u8 = 0x0a;
const TAG_SEQUENCE: u8 = 0x30;
const TAG_CONTEXT_0: u8 = 0xa0;
const TAG_CONTEXT_1: u8 = 0xa1;
const TAG_CERT_STATUS_GOOD: u8 = 0x80;
const TAG_CERT_STATUS_REVOKED: u8 = 0x81;
const TAG_CERT_STATUS_UNKNOWN: u8 = 0x82;

/// `id-pkix-ocsp-basic`: 1.3.6.1.5.5.7.48.1.1.
const OID_OCSP_BASIC: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x30, 0x01, 0x01];

/// Validate a stapled OCSP response against the current clock.
pub(crate) fn validate_staple(bytes: &[u8], now_unix: u64) -> Result<(), String> {
    let mut outer = Reader::new(bytes, "OCSPResponse");
    let response = outer
        .value(TAG_SEQUENCE)
        .ok_or_else(|| "OCSP response is not a DER SEQUENCE".to_string())?;
    outer.finish()?;

    let mut fields = Reader::new(response, "OCSPResponse fields");
    let status = fields
        .value(TAG_ENUMERATED)
        .ok_or_else(|| "OCSP responseStatus is missing".to_string())?;
    let status = single_byte_unsigned(status)
        .ok_or_else(|| "OCSP responseStatus is not a bounded ENUMERATED".to_string())?;
    if status != 0 {
        return Err(format!(
            "OCSP responseStatus is {status}, not successful(0); a non-successful staple must not be served"
        ));
    }
    let response_bytes = fields
        .value(TAG_CONTEXT_0)
        .ok_or_else(|| "OCSP successful response carries no responseBytes".to_string())?;

    let mut wrapping = Reader::new(response_bytes, "ResponseBytes");
    let wrapped = wrapping
        .value(TAG_SEQUENCE)
        .ok_or_else(|| "OCSP responseBytes is not a DER SEQUENCE".to_string())?;
    wrapping.finish()?;

    let mut inner = Reader::new(wrapped, "ResponseBytes fields");
    let oid = inner
        .value(TAG_OID)
        .ok_or_else(|| "OCSP responseType is missing".to_string())?;
    if oid != OID_OCSP_BASIC {
        return Err("OCSP responseType is not id-pkix-ocsp-basic".to_string());
    }
    let basic = inner
        .value(TAG_OCTET_STRING)
        .ok_or_else(|| "OCSP response OCTET STRING is missing".to_string())?;
    inner.finish()?;

    validate_basic_response(basic, now_unix)
}

fn validate_basic_response(bytes: &[u8], now_unix: u64) -> Result<(), String> {
    let mut outer = Reader::new(bytes, "BasicOCSPResponse");
    let content = outer
        .value(TAG_SEQUENCE)
        .ok_or_else(|| "BasicOCSPResponse is not a DER SEQUENCE".to_string())?;
    outer.finish()?;

    let mut fields = Reader::new(content, "BasicOCSPResponse fields");
    let tbs = fields
        .value(TAG_SEQUENCE)
        .ok_or_else(|| "BasicOCSPResponse has no tbsResponseData".to_string())?;
    // signatureAlgorithm, then signature; both are only present to prove the
    // response is well formed. The client verifies the signature over tbs.
    fields
        .value(TAG_SEQUENCE)
        .ok_or_else(|| "BasicOCSPResponse signatureAlgorithm is missing".to_string())?;
    let signature = fields
        .value(0x03)
        .ok_or_else(|| "BasicOCSPResponse signature is missing".to_string())?;
    if signature.len() < 2 {
        return Err("BasicOCSPResponse signature is empty".to_string());
    }
    validate_response_data(tbs, now_unix)
}

fn validate_response_data(content: &[u8], now_unix: u64) -> Result<(), String> {
    let mut fields = Reader::new(content, "ResponseData");
    if fields.peek() == Some(TAG_CONTEXT_0) {
        let version = fields
            .value(TAG_CONTEXT_0)
            .ok_or_else(|| "ResponseData version is malformed".to_string())?;
        let mut version_reader = Reader::new(version, "version");
        let value = version_reader
            .value(TAG_INTEGER)
            .ok_or_else(|| "ResponseData version is not an INTEGER".to_string())?;
        version_reader.finish()?;
        if single_byte_unsigned(value) != Some(0) {
            return Err("ResponseData version is not v1(0)".to_string());
        }
    }
    match fields.peek() {
        Some(TAG_SEQUENCE) => {
            fields
                .value(TAG_SEQUENCE)
                .ok_or_else(|| "ResponseData responderID is malformed".to_string())?;
        }
        Some(TAG_CONTEXT_1) => {
            fields
                .value(TAG_CONTEXT_1)
                .ok_or_else(|| "ResponseData responderID is malformed".to_string())?;
        }
        _ => return Err("ResponseData has no responderID".to_string()),
    }
    let produced_at = fields
        .value(TAG_GENERALIZED_TIME)
        .ok_or_else(|| "ResponseData producedAt is missing".to_string())?;
    let produced_at = parse_generalized_time(produced_at, "producedAt")?;
    if produced_at > now_unix.saturating_add(CLOCK_SKEW_SECONDS) {
        return Err("OCSP producedAt is in the future beyond the allowed clock skew".to_string());
    }
    let responses = fields
        .value(TAG_SEQUENCE)
        .ok_or_else(|| "ResponseData responses SEQUENCE is missing".to_string())?;

    let mut list = Reader::new(responses, "responses");
    let mut seen = 0usize;
    while !list.is_empty() {
        seen += 1;
        if seen > MAX_SINGLE_RESPONSES {
            return Err(format!(
                "OCSP response carries more than {MAX_SINGLE_RESPONSES} SingleResponse entries"
            ));
        }
        let entry = list
            .value(TAG_SEQUENCE)
            .ok_or_else(|| "SingleResponse is not a DER SEQUENCE".to_string())?;
        validate_single_response(entry, now_unix)?;
    }
    if seen == 0 {
        return Err("OCSP response carries no SingleResponse entry".to_string());
    }
    // Trailing responseExtensions are permitted, so only the required fields are
    // consumed; nothing after `responses` is interpreted.
    Ok(())
}

fn validate_single_response(bytes: &[u8], now_unix: u64) -> Result<(), String> {
    let mut fields = Reader::new(bytes, "SingleResponse");
    // CertID: identifier and its contents are checked by the client (see module docs).
    fields
        .value(TAG_SEQUENCE)
        .ok_or_else(|| "SingleResponse certID is missing".to_string())?;
    let status_tag = fields
        .peek()
        .ok_or_else(|| "SingleResponse certStatus is missing".to_string())?;
    match status_tag {
        TAG_CERT_STATUS_GOOD => {
            fields
                .value(TAG_CERT_STATUS_GOOD)
                .ok_or_else(|| "certStatus good is malformed".to_string())?;
        }
        TAG_CERT_STATUS_REVOKED => {
            return Err(
                "OCSP certStatus is revoked; a revoked staple must not be served".to_string(),
            );
        }
        TAG_CERT_STATUS_UNKNOWN => {
            return Err(
                "OCSP certStatus is unknown; an unknown staple must not be served".to_string(),
            );
        }
        other => {
            return Err(format!(
                "OCSP certStatus tag 0x{other:02x} is not recognised"
            ));
        }
    }
    let this_update = fields
        .value(TAG_GENERALIZED_TIME)
        .ok_or_else(|| "SingleResponse thisUpdate is missing".to_string())?;
    let this_update = parse_generalized_time(this_update, "thisUpdate")?;
    if this_update > now_unix.saturating_add(CLOCK_SKEW_SECONDS) {
        return Err("OCSP thisUpdate is in the future beyond the allowed clock skew".to_string());
    }
    let next_update = fields
        .optional(TAG_CONTEXT_0)
        .ok_or_else(|| {
            "OCSP SingleResponse has no nextUpdate; serving an unverifiable freshness window is refused"
                .to_string()
        })?
        .ok_or_else(|| "nextUpdate is malformed".to_string())?;
    let mut next_reader = Reader::new(next_update, "nextUpdate");
    let next_update = next_reader
        .value(TAG_GENERALIZED_TIME)
        .ok_or_else(|| "nextUpdate is not a GeneralizedTime".to_string())?;
    next_reader.finish()?;
    let next_update = parse_generalized_time(next_update, "nextUpdate")?;
    if next_update <= now_unix {
        return Err(format!(
            "OCSP thisUpdate {this_update} / nextUpdate {next_update} is stale at {now_unix}"
        ));
    }
    if next_update <= this_update {
        return Err("OCSP nextUpdate is not after thisUpdate".to_string());
    }
    // SingleResponse extensions, if any, are not needed for the enforced policy.
    Ok(())
}

/// Minimal DER reader: bounded lengths, no indefinite form, no high tag numbers.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], what: &'static str) -> Self {
        Self {
            bytes,
            pos: 0,
            what,
        }
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn finish(&self) -> Result<(), String> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "{} carries trailing bytes after the parsed fields",
                self.what
            ))
        }
    }

    /// Consume one TLV whose tag must match, returning its content.
    fn value(&mut self, tag: u8) -> Option<&'a [u8]> {
        if self.peek()? != tag {
            return None;
        }
        let (start, len) = self.header()?;
        let end = start.checked_add(len)?;
        let content = self.bytes.get(start..end)?;
        self.pos = end;
        Some(content)
    }

    /// `Ok(Some(content))` when the tag matches, `Ok(None)` when it is absent.
    fn optional(&mut self, tag: u8) -> Option<Option<&'a [u8]>> {
        if self.peek()? == tag {
            self.value(tag).map(Some)
        } else {
            Some(None)
        }
    }

    fn header(&mut self) -> Option<(usize, usize)> {
        let tag = *self.bytes.get(self.pos)?;
        if tag & 0x1f == 0x1f || tag == 0 {
            return None;
        }
        let first = *self.bytes.get(self.pos + 1)?;
        let (header_len, len) = if first < 0x80 {
            (2usize, first as usize)
        } else {
            let octets = (first & 0x7f) as usize;
            if octets == 0 || octets > 4 {
                return None;
            }
            let len_start = self.pos.checked_add(2)?;
            let len_end = len_start.checked_add(octets)?;
            let encoded = self.bytes.get(len_start..len_end)?;
            if encoded.first() == Some(&0) {
                return None;
            }
            let mut len = 0usize;
            for byte in encoded {
                len = len.checked_mul(256)?.checked_add(*byte as usize)?;
            }
            if len < 0x80 {
                return None;
            }
            (2 + octets, len)
        };
        let content_start = self.pos.checked_add(header_len)?;
        if content_start.checked_add(len)? > self.bytes.len() {
            return None;
        }
        self.pos = content_start;
        Some((content_start, len))
    }
}

fn single_byte_unsigned(bytes: &[u8]) -> Option<u8> {
    match bytes {
        [value] => Some(*value),
        [0, value] if *value < 0x80 => Some(*value),
        _ => None,
    }
}

/// Parse `YYMMDDHHMMSSZ` GeneralizedTime (the form OCSP uses) into Unix seconds.
fn parse_generalized_time(bytes: &[u8], field: &str) -> Result<u64, String> {
    if bytes.len() != 13 || bytes[12] != b'Z' {
        return Err(format!(
            "OCSP {field} is not a 13-byte YYMMDDHHMMSSZ GeneralizedTime"
        ));
    }
    let digits = |range: std::ops::Range<usize>| -> Result<u64, String> {
        let slice = &bytes[range];
        let mut value = 0u64;
        for byte in slice {
            if !byte.is_ascii_digit() {
                return Err(format!("OCSP {field} contains a non-digit"));
            }
            value = value * 10 + u64::from(byte - b'0');
        }
        Ok(value)
    };
    let year = 2000 + digits(0..2)?;
    let month = digits(2..4)?;
    let day = digits(4..6)?;
    let hour = digits(6..8)?;
    let minute = digits(8..10)?;
    let second = digits(10..12)?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(format!("OCSP {field} is not a valid calendar time"));
    }
    Ok(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    // 719_468 is the offset from the civil epoch to 1970-01-01.
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::validate_staple;

    const NOW: u64 = 1_800_000_000; // 2027-01-15T08:00:00Z
    const FRESH: &str = "270115070000Z";
    const NEXT: &str = "270115090000Z";
    const EXPIRED: &str = "270114070000Z";
    const FUTURE: &str = "270115090000Z";

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        let len = content.len();
        if len < 0x80 {
            out.push(len as u8);
        } else if len < 0x100 {
            out.extend_from_slice(&[0x81, len as u8]);
        } else {
            out.extend_from_slice(&[0x82, (len >> 8) as u8, len as u8]);
        }
        out.extend_from_slice(content);
        out
    }

    fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
        tlv(0x30, &parts.concat())
    }

    fn gt(value: &str) -> Vec<u8> {
        tlv(0x18, value.as_bytes())
    }

    fn single(status: u8, this_update: &str, next_update: Option<&str>) -> Vec<u8> {
        let mut parts = vec![
            seq(&[seq(&[
                tlv(0x06, &[0x2b, 0x0e, 0x03, 0x02, 0x1a]),
                tlv(0x04, &[1; 20]),
                tlv(0x04, &[2; 20]),
                tlv(0x02, &[0x01]),
            ])]),
            tlv(status, &[]),
            gt(this_update),
        ];
        if let Some(next) = next_update {
            parts.push(tlv(0xa0, &gt(next)));
        }
        seq(&parts)
    }

    fn response(entries: &[Vec<u8>], produced_at: &str) -> Vec<u8> {
        response_with(0, entries, produced_at)
    }

    fn response_with(status: u8, entries: &[Vec<u8>], produced_at: &str) -> Vec<u8> {
        let tbs = seq(&[
            seq(&[tlv(0x31, &tlv(0x02, &[0x01]))]),
            gt(produced_at),
            seq(entries),
        ]);
        let basic = seq(&[
            tbs,
            seq(&[tlv(0x06, &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02])]),
            tlv(0x03, &[0x00, 0xaa, 0xbb]),
        ]);
        seq(&[
            tlv(0x0a, &[status]),
            tlv(
                0xa0,
                &seq(&[
                    tlv(
                        0x06,
                        &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x30, 0x01, 0x01],
                    ),
                    tlv(0x04, &basic),
                ]),
            ),
        ])
    }

    #[test]
    fn fresh_good_staple_is_accepted() {
        let bytes = response(&[single(0x80, FRESH, Some(NEXT))], FRESH);
        assert_eq!(validate_staple(&bytes, NOW), Ok(()));
    }

    #[test]
    fn multiple_good_entries_are_accepted() {
        let bytes = response(
            &[
                single(0x80, FRESH, Some(NEXT)),
                single(0x80, FRESH, Some(NEXT)),
            ],
            FRESH,
        );
        assert_eq!(validate_staple(&bytes, NOW), Ok(()));
    }

    #[test]
    fn revoked_staple_is_rejected() {
        let bytes = response(&[single(0x81, FRESH, Some(NEXT))], FRESH);
        let error = validate_staple(&bytes, NOW).unwrap_err();
        assert!(error.contains("revoked"), "{error}");
    }

    #[test]
    fn unknown_staple_is_rejected() {
        let bytes = response(&[single(0x82, FRESH, Some(NEXT))], FRESH);
        let error = validate_staple(&bytes, NOW).unwrap_err();
        assert!(error.contains("unknown"), "{error}");
    }

    #[test]
    fn one_revoked_entry_rejects_the_whole_staple() {
        let bytes = response(
            &[
                single(0x80, FRESH, Some(NEXT)),
                single(0x81, FRESH, Some(NEXT)),
            ],
            FRESH,
        );
        assert!(validate_staple(&bytes, NOW).is_err());
    }

    #[test]
    fn missing_next_update_is_rejected() {
        let bytes = response(&[single(0x80, FRESH, None)], FRESH);
        let error = validate_staple(&bytes, NOW).unwrap_err();
        assert!(error.contains("nextUpdate"), "{error}");
    }

    #[test]
    fn expired_next_update_is_rejected() {
        let bytes = response(&[single(0x80, FRESH, Some(EXPIRED))], FRESH);
        let error = validate_staple(&bytes, NOW).unwrap_err();
        assert!(error.contains("stale"), "{error}");
    }

    #[test]
    fn future_this_update_is_rejected_beyond_skew() {
        let bytes = response(&[single(0x80, FUTURE, Some(FUTURE))], FRESH);
        let error = validate_staple(&bytes, NOW).unwrap_err();
        assert!(error.contains("future"), "{error}");
    }

    #[test]
    fn future_produced_at_is_rejected_beyond_skew() {
        let bytes = response(&[single(0x80, FRESH, Some(NEXT))], FUTURE);
        let error = validate_staple(&bytes, NOW).unwrap_err();
        assert!(error.contains("producedAt"), "{error}");
    }

    #[test]
    fn non_successful_response_status_is_rejected() {
        let bytes = response_with(3, &[single(0x80, FRESH, Some(NEXT))], FRESH);
        let error = validate_staple(&bytes, NOW).unwrap_err();
        assert!(error.contains("successful(0)"), "{error}");
    }

    #[test]
    fn wrong_response_type_is_rejected() {
        let tbs = seq(&[
            seq(&[tlv(0x31, &tlv(0x02, &[0x01]))]),
            gt(FRESH),
            seq(&[single(0x80, FRESH, Some(NEXT))]),
        ]);
        let basic = seq(&[tbs, seq(&[tlv(0x06, &[1, 2, 3])]), tlv(0x03, &[0x00, 0xaa])]);
        let bytes = seq(&[
            tlv(0x0a, &[0x00]),
            tlv(0xa0, &seq(&[tlv(0x06, &[0x2a, 0x03]), tlv(0x04, &basic)])),
        ]);
        let error = validate_staple(&bytes, NOW).unwrap_err();
        assert!(error.contains("id-pkix-ocsp-basic"), "{error}");
    }

    #[test]
    fn response_without_entries_is_rejected() {
        let bytes = response(&[], FRESH);
        let error = validate_staple(&bytes, NOW).unwrap_err();
        assert!(error.contains("no SingleResponse"), "{error}");
    }

    #[test]
    fn truncated_response_is_rejected() {
        let mut bytes = response(&[single(0x80, FRESH, Some(NEXT))], FRESH);
        bytes.truncate(bytes.len() - 3);
        assert!(validate_staple(&bytes, NOW).is_err());
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = response(&[single(0x80, FRESH, Some(NEXT))], FRESH);
        bytes.push(0x00);
        assert!(validate_staple(&bytes, NOW).is_err());
    }

    #[test]
    fn indefinite_lengths_are_rejected() {
        // SEQUENCE with the BER indefinite-length form (0x80) must not be accepted.
        let bytes = [0x30, 0x80, 0x0a, 0x01, 0x00, 0x00, 0x00];
        assert!(validate_staple(&bytes, NOW).is_err());
    }

    #[test]
    fn generalized_time_must_be_zulu_with_seconds() {
        let bytes = response(&[single(0x80, "2701150700Z", Some(NEXT))], FRESH);
        assert!(validate_staple(&bytes, NOW).is_err());
    }
}
