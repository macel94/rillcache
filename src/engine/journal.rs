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
use super::notify::{Category as NotifyCategory, NotifyFlags};
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
        if self.journal.is_none() && !self.notifications_enabled() {
            return None;
        }
        let data = self.data.read().ok()?;
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
            let Ok(mut data) = self.data.write() else {
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
        "hset" | "hsetnx" | "hmset" => ("hset", NotifyCategory::Hash),
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
        "zpopmin" => ("zpopmin", NotifyCategory::Zset),
        "zpopmax" => ("zpopmax", NotifyCategory::Zset),
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
