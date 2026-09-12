# Sender visibility study — 2026-09-13

Seven independent physical cases ran on an interactive Windows desktop using
the release `rt-native-acceptance.exe` and a fresh project-owned `ReceiveOnly`
sink per invocation. No case targeted Sky. The four cutoff cases ran on clean
head `f8c0921a01303274c862d126187874874ed4ffcc`; the three margin stress cases
ran on clean head `80b444908a6be2553f6ddbb9e15f388962cbb78c`. The second commit
only relaxed the runner's pre-arm check to preserve and exclude valid
pre-arm events by cursor; it did not change the harness or production code.
All seven used the same release executable SHA-256
`01f34464ce84628255ae9d8c382be581a954eef78aa32e7dc332df1ecc60e362`. Each
`run-config.json` records its exact source revision and cleanliness, executable
hash, sink HWND/PID/event-log ID, and runner SHA-256. All artifact checksums were
independently verified.

## Cutoff sweep

Timing Margin stayed at 800 µs. Every case reported the same authored timing:

```text
Hold        17,467 µs
Release Gap 17,467 µs
Targets     50,000 → 67,467 → 84,934 → 102,401 µs
```

Each sink window had exactly four events with continuous sequence numbers,
matching the report and expected Down/Up/Down/Up directions. The brief sweep
collected only two completion-hold pairs and one release-gap observation per
case; it is not enough to compare cutoff reliability.

| Late Down tolerance | Harness status | Hold min / below-frame | Release-gap min / below-frame | Max pre-call lateness |
| ---: | --- | ---: | ---: | ---: |
| 500 µs | PASS | 17.2973 ms / 0 of 2 | 17.2100 ms / 0 of 1 | 21 µs |
| 1,000 µs | PASS | 17.4031 ms / 0 of 2 | 17.1627 ms / 0 of 1 | 26 µs |
| 2,000 µs | FAIL | 15.2476 ms / 1 of 2 | 18.7354 ms / 0 of 1 | 1,617 µs |
| 5,000 µs | PASS | 16.8348 ms / 0 of 2 | 17.1124 ms / 0 of 1 | 17 µs |

All four had zero cutoff misses, hard-late misses, partial or zero-progress
SendInput calls. The 2,000 µs case failed because its completion-hold sample
fell below the fixed 16.667 ms frame floor. The sparse observations do not
establish that the selected cutoff caused or prevented that event.

## Timing Margin stress sweep

Late Down tolerance stayed at 500 µs. Each run produced 513 completion-hold
samples and 512 release-gap samples, with exactly 1,026 expected and observed
sink events in a cursor-bounded contiguous window.

| Timing Margin | Harness status | Completion holds below frame | Minimum completion hold | Release gaps below frame | Minimum release gap | Max pre-call lateness |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 800 µs | FAIL | 4 / 513 | 16.3703 ms | 14 / 512 | 15.8425 ms | 168 µs |
| 1,200 µs | FAIL | 0 / 513 | 17.1782 ms | 2 / 512 | 16.1851 ms | 150 µs |
| 1,500 µs | PASS | 0 / 513 | 17.2406 ms | 0 / 512 | 16.9542 ms | 344 µs |

All three had zero cutoff misses, hard-late misses, partial or zero-progress
SendInput calls, dropped/stuck keys, and focus losses. The 800 µs run had two
pre-arm Backspace records in its full sink log; its recorded cursor was sequence
2, so the tested window began at sequence 3 and matched all 1,026 expected
events. Their source is unknown and they are excluded from the sender window.

The 1,500 µs result clears the current 512-sample qualification for this single
controlled workload. It does not guarantee all future runs, justify changing
the product default, or prove that Sky consumes the same packets reliably. No
production scheduling, SendInput batching, chord handling, or scan-code behavior
was changed.

## Raw evidence

Each `native-case-*` directory retains its report, complete sink event log,
pre-arm cursor, cursor-bounded raw event window, run configuration, invocation
record, and executed runner copy. `SHA256SUMS.txt` uses the original captured
bytes; the study directory `.gitattributes` disables line-ending conversion so
those checksums remain valid in every checkout.
