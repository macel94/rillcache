//! Blocking list pop support with bounded polling.
//!
//! A blocked connection is not parsed for further commands until its pop succeeds
//! or its timeout expires. The owner thread/lane polls a ready list about every
//! 10 ms; this is a bounded latency/CPU tradeoff, not a condition-variable
//! guarantee or a server push notification.

use super::ShardExecutor;
use super::collection_commands::{ListMoveOutcome, ListPopOutcome, ZSetPopOutcome};
use super::collections::encode_list;
use super::data::Collection;
use super::notify::Category as NotifyCategory;

#[derive(Debug, PartialEq, Eq)]
pub enum PopOutcome {
    Value { key: Vec<u8>, value: Vec<u8> },
    Empty,
    WrongType,
    JournalError,
}

impl ShardExecutor {
    /// Pop one live list element from the first nonempty key under the store's
    /// synchronous owner turn. Expired entries are treated as absent.
    pub fn blocking_pop(&self, keys: &[&[u8]], left: bool) -> PopOutcome {
        self.blocking_pop_in_turn(keys, left)
    }

    fn blocking_pop_in_turn(&self, keys: &[&[u8]], left: bool) -> PopOutcome {
        let marks = self.journal_marks();
        let outcome = {
            let Ok(mut data) = self.data.try_borrow_mut() else {
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
                    data.put_collection(key, encoded, expires, payload, now);
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
            self.commit_journal(marks, &[], &mut output, 0);
            if output.first() == Some(&b'-') {
                return PopOutcome::JournalError;
            }
            if let PopOutcome::Value { key, .. } = &outcome {
                self.notify_event(
                    if left { "lpop" } else { "rpop" },
                    key,
                    NotifyCategory::List,
                );
            }
        }
        outcome
    }
}

impl ShardExecutor {
    /// Pop from the first ready list in one non-interleaved owner turn,
    /// matching one BLMPOP attempt.
    pub(crate) fn blocking_lmpop(
        &self,
        keys: &[&[u8]],
        left: bool,
        count: usize,
    ) -> ListPopOutcome {
        self.blocking_lmpop_in_turn(keys, left, count)
    }

    fn blocking_lmpop_in_turn(&self, keys: &[&[u8]], left: bool, count: usize) -> ListPopOutcome {
        let marks = self.journal_marks();
        let outcome = {
            let Ok(mut data) = self.data.try_borrow_mut() else {
                return ListPopOutcome::JournalError;
            };
            let now = self.now(&data);
            self.list_pop_in_data(&mut data, keys, left, count, now)
        };
        if matches!(outcome, ListPopOutcome::Popped { .. }) {
            let mut output = Vec::new();
            self.commit_journal(marks, &[], &mut output, 0);
            if output.first() == Some(&b'-') {
                return ListPopOutcome::JournalError;
            }
            if let ListPopOutcome::Popped { key, .. } = &outcome {
                self.notify_event("lmpop", key, NotifyCategory::List);
            }
        }
        outcome
    }

    /// Move one element between lists within one synchronous owner turn. Returns `Empty` when the source has no live element so the
    /// caller can keep blocking.
    pub(crate) fn blocking_list_move(
        &self,
        source: &[u8],
        destination: &[u8],
        from_left: bool,
        to_left: bool,
    ) -> ListMoveOutcome {
        self.blocking_list_move_in_turn(source, destination, from_left, to_left)
    }

    fn blocking_list_move_in_turn(
        &self,
        source: &[u8],
        destination: &[u8],
        from_left: bool,
        to_left: bool,
    ) -> ListMoveOutcome {
        let marks = self.journal_marks();
        let outcome = {
            let Ok(mut data) = self.data.try_borrow_mut() else {
                return ListMoveOutcome::JournalError;
            };
            let now = self.now(&data);
            self.list_move_in_data(&mut data, source, destination, from_left, to_left, now)
        };
        if matches!(outcome, ListMoveOutcome::Moved { .. }) {
            let mut output = Vec::new();
            self.commit_journal(marks, &[], &mut output, 0);
            if output.first() == Some(&b'-') {
                return ListMoveOutcome::JournalError;
            }
            if let ListMoveOutcome::Moved {
                source,
                source_emptied,
                ..
            } = &outcome
            {
                self.notify_list_move(source, destination, from_left, to_left, *source_emptied);
            }
        }
        outcome
    }

    /// Pop from the first ready sorted set in one owner turn, matching one
    /// BZMPOP attempt.
    pub(crate) fn blocking_zpop(&self, keys: &[&[u8]], max: bool, count: usize) -> ZSetPopOutcome {
        self.blocking_zpop_in_turn(keys, max, count)
    }

    fn blocking_zpop_in_turn(&self, keys: &[&[u8]], max: bool, count: usize) -> ZSetPopOutcome {
        let marks = self.journal_marks();
        let outcome = {
            let Ok(mut data) = self.data.try_borrow_mut() else {
                return ZSetPopOutcome::JournalError;
            };
            let now = self.now(&data);
            self.zset_pop_in_data(&mut data, keys, max, count, now)
        };
        if matches!(outcome, ZSetPopOutcome::Popped { .. }) {
            let mut output = Vec::new();
            self.commit_journal(marks, &[], &mut output, 0);
            if output.first() == Some(&b'-') {
                return ZSetPopOutcome::JournalError;
            }
            if let ZSetPopOutcome::Popped { key, .. } = &outcome {
                self.notify_event(
                    if max { "zpopmax" } else { "zpopmin" },
                    key,
                    NotifyCategory::Zset,
                );
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
