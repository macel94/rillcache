use std::collections::{BTreeSet, HashMap, VecDeque};

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

pub(super) struct Entry {
    pub value: Vec<u8>,
    pub collection: Option<Collection>,
    pub expires: Option<i64>,
}
impl Entry {
    pub fn payload_len(&self) -> usize {
        self.collection
            .as_ref()
            .map_or(self.value.len(), Collection::payload_len)
    }

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
        self.payload_bytes -= key.len() + entry.payload_len();
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
        debug_assert_eq!(self.entries.get(key).map(Entry::payload_len), previous);
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
            // Reuse admitted overwrites, but release disproportionate old capacity
            // on shrink (and all capacity for an empty replacement).
            entry.collection = None;
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
                    expires,
                },
            );
            if let Some(at) = expires {
                self.expirations.insert((at, key.to_vec()));
            }
        }
        self.payload_bytes = payload;
    }

    pub fn put_entry(&mut self, key: &[u8], entry: Entry, payload: usize) {
        if let Some(at) = entry.expires {
            self.expirations.insert((at, key.to_vec()));
        }
        self.entries.insert(key.to_vec(), entry);
        self.payload_bytes = payload;
    }

    pub fn put_collection(
        &mut self,
        key: &[u8],
        collection: Collection,
        expires: Option<i64>,
        payload: usize,
    ) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.value = Vec::new();
            entry.collection = Some(collection);
            Self::replace_expiry(&mut self.expirations, key, entry, expires);
        } else {
            self.entries.insert(
                key.to_vec(),
                Entry {
                    value: Vec::new(),
                    collection: Some(collection),
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
