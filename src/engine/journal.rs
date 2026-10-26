//! Bounded append-only persistence in RESP command form.
//!
//! Every mutating command is translated into a compact, deterministic rebuild of
//! the keys it changed (`DEL` plus `SET`/`RPUSH`/`SADD`/`HSET`/`ZADD`/`XADD` and
//! an absolute `PEXPIREAT`). Replaying the file is ordinary Redis command
//! execution, so the log is readable by a Redis-compatible tool and does not
//! require binary-format compatibility. This is intentionally not an RDB/AOF
//! rewrite implementation: the file grows without compaction.

use super::Store;
use super::data::{Collection, Data};
use crate::protocol::{Limits, dispatch_request, encode_request};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SYNC_INTERVAL: Duration = Duration::from_secs(1);
const CHUNK_BUDGET: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsyncPolicy {
    Always,
    EverySec,
    No,
}

impl FsyncPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "always" => Some(Self::Always),
            "everysec" => Some(Self::EverySec),
            "no" => Some(Self::No),
            _ => None,
        }
    }
}

pub(super) struct Journal {
    file: File,
    path: PathBuf,
    policy: FsyncPolicy,
    limits: Limits,
    dirty: bool,
    last_sync: Instant,
    failed: bool,
}

impl Journal {
    /// Open (or create) the log, replay it into a fresh store, discard a partial
    /// tail and leave the file positioned for appends.
    pub(super) fn open(path: &Path, policy: FsyncPolicy, limits: Limits) -> io::Result<Self> {
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
        Ok(Self {
            file,
            path: path.to_path_buf(),
            policy,
            limits,
            dirty: false,
            last_sync: Instant::now(),
            failed: false,
        })
    }

    pub(super) fn replay(&mut self, store: &Store) -> io::Result<()> {
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
                    let action = store.execute_locked(arguments, &mut output);
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
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.failed {
            return Err(io::Error::other("append-only file is in a failed state"));
        }
        if let Err(error) = self.file.write_all(bytes) {
            self.failed = true;
            return Err(error);
        }
        self.dirty = true;
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
    pub(super) fn limits(&self) -> Limits {
        self.limits
    }

    /// Replace the log with a compact full-state snapshot using an fsynced
    /// same-directory temporary file. Called under the store's exclusive
    /// transaction gate, so no concurrent append can be lost.
    fn rewrite(&mut self, records: &[u8]) -> io::Result<u64> {
        let directory = self.path.parent().unwrap_or_else(|| Path::new("."));
        let temporary = directory.join(format!(
            ".{}.rewrite-{}",
            self.path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("appendonly.aof"),
            std::process::id()
        ));
        let result = (|| -> io::Result<u64> {
            #[cfg(unix)]
            let mut file = {
                use std::os::unix::fs::OpenOptionsExt as _;
                OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&temporary)?
            };
            #[cfg(not(unix))]
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&temporary)?;
            file.write_all(records)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &self.path)?;
            if let Ok(directory) = fs::File::open(directory) {
                let _ = directory.sync_all();
            }
            let replacement = OpenOptions::new().read(true).write(true).open(&self.path)?;
            self.file = replacement;
            self.dirty = false;
            self.failed = false;
            self.last_sync = Instant::now();
            Ok(records.len() as u64)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        let _ = self.sync_all();
    }
}

impl Store {
    pub(super) fn attach_journal(&mut self, journal: Journal) {
        self.journal_limits = journal.limits();
        self.journal = Some(std::sync::Mutex::new(journal));
    }

    pub(super) fn journal_marks(&self) -> Option<(u64, u64)> {
        self.journal.as_ref()?;
        let data = self.data.read().ok()?;
        Some((data.journal_seq(), data.flush_seq()))
    }

    pub(super) fn commit_journal(
        &self,
        marks: Option<(u64, u64)>,
        output: &mut Vec<u8>,
        start: usize,
    ) {
        let Some((sequence, flush_sequence)) = marks else {
            return;
        };
        let Some(journal) = self.journal.as_ref() else {
            return;
        };
        let records = {
            let Ok(mut data) = self.data.write() else {
                output.truncate(start);
                output.extend_from_slice(b"-ERR internal journal error\r\n");
                return;
            };
            let flushed = data.flush_seq() != flush_sequence;
            let (keys, full) = if flushed {
                (Vec::new(), true)
            } else {
                data.changes_since(sequence)
            };
            if keys.is_empty() && !full {
                return;
            }
            let now = self.clock.now_ms().max(0);
            let mut records = Vec::new();
            if full {
                encode_request(&[b"FLUSHALL"], &mut records);
            }
            let keys = if full {
                let mut all: Vec<Vec<u8>> = data.entries.keys().cloned().collect();
                all.sort_unstable();
                all
            } else {
                keys
            };
            let budget = self
                .journal_limits
                .max_frame_bytes
                .saturating_sub(4096)
                .clamp(1024, CHUNK_BUDGET);
            for key in &keys {
                encode_key(&data, key, now, budget, &mut records);
            }
            records
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
        }
    }

    /// Rewrite the journal as a complete state snapshot. The caller must already
    /// hold the exclusive transaction gate (the journal-enabled execute path).
    pub(super) fn rewrite_journal(&self) -> Result<u64, ()> {
        let Some(journal) = self.journal.as_ref() else {
            return Err(());
        };
        let records = {
            let Ok(mut data) = self.data.write() else {
                return Err(());
            };
            let now = self.clock.now_ms().max(0);
            let budget = self
                .journal_limits
                .max_frame_bytes
                .saturating_sub(4096)
                .clamp(1024, CHUNK_BUDGET);
            let mut keys: Vec<Vec<u8>> = data.entries.keys().cloned().collect();
            keys.sort_unstable();
            let mut records = Vec::new();
            encode_request(&[b"FLUSHALL"], &mut records);
            for key in &keys {
                encode_key(&data, key, now, budget, &mut records);
            }
            data.journal_reset();
            records
        };
        let Ok(mut journal) = journal.lock() else {
            return Err(());
        };
        journal.rewrite(&records).map_err(|_| ())
    }

    pub fn journal_sync(&self) -> io::Result<()> {
        let Some(journal) = self.journal.as_ref() else {
            return Ok(());
        };
        let Ok(mut journal) = journal.lock() else {
            return Err(io::Error::other("poisoned journal lock"));
        };
        journal.sync_if_due()
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

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
