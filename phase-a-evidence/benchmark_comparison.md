# Benchmark comparison

The correction produced five baseline and five candidate JSON runs with
`RT_HANDOFF_BENCH_ITERATIONS=5000`, `RT_HANDOFF_BENCH_SCOPE=phase_a_sender_only`,
and `RT_HANDOFF_BENCH_MODE=phase_a_sender_only`.

The scope is intentionally narrow and explicit: a prepared packet is sent
through the tracked-state sender seam, with the baseline worktree retaining
the legacy start-timestamp sender and the candidate using the fused
target-aware sender. Waiter/coordinator scheduling is excluded. The default
full benchmark and real-wait mode remain available for separate Windows
review; these artifacts do not claim OS waiter or game-observed latency.

Raw artifacts:

- `baseline_bench_run_01.json` … `baseline_bench_run_05.json`
- `candidate_bench_run_01.json` … `candidate_bench_run_05.json`

Every artifact has `iterations=5000`, matching scope/mode, three scenarios
(`down_only_15`, `up_only_15`, `mixed_2`), `non_dispatches=0`, and
`early_dispatch_count=0`.

Across all five runs and scenarios, `dispatch_start_error_us` has p99/max 0
for both baseline and candidate. `pre_call_to_completion_us` and
`target_to_completion_us` have p99 0 in every run. Their observed maxima were:

| scenario | baseline max list | candidate max list |
|---|---:|---:|
| down_only_15 | 2, 1, 1, 1, 10 µs | 3, 2, 1, 1, 1 µs |
| up_only_15 | 0, 17, 0, 0, 0 µs | 0, 0, 0, 0, 0 µs |
| mixed_2 | 0, 0, 0, 0, 0 µs | 0, 0, 0, 0, 0 µs |

The completion tail shows no material regression in this sender-only A/B
scope. Human real-Windows evidence remains `AWAITING_HUMAN_REVIEW`.
