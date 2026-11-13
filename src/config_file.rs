//! Bounded startup configuration files.
//!
//! This is deliberately a Rillcache startup file, not a full `redis.conf`
//! implementation: one directive and one value per line, comments, optional
//! double/single quoting, and the same names as the long command-line options.
//! Unknown or multi-value Redis directives fail closed so an operator cannot
//! believe a setting was applied when it was ignored.

const MAX_CONFIG_BYTES: usize = 1024 * 1024;

const BOOLEAN_DIRECTIVES: &[&str] = &["aof-hmac-migrate"];

const ALIASES: &[(&str, &str)] = &[
    ("maxmemory", "max-data-bytes"),
    ("maxclients", "max-clients"),
];

const SUPPORTED: &[&str] = &[
    "bind",
    "max-clients",
    "auth-failure-limit",
    "auth-source-limit",
    "auth-source-window-ms",
    "threads",
    "io-mode",
    "storage-owners",
    "socket-workers",
    "max-data-bytes",
    "max-bulk-bytes",
    "max-frame-bytes",
    "idle-timeout-ms",
    "requirepass",
    "requirepass-file",
    "aclfile",
    "audit-log",
    "audit-log-max-bytes",
    "audit-syslog",
    "audit-hmac-key-file",
    "audit-hmac-keyring-file",
    "appendonly",
    "appendfilename",
    "appendfsync",
    "aof-hmac-key-file",
    "aof-hmac-migrate",
    "dir",
    "maxmemory-policy",
    "notify-keyspace-events",
    "error-dialect",
    "tls-bind",
    "tls-cert-file",
    "tls-key-file",
    "tls-client-ca-file",
    "tls-handshake-timeout-ms",
];

/// Read and translate a startup file into command-line arguments.
pub(crate) fn load(path: &str) -> Result<Vec<String>, String> {
    if path.is_empty() {
        return Err("--config requires a nonempty path".into());
    }
    let bytes =
        std::fs::read(path).map_err(|error| format!("--config could not read {path}: {error}"))?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "--config file exceeds the {} byte limit",
            MAX_CONFIG_BYTES
        ));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| format!("--config file {path} is not valid UTF-8"))?;
    let arguments = parse_text(text)?;
    if arguments.is_empty() {
        return Err("--config file must contain at least one directive".into());
    }
    Ok(arguments)
}

fn parse_text(text: &str) -> Result<Vec<String>, String> {
    let mut arguments = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = strip_comment(raw);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let line_number = index + 1;
        let (directive, rest) = split_directive(line)
            .ok_or_else(|| format!("line {line_number}: expected a directive and a value"))?;
        let directive = normalize_directive(directive)
            .ok_or_else(|| format!("line {line_number}: empty configuration directive"))?;
        if matches!(
            directive.as_str(),
            "config" | "help" | "version" | "healthcheck"
        ) {
            return Err(format!(
                "line {line_number}: '{directive}' is not allowed inside a configuration file"
            ));
        }
        let directive = ALIASES
            .iter()
            .find_map(|(alias, canonical)| (*alias == directive).then_some(*canonical))
            .unwrap_or(directive.as_str());
        if !SUPPORTED.contains(&directive) {
            return Err(format!(
                "line {line_number}: unsupported configuration directive '{directive}'"
            ));
        }
        if BOOLEAN_DIRECTIVES.contains(&directive) {
            let value = parse_value(rest, line_number)?;
            match value.as_str() {
                "yes" => arguments.push(format!("--{directive}")),
                "no" => {}
                _ => {
                    return Err(format!(
                        "line {line_number}: '{directive}' requires yes or no"
                    ));
                }
            }
            continue;
        }
        let value = parse_value(rest, line_number)?;
        arguments.push(format!("--{directive}"));
        arguments.push(value);
    }
    Ok(arguments)
}

fn normalize_directive(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_start_matches('-');
    (!raw.is_empty()).then(|| raw.replace('_', "-").to_ascii_lowercase())
}

fn split_directive(line: &str) -> Option<(&str, &str)> {
    let split = line
        .find(|character: char| character.is_whitespace() || character == '=')
        .filter(|index| *index > 0)?;
    let directive = line[..split].trim_end_matches('=').trim();
    let rest = line[split..].trim_start_matches([' ', '\t', '=']).trim();
    (!directive.is_empty() && !rest.is_empty()).then_some((directive, rest))
}

/// Remove an unquoted `#` comment while preserving `#` inside quoted values.
fn strip_comment(raw: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        match character {
            '"' | '\'' if quote == Some(character) => quote = None,
            '"' | '\'' if quote.is_none() => quote = Some(character),
            '#' if quote.is_none() => return &raw[..index],
            _ => {}
        }
    }
    raw
}

fn parse_value(raw: &str, line_number: usize) -> Result<String, String> {
    let raw = raw.trim();
    if let Some(body) = raw.strip_prefix('"') {
        return parse_double_quoted(body, line_number);
    }
    if let Some(body) = raw.strip_prefix('\'') {
        return parse_single_quoted(body, line_number);
    }
    if raw.split_whitespace().count() != 1 {
        return Err(format!(
            "line {line_number}: directive expects exactly one value"
        ));
    }
    Ok(raw.to_owned())
}

fn parse_double_quoted(body: &str, line_number: usize) -> Result<String, String> {
    let mut value = String::new();
    let mut characters = body.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' => {
                let trailing: String = characters.collect();
                if !trailing.trim().is_empty() {
                    return Err(format!(
                        "line {line_number}: unexpected text after the closing quote"
                    ));
                }
                return Ok(value);
            }
            '\\' => {
                let escaped = characters.next().ok_or_else(|| {
                    format!("line {line_number}: unterminated escape in quoted value")
                })?;
                match escaped {
                    '"' => value.push('"'),
                    '\\' => value.push('\\'),
                    'n' => value.push('\n'),
                    'r' => value.push('\r'),
                    't' => value.push('\t'),
                    other => {
                        return Err(format!(
                            "line {line_number}: unsupported escape '\\{other}'"
                        ));
                    }
                }
            }
            other => value.push(other),
        }
    }
    Err(format!("line {line_number}: unterminated quoted value"))
}

fn parse_single_quoted(body: &str, line_number: usize) -> Result<String, String> {
    let Some(end) = body.find('\'') else {
        return Err(format!("line {line_number}: unterminated quoted value"));
    };
    if !body[end + 1..].trim().is_empty() {
        return Err(format!(
            "line {line_number}: unexpected text after the closing quote"
        ));
    }
    Ok(body[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_aliases_and_quoted_values() {
        let text = r#"
# startup
bind 127.0.0.1:6380
maxmemory 4096
requirepass "a#b c"
dir '/var/lib/rillcache data'
appendonly yes
aof-hmac-migrate no
"#;
        assert_eq!(
            parse_text(text).unwrap(),
            vec![
                "--bind",
                "127.0.0.1:6380",
                "--max-data-bytes",
                "4096",
                "--requirepass",
                "a#b c",
                "--dir",
                "/var/lib/rillcache data",
                "--appendonly",
                "yes",
            ]
        );
    }

    #[test]
    fn escapes_and_equals_are_supported() {
        assert_eq!(
            parse_text("requirepass=\"a\\\"b\\\\c\"\nmax-clients=12\n").unwrap(),
            vec!["--requirepass", "a\"b\\c", "--max-clients", "12"]
        );
    }

    #[test]
    fn unknown_or_ambiguous_lines_fail_closed() {
        for (text, expected) in [
            (
                "include /etc/redis.conf\n",
                "unsupported configuration directive",
            ),
            ("save 900 1\n", "unsupported configuration directive"),
            ("bind 127.0.0.1:6379 extra\n", "exactly one value"),
            ("bind\n", "expected a directive"),
            ("--config other.conf\n", "not allowed"),
            ("requirepass \"unterminated\n", "unterminated"),
            ("requirepass \"a\" trailing\n", "after the closing quote"),
            ("requirepass \"bad\\q\"\n", "unsupported escape"),
        ] {
            let error = parse_text(text).unwrap_err();
            assert!(error.contains(expected), "{text:?}: {error}");
        }
    }

    #[test]
    fn booleans_must_be_explicit() {
        assert_eq!(
            parse_text("aof-hmac-migrate yes\n").unwrap(),
            vec!["--aof-hmac-migrate"]
        );
        assert!(parse_text("aof-hmac-migrate true\n").is_err());
    }

    #[test]
    fn multiline_values_are_rejected() {
        assert!(parse_text("requirepass \"a\nb\"\n").is_err());
    }
}
