| Policy | Throughput median req/s | vs AOF-off | batch p99 median us | PING p99 before ms | PING p99 during rewrite ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| baseline | 214430.36 | 1.000 | 51.32 | 0.55 | 0.557 |
| no | 32272.06 | 0.151 | 307.918 | 0.548 | 0.578 |
| everysec | 31440.09 | 0.147 | 313.653 | 0.615 | 0.544 |
| always | 7509.57 | 0.035 | 1396.206 | 0.621 | 0.496 |
