//! Blocking list pop support with bounded polling.
//!
//! A blocked connection is not parsed for further commands until its pop succeeds
//! or its timeout expires. The owner thread/lane polls a ready list about every
//! 10 ms; this is a bounded latency/CPU tradeoff, not a condition-variable
//! guarantee or a server push notification.

use super::Store;
use super::collections::encode_list;
use super::data::Collection;

#[derive(Debug, PartialEq, Eq)]
pub enum PopOutcome {
    Value { key: Vec<u8>, value: Vec<u8> },
    Empty,
    WrongType,
    JournalError,
}

impl Store {
    /// Pop one live list element from the first nonempty key under the store's
    /// normal transaction/journal gate. Expired entries are treated as absent.
    pub fn blocking_pop(&self, keys: &[&[u8]], left: bool) -> PopOutcome {
        if self.journal.is_some() {
            let Ok(_gate) = self.transaction_gate.write() else {
                return PopOutcome::JournalError;
            };
            self.blocking_pop_gated(keys, left)
        } else {
            let Ok(_gate) = self.transaction_gate.read() else {
                return PopOutcome::JournalError;
            };
            self.blocking_pop_gated(keys, left)
        }
    }

    fn blocking_pop_gated(&self, keys: &[&[u8]], left: bool) -> PopOutcome {
        let marks = self.journal_marks();
        let outcome = {
            let Ok(mut data) = self.data.write() else {
                return PopOutcome::JournalError;
            };
            let now = self.now(&data);
            let mut result = PopOutcome::Empty;
            for key in keys {
                data.expire_key(key, now);
                let Some(entry) = data.entries.get(*key) else {
                    continue;
                };
                let Some(mut list) = entry.collection.as_ref().and_then(Collection::list_values)
                else {
                    result = PopOutcome::WrongType;
                    break;
                };
                if list.is_empty() {
                    continue;
                }
                let value = if left {
                    list.pop_front().expect("nonempty list")
                } else {
                    list.pop_back().expect("nonempty list")
                };
                let expires = entry.expires;
                let previous = entry.payload_len();
                let Ok(config) = self.collection_config.read() else {
                    return PopOutcome::JournalError;
                };
                if list.is_empty() {
                    data.remove(key);
                } else {
                    let bytes = list.iter().map(Vec::len).sum::<usize>();
                    let payload = data.payload_bytes - previous + bytes;
                    let encoded = encode_list(list, config.list_max_listpack_size);
                    data.put_collection(key, encoded, expires, payload);
                }
                result = PopOutcome::Value {
                    key: key.to_vec(),
                    value,
                };
                break;
            }
            result
        };
        if matches!(outcome, PopOutcome::Value { .. }) {
            let mut output = Vec::new();
            self.commit_journal(marks, &mut output, 0);
            if output.first() == Some(&b'-') {
                return PopOutcome::JournalError;
            }
        }
        outcome
    }
}

pub fn reply_value(outcome: PopOutcome, output: &mut Vec<u8>) {
    match outcome {
        PopOutcome::Value { key, value } => {
            output.extend_from_slice(b"*2\r\n");
            super::encode_bulk(Some(&key), output);
            super::encode_bulk(Some(&value), output);
        }
        PopOutcome::Empty => {
            output.extend_from_slice(b"*-1\r\n");
        }
        PopOutcome::WrongType => super::wrong_type(output),
        PopOutcome::JournalError => {
            output.extend_from_slice(b"-ERR append-only write failed\\r\\n")
        }
    }
}
