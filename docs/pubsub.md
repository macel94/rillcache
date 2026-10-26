# Publish/subscribe

Rillcache implements channel and pattern publish/subscribe for connected clients.
Delivery is in-process, at-most-once, and never persisted or replicated. This is
not Redis Cluster sharded pub/sub, keyspace notifications or a durable queue.

## Commands

- `SUBSCRIBE channel [channel ...]` and `PSUBSCRIBE pattern [pattern ...]`
  register the connection and return one confirmation per name with the current
  total subscription count.
- `UNSUBSCRIBE [channel ...]` and `PUNSUBSCRIBE [pattern ...]` remove subscriptions.
  With no names all matching subscriptions are removed; with no subscriptions a
  single confirmation with a null channel is returned.
- `PUBLISH channel message` sends to every connection subscribed to that channel
  and to every pattern matching it, returning the number of delivered frames.
- `PUBSUB CHANNELS [pattern]`, `PUBSUB NUMSUB [channel ...]` and `PUBSUB NUMPAT`
  expose current channel names, subscriber counts and total pattern subscriptions.
  `PUBSUB SHARDCHANNELS`/`SHARDNUMSUB` return an explicit unsupported error.

Messages are RESP2 arrays or RESP3 push frames:

```text
*3\r\n$7\r\nmessage\r\n$5\r\nnews\r\n$5\r\nhello\r\n
*4\r\n$8\r\npmessage\r\n$6\r\nnews.*\r\n$9\r\nnews.tech\r\n$5\r\nhello\r\n
>3 ...   (RESP3 message push)
>4 ...   (RESP3 pattern-message push)
```

## Delivery model

Each connection owns a bounded message queue (1,024 frames). A publisher uses
non-blocking sends; a full or disconnected queue is removed and not delivered to,
so a slow subscriber cannot block the publisher or other clients. Delivery is
at-most-once: messages can be lost on disconnect, backpressure, process restart
or crash. There is no replay, acknowledgment, durability or ordering guarantee
across different publishers beyond the order their frames were enqueued.

The connection thread drains queued frames in bounded batches and never writes
messages from another thread, so reply and push frames cannot interleave. In
default thread-per-client mode, subscribed connections use a 10 ms socket read
timeout to alternate message draining with command reads; this is a bounded
delivery-latency mechanism, not busy polling or a real-time guarantee. Fixed
worker lanes drain the same queue during their normal poll passes. Subscription
changes are checked on every pass, and closing a connection removes its entries.

Kept small: at most 1,024 queued frames per subscriber, one queue per admitted
connection, and no unbounded message history. `max_clients` still bounds
connections. A visible subscriber with an unread socket can be removed when its
queue fills; no partial frame is written because frames are preformatted and
written whole only by the owning connection thread.

## Subscriber mode

In RESP2, while at least one subscription exists, only `SUBSCRIBE`, `PSUBSCRIBE`,
`UNSUBSCRIBE`, `PUNSUBSCRIBE`, `PING` and `QUIT` are allowed. `PING [message]`
returns the subscriber-style `["pong", message]` array. Other commands return:

```text
ERR Can't execute '<cmd>': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT are allowed in this context
```

In RESP3, normal commands remain available and message/confirmation frames use
push type. `RESET` is allowed from RESP2 subscriber mode and unsubscribes
everything while returning the connection to RESP2; otherwise unsubscribe or
reconnect to leave subscriber mode. Connection-local commands are not queued
inside `MULTI`; pub/sub commands are rejected there.

## ACL channels

ACL channel patterns are enforced for `SUBSCRIBE`, `PSUBSCRIBE`, `UNSUBSCRIBE`,
`PUNSUBSCRIBE` and `PUBLISH`. A denied channel returns
`NOPERM this user has no permissions to access one of the channels used as arguments`.
`PUBSUB` introspection exposes global channel names/counts and is governed by the
`@pubsub` command category, not by per-channel patterns; do not grant it to a
tenant that must not learn other channel names.

## Limits

- No sharded pub/sub, cluster bus, keyspace notifications, `CLIENT` tracking or
  tracking/notification integration.
- No persistence, replication, ordering across publishers, delivery accounting,
  dead-letter handling or exactly-once semantics.
- Pattern matching uses the server's bounded Redis-style glob matcher; pathological
  patterns share the existing glob work budget and may fail to match rather than
  blocking indefinitely.
- Messages are copied once per delivered subscriber; a large fan-out is bounded by
  the number of admitted clients and per-subscriber queue capacity, not by a
  global byte budget. This is not an RSS limit.

## Evidence

Unit tests cover confirmations/counts, channel and pattern frames, binary payloads,
RESP3 push framing, unsubscribe modes, subscriber-mode rejection/PING, introspection
and ACL channel denial. Wire tests exercise two live connections, binary channels
and messages, pattern delivery, introspection, RESP3 pushes and subscriber-mode
QUIT in thread-per-client, one-worker and four-worker modes. Official references:
[SUBSCRIBE](https://redis.io/docs/latest/commands/subscribe/),
[PSUBSCRIBE](https://redis.io/docs/latest/commands/psubscribe/),
[PUBLISH](https://redis.io/docs/latest/commands/publish/) and
[PUBSUB](https://redis.io/docs/latest/commands/pubsub/).
