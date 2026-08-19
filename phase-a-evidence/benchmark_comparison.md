# Phase A benchmark comparison

Status: corrected automated evidence complete; human acceptance is still
required. Phase B remains locked.

## Authority and command

The acceptance authority is the full coordinator dispatch/admission/commit
path at the production sender boundary. It uses:

```text
RT_HANDOFF_BENCH_ITERATIONS=5000
RT_HANDOFF_BENCH_SCOPE=phase_a_production_matrix
RT_HANDOFF_BENCH_MODE=phase_a_production_boundary
```

The measured dispatch calls the same production worker path on both worktrees.
The candidate receives `test_started_ticks=None`, so its fused sender owns the
target-crossing QPC loop. The baseline uses the legacy sender. The test-only
direct boundary removes kernel-wait variability; it does not inject the
sender start timestamp. The mock emitter samples completion QPC after the
sender boundary. Production `700 us` spin, `500 us` Down grace, MMCSS, and
wait policy were not changed.

Each run contains all nine required scenarios:

```text
down_only_1, down_only_5, down_only_15
up_only_1, up_only_5, up_only_15
mixed_2, mixed_10, mixed_30
```

Raw artifacts are `baseline_bench_run_01..05.json` and
`candidate_bench_run_01..05.json`. Every artifact reports the production
scope/mode and 5,000 iterations per scenario.

## Result summary

The table shows the mean of the five per-run summaries. Each cell is
`baseline / candidate` in microseconds. Percentiles are calculated inside each
5,000-sample scenario run before averaging across the five runs.

| scenario | metric | p50 | p95 | p99 | p99.9 | max |
|---|---|---:|---:|---:|---:|---:|
| down_only_1 | dispatch_start_error | 109.6 / 108.4 | 113.6 / 112.6 | 121.8 / 121.6 | 173.6 / 201.8 | 276.6 / 328.4 |
|  | pre_call_to_completion | 0 / 0 | 0 / 0 | 0 / 0 | 1.0 / 1.6 | 12.8 / 34.2 |
|  | target_to_completion | 109.6 / 108.6 | 113.8 / 112.8 | 122.4 / 122.6 | 174.8 / 202.0 | 278.8 / 329.4 |
| down_only_5 | dispatch_start_error | 110.4 / 109.2 | 115.0 / 113.0 | 121.6 / 120.0 | 168.4 / 181.0 | 261.6 / 278.6 |
|  | pre_call_to_completion | 0 / 0 | 0 / 0 | 0 / 0 | 0.6 / 0 | 5.2 / 4.8 |
|  | target_to_completion | 110.4 / 110.0 | 115.4 / 113.4 | 122.0 / 120.4 | 169.0 / 181.4 | 262.0 / 278.8 |
| down_only_15 | dispatch_start_error | 112.6 / 112.0 | 118.2 / 117.2 | 126.4 / 123.8 | 165.2 / 193.4 | 226.8 / 335.4 |
|  | pre_call_to_completion | 0 / 0 | 0 / 0 | 0 / 0 | 0.8 / 0 | 68.6 / 83.0 |
|  | target_to_completion | 113.2 / 112.2 | 118.2 / 117.6 | 126.8 / 124.2 | 166.0 / 195.2 | 243.4 / 359.4 |
| up_only_1 | dispatch_start_error | 105.8 / 106.0 | 107.2 / 107.6 | 111.2 / 110.8 | 125.8 / 129.6 | 229.2 / 214.4 |
|  | pre_call_to_completion | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 4.8 / 4.6 |
|  | target_to_completion | 106.0 / 106.0 | 107.2 / 107.8 | 111.2 / 110.8 | 126.6 / 129.8 | 229.8 / 214.6 |
| up_only_5 | dispatch_start_error | 107.0 / 107.0 | 108.6 / 109.2 | 111.6 / 112.4 | 126.4 / 133.6 | 158.8 / 258.8 |
|  | pre_call_to_completion | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 12.8 / 1.8 |
|  | target_to_completion | 107.0 / 107.0 | 108.8 / 109.4 | 112.0 / 113.0 | 127.8 / 134.0 | 159.4 / 259.2 |
| up_only_15 | dispatch_start_error | 109.0 / 110.0 | 112.4 / 114.0 | 116.2 / 119.6 | 136.6 / 149.2 | 171.2 / 298.6 |
|  | pre_call_to_completion | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 4.8 / 0.4 |
|  | target_to_completion | 109.4 / 110.2 | 112.4 / 114.0 | 116.6 / 119.8 | 137.0 / 149.8 | 172.4 / 299.2 |
| mixed_2 | dispatch_start_error | 106.2 / 106.8 | 108.0 / 107.8 | 111.8 / 111.4 | 121.8 / 128.8 | 175.4 / 174.0 |
|  | pre_call_to_completion | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 1.4 / 2.2 |
|  | target_to_completion | 106.8 / 107.0 | 108.4 / 108.2 | 111.8 / 112.0 | 122.4 / 129.2 | 176.0 / 174.0 |
| mixed_10 | dispatch_start_error | 109.0 / 109.0 | 110.6 / 112.4 | 114.2 / 117.2 | 126.0 / 149.4 | 157.6 / 226.4 |
|  | pre_call_to_completion | 0 / 0 | 0 / 0 | 0 / 0 | 0.2 / 0.2 | 1.2 / 4.6 |
|  | target_to_completion | 109.0 / 109.0 | 110.6 / 112.4 | 114.6 / 117.4 | 126.0 / 150.0 | 157.8 / 226.8 |
| mixed_30 | dispatch_start_error | 115.2 / 114.8 | 121.2 / 121.2 | 133.4 / 128.8 | 210.6 / 165.4 | 300.6 / 265.0 |
|  | pre_call_to_completion | 0 / 0 | 0 / 0 | 0 / 0 | 0.2 / 0 | 1.4 / 3.2 |
|  | target_to_completion | 115.2 / 114.8 | 121.4 / 121.2 | 133.8 / 129.0 | 211.0 / 165.8 | 301.4 / 265.2 |

The central p50/p95 distributions are close, and candidate completion tails are
in the same sub-millisecond range. The p99.9/max columns contain ordinary
host-QPC outliers; they must remain visible for human review rather than being
discarded. No run reported early dispatches or
`deadline_missed_before_send_count`. Baseline run 03 recorded ten
`mixed_30` `missed Down safety Up missing start boundary` non-dispatches;
candidate recorded zero non-dispatches. All other baseline/candidate runs had
zero non-dispatches.

This packet supplies automated A/B evidence for the corrected authority. It
does not claim Raw Input or game-observed latency, and real Windows evidence
remains `AWAITING_HUMAN_REVIEW`.
