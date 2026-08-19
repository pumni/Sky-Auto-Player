# Benchmark comparison

The prescribed benchmark was run with `RT_HANDOFF_BENCH_ITERATIONS=5000`.

Baseline command:

```text
RT_HANDOFF_BENCH_ITERATIONS=5000 cargo run --manifest-path rust/Cargo.toml -p sky_player_rs --example rt_handoff_bench --features test-support --release
```

Result: `INCONCLUSIVE`. The baseline aborted before emitting JSON at
`down_hard_late_abort`.

Candidate command:

```text
RT_HANDOFF_BENCH_ITERATIONS=5000 cargo run --manifest-path rust/Cargo.toml -p sky_player_rs --example rt_handoff_bench --features test-support --release phase-a-evidence/candidate_bench_run_01.json
```

Result: `INCONCLUSIVE`. The candidate aborted before emitting JSON at
`down_deadline_missed_before_send`. No timing numbers are claimed and no
baseline/candidate JSON comparison is fabricated.

Human Windows evidence remains `AWAITING_HUMAN_REVIEW`.
