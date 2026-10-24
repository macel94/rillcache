pub(super) const SYNTAX: &[u8] = b"-ERR syntax error\r\n";
pub(super) const NOT_INTEGER: &[u8] = b"-ERR value is not an integer or out of range\r\n";
pub(super) const OOM: &[u8] = b"-OOM configured payload limit exceeded\r\n";

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Condition {
    #[default]
    Always,
    Missing,
    Present,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ExpiryKind {
    Ex,
    Px,
    Exat,
    Pxat,
}
impl ExpiryKind {
    fn parse(value: &[u8]) -> Option<Self> {
        if value.eq_ignore_ascii_case(b"EX") {
            Some(Self::Ex)
        } else if value.eq_ignore_ascii_case(b"PX") {
            Some(Self::Px)
        } else if value.eq_ignore_ascii_case(b"EXAT") {
            Some(Self::Exat)
        } else if value.eq_ignore_ascii_case(b"PXAT") {
            Some(Self::Pxat)
        } else {
            None
        }
    }
}
#[derive(Clone, Copy, Default)]
pub(super) struct SetOptions<'a> {
    pub condition: Condition,
    pub get: bool,
    pub keep: bool,
    pub persist: bool,
    pub expiry: Option<(ExpiryKind, &'a [u8])>,
}
impl<'a> SetOptions<'a> {
    pub fn parse(arguments: &[&'a [u8]], getex: bool) -> Result<Self, &'static [u8]> {
        let mut options = Self::default();
        let mut index = 0;
        while index < arguments.len() {
            let option = arguments[index];
            if !getex && option.eq_ignore_ascii_case(b"GET") {
                options.get = true;
            } else if !getex
                && (option.eq_ignore_ascii_case(b"NX") || option.eq_ignore_ascii_case(b"XX"))
            {
                let next = if option.eq_ignore_ascii_case(b"NX") {
                    Condition::Missing
                } else {
                    Condition::Present
                };
                if options.condition != Condition::Always && options.condition != next {
                    return Err(SYNTAX);
                }
                options.condition = next;
            } else if !getex && option.eq_ignore_ascii_case(b"KEEPTTL") && options.expiry.is_none()
            {
                options.keep = true;
            } else if getex && option.eq_ignore_ascii_case(b"PERSIST") && options.expiry.is_none() {
                options.persist = true;
            } else if let Some(kind) = ExpiryKind::parse(option) {
                if options.keep
                    || options.persist
                    || options.expiry.is_some_and(|(previous, _)| previous != kind)
                {
                    return Err(SYNTAX);
                }
                index += 1;
                let value = *arguments.get(index).ok_or(SYNTAX)?;
                // Upstream allows repeated identical expiry kinds; the last value wins.
                options.expiry = Some((kind, value));
            } else {
                return Err(SYNTAX);
            }
            index += 1;
        }
        Ok(options)
    }

    pub fn deadline(self, now: i64, command: &str) -> Result<Option<i64>, Vec<u8>> {
        let Some((kind, raw)) = self.expiry else {
            return Ok(None);
        };
        let value = parse_integer(raw).ok_or_else(|| NOT_INTEGER.to_vec())?;
        let invalid =
            || format!("-ERR invalid expire time in '{command}' command\r\n").into_bytes();
        if value <= 0 {
            return Err(invalid());
        }
        let value = if matches!(kind, ExpiryKind::Ex | ExpiryKind::Exat) {
            value.checked_mul(1000).ok_or_else(invalid)?
        } else {
            value
        };
        let value = if matches!(kind, ExpiryKind::Ex | ExpiryKind::Px) {
            value.checked_add(now).ok_or_else(invalid)?
        } else {
            value
        };
        Ok(Some(value))
    }
}

pub(super) fn parse_integer(raw: &[u8]) -> Option<i64> {
    let text = std::str::from_utf8(raw).ok()?;
    let value = text.parse::<i64>().ok()?;
    // Redis rejects leading zeros, '+', '-0', whitespace and trailing bytes.
    (value.to_string().as_bytes() == raw).then_some(value)
}

#[derive(Default)]
pub(super) struct ExpireFlags {
    pub nx: bool,
    pub xx: bool,
    pub gt: bool,
    pub lt: bool,
}
impl ExpireFlags {
    pub fn parse(arguments: &[&[u8]]) -> Result<Self, &'static [u8]> {
        let mut flags = Self::default();
        for option in arguments {
            if option.eq_ignore_ascii_case(b"NX") {
                flags.nx = true;
            } else if option.eq_ignore_ascii_case(b"XX") {
                flags.xx = true;
            } else if option.eq_ignore_ascii_case(b"GT") {
                flags.gt = true;
            } else if option.eq_ignore_ascii_case(b"LT") {
                flags.lt = true;
            } else {
                return Err(b"-ERR unsupported expire option\r\n");
            }
        }
        if flags.nx && (flags.xx || flags.gt || flags.lt) {
            return Err(
                b"-ERR NX and XX, GT or LT options at the same time are not compatible\r\n",
            );
        }
        if flags.gt && flags.lt {
            return Err(b"-ERR GT and LT options at the same time are not compatible\r\n");
        }
        Ok(flags)
    }
}
