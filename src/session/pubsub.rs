//! Channel, pattern, shard and MONITOR subscription state and delivery.

use std::io::Write;
use std::sync::mpsc::SyncSender;

use crate::engine::Store;
use crate::protocol::encode_bulk;

use super::Protocol;
use super::Session;
use super::command::arity;

impl Session {
    /// Attach the bounded message channel used by this connection's pub/sub hub.
    pub fn set_pubsub_sender(&mut self, sender: SyncSender<Vec<u8>>) {
        self.pubsub_sender = Some(sender);
    }

    pub fn is_monitoring(&self) -> bool {
        self.monitoring
    }

    pub fn is_subscribed(&self) -> bool {
        !self.pubsub_channels.is_empty()
            || !self.pubsub_patterns.is_empty()
            || !self.pubsub_shard_channels.is_empty()
    }

    pub(super) fn subscription_count(&self) -> usize {
        self.pubsub_channels.len() + self.pubsub_patterns.len()
    }

    pub(super) fn shard_subscription_count(&self) -> usize {
        self.pubsub_shard_channels.len()
    }

    pub(super) fn refresh_pubsub_protocol(&self, store: &Store) {
        if self.is_subscribed() {
            store.pubsub_set_resp3(self.id, self.protocol == Protocol::Resp3);
        }
    }

    pub(super) fn subscribe(
        &mut self,
        store: &Store,
        names: &[&[u8]],
        pattern: bool,
        output: &mut Vec<u8>,
    ) {
        let command = if pattern { "psubscribe" } else { "subscribe" };
        if names.is_empty() {
            arity(command, output);
            return;
        }
        let Some(sender) = self.pubsub_sender.clone() else {
            output.extend_from_slice(b"-ERR pub/sub is unavailable on this connection\r\n");
            return;
        };
        for name in names {
            if pattern {
                self.pubsub_patterns.insert(name.to_vec());
            } else {
                self.pubsub_channels.insert(name.to_vec());
            }
            store.pubsub_subscribe(
                pattern,
                name,
                self.id,
                sender.clone(),
                self.protocol == Protocol::Resp3,
            );
            self.subscription_reply(command, Some(name), output);
        }
    }

    pub(super) fn subscribe_shard(&mut self, store: &Store, names: &[&[u8]], output: &mut Vec<u8>) {
        if names.is_empty() {
            arity("ssubscribe", output);
            return;
        }
        let Some(sender) = self.pubsub_sender.clone() else {
            output.extend_from_slice(b"-ERR pub/sub is unavailable on this connection\r\n");
            return;
        };
        for name in names {
            self.pubsub_shard_channels.insert(name.to_vec());
            store.pubsub_subscribe_shard(
                name,
                self.id,
                sender.clone(),
                self.protocol == Protocol::Resp3,
            );
            self.shard_subscription_reply("ssubscribe", Some(name), output);
        }
    }

    pub(super) fn unsubscribe_shard(
        &mut self,
        store: &Store,
        names: &[&[u8]],
        output: &mut Vec<u8>,
    ) {
        let local: Vec<Vec<u8>> = if names.is_empty() {
            self.pubsub_shard_channels.iter().cloned().collect()
        } else {
            names.iter().map(|name| name.to_vec()).collect()
        };
        if local.is_empty() {
            self.shard_subscription_reply("sunsubscribe", None, output);
            return;
        }
        for name in &local {
            self.pubsub_shard_channels.remove(name);
            store.pubsub_unsubscribe_shard(name, self.id);
            self.shard_subscription_reply("sunsubscribe", Some(name), output);
        }
    }

    pub(super) fn shard_subscription_reply(
        &self,
        kind: &str,
        channel: Option<&[u8]>,
        output: &mut Vec<u8>,
    ) {
        output.push(if self.protocol == Protocol::Resp3 {
            b'>'
        } else {
            b'*'
        });
        output.extend_from_slice(b"3\r\n");
        encode_bulk(Some(kind.as_bytes()), output);
        encode_bulk(channel, output);
        let _ = write!(output, ":{}", self.shard_subscription_count());
        output.extend_from_slice(b"\r\n");
    }

    pub(super) fn unsubscribe(
        &mut self,
        store: &Store,
        names: &[&[u8]],
        pattern: bool,
        output: &mut Vec<u8>,
    ) {
        let command = if pattern {
            "punsubscribe"
        } else {
            "unsubscribe"
        };
        let local: Vec<Vec<u8>> = if names.is_empty() {
            if pattern {
                self.pubsub_patterns.iter().cloned().collect()
            } else {
                self.pubsub_channels.iter().cloned().collect()
            }
        } else {
            names.iter().map(|name| name.to_vec()).collect()
        };
        if local.is_empty() {
            self.subscription_reply(command, None, output);
            return;
        }
        for name in &local {
            if pattern {
                self.pubsub_patterns.remove(name);
            } else {
                self.pubsub_channels.remove(name);
            }
            store.pubsub_unsubscribe(pattern, name, self.id);
            self.subscription_reply(command, Some(name), output);
        }
    }

    pub(super) fn subscription_reply(
        &self,
        kind: &str,
        channel: Option<&[u8]>,
        output: &mut Vec<u8>,
    ) {
        output.push(if self.protocol == Protocol::Resp3 {
            b'>'
        } else {
            b'*'
        });
        output.extend_from_slice(b"3\r\n");
        encode_bulk(Some(kind.as_bytes()), output);
        encode_bulk(channel, output);
        let _ = write!(output, ":{}", self.subscription_count());
        output.extend_from_slice(b"\r\n");
    }

    pub(super) fn publish(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 3 {
            arity("publish", output);
            return;
        }
        let delivered = store.publish(arguments[1], arguments[2]);
        let _ = write!(output, ":{delivered}\r\n");
    }

    pub(super) fn spublish(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 3 {
            arity("spublish", output);
            return;
        }
        let delivered = store.publish_shard(arguments[1], arguments[2]);
        let _ = write!(output, ":{delivered}\r\n");
    }

    pub(super) fn pubsub(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        let Some(subcommand) = arguments.first() else {
            arity("pubsub", output);
            return;
        };
        if subcommand.eq_ignore_ascii_case(b"CHANNELS") {
            if arguments.len() > 2 {
                arity("pubsub|channels", output);
                return;
            }
            let names = store.pubsub_channels(arguments.get(1).copied());
            let _ = write!(output, "*{}\r\n", names.len());
            for name in names {
                encode_bulk(Some(&name), output);
            }
        } else if subcommand.eq_ignore_ascii_case(b"NUMSUB") {
            let counts = store.pubsub_numsub(&arguments[1..]);
            let _ = write!(output, "*{}\r\n", counts.len() * 2);
            for (channel, count) in counts {
                encode_bulk(Some(&channel), output);
                let _ = write!(output, ":{count}\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"NUMPAT") {
            if arguments.len() != 1 {
                arity("pubsub|numpat", output);
            } else {
                let _ = write!(output, ":{}", store.pubsub_numpat());
                output.extend_from_slice(b"\r\n");
            }
        } else if subcommand.eq_ignore_ascii_case(b"SHARDCHANNELS") {
            if arguments.len() > 2 {
                output.extend_from_slice(
                    b"-ERR unknown subcommand or wrong number of arguments for 'shardchannels'. Try PUBSUB HELP.\r\n",
                );
                return;
            }
            let names = store.pubsub_shard_channels(arguments.get(1).copied());
            let _ = write!(output, "*{}\r\n", names.len());
            for name in names {
                encode_bulk(Some(&name), output);
            }
        } else if subcommand.eq_ignore_ascii_case(b"SHARDNUMSUB") {
            let counts = store.pubsub_shard_numsub(&arguments[1..]);
            let _ = write!(output, "*{}\r\n", counts.len() * 2);
            for (channel, count) in counts {
                encode_bulk(Some(&channel), output);
                let _ = write!(output, ":{count}\r\n");
            }
        } else {
            output.extend_from_slice(b"-ERR unsupported PUBSUB subcommand\r\n");
        }
    }
    pub(super) fn subscriber_ping(&self, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() > 1 {
            arity("ping", output);
            return;
        }
        output.extend_from_slice(b"*2\r\n$4\r\npong\r\n");
        encode_bulk(Some(arguments.first().copied().unwrap_or(b"")), output);
    }
    pub(super) fn monitor(&mut self, store: &Store, arguments: &[&[u8]], output: &mut Vec<u8>) {
        if arguments.len() != 1 {
            arity("monitor", output);
            return;
        }
        if !self.monitoring {
            let Some(sender) = self.pubsub_sender.clone() else {
                output.extend_from_slice(b"-ERR MONITOR is unavailable for this connection\r\n");
                return;
            };
            if !store.add_monitor(self.id, sender) {
                output.extend_from_slice(b"-ERR internal store error\r\n");
                return;
            }
            self.monitoring = true;
        }
        output.extend_from_slice(b"+OK\r\n");
    }
}
