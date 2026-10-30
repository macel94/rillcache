# Owner-local default-runtime A/B (advisory)

default Unix plain TCP, one logical server CPU, 8 clients, 1024 keys, 64-byte values, 1000 warmups/client; shared agent host, not isolated performance hardware; no Valkey/multi-core claim.

Three fresh alternating repetitions per build/pipeline; one owned server at a time, all replies validated. No concurrent task-local build/test ran during timing. Baseline and candidate use the same frozen client. Raw records retain batch tails and sampled CPU/accounting windows. One logical CPU does not establish physical-core isolation; host activity/frequency is uncontrolled. This is not either protected Valkey gate, a universal win, or scaling evidence.

Baseline: `f0d7d186f01e74323cb7dfee19a99829de13b3af` (clean diff). Candidate: base `f0d7d186f01e74323cb7dfee19a99829de13b3af` plus combined patch SHA256 `d43ebec74434757e858a49d76c468ed7e981de373ee551a5405dcd5a3e9e08d8`. See manifest and candidate.patch for tracked/untracked source attribution. Binary hashes are frozen in manifest.

| P | Workload | Before cmd/s median [min–max] | After cmd/s median [min–max] | Delta | Before/after batch p99 µs | Before/after CPU µs/cmd |
| --: | -- | --: | --: | --: | --: | --: |
| 1 | ping | 18473 [8070–18618] | 8012 [6696–26570] | -56.6% | 4232.6 / 9159.4 | 44.50 / 57.00 |
| 1 | set | 19647 [5749–29140] | 8633 [8224–13898] | -56.1% | 3603.7 / 8846.6 | 42.00 / 71.50 |
| 1 | get | 11423 [6128–22100] | 13300 [11647–15596] | +16.4% | 6015.7 / 5521.9 | 55.50 / 50.00 |
| 1 | set-nx-existing | 14238 [5032–21700] | 6268 [4737–7538] | -56.0% | 5352.2 / 11057.0 | 39.00 / 56.00 |
| 1 | set-xx-existing | 11124 [7949–22555] | 8680 [6256–25935] | -22.0% | 7704.3 / 8774.3 | 45.00 / 40.50 |
| 16 | ping | 104517 [100692–324862] | 244057 [60780–318937] | +133.5% | 9809.6 / 3904.6 | 3.20 / 3.40 |
| 16 | set | 173612 [111751–246779] | 140317 [69674–220656] | -19.2% | 5812.1 / 8457.0 | 3.60 / 3.90 |
| 16 | get | 130659 [84842–227672] | 102135 [58334–166894] | -21.8% | 10488.8 / 9139.5 | 4.00 / 5.50 |
| 16 | set-nx-existing | 212677 [70694–276229] | 125778 [92209–161777] | -40.9% | 5626.6 / 9759.9 | 4.00 / 5.10 |
| 16 | set-xx-existing | 121971 [85823–249440] | 92955 [90587–162063] | -23.8% | 8359.8 / 12027.0 | 4.20 / 5.10 |

Whole-batch p99 is not per-command latency. CPU is tick-quantized, not an energy measure. No rows are omitted, including losses. External/multi-worker compatibility mode is outside this local-path cohort and pays synchronous routing/copy costs; release CI smoke/core evidence is retained separately, not compared to this cohort.
