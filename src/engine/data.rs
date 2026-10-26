use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct StreamId {
    pub ms: u64,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StreamEntry {
    pub fields: Vec<(Vec<u8>, Vec<u8>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pending {
    pub consumer: Vec<u8>,
    pub deliveries: u64,
    pub delivered_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Consumer {
    pub seen_at: i64,
    pub pending: BTreeMap<StreamId, Pending>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ConsumerGroup {
    pub last_delivered: StreamId,
    pub consumers: BTreeMap<Vec<u8>, Consumer>,
    pub pending: BTreeMap<StreamId, Pending>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Stream {
    pub entries: BTreeMap<StreamId, StreamEntry>,
    pub last_generated: StreamId,
    pub groups: BTreeMap<Vec<u8>, ConsumerGroup>,
}

impl Stream {
    pub fn payload_bytes(&self) -> Option<usize> {
        self.entries.values().try_fold(0usize, |total, entry| {
            entry
                .fields
                .iter()
                .try_fold(total, |total, (field, value)| {
                    total.checked_add(field.len())?.checked_add(value.len())
                })
        })
    }
}

pub(super) struct Entry {
    pub value: Vec<u8>,
    pub stream: Option<Stream>,
    pub payload_len: usize,
    pub expires: Option<i64>,
}
impl Entry {
    pub fn alive(&self, now: i64) -> bool {
        // Redis keyIsExpired uses now > deadline; TTL may be zero for one tick.
        self.expires.is_none_or(|deadline| now <= deadline)
    }
}
#[derive(Default)]
pub(super) struct Data {
    pub entries: HashMap<Vec<u8>, Entry>,
    pub payload_bytes: usize,
    pub expirations: BTreeSet<(i64, Vec<u8>)>,
}
impl Data {
    pub fn remove(&mut self, key: &[u8]) -> Option<Entry> {
        let (key, entry) = self.entries.remove_entry(key)?;
        self.payload_bytes -= key.len() + entry.payload_len;
        if let Some(at) = entry.expires {
            self.expirations.remove(&(at, key));
        }
        Some(entry)
    }
    pub fn expire_key(&mut self, key: &[u8], now: i64) {
        // The index is exact: no expirations means no key can be expired.
        if self.expirations.is_empty() {
            return;
        }
        if self.entries.get(key).is_some_and(|entry| !entry.alive(now)) {
            self.remove(key);
        }
    }
    pub fn reap(&mut self, now: i64, limit: usize) -> usize {
        let mut count = 0;
        while count < limit {
            let Some((at, _)) = self.expirations.first() else {
                break;
            };
            if *at >= now {
                break;
            }
            let (_, key) = self.expirations.pop_first().expect("checked expiry entry");
            self.remove(&key);
            count += 1;
        }
        count
    }
    pub fn set_expiry(&mut self, key: &[u8], expires: Option<i64>) {
        let entry = self.entries.get_mut(key).expect("existing entry");
        Self::replace_expiry(&mut self.expirations, key, entry, expires);
    }
    fn replace_expiry(
        index: &mut BTreeSet<(i64, Vec<u8>)>,
        key: &[u8],
        entry: &mut Entry,
        expires: Option<i64>,
    ) {
        if entry.expires == expires {
            return;
        }
        if let Some(at) = entry.expires {
            index.remove(&(at, key.to_vec()));
        }
        entry.expires = expires;
        if let Some(at) = expires {
            index.insert((at, key.to_vec()));
        }
    }
    /// `previous` is the live value length already looked up under this write lock.
    /// Reaping cannot remove that key: callers have expired it at the same `now`.
    pub fn reserve(
        &mut self,
        key: &[u8],
        bytes: usize,
        previous: Option<usize>,
        maximum: usize,
        now: i64,
    ) -> Option<usize> {
        debug_assert_eq!(
            self.entries.get(key).map(|entry| entry.payload_len),
            previous
        );
        let size = |data: &Self| {
            (data.payload_bytes - previous.unwrap_or(0))
                .checked_add(if previous.is_none() { key.len() } else { 0 })?
                .checked_add(bytes)
                .filter(|&size| size <= maximum)
        };
        if let Some(bytes) = size(self) {
            return Some(bytes);
        }
        // A logically expired key must not cause a false OOM for a new write.
        self.reap(now, usize::MAX);
        size(self)
    }
    pub fn put(&mut self, key: &[u8], value: &[u8], expires: Option<i64>, payload: usize) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.stream = None;
            entry.payload_len = value.len();
            // Reuse admitted overwrites, but release disproportionate old capacity
            // on shrink (and all capacity for an empty replacement).
            if entry.value.capacity() > value.len().saturating_mul(2) {
                entry.value = value.to_vec();
            } else {
                value.clone_into(&mut entry.value);
            }
            Self::replace_expiry(&mut self.expirations, key, entry, expires);
        } else {
            self.entries.insert(
                key.to_vec(),
                Entry {
                    value: value.to_vec(),
                    stream: None,
                    payload_len: value.len(),
                    expires,
                },
            );
            if let Some(at) = expires {
                self.expirations.insert((at, key.to_vec()));
            }
        }
        self.payload_bytes = payload;
    }

    pub fn put_stream(&mut self, key: &[u8], stream: Stream, expires: Option<i64>, payload: usize) {
        let stream_bytes = stream.payload_bytes().expect("admitted stream payload");
        if let Some(entry) = self.entries.get_mut(key) {
            entry.value.clear();
            entry.stream = Some(stream);
            entry.payload_len = stream_bytes;
            Self::replace_expiry(&mut self.expirations, key, entry, expires);
        } else {
            self.entries.insert(
                key.to_vec(),
                Entry {
                    value: Vec::new(),
                    stream: Some(stream),
                    payload_len: stream_bytes,
                    expires,
                },
            );
            if let Some(at) = expires {
                self.expirations.insert((at, key.to_vec()));
            }
        }
        self.payload_bytes = payload;
    }
}
