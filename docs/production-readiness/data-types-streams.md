# Data types and command surface: Streams

Part of the [production-readiness plan](../production-readiness.md). This file owns
the `Streams` clauses of the `Data types and command surface` capability row. The
original row began with the lead-in `Implemented S37/S39/S40/S42/S44/S45/S46/S47/S48/S49/S50/S51/S52/S53/S54/S55/S56/S57/S58/S59/S60/S61/S62/S63/S64/S65/S66/S67/S68/S69/S70/S71/S72/S73/S74/S75/S-HC: `; it applies to every clause below.

| Capability | Current boundary / evidence required before closing |
| --- | --- |
| Data types and command surface: streams | a bounded stream subset with pending claiming/autoclaim, group/consumer introspection (S46), bounded XTRIM retention (S48), XADD trim/NOMKSTREAM retention (S54), journaled `XSETID` metadata (S57), single-stream consumer history reads (S58), blocking multi-stream `XREAD` (S59) and blocking consumer-group reads (S62); multi-stream consumer-group new/history reads (S67) |
