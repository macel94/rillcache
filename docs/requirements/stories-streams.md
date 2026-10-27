# Stream stories

Part of the [requirements and incremental delivery index](../requirements.md). Rows are preserved verbatim from the former monolithic story table; add new stories only in the matching area file.

| ID | Story and value | Size | Acceptance evidence |
| --- | --- | --- | --- |
| S43 | As a stream consumer, I can range-read entries, inspect and acknowledge pending deliveries, and read multiple streams without blocking so that consumer groups are usable beyond one read. | S | R169–R172; ack/pending/range/read/group lifecycle and all-mode wire tests |
| S46 | As a stream consumer-group operator, I can recover, reassign and inspect pending deliveries so that stalled consumers can be replaced without losing acknowledged work. | S | R181–R184; claim/autoclaim/introspection, errors and all-mode wire tests |
| S48 | As a stream operator, I can bound retained history by length or minimum ID so that long-lived streams do not grow without limit. | XS | R189–R192; exact/approx/LIMIT/errors/TTL/WATCH and all-mode wire tests |
| S54 | As a stream producer, I can append with explicit retention and skip creating a missing stream so that hot streams stay bounded without a separate trim call. | S | R213–R216; XADD trim/NOMKSTREAM/errors/TTL/WATCH and all-mode wire tests |
| S57 | As a stream operator, I can align a stream's last ID and retention counters after restore so that producers and groups resume without ID conflicts. | S | R225–R228; last-id/entries-added/max-deleted validation, XINFO, WATCH and all-mode wire tests |
| S58 | As a stream consumer, I can re-read my owned pending deliveries from a chosen ID so that recovery workflows can inspect and replay unacknowledged work. | S | R229–R232; history IDs/deleted entries/delivery updates/errors and all-mode wire tests |
| S59 | As a stream consumer, I can wait for new entries on one or more streams so that tailing clients do not have to poll. | S | R233–R236; BLOCK parsing/timeout/wakeup/$/multi-key/MULTI and all-mode wire tests |
| S62 | As a stream group consumer, I can wait for new deliveries without polling empty streams so that workers can consume as producers append. | S | R245–R248; BLOCK group/timeout/wakeup/NOACK/history/MULTI and all-mode wire tests |
| S67 | As a stream consumer, I can read new or pending history entries across several group streams in one command so that sharded workloads avoid per-stream round trips. | S | R265–R268; multi-stream new/history/mixed/BLOCK/count/errors and all-mode wire tests |
