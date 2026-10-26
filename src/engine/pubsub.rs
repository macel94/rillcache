//! In-process publish/subscribe fan-out.
//!
//! Each subscriber owns a bounded channel drained by its connection loop. The
//! publisher uses non-blocking sends, so a slow/dead subscriber cannot block a
//! publisher or another subscriber. Messages are at-most-once and never persisted.

use std::collections::HashMap;
use std::sync::mpsc::SyncSender;

const SUBSCRIBER_QUEUE: usize = 1024;

#[derive(Clone)]
struct Subscriber {
    id: u64,
    sender: SyncSender<Vec<u8>>,
    resp3: bool,
}

#[derive(Default)]
pub(super) struct PubSub {
    channels: HashMap<Vec<u8>, Vec<Subscriber>>,
    patterns: HashMap<Vec<u8>, Vec<Subscriber>>,
}

impl PubSub {
    pub(super) fn queue_capacity() -> usize {
        SUBSCRIBER_QUEUE
    }

    pub(super) fn subscribe(
        &mut self,
        pattern: bool,
        name: &[u8],
        id: u64,
        sender: SyncSender<Vec<u8>>,
        resp3: bool,
    ) {
        let table = if pattern {
            &mut self.patterns
        } else {
            &mut self.channels
        };
        let entry = table.entry(name.to_vec()).or_default();
        entry.retain(|subscriber| subscriber.id != id);
        entry.push(Subscriber { id, sender, resp3 });
    }

    pub(super) fn unsubscribe(&mut self, pattern: bool, name: &[u8], id: u64) -> bool {
        let table = if pattern {
            &mut self.patterns
        } else {
            &mut self.channels
        };
        let Some(entry) = table.get_mut(name) else {
            return false;
        };
        let before = entry.len();
        entry.retain(|subscriber| subscriber.id != id);
        let removed = entry.len() != before;
        if entry.is_empty() {
            table.remove(name);
        }
        removed
    }

    pub(super) fn remove_client(&mut self, id: u64) {
        for table in [&mut self.channels, &mut self.patterns] {
            table.retain(|_, subscribers| {
                subscribers.retain(|subscriber| subscriber.id != id);
                !subscribers.is_empty()
            });
        }
    }

    pub(super) fn set_resp3(&mut self, id: u64, resp3: bool) {
        for table in [&mut self.channels, &mut self.patterns] {
            for subscribers in table.values_mut() {
                for subscriber in subscribers {
                    if subscriber.id == id {
                        subscriber.resp3 = resp3;
                    }
                }
            }
        }
    }

    pub(super) fn publish(&mut self, channel: &[u8], message: &[u8]) -> usize {
        let mut delivered = 0;
        for (name, subscribers) in self.channels.iter_mut() {
            if name.as_slice() != channel {
                continue;
            }
            subscribers.retain(|subscriber| {
                let frame = message_frame(subscriber.resp3, false, None, channel, message);
                if subscriber.sender.try_send(frame).is_ok() {
                    delivered += 1;
                    true
                } else {
                    false
                }
            });
        }
        let mut empty = Vec::new();
        for (pattern, subscribers) in self.patterns.iter_mut() {
            if !super::keyspace::glob_match(pattern, channel) {
                continue;
            }
            subscribers.retain(|subscriber| {
                let frame = message_frame(subscriber.resp3, true, Some(pattern), channel, message);
                if subscriber.sender.try_send(frame).is_ok() {
                    delivered += 1;
                    true
                } else {
                    false
                }
            });
            if subscribers.is_empty() {
                empty.push(pattern.clone());
            }
        }
        for pattern in empty {
            self.patterns.remove(&pattern);
        }
        self.channels
            .retain(|_, subscribers| !subscribers.is_empty());
        delivered
    }

    pub(super) fn channel_names(&self, pattern: Option<&[u8]>) -> Vec<Vec<u8>> {
        let mut names: Vec<Vec<u8>> = self
            .channels
            .keys()
            .filter(|name| pattern.is_none_or(|pattern| super::keyspace::glob_match(pattern, name)))
            .cloned()
            .collect();
        names.sort_unstable();
        names
    }

    pub(super) fn numsub(&self, channels: &[&[u8]]) -> Vec<(Vec<u8>, usize)> {
        channels
            .iter()
            .map(|channel| {
                (
                    channel.to_vec(),
                    self.channels.get(*channel).map_or(0, Vec::len),
                )
            })
            .collect()
    }

    pub(super) fn numpat(&self) -> usize {
        self.patterns.values().map(Vec::len).sum()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.channels.is_empty() && self.patterns.is_empty()
    }
}

fn message_frame(
    resp3: bool,
    pattern: bool,
    pattern_name: Option<&[u8]>,
    channel: &[u8],
    message: &[u8],
) -> Vec<u8> {
    let kind = if pattern {
        b"pmessage".as_slice()
    } else {
        b"message"
    };
    let fields = if pattern { 4 } else { 3 };
    let mut output = Vec::with_capacity(
        32 + channel.len() + message.len() + pattern_name.map_or(0, <[u8]>::len),
    );
    output.push(if resp3 { b'>' } else { b'*' });
    output.extend_from_slice(fields.to_string().as_bytes());
    output.extend_from_slice(b"\r\n");
    super::encode_bulk(Some(kind), &mut output);
    if let Some(pattern_name) = pattern_name {
        super::encode_bulk(Some(pattern_name), &mut output);
    }
    super::encode_bulk(Some(channel), &mut output);
    super::encode_bulk(Some(message), &mut output);
    output
}
