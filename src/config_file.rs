//! Bounded startup configuration files.
//!
//! This is deliberately a Rillcache startup file, not a full `redis.conf`
//! implementation: one directive and one value per line, comments, optional
//! double/single quoting, and the same names as the long command-line options.
//! Unknown or multi-value Redis directives fail closed so an operator cannot
//! believe a setting was applied when it was ignored.

use std::io::{self, Read};

const MAX_CONFIG_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_TOTAL_CONFIG_BYTES: usize = 4 * MAX_CONFIG_BYTES;
const CONFIG_VERSION: &str = "1";

const ALIASES: &[(&str, &str)] = &[
    ("maxmemory", "max-data-bytes"),
    ("maxclients", "max-clients"),
];

const SUPPORTED: &[&str] = &[
    "bind",
    "max-clients",
    "auth-failure-limit",
    "auth-source-limit",
    "auth-global-limit",
    "auth-source-window-ms",
    "threads",
    "io-mode",
    "storage-owners",
    "socket-workers",
    "max-data-bytes",
    "memory-bound-bytes",
    "memory-bound-headroom-bytes",
    "max-bulk-bytes",
    "max-frame-bytes",
    "idle-timeout-ms",
    "frame-assembly-timeout-ms",
    "requirepass",
    "requirepass-file",
    "secure-secret-files",
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
    "aof-anchor-file",
    "dir",
    "maxmemory-policy",
    "notify-keyspace-events",
    "error-dialect",
    "tls-bind",
    "tls-cert-file",
    "tls-key-file",
    "tls-ocsp-file",
    "tls-client-ca-file",
    "tls-client-crl-file",
    "tls-client-cert-user",
    "tls-client-cert-users-file",
    "tls-reload-trigger",
    "tls-sni-cert-file",
    "tls-sni-key-file",
    "tls-sni-ocsp-file",
    "tls-handshake-timeout-ms",
];

/// Read and translate a startup file into command-line arguments.
pub(crate) fn load(path: &str, remaining: &mut usize) -> Result<Vec<String>, String> {
    if path.is_empty() {
        return Err("--config requires a nonempty path".into());
    }
    let limit = MAX_CONFIG_BYTES.min(*remaining);
    let bytes = std::fs::File::open(path)
        .and_then(|file| read_capped(file, limit))
        .map_err(|error| format!("--config could not read {path}: {error}"))?;
    if bytes.len() > limit {
        if limit < MAX_CONFIG_BYTES {
            return Err(format!(
                "--config files exceed the {MAX_TOTAL_CONFIG_BYTES} byte total limit"
            ));
        }
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
    *remaining -= bytes.len();
    Ok(arguments)
}

fn read_capped(reader: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    // Read one extra byte so an oversized file cannot pass as a valid prefix.
    let mut bytes = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn parse_text(text: &str) -> Result<Vec<String>, String> {
    let mut arguments = Vec::new();
    let mut first_directive = true;
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
        let is_first = std::mem::take(&mut first_directive);
        if directive == "config-version" {
            if !is_first {
                return Err(format!(
                    "line {line_number}: config-version must appear once, as the first directive"
                ));
            }
            if parse_value(rest, line_number)? != CONFIG_VERSION {
                return Err(format!(
                    "line {line_number}: unsupported configuration version (expected {CONFIG_VERSION})"
                ));
            }
            continue;
        }
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
        let value = parse_value(rest, line_number)?;
        match (directive, value.as_str()) {
            ("secure-secret-files", "yes" | "no") => {
                arguments.push(format!("--{directive}={value}"));
            }
            ("aof-hmac-migrate", "yes") => arguments.push(format!("--{directive}")),
            ("aof-hmac-migrate", "no") => {}
            ("secure-secret-files" | "aof-hmac-migrate", _) => {
                return Err(format!(
                    "line {line_number}: '{directive}' requires yes or no"
                ));
            }
            _ => {
                arguments.push(format!("--{directive}"));
                arguments.push(value);
            }
        }
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
    fn startup_read_does_not_consume_past_the_oversize_sentinel() {
        let data = vec![b' '; MAX_CONFIG_BYTES + 32];
        let mut reader = io::Cursor::new(data);
        assert_eq!(
            read_capped(&mut reader, MAX_CONFIG_BYTES).unwrap().len(),
            MAX_CONFIG_BYTES + 1
        );
        assert_eq!(reader.position() as usize, MAX_CONFIG_BYTES + 1);
        let mut reader = std::io::Cursor::new(b"12345");
        assert_eq!(read_capped(&mut reader, 2).unwrap().as_slice(), b"123");
        assert_eq!(reader.position(), 3);
    }

    #[test]
    fn version_marker_is_file_metadata_and_unversioned_files_are_unchanged() {
        let legacy = parse_text("maxclients=12\n").unwrap();
        assert_eq!(legacy, vec!["--max-clients", "12"]);
        assert_eq!(
            parse_text("# before header\n\nconfig-version 1 # v1\nmaxclients=12\n").unwrap(),
            legacy
        );
        for (text, expected) in [
            ("config-version 2\nmaxclients 12", "line 1: unsupported"),
            ("maxclients 12\nconfig-version 1", "line 2: config-version"),
            (
                "config-version 1\nconfig-version 1\nmaxclients 12",
                "line 2: config-version",
            ),
            ("config-version 1 unexpected", "exactly one value"),
        ] {
            let error = parse_text(text).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn documented_startup_file_parses_with_the_current_explicit_version() {
        let docs = include_str!("../docs/configuration.md");
        let (_, example) = docs.split_once("```conf\n").unwrap();
        let (example, _) = example.split_once("\n```").unwrap();
        let header = format!("config-version {CONFIG_VERSION}");
        assert!(example.lines().any(|line| line == header));
        assert!(!parse_text(example).unwrap().is_empty());
    }

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
    fn repeated_sni_identity_directives_map_to_named_cli_options() {
        assert_eq!(
            parse_text(
                "tls-sni-cert-file a.test=/etc/a-chain.pem\ntls-sni-key-file a.test=/etc/a-key.pem\n"
            )
            .unwrap(),
            vec![
                "--tls-sni-cert-file",
                "a.test=/etc/a-chain.pem",
                "--tls-sni-key-file",
                "a.test=/etc/a-key.pem",
            ]
        );
    }

    #[test]
    fn tls_client_certificate_mapping_directive_maps_to_cli_option() {
        let mapping = format!("{}=reader", "ab".repeat(32));
        assert_eq!(
            parse_text(&format!("tls-client-cert-user {mapping}")).unwrap(),
            vec!["--tls-client-cert-user", mapping.as_str()]
        );
    }

    #[test]
    fn tls_client_certificate_mapping_file_directive_maps_to_cli_option() {
        assert_eq!(
            parse_text("tls-client-cert-users-file /etc/rillcache/client-users.map").unwrap(),
            vec![
                "--tls-client-cert-users-file",
                "/etc/rillcache/client-users.map"
            ]
        );
    }

    #[test]
    fn tls_reload_trigger_directive_maps_to_cli_option() {
        assert_eq!(
            parse_text("tls-reload-trigger /run/rillcache/tls-generation").unwrap(),
            vec!["--tls-reload-trigger", "/run/rillcache/tls-generation"]
        );
    }

    #[test]
    fn frame_assembly_timeout_directive_maps_to_cli_option() {
        assert_eq!(
            parse_text("frame-assembly-timeout-ms 0\n").unwrap(),
            vec!["--frame-assembly-timeout-ms", "0"]
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
        assert_eq!(
            parse_text("aof-anchor-file /run/anchors/aof.anchor\n").unwrap(),
            vec!["--aof-anchor-file", "/run/anchors/aof.anchor"]
        );
        assert_eq!(
            parse_text("secure-secret-files yes\nsecure-secret-files=no\n").unwrap(),
            ["--secure-secret-files=yes", "--secure-secret-files=no"]
        );
        assert!(parse_text("secure-secret-files true\n").is_err());
    }

    #[test]
    fn multiline_values_are_rejected() {
        assert!(parse_text("requirepass \"a\nb\"\n").is_err());
    }
}
