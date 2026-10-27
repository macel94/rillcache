# Store component scaling

No sockets, session observers, or reference servers. Not TCP/Valkey acceptance.
Handler percentiles include two Instant reads/black-box/assertion overhead; whole-loop elapsed also includes reply validation. Seeding and 16 warmups are excluded. Shared-host observations, not SLOs.

| Operation | Elements/keys | Handler p50 median ns [min–max] | Median handler p99 ns | Whole loop ns/call |
|---|---:|---:|---:|---:|
| ping | 128 | 60 [60–60] | 80 | 167 |
| ping | 1024 | 60 [60–60] | 70 | 186 |
| ping | 8192 | 60 [60–60] | 70 | 133 |
| ping | 32768 | 60 [60–60] | 80 | 114 |
| hget | 128 | 350 [330–440] | 450 | 604 |
| hget | 1024 | 400 [380–400] | 520 | 920 |
| hget | 8192 | 380 [380–420] | 740 | 567 |
| hget | 32768 | 380 [370–430] | 520 | 529 |
| hset | 128 | 37911 [37311–39561] | 4170161 | 156071 |
| hset | 1024 | 220357 [205617–230657] | 2012988 | 399365 |
| hset | 8192 | 4071088 [3954095–4132480] | 39173656 | 6258729 |
| hset | 32768 | 27301324 [26938651–28845527] | 532493208 | 49550426 |
| hlen | 128 | 630 [600–660] | 940 | 889 |
| hlen | 1024 | 4770 [4620–4840] | 20720 | 5992 |
| hlen | 8192 | 42081 [40482–42872] | 149065 | 50156 |
| hlen | 32768 | 182216 [180216–214047] | 2793374 | 1119033 |
| scard | 128 | 7640 [7530–7970] | 73963 | 13270 |
| scard | 1024 | 68602 [67702–69323] | 323790 | 82907 |
| scard | 8192 | 559589 [511177–566069] | 6945065 | 1387295 |
| scard | 32768 | 2418652 [2395161–2612969] | 42296542 | 4284722 |
| info-server | 128 | 1390 [1350–1400] | 1970 | 1797 |
| info-server | 1024 | 11540 [9990–11640] | 32542 | 14772 |
| info-server | 8192 | 124364 [116753–126295] | 3687245 | 278338 |
| info-server | 32768 | 2690191 [2671261–2740963] | 36693341 | 4634613 |
| memory-stats | 128 | 1160 [1110–1210] | 3630 | 2174 |
| memory-stats | 1024 | 3310 [3220–3350] | 6210 | 4270 |
| memory-stats | 8192 | 26691 [24931–29461] | 82843 | 31492 |
| memory-stats | 32768 | 612450 [599940–631371] | 6518680 | 1290672 |
