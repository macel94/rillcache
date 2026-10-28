//! Optional append-only ACL denial audit.
//!
//! The in-memory `ACL LOG` aggregates recent denials and is intentionally
//! bounded. This module adds an operator-selected on-disk trail that records
//! every denial event as one bounded line. It is deliberately simple: one
//! generation of rotation, no external dependencies and no secret material.

use super::sha256;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MIN_MAX_BYTES: u64 = 1024;
pub const MAX_MAX_BYTES: u64 = 1024 * 1024 * 1024;

const MAX_TOKEN_BYTES: usize = 32;
const MAX_USERNAME_BYTES: usize = 128;
const MAX_OBJECT_BYTES: usize = 256;

pub struct AuditLog {
    file: File,
    path: PathBuf,
    rotated: PathBuf,
    max_bytes: u64,
    written: u64,
}

impl AuditLog {
    pub fn open(path: &Path, max_bytes: u64) -> io::Result<Self> {
        if path.as_os_str().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audit log path must not be empty",
            ));
        }
        if !(MIN_MAX_BYTES..=MAX_MAX_BYTES).contains(&max_bytes) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "audit log max bytes is outside the supported bound",
            ));
        }
        let file = open_append(path)?;
        let written = file.metadata().map_or(0, |metadata| metadata.len());
        let mut rotated = OsString::from(path.as_os_str());
        rotated.push(".1");
        Ok(Self {
            file,
            path: path.to_path_buf(),
            rotated: PathBuf::from(rotated),
            max_bytes,
            written,
        })
    }

    /// Append one denial line, rotating the previous file when the bound would
    /// be exceeded. The caller decides whether a write failure is fatal; the
    /// server records a visible counter instead of failing the command.
    #[cfg(test)]
    pub fn record(
        &mut self,
        reason: &str,
        context: &str,
        object: &[u8],
        username: &[u8],
    ) -> io::Result<()> {
        let line = format_line(reason, context, object, username, None);
        self.record_line(&line)
    }

    pub fn record_line(&mut self, line: &str) -> io::Result<()> {
        if self.written > 0 && self.written.saturating_add(line.len() as u64) > self.max_bytes {
            self.rotate()?;
        }
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        self.written = self.written.saturating_add(line.len() as u64);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        match fs::remove_file(&self.rotated) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        fs::rename(&self.path, &self.rotated)?;
        self.file = open_append(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

pub(super) fn format_line(
    reason: &str,
    context: &str,
    object: &[u8],
    username: &[u8],
    key: Option<&[u8]>,
) -> String {
    let body = format!(
        "{} {} {} {} {}",
        timestamp_millis(),
        token(reason),
        token(context),
        hex(username, MAX_USERNAME_BYTES),
        hex(object, MAX_OBJECT_BYTES),
    );
    match key {
        Some(key) => {
            let mac = sha256::hmac_sha256(key, body.as_bytes());
            format!("{body} {}\n", hex(&mac, 32))
        }
        None => format!("{body}\n"),
    }
}

/// Parse a hex-encoded audit keyring. Blank lines and `#` comments are ignored;
/// at most eight unique 16..=1024-byte keys are accepted. The first key is the
/// signing key for this process, while a collector may verify against all.
pub(super) fn parse_keyring(text: &str) -> Result<Vec<Vec<u8>>, String> {
    let mut keys: Vec<Vec<u8>> = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(bytes) = hex_decode(line) else {
            return Err(format!(
                "audit keyring line {} is not valid hex",
                line_number + 1
            ));
        };
        if !(16..=1024).contains(&bytes.len()) {
            return Err(format!(
                "audit keyring line {} must decode to 16..=1024 bytes",
                line_number + 1
            ));
        }
        if keys.len() >= 8 {
            return Err("audit keyring accepts at most eight keys".into());
        }
        if !keys.iter().any(|existing| existing == &bytes) {
            keys.push(bytes);
        }
    }
    if keys.is_empty() {
        return Err("audit keyring contains no keys".into());
    }
    Ok(keys)
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut output = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().as_chunks::<2>().0 {
        let high = (pair[0] as char).to_digit(16)?;
        let low = (pair[1] as char).to_digit(16)?;
        output.push(((high << 4) | low) as u8);
    }
    Some(output)
}

pub(super) fn verify_line(key: &[u8], line: &str) -> bool {
    let line = line.trim_end_matches(['\r', '\n']);
    let Some((body, mac)) = line.rsplit_once(' ') else {
        return false;
    };
    if mac.len() != 64 || !mac.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    let expected = hex(&sha256::hmac_sha256(key, body.as_bytes()), 32);
    mac.eq_ignore_ascii_case(&expected)
}

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}

fn token(value: &str) -> String {
    value
        .bytes()
        .take(MAX_TOKEN_BYTES)
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' {
                byte as char
            } else {
                '_'
            }
        })
        .collect()
}

fn hex(value: &[u8], limit: usize) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(limit.min(value.len()) * 2);
    for byte in value.iter().take(limit) {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rillcache-audit-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn lines_are_bounded_hex_and_rotate_once() {
        let path = temp_path("rotate");
        let mut log = AuditLog::open(&path, MIN_MAX_BYTES).unwrap();
        for _ in 0..40 {
            log.record("command", "toplevel", b"set", b"alice").unwrap();
        }
        let current = fs::read_to_string(&path).unwrap();
        assert!(!current.is_empty());
        let mut rotated_path = OsString::from(path.as_os_str());
        rotated_path.push(".1");
        assert!(PathBuf::from(rotated_path).exists());
        let line = current.lines().next().unwrap();
        let fields: Vec<_> = line.split(' ').collect();
        assert_eq!(fields.len(), 5);
        assert!(fields[0].chars().all(|ch| ch.is_ascii_digit()));
        assert_eq!(fields[1], "command");
        assert_eq!(fields[2], "toplevel");
        assert_eq!(fields[3], "616c696365");
        assert_eq!(fields[4], "736574");
        let _ = fs::remove_file(&path);
        let mut rotated_path = OsString::from(path.as_os_str());
        rotated_path.push(".1");
        let _ = fs::remove_file(rotated_path);
    }

    #[test]
    fn binary_and_long_values_are_hex_and_truncated() {
        let path = temp_path("hex");
        let mut log = AuditLog::open(&path, MIN_MAX_BYTES).unwrap();
        log.record(
            "key",
            "multi\n",
            b"a\0b\xff",
            &[b'u'; MAX_USERNAME_BYTES + 20],
        )
        .unwrap();
        let line = fs::read_to_string(&path).unwrap();
        let fields: Vec<_> = line.trim_end().split(' ').collect();
        assert_eq!(fields[1], "key");
        assert_eq!(fields[2], "multi_");
        assert_eq!(fields[3].len(), MAX_USERNAME_BYTES * 2);
        assert_eq!(fields[4], "610062ff");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn signed_lines_are_verifiable_and_tamper_evident() {
        let key = b"0123456789abcdef";
        let line = format_line("command", "toplevel", b"set", b"alice", Some(key));
        let (body, mac) = line.trim_end().rsplit_once(' ').unwrap();
        assert_eq!(mac.len(), 64);
        assert_eq!(
            mac,
            hex(&sha256::hmac_sha256(key, body.as_bytes()), 32).as_str()
        );
        let tampered = format!("{body}x");
        assert_ne!(
            mac,
            hex(&sha256::hmac_sha256(key, tampered.as_bytes()), 32).as_str()
        );
        let unsigned = format_line("command", "toplevel", b"set", b"alice", None);
        assert_eq!(unsigned.trim_end().split(' ').count(), 5);
    }

    #[test]
    fn signed_lines_verify_and_reject_tampering_or_wrong_keys() {
        let key = b"0123456789abcdef";
        let line = format_line("command", "toplevel", b"set", b"alice", Some(key));
        assert!(verify_line(key, &line));
        assert!(verify_line(key, line.trim_end()));
        assert!(!verify_line(b"fedcba9876543210", &line));
        let tampered = line.replacen("736574", "736575", 1);
        assert!(!verify_line(key, &tampered));
        assert!(!verify_line(key, "unsigned five field line"));
        assert!(!verify_line(
            key,
            "body zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
        ));
    }

    #[test]
    fn keyrings_parse_hex_comments_duplicates_and_bounds() {
        let first = "0123456789abcdef0123456789abcdef";
        let second = "fedcba9876543210fedcba9876543210";
        let keys = parse_keyring(&format!("# rotation\n{first}\n\n{second}\n{first}\n")).unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].len(), 16);
        assert_eq!(keys[0][0], 0x01);
        assert_eq!(keys[1][0], 0xfe);
        assert!(parse_keyring("").is_err());
        assert!(parse_keyring("zz").is_err());
        assert!(parse_keyring("00").is_err());
        let too_many = (0..9)
            .map(|index| format!("{:032x}", index + 1))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(parse_keyring(&too_many).is_err());
    }

    #[test]
    fn invalid_bounds_and_empty_paths_fail() {
        assert!(AuditLog::open(Path::new(""), MIN_MAX_BYTES).is_err());
        let path = temp_path("bounds");
        assert!(AuditLog::open(&path, MIN_MAX_BYTES - 1).is_err());
        assert!(AuditLog::open(&path, MAX_MAX_BYTES + 1).is_err());
    }
}
