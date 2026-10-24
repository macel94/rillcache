use std::collections::{BTreeSet, HashMap};

pub(super) struct Entry {
    pub value: Vec<u8>,
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
        self.payload_bytes -= key.len() + entry.value.len();
        if let Some(at) = entry.expires {
            self.expirations.remove(&(at, key));
        }
        Some(entry)
    }
    pub fn expire_key(&mut self, key: &[u8], now: i64) {
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
        if entry.expires == expires {
            return;
        }
        if let Some(at) = entry.expires {
            self.expirations.remove(&(at, key.to_vec()));
        }
        entry.expires = expires;
        if let Some(at) = expires {
            self.expirations.insert((at, key.to_vec()));
        }
    }
    pub fn reserve(&mut self, key: &[u8], bytes: usize, maximum: usize, now: i64) -> Option<usize> {
        let size = |data: &Self| {
            let existing = data.entries.get(key);
            (data.payload_bytes - existing.map_or(0, |entry| entry.value.len()))
                .checked_add(if existing.is_none() { key.len() } else { 0 })?
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
            entry.value = value.to_vec();
            self.set_expiry(key, expires);
        } else {
            self.entries.insert(
                key.to_vec(),
                Entry {
                    value: value.to_vec(),
                    expires: None,
                },
            );
            self.set_expiry(key, expires);
        }
        self.payload_bytes = payload;
    }
}
