# Rust-first Refactor CI Baseline

Date recorded: 2026-08-31
Repository baseline: `main@3998823b6a07c5b31f8457b64239b17b4afa5148`

This is the Phase 0 baseline for CI work. It records the successful-run
evidence available in the handoff pack and local gate timings before the cache
change. It is not a claim that one local machine represents GitHub runner
latency.

## Historical GitHub baseline

The handoff source ledger identifies successful main run `33327920828` and the
detailed packaged/validate evidence run `33326560838`. The reported durations
are approximate, as captured in the handoff pack:

| Measurement | Baseline |
| --- | ---: |
| Total CI wall time | ~14m42s |
| Validate critical path | ~14m17s |
| Packaged job | ~11m48s |
| Static job | ~1m08s |
| Rust cache state | not recorded in source evidence |

The expensive work includes native wheel compilation, overlapping Cargo
graphs, Python tests, Playwright setup, and exact portable packaging.

## Local pre-change evidence

Measured from the clean Windows checkout at the baseline SHA on 2026-08-31.
The commands used a writable temporary `UV_CACHE_DIR` because the machine's
default uv cache path is access-restricted.

| Gate | Result | Elapsed |
| --- | --- | ---: |
| `uv run python scripts/check.py static` | PASS | 39.63s |
| `uv run python scripts/check.py rust` | PASS | 127.19s |

The Rust run was a first local build of the current workspace and compiled the
existing `sky_player_rs`/PyO3 and Tauri shell graph. No GitHub cache hit/miss
was available locally.

## Collection rule for the next baseline

After this change lands, collect at least 10 representative warm-cache runs
for ordinary PR, desktop-heavy PR, and merged-main exact qualification. Record
per-job elapsed time, Rust cache hit/miss, artifact size, and the exact commit.
Report p50/p95 separately; do not infer a speedup from the local measurements
above alone.

Engineering targets from the migration plan are:

| Loop | p50 | p95 |
| --- | ---: | ---: |
| ordinary PR | ≤5m | ≤8m |
| desktop-heavy PR | ≤7m | ≤10m |
| main exact qualification | ≤10m | ≤15m |
