//! Bounded append-only persistence in RESP command form.
//!
//! Every mutating command is translated into a compact, deterministic rebuild of
//! the keys it changed (`DEL` plus `SET`/`RPUSH`/`SADD`/`HSET`/`ZADD`/`XADD` and
//! an absolute `PEXPIREAT`). Replaying the file is ordinary Redis command
//! execution, so the log is readable by a Redis-compatible tool and does not
//! require binary-format compatibility. Compaction is a cooperative rewrite: the
//! owner materialises the key list, then maintenance encodes bounded slices
//! between client commands and swaps in the synced snapshot under a short gate.

use super::ShardExecutor;
use super::data::{Collection, Data};
use super::notify::{Category as NotifyCategory, NotifyFlags};
use crate::protocol::{Limits, dispatch_request, encode_request};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SYNC_INTERVAL: Duration = Duration::from_secs(1);
const CHUNK_BUDGET: usize = 256 * 1024;
const CHECKSUM_HEADER: &[u8] = b"__RILLCACHE_AOF_CRC32_V1__:";
const CHECKSUM_RECORD: &[u8] = b"__RILLCACHE_RECORD_CRC32_V1__:";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsyncPolicy {
    Always,
    EverySec,
    No,
}

impl FsyncPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "always" => Some(Self::Always),
            "everysec" => Some(Self::EverySec),
            "no" => Some(Self::No),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::EverySec => "everysec",
            Self::No => "no",
        }
    }
}

/// Startup journal mode plus the fsync policy that can be changed at runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AofConfig {
    pub appendonly: bool,
    pub appendfilename: String,
    pub policy: FsyncPolicy,
}

impl Default for AofConfig {
    fn default() -> Self {
        Self {
            appendonly: false,
            appendfilename: "appendonly.aof".to_string(),
            policy: FsyncPolicy::EverySec,
        }
    }
}

const REWRITE_STEP_KEYS: usize = 512;
const REWRITE_STEP_BYTES: usize = 1024 * 1024;

fn new_checksum_nonce() -> io::Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    Ok(format!("{:x}-{:x}", std::process::id(), now))
}

fn checksum_header(nonce: &str) -> Vec<u8> {
    let payload = format!("{}{nonce}", String::from_utf8_lossy(CHECKSUM_HEADER));
    let mut out = Vec::new();
    encode_request(&[b"ECHO", payload.as_bytes()], &mut out);
    out
}

fn checksum_record(nonce: &str, len: u64, crc: u32) -> Vec<u8> {
    let payload = format!(
        "{}{nonce}:{len}:{crc:08x}",
        String::from_utf8_lossy(CHECKSUM_RECORD)
    );
    let mut out = Vec::new();
    encode_request(&[b"ECHO", payload.as_bytes()], &mut out);
    out
}

fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    crc
}

fn parse_checksum_header(value: &[u8]) -> Option<String> {
    let suffix = value.strip_prefix(CHECKSUM_HEADER)?;
    let nonce = std::str::from_utf8(suffix).ok()?;
    (!nonce.is_empty()
        && nonce.len() <= 96
        && nonce.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
    .then(|| nonce.to_owned())
}

fn parse_checksum_record(value: &[u8]) -> Option<(String, u64, u32)> {
    let suffix = value.strip_prefix(CHECKSUM_RECORD)?;
    let text = std::str::from_utf8(suffix).ok()?;
    let mut parts = text.split(':');
    let nonce = parts.next()?;
    let len = parts.next()?.parse().ok()?;
    let crc_text = parts.next()?;
    if crc_text.len() != 8 {
        return None;
    }
    let crc = u32::from_str_radix(crc_text, 16).ok()?;
    if parts.next().is_some()
        || nonce.is_empty()
        || nonce.len() > 96
        || !nonce
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return None;
    }
    Some((nonce.to_owned(), len, crc))
}

pub(super) struct RewriteJob {
    keys: Vec<Vec<u8>>,
    cursor: usize,
    pending: VecDeque<Vec<u8>>,
    synced: bool,
    rounds: usize,
}

/// Exclusive local-filesystem ownership of one append-only journal.
///
/// Two processes pointed at the same journal would interleave appends and corrupt
/// the history, so startup claims `<journal>.lock` with `create_new`. A stale lock
/// from a crashed process is reclaimed only when its recorded PID is gone or has
/// been reused (start-time mismatch). Shared filesystems are not supported: this
/// is an advisory local lock, not a distributed one.
struct Ownership {
    path: PathBuf,
    contents: String,
}

impl Ownership {
    fn acquire(journal: &Path) -> io::Result<Self> {
        let path = journal.with_extension(format!(
            "{}lock",
            journal
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| format!("{extension}."))
                .unwrap_or_default()
        ));
        #[cfg(target_os = "linux")]
        let start = process_start_ticks(std::process::id())?
            .ok_or_else(|| io::Error::other("cannot read this process start time"))?;
        // On platforms without /proc we still enforce one writer, but cannot
        // prove a stale lock's owner is dead; startup therefore fails closed and
        // an operator removes the lock after confirming the prior process exited.
        #[cfg(not(target_os = "linux"))]
        let start = 0u64;
        let contents = format!("{} {start}\n", std::process::id());
        for attempt in 0..2 {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    if let Err(error) = file
                        .write_all(contents.as_bytes())
                        .and_then(|()| file.sync_all())
                    {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(error);
                    }
                    return Ok(Self { path, contents });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempt == 0 => {
                    let held = fs::read_to_string(&path).unwrap_or_default();
                    let mut fields = held.split_whitespace();
                    let pid = fields.next().and_then(|value| value.parse::<u32>().ok());
                    let start = fields.next().and_then(|value| value.parse::<u64>().ok());
                    let live = match (pid, start) {
                        (Some(pid), Some(start)) => {
                            #[cfg(target_os = "linux")]
                            {
                                match process_start_ticks(pid) {
                                    // A live owner with the same start time blocks startup.
                                    Ok(Some(current)) if current == start => true,
                                    // Gone or reused: the lock is stale.
                                    Ok(Some(_)) | Ok(None) => false,
                                    // Cannot inspect the process: fail closed.
                                    Err(_) => true,
                                }
                            }
                            #[cfg(not(target_os = "linux"))]
                            {
                                let _ = (pid, start);
                                true
                            }
                        }
                        // Unreadable or malformed lock: fail closed and let an
                        // operator inspect it rather than guessing ownership.
                        _ => true,
                    };
                    if live {
                        return Err(io::Error::other(format!(
                            "append-only journal {} is already owned by process {}; stop it or use a different --dir",
                            journal.display(),
                            pid.map(|pid| pid.to_string()).unwrap_or_else(|| "?".into())
                        )));
                    }
                    fs::remove_file(&path)?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other(
            "could not claim append-only journal ownership",
        ))
    }
}

impl Drop for Ownership {
    fn drop(&mut self) {
        // Only remove the lock if it still records this process, so a reclaimed
        // lock from another instance is never deleted.
        if fs::read_to_string(&self.path).is_ok_and(|held| held == self.contents) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Kernel start time of `pid` (clock ticks since boot), used to distinguish a
/// live owner from PID reuse. `Ok(None)` means the process is gone; errors mean
/// ownership could not be verified and callers must fail closed.
#[cfg(target_os = "linux")]
fn process_start_ticks(pid: u32) -> io::Result<Option<u64>> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        // Permission or I/O failure is not proof that the owner is dead. Fail
        // closed so a second process never steals a lock we cannot inspect.
        Err(error) => return Err(error),
    };
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed proc stat"))?;
    let fields: Vec<&str> = fields.split_whitespace().collect();
    let start = fields
        .get(19)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "proc stat lacks start time"))?
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid proc start time"))?;
    Ok(Some(start))
}

#[cfg(not(target_os = "linux"))]
fn process_start_ticks(_pid: u32) -> io::Result<Option<u64>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "process start-time verification requires Linux /proc",
    ))
}

struct RewriteState {
    file: File,
    temporary: PathBuf,
    nonce: String,
    crc: u32,
    record_len: u64,
    bytes: u64,
}

pub(super) struct Journal {
    file: File,
    path: PathBuf,
    policy: FsyncPolicy,
    limits: Limits,
    dirty: bool,
    last_sync: Instant,
    failed: bool,
    last_rewrite_failed: bool,
    rewrite: Option<RewriteState>,
    _ownership: Ownership,
    checksum_nonce: Option<String>,
    size: u64,
    rewrite_base: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PersistenceStatus {
    pub aof_enabled: bool,
    pub current_size: Option<u64>,
    pub write_failed: bool,
    pub rewrite_failed: bool,
    pub rewrite_in_progress: bool,
}

impl Journal {
    /// Open (or create) the log, replay it into a fresh store, discard a partial
    /// tail and leave the file positioned for appends.
    pub(super) fn open(path: &Path, policy: FsyncPolicy, limits: Limits) -> io::Result<Self> {
        // Claim ownership before opening or replaying: a second process must not
        // read a journal it is about to corrupt.
        let ownership = Ownership::acquire(path)?;
        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::OpenOptionsExt as _;
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path)?
        };
        #[cfg(not(unix))]
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        Ok(Self {
            file,
            path: path.to_path_buf(),
            policy,
            limits,
            dirty: false,
            last_sync: Instant::now(),
            failed: false,
            last_rewrite_failed: false,
            rewrite: None,
            _ownership: ownership,
            checksum_nonce: None,
            size,
            rewrite_base: size,
        })
    }

    /// Validate checksummed transaction groups before executing any record. Legacy
    /// AOFs without the header remain readable; the first subsequent append starts
    /// a checksummed segment. A torn final group is truncated to the last verified
    /// boundary, while any complete-but-mismatched group fails closed.
    fn validate_checksums(&mut self) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut buffer = Vec::new();
        let mut consumed = 0usize;
        let mut chunk = [0u8; 64 * 1024];
        let mut eof = false;
        let mut offset = 0u64;
        let mut valid_end = 0u64;
        let mut nonce: Option<String> = None;
        let mut crc = !0u32;
        let mut record_len = 0u64;
        let mut group_start = 0u64;
        while !eof {
            while consumed < buffer.len() {
                let marker = dispatch_request(&buffer[consumed..], self.limits, |args| {
                    let command = args.first().copied().unwrap_or_default();
                    let payload = args.get(1).copied().unwrap_or_default();
                    if args.len() == 2 && command.eq_ignore_ascii_case(b"ECHO") {
                        Some((
                            parse_checksum_header(payload),
                            parse_checksum_record(payload),
                            payload.starts_with(b"__RILLCACHE_AOF_")
                                || payload.starts_with(b"__RILLCACHE_RECORD_"),
                        ))
                    } else {
                        None
                    }
                });
                match marker {
                    Ok(Some((marker, used))) => {
                        let raw = &buffer[consumed..consumed + used];
                        consumed += used;
                        offset = offset.saturating_add(used as u64);
                        match marker {
                            Some((Some(header_nonce), _, _)) if nonce.is_none() => {
                                nonce = Some(header_nonce);
                                group_start = offset;
                                valid_end = offset;
                                crc = !0u32;
                                record_len = 0;
                            }
                            Some((_, Some((marker_nonce, expected_len, expected_crc)), _))
                                if nonce.as_deref() == Some(marker_nonce.as_str()) =>
                            {
                                if expected_len != record_len || expected_crc != !crc {
                                    return Err(io::Error::new(
                                        io::ErrorKind::InvalidData,
                                        format!("AOF checksum mismatch at byte {group_start}"),
                                    ));
                                }
                                valid_end = offset;
                                group_start = offset;
                                crc = !0u32;
                                record_len = 0;
                            }
                            Some((_, _, true)) => {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    format!(
                                        "invalid or mismatched AOF checksum marker at byte {}",
                                        offset - used as u64
                                    ),
                                ));
                            }
                            _ => {
                                if nonce.is_some() {
                                    crc = crc32_update(crc, raw);
                                    record_len = record_len.saturating_add(raw.len() as u64);
                                } else {
                                    // Legacy records before a format marker retain
                                    // the original partial-tail recovery behavior.
                                    valid_end = offset;
                                }
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("corrupt AOF checksum framing at byte {offset}: {error}"),
                        ));
                    }
                }
            }
            if consumed > 0 {
                buffer.drain(..consumed);
                consumed = 0;
            }
            let count = self.file.read(&mut chunk)?;
            if count == 0 {
                eof = true;
            } else {
                buffer.extend_from_slice(&chunk[..count]);
            }
        }
        if nonce.is_some() && (record_len > 0 || !buffer.is_empty()) {
            self.file.set_len(valid_end)?;
            self.size = valid_end;
        }
        self.checksum_nonce = nonce;
        self.file.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    pub(super) fn replay(&mut self, store: &ShardExecutor) -> io::Result<()> {
        self.validate_checksums()?;
        self.file.seek(SeekFrom::Start(0))?;
        let mut buffer = Vec::new();
        let mut consumed = 0usize;
        let mut good_total = 0u64;
        let mut chunk = [0u8; 64 * 1024];
        let mut eof = false;
        loop {
            while consumed < buffer.len() {
                let parsed = dispatch_request(&buffer[consumed..], self.limits, |arguments| {
                    let mut output = Vec::new();
                    let action = store.execute_command(arguments, &mut output);
                    if output.first() == Some(&b'-') {
                        Err(())
                    } else {
                        Ok(action)
                    }
                });
                match parsed {
                    Ok(Some((Ok(_), used))) => {
                        consumed += used;
                        good_total = good_total.saturating_add(used as u64);
                    }
                    Ok(Some((Err(()), _))) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("AOF command failed during replay at byte {}", good_total),
                        ));
                    }
                    Ok(None) => break,
                    Err(error) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("corrupt AOF at byte {}: {error}", good_total),
                        ));
                    }
                }
            }
            if consumed > 0 {
                buffer.drain(..consumed);
                consumed = 0;
            }
            if eof {
                break;
            }
            let read = self.file.read(&mut chunk)?;
            if read == 0 {
                eof = true;
            } else {
                buffer.extend_from_slice(&chunk[..read]);
            }
        }
        if !buffer.is_empty() {
            // A crash can leave one incomplete trailing command; Redis's default
            // is to load everything before it. Truncate so future appends do not
            // follow garbage.
            self.file.set_len(good_total)?;
        }
        self.file.seek(SeekFrom::End(0))?;
        self.size = self
            .file
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        self.rewrite_base = self.size;
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other("append-only file is in a failed state"));
        }
        let (nonce, new_header) = match self.checksum_nonce.clone() {
            Some(nonce) => (nonce, Vec::new()),
            None => {
                let nonce = new_checksum_nonce()?;
                let header = checksum_header(&nonce);
                (nonce, header)
            }
        };
        let crc = !crc32_update(!0u32, bytes);
        let mut transaction = Vec::with_capacity(new_header.len() + bytes.len() + 128);
        transaction.extend_from_slice(&new_header);
        transaction.extend_from_slice(bytes);
        transaction.extend_from_slice(&checksum_record(&nonce, bytes.len() as u64, crc));
        if let Err(error) = self.file.write_all(&transaction) {
            self.failed = true;
            return Err(error);
        }
        self.checksum_nonce = Some(nonce);
        self.dirty = true;
        self.size = self.size.saturating_add(transaction.len() as u64);
        if self.policy == FsyncPolicy::Always {
            if let Err(error) = self.file.sync_data() {
                self.failed = true;
                return Err(error);
            }
            self.dirty = false;
            self.last_sync = Instant::now();
        } else if self.policy == FsyncPolicy::No {
            self.dirty = false;
        }
        Ok(())
    }

    /// Whether the current log has grown enough to justify a rewrite.
    fn auto_rewrite_due(&self, percentage: u64, min_size: u64) -> bool {
        if percentage == 0 || self.size < min_size {
            return false;
        }
        let growth = self.size.saturating_sub(self.rewrite_base);
        u128::from(growth) * 100 >= u128::from(self.rewrite_base) * u128::from(percentage)
    }

    fn sync_if_due(&mut self) -> io::Result<()> {
        if self.policy != FsyncPolicy::EverySec || !self.dirty {
            return Ok(());
        }
        if self.last_sync.elapsed() < SYNC_INTERVAL {
            return Ok(());
        }
        if let Err(error) = self.file.sync_data() {
            self.failed = true;
            return Err(error);
        }
        self.dirty = false;
        self.last_sync = Instant::now();
        Ok(())
    }

    fn sync_all(&mut self) -> io::Result<()> {
        if let Err(error) = self.file.flush().and_then(|()| self.file.sync_all()) {
            self.failed = true;
            return Err(error);
        }
        self.dirty = false;
        Ok(())
    }
}

impl Journal {
    /// Create the same-directory temporary file that will replace the live
    /// journal once a consistent snapshot has been synced.
    fn begin_rewrite(&mut self) -> io::Result<()> {
        self.abort_rewrite();
        let directory = self.path.parent().unwrap_or_else(|| Path::new("."));
        let temporary = directory.join(format!(
            ".{}.rewrite-{}",
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("appendonly.aof"),
            std::process::id()
        ));
        let nonce = new_checksum_nonce()?;
        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::OpenOptionsExt as _;
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&temporary)?
        };
        #[cfg(not(unix))]
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)?;
        let header = checksum_header(&nonce);
        let mut file = file;
        file.write_all(&header)?;
        self.rewrite = Some(RewriteState {
            file,
            temporary,
            nonce,
            crc: !0u32,
            record_len: 0,
            bytes: header.len() as u64,
        });
        Ok(())
    }

    /// Append encoded snapshot records to the temporary file. The caller holds
    /// the data gate only while encoding, never while these writes block.
    fn write_rewrite(&mut self, bytes: &[u8]) -> io::Result<()> {
        let Some(state) = self.rewrite.as_mut() else {
            return Err(io::Error::other("no rewrite in progress"));
        };
        if let Err(error) = state.file.write_all(bytes) {
            self.last_rewrite_failed = true;
            self.abort_rewrite();
            return Err(error);
        }
        state.crc = crc32_update(state.crc, bytes);
        state.record_len = state.record_len.saturating_add(bytes.len() as u64);
        state.bytes = state.bytes.saturating_add(bytes.len() as u64);
        Ok(())
    }

    fn sync_rewrite(&mut self) -> io::Result<()> {
        let Some(state) = self.rewrite.as_mut() else {
            return Err(io::Error::other("no rewrite in progress"));
        };
        if let Err(error) = state.file.flush().and_then(|()| state.file.sync_all()) {
            self.last_rewrite_failed = true;
            self.abort_rewrite();
            return Err(error);
        }
        Ok(())
    }

    /// Atomically replace the live journal with the synced snapshot. The caller
    /// holds the exclusive data gate, so no append can interleave with the swap.
    fn commit_rewrite(&mut self) -> io::Result<u64> {
        let Some(mut state) = self.rewrite.take() else {
            return Err(io::Error::other("no rewrite in progress"));
        };
        let marker = checksum_record(&state.nonce, state.record_len, !state.crc);
        state.file.write_all(&marker)?;
        state.bytes = state.bytes.saturating_add(marker.len() as u64);
        let result = (|| -> io::Result<u64> {
            state.file.flush()?;
            state.file.sync_all()?;
            drop(state.file);
            fs::rename(&state.temporary, &self.path)?;
            let directory = self.path.parent().unwrap_or_else(|| Path::new("."));
            if let Ok(directory) = fs::File::open(directory) {
                let _ = directory.sync_all();
            }
            let mut replacement = OpenOptions::new().read(true).write(true).open(&self.path)?;
            replacement.seek(SeekFrom::End(0))?;
            self.file = replacement;
            self.checksum_nonce = Some(state.nonce.clone());
            self.dirty = false;
            self.failed = false;
            self.last_sync = Instant::now();
            Ok(state.bytes)
        })();
        self.last_rewrite_failed = result.is_err();
        match &result {
            Ok(size) => {
                self.size = *size;
                self.rewrite_base = *size;
            }
            Err(_) => {
                let _ = fs::remove_file(&state.temporary);
            }
        }
        result
    }

    fn abort_rewrite(&mut self) {
        if let Some(state) = self.rewrite.take() {
            drop(state.file);
            let _ = fs::remove_file(&state.temporary);
        }
    }

    pub(super) fn status(&self) -> PersistenceStatus {
        PersistenceStatus {
            aof_enabled: true,
            current_size: self.file.metadata().ok().map(|metadata| metadata.len()),
            write_failed: self.failed,
            rewrite_failed: self.last_rewrite_failed,
            rewrite_in_progress: self.rewrite.is_some(),
        }
    }

    #[cfg(test)]
    pub(super) fn policy(&self) -> FsyncPolicy {
        self.policy
    }

    /// Apply a new runtime fsync policy. Leaving `no` first syncs writes that
    /// were previously only handed to the operating system, so the stronger
    /// policy takes effect for records already acknowledged under the old one.
    pub(super) fn set_policy(&mut self, policy: FsyncPolicy) -> io::Result<()> {
        if self.policy != policy {
            if self.policy == FsyncPolicy::No && policy != FsyncPolicy::No {
                self.sync_all()?;
            }
            self.policy = policy;
        }
        Ok(())
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        self.abort_rewrite();
        let _ = self.sync_all();
    }
}

impl ShardExecutor {
    pub(super) fn journal_marks(&self) -> Option<(u64, u64)> {
        self.refresh_data_config();
        if self.journal.is_none() && !self.notifications_enabled() {
            return None;
        }
        let data = self.data.try_borrow().ok()?;
        Some((data.journal_seq(), data.flush_seq()))
    }

    pub(super) fn commit_journal(
        &self,
        marks: Option<(u64, u64)>,
        arguments: &[&[u8]],
        output: &mut Vec<u8>,
        start: usize,
    ) {
        let Some((sequence, flush_sequence)) = marks else {
            return;
        };
        let journaled = self.journal.is_some();
        let notified = self.notifications_enabled();
        if !journaled && !notified {
            return;
        }
        let (records, events, lifecycle) = {
            let Ok(mut data) = self.data.try_borrow_mut() else {
                if journaled {
                    output.truncate(start);
                    output.extend_from_slice(b"-ERR internal journal error\r\n");
                }
                return;
            };
            let flushed = data.flush_seq() != flush_sequence;
            let (keys, full) = if flushed {
                (Vec::new(), true)
            } else {
                data.changes_since(sequence)
            };
            if keys.is_empty() && !full {
                let lifecycle = if notified {
                    data.drain_lifecycle()
                } else {
                    (Vec::new(), Vec::new())
                };
                (Vec::new(), Vec::new(), lifecycle)
            } else {
                let now = self.clock.now_ms().max(0);
                let keys = if full && journaled {
                    let mut all: Vec<Vec<u8>> = data.entries.keys().cloned().collect();
                    all.sort_unstable();
                    all
                } else {
                    keys
                };
                let events = if notified && !full {
                    notification_events(
                        &data,
                        arguments,
                        &keys,
                        now,
                        self.notify_flags(),
                        &output[start..],
                    )
                } else {
                    Vec::new()
                };
                let lifecycle = if notified {
                    data.drain_lifecycle()
                } else {
                    (Vec::new(), Vec::new())
                };
                let records = if journaled {
                    let mut records = Vec::new();
                    if full {
                        encode_request(&[b"FLUSHALL"], &mut records);
                    }
                    let budget = self
                        .journal_limits
                        .max_frame_bytes
                        .saturating_sub(4096)
                        .clamp(1024, CHUNK_BUDGET);
                    for key in &keys {
                        encode_key(&data, key, now, budget, &mut records);
                    }
                    records
                } else {
                    Vec::new()
                };
                (records, events, lifecycle)
            }
        };
        if journaled && !records.is_empty() {
            let Some(journal) = self.journal.as_ref() else {
                return;
            };
            let Ok(mut journal) = journal.lock() else {
                output.truncate(start);
                output.extend_from_slice(b"-ERR internal journal error\r\n");
                return;
            };
            if let Err(error) = journal.append(&records) {
                output.truncate(start);
                let _ = std::io::Write::write_fmt(
                    output,
                    format_args!("-ERR append-only write failed: {error}\r\n"),
                );
                return;
            }
        }
        self.publish_lifecycle(lifecycle);
        for (channel, payload) in events {
            self.publish(&channel, &payload);
        }
    }

    fn publish_lifecycle(&self, lifecycle: (Vec<Vec<u8>>, Vec<Vec<u8>>)) {
        let (expired, evicted) = lifecycle;
        for key in expired {
            self.notify_event("expired", &key, NotifyCategory::Expired);
        }
        for key in evicted {
            self.notify_event("evicted", &key, NotifyCategory::Evicted);
        }
    }

    pub(super) fn notify_event(&self, event: &str, key: &[u8], category: NotifyCategory) {
        let flags = self.notify_flags();
        if !flags.category(category) {
            return;
        }
        for (channel, payload) in build_event(flags, event.as_bytes(), key) {
            self.publish(&channel, &payload);
        }
    }

    /// Redis-compatible keyspace notifications for LMOVE/RPOPLPUSH and their
    /// blocking forms: a push event on the destination, a pop event on the
    /// source, and a generic `del` when moving emptied the source.
    pub(super) fn notify_list_move(
        &self,
        source: &[u8],
        destination: &[u8],
        from_left: bool,
        to_left: bool,
        source_emptied: bool,
    ) {
        let flags = self.notify_flags();
        for (channel, payload) in list_move_events(
            flags,
            source,
            destination,
            from_left,
            to_left,
            source_emptied,
        ) {
            self.publish(&channel, &payload);
        }
    }

    /// Cheap probe for the multi-owner router: never builds a snapshot when no
    /// rewrite is configured or due.
    pub(crate) fn auto_rewrite_due(&self) -> bool {
        let percentage = self.auto_aof_rewrite_percentage();
        if percentage == 0 {
            return false;
        }
        let min_size = self.auto_aof_rewrite_min_size();
        let Some(journal) = self.journal.as_ref() else {
            return false;
        };
        journal
            .lock()
            .is_ok_and(|journal| journal.auto_rewrite_due(percentage, min_size))
    }

    /// Rewrite the journal as a complete state snapshot within a synchronous
    /// compatibility-owner turn; no append can interleave with replacement.
    pub fn auto_rewrite_if_due(&self) -> bool {
        if !self.auto_rewrite_due() {
            return false;
        }
        let percentage = self.auto_aof_rewrite_percentage();
        let min_size = self.auto_aof_rewrite_min_size();
        let Some(journal) = self.journal.as_ref() else {
            return false;
        };
        let due = journal
            .lock()
            .is_ok_and(|journal| journal.auto_rewrite_due(percentage, min_size));
        if !due {
            return false;
        }
        self.rewrite_journal().is_ok()
    }

    fn rewrite_budget(&self) -> usize {
        self.journal_limits
            .max_frame_bytes
            .saturating_sub(4096)
            .clamp(1024, CHUNK_BUDGET)
    }

    /// Encode one bounded slice and append it to the in-progress rewrite file.
    fn rewrite_write_slice(
        &self,
        journal: &std::sync::Mutex<Journal>,
        keys: &[Vec<u8>],
    ) -> io::Result<usize> {
        let now = self.clock.now_ms().max(0);
        let budget = self.rewrite_budget();
        let mut records = Vec::new();
        {
            let data = self
                .data
                .try_borrow()
                .map_err(|_| io::Error::other("data busy during rewrite"))?;
            for key in keys {
                encode_key(&data, key, now, budget, &mut records);
            }
        }
        journal
            .lock()
            .map_err(|_| io::Error::other("poisoned journal lock"))?
            .write_rewrite(&records)?;
        Ok(keys.len())
    }

    /// Start a cooperative rewrite. It returns immediately; maintenance advances
    /// one bounded slice per tick so client commands interleave with it.
    pub(super) fn rewrite_journal(&self) -> Result<u64, ()> {
        let Some(journal) = self.journal.as_ref() else {
            return Err(());
        };
        if self.rewrite_job.borrow().is_some() {
            return Ok(0);
        }
        let keys = {
            let Ok(mut data) = self.data.try_borrow_mut() else {
                return Err(());
            };
            data.begin_rewrite_tracking();
            data.entries.keys().cloned().collect::<Vec<_>>()
        };
        let mut header = Vec::new();
        encode_request(&[b"FLUSHALL"], &mut header);
        let started = (|| -> std::io::Result<()> {
            let mut journal = journal
                .lock()
                .map_err(|_| io::Error::other("poisoned journal lock"))?;
            journal.begin_rewrite()?;
            journal.write_rewrite(&header)
        })();
        if started.is_err() {
            if let Ok(mut data) = self.data.try_borrow_mut() {
                data.end_rewrite_tracking();
            }
            if let Ok(mut journal) = journal.lock() {
                journal.abort_rewrite();
            }
            return Err(());
        }
        *self.rewrite_job.borrow_mut() = Some(RewriteJob {
            keys,
            cursor: 0,
            pending: VecDeque::new(),
            synced: false,
            rounds: 0,
        });
        Ok(0)
    }

    /// Advance the cooperative rewrite by at most [`REWRITE_STEP_BYTES`] of
    /// encoded state and one bounded batch of changed keys. Called from
    /// maintenance, so clients are served between slices.
    pub(super) fn rewrite_step(&self) -> io::Result<()> {
        let Some(journal) = self.journal.as_ref() else {
            return Ok(());
        };
        let finished = {
            let mut guard = self.rewrite_job.borrow_mut();
            let Some(job) = guard.as_mut() else {
                return Ok(());
            };
            // Phase 1: snapshot keys in bounded slices.
            if job.cursor < job.keys.len() {
                let mut end = job.cursor;
                let mut bytes = 0usize;
                while end < job.keys.len() && end - job.cursor < REWRITE_STEP_KEYS {
                    bytes += job.keys[end].len() + 64;
                    end += 1;
                    if bytes >= REWRITE_STEP_BYTES {
                        break;
                    }
                }
                let slice: Vec<Vec<u8>> = job.keys[job.cursor..end].to_vec();
                self.rewrite_write_slice(journal, &slice)?;
                job.cursor = end;
                return Ok(());
            }
            // Phase 2: re-encode whole values for keys changed since the last
            // round. Whole-value records are idempotent, so a key visited twice
            // cannot duplicate collection members.
            if !job.pending.is_empty() {
                let take = job.pending.len().min(REWRITE_STEP_KEYS);
                let slice: Vec<Vec<u8>> = job.pending.drain(..take).collect();
                self.rewrite_write_slice(journal, &slice)?;
                return Ok(());
            }
            let (mut changed, overflow) = match self.data.try_borrow_mut() {
                Ok(mut data) => data.take_rewrite_changes(),
                Err(_) => return Err(io::Error::other("data busy during rewrite")),
            };
            if overflow {
                let data = self
                    .data
                    .try_borrow()
                    .map_err(|_| io::Error::other("data busy during rewrite"))?;
                changed = data.entries.keys().cloned().collect();
            }
            if !changed.is_empty() {
                job.rounds = job.rounds.saturating_add(1);
                job.pending.extend(changed);
                return Ok(());
            }
            if !job.synced {
                // Bulk durability happens outside the data gate.
                journal
                    .lock()
                    .map_err(|_| io::Error::other("poisoned journal lock"))?
                    .sync_rewrite()?;
                job.synced = true;
                return Ok(());
            }
            // Phase 3: the snapshot matches the live state; commit under the gate.
            let Ok(mut data) = self.data.try_borrow_mut() else {
                return Err(io::Error::other("data busy during rewrite commit"));
            };
            let (mut tail, overflow) = data.take_rewrite_changes();
            if overflow {
                tail = data.entries.keys().cloned().collect();
            }
            let now = self.clock.now_ms().max(0);
            let budget = self.rewrite_budget();
            let mut records = Vec::new();
            for key in &tail {
                encode_key(&data, key, now, budget, &mut records);
            }
            let mut journal = journal
                .lock()
                .map_err(|_| io::Error::other("poisoned journal lock"))?;
            if !records.is_empty() {
                journal.write_rewrite(&records)?;
            }
            journal.sync_rewrite()?;
            journal.commit_rewrite()?;
            data.journal_reset();
            data.end_rewrite_tracking();
            true
        };
        if finished {
            *self.rewrite_job.borrow_mut() = None;
        }
        Ok(())
    }

    pub(super) fn rewrite_in_progress(&self) -> bool {
        self.rewrite_job.borrow().is_some()
    }

    fn abort_rewrite_job(&self) {
        if self.rewrite_job.borrow_mut().take().is_some() {
            if let Ok(mut data) = self.data.try_borrow_mut() {
                data.end_rewrite_tracking();
            }
            if let Some(journal) = self.journal.as_ref()
                && let Ok(mut journal) = journal.lock()
            {
                journal.abort_rewrite();
            }
        }
    }

    pub(super) fn persistence_status(&self) -> PersistenceStatus {
        let Some(journal) = self.journal.as_ref() else {
            return PersistenceStatus {
                aof_enabled: false,
                current_size: None,
                write_failed: false,
                rewrite_failed: false,
                rewrite_in_progress: false,
            };
        };
        let in_progress = self.rewrite_in_progress();
        journal.lock().map_or(
            PersistenceStatus {
                aof_enabled: true,
                current_size: None,
                write_failed: true,
                rewrite_failed: true,
                rewrite_in_progress: in_progress,
            },
            |journal| {
                let mut status = journal.status();
                status.rewrite_in_progress = in_progress;
                status
            },
        )
    }

    pub fn journal_sync(&self) -> io::Result<()> {
        // A background rewrite is best effort: abort it on failure instead of
        // turning maintenance into an error for unrelated client commands.
        if self.rewrite_step().is_err() {
            self.abort_rewrite_job();
        }
        let Some(journal) = self.journal.as_ref() else {
            return Ok(());
        };
        let Ok(mut journal) = journal.lock() else {
            return Err(io::Error::other("poisoned journal lock"));
        };
        journal.sync_if_due()
    }

    /// Validate WAITAOF arguments and synchronously fsync the local journal when
    /// the AOF is enabled. Returns `(local acknowledgements, replica
    /// acknowledgements)`; this standalone server never has replicas.
    pub(crate) fn waitaof_probe(
        &self,
        numlocal: i64,
        numreplicas: i64,
    ) -> Result<(u64, u64), &'static [u8]> {
        if !(0..=1).contains(&numlocal) {
            return Err(b"-ERR numlocal can't be greater than 1\r\n");
        }
        if numreplicas < 0 {
            return Err(b"-ERR numreplicas can't be negative\r\n");
        }
        let enabled = self.aof_config().is_some_and(|config| config.appendonly);
        if numlocal > 0 && !enabled {
            return Err(
                b"-ERR WAITAOF cannot be used when numlocal is set to 1 but the AOF is not enabled.\r\n",
            );
        }
        if !enabled {
            return Ok((0, 0));
        }
        self.journal_flush()
            .map_err(|_| &b"-ERR append-only journal sync failed\r\n"[..])?;
        Ok((1, 0))
    }

    /// Force a durable flush before process shutdown, regardless of fsync policy.
    pub fn journal_flush(&self) -> io::Result<()> {
        let Some(journal) = self.journal.as_ref() else {
            return Ok(());
        };
        let Ok(mut journal) = journal.lock() else {
            return Err(io::Error::other("poisoned journal lock"));
        };
        journal.sync_all()
    }
}

/// Encode the final state of one key as a bounded sequence of RESP commands.
fn encode_key(data: &Data, key: &[u8], now: i64, budget: usize, output: &mut Vec<u8>) {
    encode_request(&[b"DEL", key], output);
    let Some(entry) = data.entries.get(key) else {
        return;
    };
    if !entry.alive(now) {
        return;
    }
    if let Some(collection) = &entry.collection {
        match collection {
            Collection::ListPack(values) => encode_chunks(
                b"RPUSH",
                key,
                values.iter().map(Vec::as_slice),
                budget,
                output,
            ),
            Collection::ListQuick(nodes) => encode_chunks(
                b"RPUSH",
                key,
                nodes.iter().flatten().map(Vec::as_slice),
                budget,
                output,
            ),
            Collection::Set(values) => encode_chunks(
                b"SADD",
                key,
                values.iter().map(Vec::as_slice),
                budget,
                output,
            ),
            Collection::SetPack(values) => encode_chunks(
                b"SADD",
                key,
                values.iter().map(Vec::as_slice),
                budget,
                output,
            ),
            Collection::SetInt(values) => {
                let owned: Vec<Vec<u8>> = values
                    .iter()
                    .map(|value| value.to_string().into_bytes())
                    .collect();
                encode_chunks(
                    b"SADD",
                    key,
                    owned.iter().map(Vec::as_slice),
                    budget,
                    output,
                );
            }
            Collection::Hash(values) => {
                let mut fields: Vec<(&[u8], &[u8])> = values
                    .iter()
                    .map(|(field, value)| (field.as_slice(), value.as_slice()))
                    .collect();
                fields.sort_unstable_by(|a, b| a.0.cmp(b.0));
                let mut arguments: Vec<&[u8]> = vec![b"HSET", key];
                let mut bytes = key.len() + 4;
                for (field, value) in fields {
                    if arguments.len() > 2 && bytes + field.len() + value.len() > budget {
                        encode_request(&arguments, output);
                        arguments = vec![b"HSET", key];
                        bytes = key.len() + 4;
                    }
                    arguments.push(field);
                    arguments.push(value);
                    bytes += field.len() + value.len();
                }
                if arguments.len() > 2 {
                    encode_request(&arguments, output);
                }
            }
            Collection::HashPack(values) => {
                let mut fields: Vec<(&[u8], &[u8])> = values
                    .iter()
                    .map(|(field, value)| (field.as_slice(), value.as_slice()))
                    .collect();
                fields.sort_unstable_by(|a, b| a.0.cmp(b.0));
                let mut arguments: Vec<&[u8]> = vec![b"HSET", key];
                let mut bytes = key.len() + 4;
                for (field, value) in fields {
                    if arguments.len() > 2 && bytes + field.len() + value.len() > budget {
                        encode_request(&arguments, output);
                        arguments = vec![b"HSET", key];
                        bytes = key.len() + 4;
                    }
                    arguments.push(field);
                    arguments.push(value);
                    bytes += field.len() + value.len();
                }
                if arguments.len() > 2 {
                    encode_request(&arguments, output);
                }
            }
            Collection::ZSet(values) => {
                let mut members: Vec<(&[u8], f64)> = values
                    .iter()
                    .map(|(member, score)| (member.as_slice(), *score))
                    .collect();
                members.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(b.0)));
                let scores: Vec<String> =
                    members.iter().map(|(_, score)| score.to_string()).collect();
                let mut arguments: Vec<&[u8]> = vec![b"ZADD", key];
                let mut bytes = key.len() + 4;
                for ((member, _), score) in members.iter().zip(&scores) {
                    if arguments.len() > 2 && bytes + member.len() + score.len() > budget {
                        encode_request(&arguments, output);
                        arguments = vec![b"ZADD", key];
                        bytes = key.len() + 4;
                    }
                    arguments.push(score.as_bytes());
                    arguments.push(member);
                    bytes += member.len() + score.len();
                }
                if arguments.len() > 2 {
                    encode_request(&arguments, output);
                }
            }
            Collection::ZSetPack(values) => {
                let mut members: Vec<(&[u8], f64)> = values
                    .iter()
                    .map(|(member, score)| (member.as_slice(), *score))
                    .collect();
                members.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(b.0)));
                let scores: Vec<String> =
                    members.iter().map(|(_, score)| score.to_string()).collect();
                let mut arguments: Vec<&[u8]> = vec![b"ZADD", key];
                let mut bytes = key.len() + 4;
                for ((member, _), score) in members.iter().zip(&scores) {
                    if arguments.len() > 2 && bytes + member.len() + score.len() > budget {
                        encode_request(&arguments, output);
                        arguments = vec![b"ZADD", key];
                        bytes = key.len() + 4;
                    }
                    arguments.push(score.as_bytes());
                    arguments.push(member);
                    bytes += member.len() + score.len();
                }
                if arguments.len() > 2 {
                    encode_request(&arguments, output);
                }
            }
        }
        for (field, deadline) in &entry.field_ttl {
            let deadline = deadline.to_string();
            encode_request(
                &[
                    b"HPEXPIREAT",
                    key,
                    deadline.as_bytes(),
                    b"FIELDS",
                    b"1",
                    field,
                ],
                output,
            );
        }
    } else if let Some(stream) = &entry.stream {
        let mut last_entry = None;
        for (id, record) in &stream.entries {
            last_entry = Some(*id);
            let id = id_text(*id);
            let mut arguments: Vec<&[u8]> = vec![b"XADD", key, &id];
            for (field, value) in &record.fields {
                arguments.push(field);
                arguments.push(value);
            }
            encode_request(&arguments, output);
        }
        let max_group_id = stream
            .groups
            .values()
            .map(|group| group.last_delivered)
            .max()
            .unwrap_or_default();
        let last_entry = last_entry.unwrap_or_default();
        let sentinel = stream.last_generated.max(max_group_id).max(last_entry);
        let needs_sentinel = stream.last_generated > last_entry
            || (stream.entries.is_empty()
                && (stream.last_generated.ms != 0
                    || stream.last_generated.seq != 0
                    || !stream.groups.is_empty()));
        if needs_sentinel {
            let id = id_text(sentinel);
            encode_request(&[b"XADD", key, &id, b"_", b"_"], output);
            encode_request(&[b"XDEL", key, &id], output);
        }
        for (name, group) in &stream.groups {
            let id = id_text(group.last_delivered);
            encode_request(&[b"XGROUP", b"CREATE", key, name, &id], output);
            // Consumers and their pending entries are delivery state, not just
            // stream data: recreate the consumer object, then claim each pending
            // id back to its owner with the recorded delivery time and count.
            // XCLAIM FORCE creates the PEL entry; XGROUP CREATECONSUMER keeps a
            // consumer that currently has no pending entries. Entries that were
            // trimmed or deleted can no longer be claimed and are dropped, which
            // matches this subset's documented limitation.
            let mut consumers: Vec<&Vec<u8>> = group.consumers.keys().collect();
            consumers.sort();
            for consumer in consumers {
                encode_request(&[b"XGROUP", b"CREATECONSUMER", key, name, consumer], output);
            }
            for (id, pending) in &group.pending {
                let id = id_text(*id);
                let delivered_at = pending.delivered_at.max(0).to_string();
                let deliveries = pending.deliveries.to_string();
                encode_request(
                    &[
                        b"XCLAIM",
                        key,
                        name,
                        pending.consumer.as_slice(),
                        b"0",
                        &id,
                        b"TIME",
                        delivered_at.as_bytes(),
                        b"RETRYCOUNT",
                        deliveries.as_bytes(),
                        b"JUSTID",
                        b"FORCE",
                    ],
                    output,
                );
            }
        }
        if needs_sentinel || !stream.entries.is_empty() || !stream.groups.is_empty() {
            let last = id_text(stream.last_generated);
            let entries_added = stream.entries_added.to_string();
            let max_deleted = id_text(stream.max_deleted);
            encode_request(
                &[
                    b"XSETID",
                    key,
                    &last,
                    b"ENTRIESADDED",
                    entries_added.as_bytes(),
                    b"MAXDELETEDID",
                    &max_deleted,
                ],
                output,
            );
        }
    } else {
        encode_request(&[b"SET", key, entry.value.as_slice()], output);
    }
    if let Some(deadline) = entry.expires {
        let deadline = deadline.to_string();
        encode_request(&[b"PEXPIREAT", key, deadline.as_bytes()], output);
    }
}

fn encode_chunks<'a>(
    command: &'static [u8],
    key: &[u8],
    values: impl Iterator<Item = &'a [u8]>,
    budget: usize,
    output: &mut Vec<u8>,
) {
    let mut arguments: Vec<&[u8]> = vec![command, key];
    let mut bytes = command.len() + key.len();
    for value in values {
        if arguments.len() > 2 && bytes + value.len() > budget {
            encode_request(&arguments, output);
            arguments = vec![command, key];
            bytes = command.len() + key.len();
        }
        arguments.push(value);
        bytes += value.len();
    }
    if arguments.len() > 2 {
        encode_request(&arguments, output);
    }
}

fn id_text(id: super::data::StreamId) -> Vec<u8> {
    format!("{}-{}", id.ms, id.seq).into_bytes()
}

fn notification_events(
    data: &Data,
    arguments: &[&[u8]],
    keys: &[Vec<u8>],
    now: i64,
    flags: NotifyFlags,
    reply: &[u8],
) -> Vec<(Vec<u8>, Vec<u8>)> {
    let Some(command) = arguments.first() else {
        return Vec::new();
    };
    if command.eq_ignore_ascii_case(b"MSETEX") {
        return msetex_notification_events(data, arguments, keys, flags, reply);
    }
    let hsetex = command.eq_ignore_ascii_case(b"HSETEX");
    if hsetex && reply != b":1\r\n" {
        return Vec::new();
    }
    if let Some((from_left, to_left)) = list_move_sides(command, arguments) {
        if !reply.starts_with(b"$") || reply == b"$-1\r\n" {
            return Vec::new();
        }
        let source = arguments[1];
        return list_move_events(
            flags,
            source,
            arguments[2],
            from_left,
            to_left,
            !data.entries.contains_key(source),
        );
    }
    let mut events = Vec::new();
    for key in keys {
        // Admission/lazy expiry can change unrelated keys even when HSETEX
        // succeeds. Only its supplied target can receive its write event.
        if hsetex && arguments.get(1).copied() != Some(key.as_slice()) {
            continue;
        }
        if data.was_evicted(key) {
            continue;
        }
        let Some((event, category)) = event_for(data, command, arguments, key, now) else {
            continue;
        };
        if !flags.category(category) {
            continue;
        }
        events.extend(build_event(flags, event.as_bytes(), key));
    }
    events
}

/// Final-state journal keys are sorted/deduplicated, but MSETEX notifications
/// follow every supplied pair (including duplicates), with set then expire.
fn msetex_notification_events(
    data: &Data,
    arguments: &[&[u8]],
    keys: &[Vec<u8>],
    flags: NotifyFlags,
    reply: &[u8],
) -> Vec<(Vec<u8>, Vec<u8>)> {
    // Reaping may change keys even when NX/XX rejects the write or admission
    // fails. Those removals belong only to the separate lifecycle events.
    if reply != b":1\r\n" {
        return Vec::new();
    }
    let Some(pair_end) = arguments
        .get(1)
        .and_then(|raw| super::options::parse_integer(raw))
        .and_then(|count| usize::try_from(count).ok())
        .and_then(|count| count.checked_mul(2)?.checked_add(2))
        .filter(|end| *end <= arguments.len())
    else {
        return Vec::new();
    };
    let Ok(options) = super::options::SetOptions::parse(&arguments[pair_end..], false) else {
        return Vec::new();
    };
    let mut events = Vec::new();
    for pair in arguments[2..pair_end].as_chunks::<2>().0 {
        let key = pair[0];
        // changes_since supplies sorted keys when the log has not overflowed;
        // full/overflow snapshots never enter notification_events.
        if keys
            .binary_search_by(|changed| changed.as_slice().cmp(key))
            .is_err()
            || data.was_evicted(key)
        {
            continue;
        }
        if flags.string {
            events.extend(build_event(flags, b"set", key));
        }
        // KEEPTTL retains the existing deadline but does not assign a new one.
        if flags.generic && options.expiry.is_some() {
            events.extend(build_event(flags, b"expire", key));
        }
    }
    events
}

fn list_move_sides(command: &[u8], arguments: &[&[u8]]) -> Option<(bool, bool)> {
    let side = |raw: &[u8]| {
        if raw.eq_ignore_ascii_case(b"LEFT") {
            Some(true)
        } else if raw.eq_ignore_ascii_case(b"RIGHT") {
            Some(false)
        } else {
            None
        }
    };
    if command.eq_ignore_ascii_case(b"LMOVE") || command.eq_ignore_ascii_case(b"BLMOVE") {
        Some((side(arguments.get(3)?)?, side(arguments.get(4)?)?))
    } else if command.eq_ignore_ascii_case(b"RPOPLPUSH")
        || command.eq_ignore_ascii_case(b"BRPOPLPUSH")
    {
        Some((false, true))
    } else {
        None
    }
}

fn list_move_events(
    flags: NotifyFlags,
    source: &[u8],
    destination: &[u8],
    from_left: bool,
    to_left: bool,
    source_emptied: bool,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut events = Vec::new();
    if flags.list {
        let push = if to_left {
            b"lpush".as_slice()
        } else {
            b"rpush"
        };
        let pop = if from_left {
            b"lpop".as_slice()
        } else {
            b"rpop"
        };
        events.extend(build_event(flags, push, destination));
        events.extend(build_event(flags, pop, source));
    }
    if flags.generic && source_emptied {
        events.extend(build_event(flags, b"del", source));
    }
    events
}

fn build_event(flags: NotifyFlags, event: &[u8], key: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut events = Vec::with_capacity(2);
    if flags.keyspace {
        let mut channel = b"__keyspace@0__:".to_vec();
        channel.extend_from_slice(key);
        events.push((channel, event.to_vec()));
    }
    if flags.keyevent {
        let mut channel = b"__keyevent@0__:".to_vec();
        channel.extend_from_slice(event);
        events.push((channel, key.to_vec()));
    }
    events
}

fn event_for(
    data: &Data,
    command: &[u8],
    arguments: &[&[u8]],
    key: &[u8],
    now: i64,
) -> Option<(&'static str, NotifyCategory)> {
    let name = std::str::from_utf8(command).ok()?.to_ascii_lowercase();
    if name == "hgetdel" {
        let stored = data
            .entries
            .get(key)
            .is_some_and(|entry| entry.collection.as_ref().is_some_and(Collection::is_hash));
        return Some(if stored {
            ("hdel", NotifyCategory::Hash)
        } else {
            ("del", NotifyCategory::Generic)
        });
    }
    let category = match name.as_str() {
        "del" | "delex" | "delifeq" | "unlink" => {
            return Some(("del", NotifyCategory::Generic));
        }
        "expire" | "pexpire" | "expireat" | "pexpireat" => {
            let exists = data.entries.get(key).is_some_and(|entry| entry.alive(now));
            return Some((
                if exists { "expire" } else { "del" },
                NotifyCategory::Generic,
            ));
        }
        "persist" => return Some(("persist", NotifyCategory::Generic)),
        "rename" | "renamenx" => {
            let source = arguments.get(1).copied();
            if source == Some(key) {
                return Some(("rename_from", NotifyCategory::Generic));
            }
            return Some(("rename_to", NotifyCategory::Generic));
        }
        "copy" => {
            if arguments
                .get(2)
                .is_some_and(|destination| *destination == key)
            {
                return Some(("copy_to", NotifyCategory::Generic));
            }
            return None;
        }
        "getdel" => return Some(("del", NotifyCategory::Generic)),
        "set" | "setex" | "psetex" | "setnx" | "getset" | "mset" | "msetnx" | "msetex"
        | "getex" => ("set", NotifyCategory::String),
        "incr" | "incrby" | "incrbyfloat" => ("incrby", NotifyCategory::String),
        "decr" | "decrby" => ("decrby", NotifyCategory::String),
        "append" => ("append", NotifyCategory::String),
        "setrange" => ("setrange", NotifyCategory::String),
        "setbit" => ("setbit", NotifyCategory::String),
        "geoadd" => ("zadd", NotifyCategory::Zset),
        "geosearchstore" => {
            let stored = data
                .entries
                .get(key)
                .is_some_and(|entry| entry.collection.as_ref().is_some_and(Collection::is_zset));
            return Some(if stored {
                ("geosearchstore", NotifyCategory::Zset)
            } else {
                ("del", NotifyCategory::Generic)
            });
        }
        "pfadd" => ("pfadd", NotifyCategory::String),
        "pfmerge" => ("pfadd", NotifyCategory::String),
        "lpush" => ("lpush", NotifyCategory::List),
        "rpush" => ("rpush", NotifyCategory::List),
        "lpop" => ("lpop", NotifyCategory::List),
        "rpop" => ("rpop", NotifyCategory::List),
        "linsert" => ("linsert", NotifyCategory::List),
        "lset" => ("lset", NotifyCategory::List),
        "ltrim" => ("ltrim", NotifyCategory::List),
        "lrem" => ("lrem", NotifyCategory::List),
        "blmpop" | "lmpop" => ("lmpop", NotifyCategory::List),
        "sadd" => ("sadd", NotifyCategory::Set),
        "srem" => ("srem", NotifyCategory::Set),
        "spop" => ("spop", NotifyCategory::Set),
        "smove" => ("smove", NotifyCategory::Set),
        "sinterstore" => ("sinterstore", NotifyCategory::Set),
        "sunionstore" => ("sunionstore", NotifyCategory::Set),
        "sdiffstore" => ("sdiffstore", NotifyCategory::Set),
        "hset" | "hsetnx" | "hmset" | "hsetex" => ("hset", NotifyCategory::Hash),
        "hdel" => ("hdel", NotifyCategory::Hash),
        "hincrby" => ("hincrby", NotifyCategory::Hash),
        "hincrbyfloat" => ("hincrbyfloat", NotifyCategory::Hash),
        "zunionstore" | "zinterstore" | "zdiffstore" => {
            if !arguments
                .get(1)
                .is_some_and(|destination| *destination == key)
            {
                return None;
            }
            if !data.entries.contains_key(key) {
                return Some(("del", NotifyCategory::Generic));
            }
            let event = match name.as_str() {
                "zunionstore" => "zunionstore",
                "zinterstore" => "zinterstore",
                _ => "zdiffstore",
            };
            return Some((event, NotifyCategory::Zset));
        }
        "zadd" => ("zadd", NotifyCategory::Zset),
        "zrem" => ("zrem", NotifyCategory::Zset),
        "zincrby" => ("zincrby", NotifyCategory::Zset),
        "sort" => {
            let stored = data
                .entries
                .get(key)
                .is_some_and(|entry| entry.collection.as_ref().is_some_and(Collection::is_list));
            return Some(if stored {
                ("sortstore", NotifyCategory::List)
            } else {
                ("del", NotifyCategory::Generic)
            });
        }
        "zpopmin" | "bzpopmin" => ("zpopmin", NotifyCategory::Zset),
        "zpopmax" | "bzpopmax" => ("zpopmax", NotifyCategory::Zset),
        "zmpop" | "bzmpop" => {
            let numkeys_index = if name == "bzmpop" { 2 } else { 1 };
            let numkeys = arguments
                .get(numkeys_index)
                .and_then(|raw| std::str::from_utf8(raw).ok())
                .and_then(|raw| raw.parse::<usize>().ok())
                .unwrap_or(0);
            let direction = arguments.get(numkeys_index.saturating_add(1).saturating_add(numkeys));
            if direction.is_some_and(|value| value.eq_ignore_ascii_case(b"MAX")) {
                return Some(("zpopmax", NotifyCategory::Zset));
            }
            return Some(("zpopmin", NotifyCategory::Zset));
        }
        "zremrangebyrank" => ("zremrangebyrank", NotifyCategory::Zset),
        "zremrangebyscore" => ("zremrangebyscore", NotifyCategory::Zset),
        "zremrangebylex" => ("zremrangebylex", NotifyCategory::Zset),
        "xadd" => ("xadd", NotifyCategory::Stream),
        "xdel" => ("xdel", NotifyCategory::Stream),
        "xgroup" => ("xgroup", NotifyCategory::Stream),
        "xreadgroup" => ("xreadgroup", NotifyCategory::Stream),
        "xack" => ("xack", NotifyCategory::Stream),
        "xclaim" => ("xclaim", NotifyCategory::Stream),
        "xautoclaim" => ("xautoclaim", NotifyCategory::Stream),
        "xtrim" => ("xtrim", NotifyCategory::Stream),
        _ => return None,
    };
    Some(category)
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
