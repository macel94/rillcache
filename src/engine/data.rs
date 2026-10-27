use super::eviction::EvictionPolicy;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Collection {
    ListPack(VecDeque<Vec<u8>>),
    ListQuick(VecDeque<VecDeque<Vec<u8>>>),
    Set(BTreeSet<Vec<u8>>),
    SetPack(Vec<Vec<u8>>),
    SetInt(BTreeSet<i64>),
    Hash(HashMap<Vec<u8>, Vec<u8>>),
    HashPack(Vec<(Vec<u8>, Vec<u8>)>),
    ZSet(HashMap<Vec<u8>, f64>),
    ZSetPack(Vec<(Vec<u8>, f64)>),
}
impl Collection {
    pub fn is_list(&self) -> bool {
        matches!(self, Self::ListPack(_) | Self::ListQuick(_))
    }

    pub fn is_set(&self) -> bool {
        matches!(self, Self::Set(_) | Self::SetPack(_) | Self::SetInt(_))
    }

    pub fn is_hash(&self) -> bool {
        matches!(self, Self::Hash(_) | Self::HashPack(_))
    }

    pub fn is_zset(&self) -> bool {
        matches!(self, Self::ZSet(_) | Self::ZSetPack(_))
    }

    pub fn list_items(&self) -> Option<Vec<&[u8]>> {
        match self {
            Self::ListPack(values) => Some(values.iter().map(Vec::as_slice).collect()),
            Self::ListQuick(nodes) => Some(nodes.iter().flatten().map(Vec::as_slice).collect()),
            _ => None,
        }
    }

    pub fn list_values(&self) -> Option<VecDeque<Vec<u8>>> {
        match self {
            Self::ListPack(values) => Some(values.clone()),
            Self::ListQuick(nodes) => Some(nodes.iter().flatten().cloned().collect()),
            _ => None,
        }
    }

    pub fn set_items(&self) -> Option<Vec<Vec<u8>>> {
        let mut items = match self {
            Self::Set(values) => values.iter().cloned().collect(),
            Self::SetPack(values) => values.clone(),
            Self::SetInt(values) => values
                .iter()
                .map(ToString::to_string)
                .map(String::into_bytes)
                .collect(),
            _ => return None,
        };
        items.sort_unstable();
        Some(items)
    }

    pub fn hash_items(&self) -> Option<Vec<(&[u8], &[u8])>> {
        match self {
            Self::Hash(values) => Some(
                values
                    .iter()
                    .map(|(field, value)| (field.as_slice(), value.as_slice()))
                    .collect(),
            ),
            Self::HashPack(values) => Some(
                values
                    .iter()
                    .map(|(field, value)| (field.as_slice(), value.as_slice()))
                    .collect(),
            ),
            _ => None,
        }
    }

    pub fn remove_hash_field(&mut self, field: &[u8]) {
        match self {
            Self::Hash(values) => {
                values.remove(field);
            }
            Self::HashPack(values) => values.retain(|(name, _)| name.as_slice() != field),
            _ => {}
        }
    }

    pub fn hash_is_empty(&self) -> bool {
        match self {
            Self::Hash(values) => values.is_empty(),
            Self::HashPack(values) => values.is_empty(),
            _ => false,
        }
    }

    pub fn hash_get(&self, field: &[u8]) -> Option<&[u8]> {
        match self {
            Self::Hash(values) => values.get(field).map(Vec::as_slice),
            Self::HashPack(values) => values
                .iter()
                .find(|(name, _)| name.as_slice() == field)
                .map(|(_, value)| value.as_slice()),
            _ => None,
        }
    }

    pub fn zset_items(&self) -> Option<Vec<(&[u8], f64)>> {
        match self {
            Self::ZSet(values) => Some(
                values
                    .iter()
                    .map(|(member, score)| (member.as_slice(), *score))
                    .collect(),
            ),
            Self::ZSetPack(values) => Some(
                values
                    .iter()
                    .map(|(member, score)| (member.as_slice(), *score))
                    .collect(),
            ),
            _ => None,
        }
    }

    pub fn payload_len(&self) -> usize {
        match self {
            Self::ListPack(values) => values.iter().map(Vec::len).sum(),
            Self::ListQuick(nodes) => nodes.iter().flatten().map(Vec::len).sum(),
            Self::Set(values) => values.iter().map(Vec::len).sum(),
            Self::SetPack(values) => values.iter().map(Vec::len).sum(),
            Self::SetInt(values) => values.iter().map(|value| value.to_string().len()).sum(),
            Self::Hash(values) => values
                .iter()
                .map(|(field, value)| field.len() + value.len())
                .sum(),
            Self::HashPack(values) => values
                .iter()
                .map(|(field, value)| field.len() + value.len())
                .sum(),
            Self::ZSet(values) => values.keys().map(Vec::len).sum(),
            Self::ZSetPack(values) => values.iter().map(|(member, _)| member.len()).sum(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct StreamId {
    pub ms: u64,
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StreamEntry {
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
    pub entries_added: u64,
    pub max_deleted: StreamId,
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

#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub value: Vec<u8>,
    pub collection: Option<Collection>,
    pub stream: Option<Stream>,
    pub payload_len: usize,
    pub expires: Option<i64>,
    pub field_ttl: BTreeMap<Vec<u8>, i64>,
    pub last_access: u64,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
            && self.collection == other.collection
            && self.stream == other.stream
            && self.payload_len == other.payload_len
            && self.expires == other.expires
            && self.field_ttl == other.field_ttl
    }
}
impl Entry {
    pub fn is_string(&self) -> bool {
        self.collection.is_none() && self.stream.is_none()
    }

    pub fn payload_len(&self) -> usize {
        self.payload_len
    }

    pub fn alive(&self, now: i64) -> bool {
        // Redis keyIsExpired uses now > deadline; TTL may be zero for one tick.
        self.expires.is_none_or(|deadline| now <= deadline)
    }
}

/// Bounded per-key mutation versions for `WATCH`. A full table or `FLUSHDB`
/// advances `epoch`, conservatively aborting every outstanding watch rather than
/// silently forgetting a mutation. The bound keeps memory independent of the
/// number of historical keys.
pub(super) const MAX_TRACKED_VERSIONS: usize = 65_536;
const MAX_JOURNAL_EVENTS: usize = 65_536;

#[derive(Default)]
pub(super) struct Data {
    pub entries: HashMap<Vec<u8>, Entry>,
    pub payload_bytes: usize,
    pub expirations: BTreeSet<(i64, Vec<u8>)>,
    pub(super) versions: HashMap<Vec<u8>, u64>,
    version_seq: u64,
    epoch: u64,
    flush_seq: u64,
    journal_events: VecDeque<(u64, Vec<u8>)>,
    journal_overflow: bool,
    collect_events: bool,
    expired_events: VecDeque<Vec<u8>>,
    evicted_events: VecDeque<Vec<u8>>,
    pub(in crate::engine) expired_keys: u64,
    pub(in crate::engine) expired_subkeys: u64,
    pub(in crate::engine) evicted_keys: u64,
    pub(in crate::engine) eviction: EvictionPolicy,
    pub field_ttls: usize,
    eviction_seq: u64,
    access_seq: u64,
}
const MAX_LIFECYCLE_EVENTS: usize = 1024;

fn push_lifecycle(queue: &mut VecDeque<Vec<u8>>, key: &[u8]) {
    if queue.len() >= MAX_LIFECYCLE_EVENTS {
        queue.pop_front();
    }
    queue.push_back(key.to_vec());
}

impl Data {
    /// True while a key evicted during the current command is pending publication.
    pub(super) fn was_evicted(&self, key: &[u8]) -> bool {
        self.evicted_events
            .iter()
            .any(|evicted| evicted.as_slice() == key)
    }

    /// Drain bounded lazy-expiration and eviction keys for notification delivery.
    pub(super) fn drain_lifecycle(&mut self) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        (
            self.expired_events.drain(..).collect(),
            self.evicted_events.drain(..).collect(),
        )
    }

    /// Discard pending lifecycle notifications that were queued while their
    /// classes were disabled, so enabling them cannot deliver stale events.
    pub(super) fn clear_lifecycle(&mut self) {
        self.expired_events.clear();
        self.evicted_events.clear();
    }

    /// Record a mutation visible to `WATCH`/`EXEC` conflict checks and to the
    /// optional append-only journal. The event log is bounded; overflow forces a
    /// full-state journal snapshot instead of allowing a missed key.
    pub fn touch(&mut self, key: &[u8]) {
        self.version_seq = self.version_seq.wrapping_add(1);
        if self.versions.len() >= MAX_TRACKED_VERSIONS && !self.versions.contains_key(key) {
            self.versions.clear();
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.versions.insert(key.to_vec(), self.version_seq);
        // Mutation history is only consumed by persistence and keyspace
        // notifications. Keeping WATCH versions above does not require it.
        if !self.collect_events {
            return;
        }
        if self.journal_events.len() >= MAX_JOURNAL_EVENTS {
            self.journal_events.clear();
            self.journal_overflow = true;
        }
        self.journal_events
            .push_back((self.version_seq, key.to_vec()));
    }

    pub(super) fn set_collect_events(&mut self, collect: bool) {
        self.collect_events = collect;
        if !collect {
            self.journal_events.clear();
            self.journal_overflow = false;
        }
    }

    pub(super) fn journal_seq(&self) -> u64 {
        self.version_seq
    }

    pub(super) fn flush_seq(&self) -> u64 {
        self.flush_seq
    }

    /// Keys mutated after `seq`, plus whether the bounded log overflowed and a
    /// full-state snapshot is required. Clears events that can no longer matter.
    pub(super) fn journal_reset(&mut self) {
        self.journal_events.clear();
        self.journal_overflow = false;
    }

    pub(super) fn changes_since(&mut self, seq: u64) -> (Vec<Vec<u8>>, bool) {
        if self.journal_overflow {
            self.journal_overflow = false;
            self.journal_events.clear();
            return (self.entries.keys().cloned().collect(), true);
        }
        let mut keys = BTreeSet::new();
        for (sequence, key) in &self.journal_events {
            if *sequence > seq {
                keys.insert(key.clone());
            }
        }
        self.journal_events.retain(|(sequence, _)| *sequence > seq);
        (keys.into_iter().collect(), false)
    }

    pub fn version(&self, key: &[u8]) -> Option<u64> {
        self.versions.get(key).copied()
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Clear every key and conservatively invalidate every outstanding watch.
    pub fn flush(&mut self) {
        self.entries.clear();
        self.expirations.clear();
        self.payload_bytes = 0;
        self.versions.clear();
        self.epoch = self.epoch.wrapping_add(1);
        self.flush_seq = self.flush_seq.wrapping_add(1);
        self.journal_events.clear();
        self.journal_overflow = false;
        self.expired_events.clear();
        self.evicted_events.clear();
        self.field_ttls = 0;
    }

    pub fn remove(&mut self, key: &[u8]) -> Option<Entry> {
        let (key, entry) = self.entries.remove_entry(key)?;
        self.field_ttls = self.field_ttls.saturating_sub(entry.field_ttl.len());
        self.touch(&key);
        self.payload_bytes -= key.len() + entry.payload_len;
        if let Some(at) = entry.expires {
            self.expirations.remove(&(at, key));
        }
        Some(entry)
    }

    pub fn expire_key(&mut self, key: &[u8], now: i64) {
        if self.field_ttls != 0 {
            self.expire_hash_fields(key, now);
        }
        if self.entries.get(key).is_some_and(|entry| !entry.alive(now)) {
            self.remove(key);
            push_lifecycle(&mut self.expired_events, key);
            self.expired_keys = self.expired_keys.saturating_add(1);
        }
    }

    fn expire_hash_fields(&mut self, key: &[u8], now: i64) {
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        if entry.field_ttl.is_empty() {
            return;
        }
        let expired: Vec<Vec<u8>> = entry
            .field_ttl
            .iter()
            .filter(|(_, deadline)| **deadline <= now)
            .map(|(field, _)| field.clone())
            .collect();
        if expired.is_empty() {
            return;
        }
        for field in &expired {
            entry.field_ttl.remove(field);
            if let Some(collection) = entry.collection.as_mut() {
                collection.remove_hash_field(field);
            }
        }
        self.field_ttls = self.field_ttls.saturating_sub(expired.len());
        self.expired_subkeys = self
            .expired_subkeys
            .saturating_add(u64::try_from(expired.len()).unwrap_or(u64::MAX));
        let remove_key = entry
            .collection
            .as_ref()
            .is_some_and(|collection| collection.is_hash() && collection.hash_is_empty());
        if remove_key {
            self.remove(key);
            return;
        }
        if let Some(collection) = entry.collection.as_ref() {
            let bytes = collection.payload_len();
            self.payload_bytes = self.payload_bytes - entry.payload_len + bytes;
            entry.payload_len = bytes;
        }
        self.touch(key);
    }

    pub fn remove_field_ttls(&mut self, key: &[u8], fields: &[&[u8]]) {
        if let Some(entry) = self.entries.get_mut(key) {
            for field in fields {
                if entry.field_ttl.remove(*field).is_some() {
                    self.field_ttls = self.field_ttls.saturating_sub(1);
                }
            }
        }
    }

    pub fn set_field_ttl(&mut self, key: &[u8], field: &[u8], deadline: i64) {
        if let Some(entry) = self.entries.get_mut(key)
            && entry.field_ttl.insert(field.to_vec(), deadline).is_none()
        {
            self.field_ttls = self.field_ttls.saturating_add(1);
        }
    }

    pub fn persist_field(&mut self, key: &[u8], field: &[u8]) -> bool {
        let removed = self
            .entries
            .get_mut(key)
            .is_some_and(|entry| entry.field_ttl.remove(field).is_some());
        if removed {
            self.field_ttls = self.field_ttls.saturating_sub(1);
        }
        removed
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
            push_lifecycle(&mut self.expired_events, &key);
            self.expired_keys = self.expired_keys.saturating_add(1);
            count += 1;
        }
        count
    }

    pub fn set_expiry(&mut self, key: &[u8], expires: Option<i64>) {
        let changed = self.entries.get(key).expect("existing entry").expires != expires;
        if changed {
            self.touch(key);
        }
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
        if let Some(bytes) = size(self) {
            return Some(bytes);
        }
        if self.eviction == EvictionPolicy::NoEviction {
            return None;
        }
        let own = if previous.is_none() { key.len() } else { 0 }.checked_add(bytes)?;
        if own > maximum {
            return None;
        }
        let current = self
            .payload_bytes
            .checked_sub(previous.unwrap_or(0))?
            .checked_add(if previous.is_none() { key.len() } else { 0 })?
            .checked_add(bytes)?;
        if current.saturating_sub(self.evictable_bytes(key)) > maximum {
            return None;
        }
        loop {
            if let Some(bytes) = size(self) {
                return Some(bytes);
            }
            if !self.evict_one(key) {
                return None;
            }
        }
    }

    fn evictable_bytes(&self, target: &[u8]) -> usize {
        self.entries
            .iter()
            .filter(|(key, entry)| {
                key.as_slice() != target && (!self.eviction.volatile() || entry.expires.is_some())
            })
            .map(|(key, entry)| key.len().saturating_add(entry.payload_len))
            .fold(0usize, usize::saturating_add)
    }

    /// Choose and remove one bounded candidate. Returns false when no eligible
    /// key exists, so a volatile policy can still fail without deleting persistent
    /// data. LRU is approximate mutation recency, not read recency.
    fn evict_one(&mut self, target: &[u8]) -> bool {
        let volatile = self.eviction.volatile();
        let random = matches!(
            self.eviction,
            EvictionPolicy::AllKeysRandom | EvictionPolicy::VolatileRandom
        );
        let count = self
            .entries
            .keys()
            .filter(|key| {
                key.as_slice() != target && (!volatile || self.entries[*key].expires.is_some())
            })
            .count();
        if count == 0 {
            return false;
        }
        let chosen = if random {
            self.eviction_seq = self.eviction_seq.wrapping_add(1);
            let target_index = self.eviction_seq as usize % count;
            self.entries
                .keys()
                .filter(|key| {
                    key.as_slice() != target && (!volatile || self.entries[*key].expires.is_some())
                })
                .nth(target_index)
                .cloned()
        } else {
            let mut best: Option<(&[u8], i128)> = None;
            for (key, entry) in &self.entries {
                if key.as_slice() == target || (volatile && entry.expires.is_none()) {
                    continue;
                }
                let rank = match self.eviction {
                    EvictionPolicy::VolatileTtl => entry.expires.unwrap_or(i64::MAX) as i128,
                    _ => entry.last_access as i128,
                };
                if best.is_none_or(|(_, current)| rank < current) {
                    best = Some((key.as_slice(), rank));
                }
            }
            best.map(|(key, _)| key.to_vec())
        };
        let Some(chosen) = chosen else {
            return false;
        };
        if self.remove(&chosen).is_some() {
            push_lifecycle(&mut self.evicted_events, &chosen);
            self.evicted_keys = self.evicted_keys.saturating_add(1);
            true
        } else {
            false
        }
    }

    pub fn put(
        &mut self,
        key: &[u8],
        value: &[u8],
        expires: Option<i64>,
        payload: usize,
        _now: i64,
    ) {
        self.access_seq = self.access_seq.wrapping_add(1);
        let access = self.access_seq;
        if let Some(entry) = self.entries.get_mut(key) {
            entry.collection = None;
            entry.stream = None;
            entry.payload_len = value.len();
            entry.last_access = access;
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
                    collection: None,
                    stream: None,
                    payload_len: value.len(),
                    expires,
                    field_ttl: Default::default(),
                    last_access: access,
                },
            );
            if let Some(at) = expires {
                self.expirations.insert((at, key.to_vec()));
            }
        }
        self.payload_bytes = payload;
        self.touch(key);
    }

    /// Install an already admitted owned value without copying its buffers or tree.
    /// `payload` is the final total, including replacement of any destination.
    pub fn put_owned(&mut self, key: &[u8], mut entry: Entry, payload: usize, _now: i64) {
        self.remove(key);
        self.access_seq = self.access_seq.wrapping_add(1);
        entry.last_access = self.access_seq;
        if let Some(at) = entry.expires {
            self.expirations.insert((at, key.to_vec()));
        }
        self.field_ttls = self.field_ttls.saturating_add(entry.field_ttl.len());
        self.entries.insert(key.to_vec(), entry);
        self.payload_bytes = payload;
        self.touch(key);
    }

    pub fn put_collection(
        &mut self,
        key: &[u8],
        collection: Collection,
        expires: Option<i64>,
        payload: usize,
        _now: i64,
    ) {
        self.access_seq = self.access_seq.wrapping_add(1);
        let access = self.access_seq;
        let collection_bytes = collection.payload_len();
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_access = access;
            entry.value.clear();
            let hash = collection.is_hash();
            entry.collection = Some(collection);
            entry.stream = None;
            entry.payload_len = collection_bytes;
            if !hash {
                self.field_ttls = self.field_ttls.saturating_sub(entry.field_ttl.len());
                entry.field_ttl.clear();
            }
            Self::replace_expiry(&mut self.expirations, key, entry, expires);
        } else {
            self.entries.insert(
                key.to_vec(),
                Entry {
                    value: Vec::new(),
                    collection: Some(collection),
                    stream: None,
                    payload_len: collection_bytes,
                    expires,
                    field_ttl: Default::default(),
                    last_access: access,
                },
            );
            if let Some(at) = expires {
                self.expirations.insert((at, key.to_vec()));
            }
        }
        self.payload_bytes = payload;
        self.touch(key);
    }

    pub fn put_stream(
        &mut self,
        key: &[u8],
        stream: Stream,
        expires: Option<i64>,
        payload: usize,
        _now: i64,
    ) {
        self.access_seq = self.access_seq.wrapping_add(1);
        let access = self.access_seq;
        let stream_bytes = stream.payload_bytes().expect("admitted stream payload");
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_access = access;
            entry.value.clear();
            entry.collection = None;
            entry.stream = Some(stream);
            entry.payload_len = stream_bytes;
            self.field_ttls = self.field_ttls.saturating_sub(entry.field_ttl.len());
            entry.field_ttl.clear();
            Self::replace_expiry(&mut self.expirations, key, entry, expires);
        } else {
            self.entries.insert(
                key.to_vec(),
                Entry {
                    value: Vec::new(),
                    collection: None,
                    stream: Some(stream),
                    payload_len: stream_bytes,
                    expires,
                    field_ttl: Default::default(),
                    last_access: access,
                },
            );
            if let Some(at) = expires {
                self.expirations.insert((at, key.to_vec()));
            }
        }
        self.payload_bytes = payload;
        self.touch(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_history_is_opt_in_and_cleared_when_disabled() {
        let mut data = Data::default();
        data.put(b"k", b"v", None, 0, 0);
        assert!(data.journal_events.is_empty());
        assert_eq!(data.changes_since(0), (Vec::new(), false));

        data.set_collect_events(true);
        data.put(b"k", b"v2", None, 0, 0);
        assert_eq!(data.changes_since(0), (vec![b"k".to_vec()], false));

        data.set_collect_events(false);
        assert!(data.journal_events.is_empty());
        assert!(!data.journal_overflow);
        data.put(b"k", b"v3", None, 0, 0);
        assert_eq!(data.changes_since(0), (Vec::new(), false));
    }

    #[test]
    fn lifecycle_queues_are_bounded_and_clearable() {
        let mut data = Data::default();
        push_lifecycle(&mut data.expired_events, b"a");
        push_lifecycle(&mut data.evicted_events, b"b");
        let (expired, evicted) = data.drain_lifecycle();
        assert_eq!(expired, vec![b"a".to_vec()]);
        assert_eq!(evicted, vec![b"b".to_vec()]);

        push_lifecycle(&mut data.expired_events, b"stale");
        data.clear_lifecycle();
        let (expired, evicted) = data.drain_lifecycle();
        assert!(expired.is_empty());
        assert!(evicted.is_empty());

        for index in 0..MAX_LIFECYCLE_EVENTS + 10 {
            push_lifecycle(&mut data.expired_events, index.to_string().as_bytes());
        }
        assert_eq!(data.expired_events.len(), MAX_LIFECYCLE_EVENTS);
    }
}
