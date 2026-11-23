#![forbid(unsafe_code)]

use std::sync::mpsc::{TryRecvError, sync_channel};

use rillcache::engine::Store;

#[test]
fn full_queue_losses_are_counted_without_blocking_healthy_subscribers_or_replaying() {
    let store = Store::new(4096);
    let capacity = Store::pubsub_queue_capacity();
    let count = || {
        let mut reply = Vec::new();
        store.execute(&[b"INFO", b"stats"], &mut reply);
        std::str::from_utf8(&reply)
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("pubsub_queue_full_drops:"))
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    assert_eq!(count(), 0);
    for (index, kind) in [b"channel".as_slice(), b"pattern", b"shard"]
        .into_iter()
        .enumerate()
    {
        let subscribe = |id, sender| match kind {
            b"channel" => store.pubsub_subscribe(false, b"topic", id, sender, false),
            b"pattern" => store.pubsub_subscribe(true, b"to*", id, sender, false),
            _ => store.pubsub_subscribe_shard(b"topic", id, sender, false),
        };
        let publish = |msg: &[u8]| {
            if kind == b"shard" {
                store.publish_shard(b"topic", msg)
            } else {
                store.publish(b"topic", msg)
            }
        };
        let (slow, slow_messages) = sync_channel(capacity);
        assert!(subscribe(1, slow));
        for _ in 0..capacity {
            assert_eq!(publish(b"earlier"), 1);
        }
        let (healthy, healthy_messages) = sync_channel(capacity);
        assert!(subscribe(2, healthy));
        let (closed, closed_messages) = sync_channel(capacity);
        assert!(subscribe(3, closed));
        drop(closed_messages);
        assert_eq!(publish(b"overflow"), 1);
        assert!(
            healthy_messages
                .try_recv()
                .unwrap()
                .ends_with(b"$8\r\noverflow\r\n")
        );
        assert_eq!(count(), index as u64 + 1);
        for _ in 0..capacity {
            assert!(
                slow_messages
                    .try_recv()
                    .unwrap()
                    .ends_with(b"$7\r\nearlier\r\n")
            );
        }
        assert_eq!(slow_messages.try_recv(), Err(TryRecvError::Disconnected));

        let (resubscribed, fresh_messages) = sync_channel(capacity);
        assert!(subscribe(1, resubscribed));
        assert_eq!(fresh_messages.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(publish(b"fresh"), 2);
        assert!(
            fresh_messages
                .try_recv()
                .unwrap()
                .ends_with(b"$5\r\nfresh\r\n")
        );
        assert_eq!(count(), index as u64 + 1);
        store.pubsub_remove_client(1);
        store.pubsub_remove_client(2);
    }
}
