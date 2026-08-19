# Phase A completion report

## Status

`CORRECTION EVIDENCE COMPLETE — HARD STOP ACTIVE`

## Scope

Phase A precision-sender fusion is implemented in six Rust files:

- `rust/crates/sky_dispatch_win32/src/input/packet.rs`
- `rust/crates/sky_dispatch_win32/src/input/tracked/packet_send.rs`
- `rust/crates/sky_dispatch_win32/src/input.rs`
- `rust/crates/sky_player_rs/src/engine/worker/dispatch/authored.rs`
- `rust/crates/sky_player_rs/src/engine/worker/dispatch/mod.rs`
- `rust/crates/sky_player_rs/src/engine/worker/dispatch/recovery.rs`

The target-crossing sample is fused into the trusted prepared sender. Down
cutoff strictness, equality, Up-only behavior, one-send behavior, and
completion timing are covered by tests. Calibration files, calibration
schema, calibration binary behavior, and Phase B code were not changed.

## Evidence

- QPC ordering: `qpc_read_proof.txt`
- Code path: `codepath_before.md`, `codepath_after.md`
- Validation: `gate_results.md`
- Targeted tests: `targeted_tests.txt`
- Rust workspace tests: `rust_workspace_tests.txt`
- Benchmark status: `benchmark_comparison.md`
- Raw A/B runs: `baseline_bench_run_01..05.json`, `candidate_bench_run_01..05.json`
- Baseline SHA: `base_sha.txt`
- Candidate SHA: `candidate_sha.txt`

Benchmark evidence is complete for the explicitly labelled Phase-A sender-only
A/B scope: five baseline and five candidate runs at 5,000 iterations each.
The full real-wait benchmark remains available but is not used to claim OS
waiter performance. Real Windows evidence is `AWAITING_HUMAN_REVIEW`.

The exact reviewed implementation candidate SHA is recorded in
`candidate_sha.txt`. The correction benchmark/test-support changes are still
uncommitted; no commit or push was performed.

## Security boundary

The implementation retains Windows `SendInput` as the only input simulation
mechanism. No game tampering, memory access, hooks, injection, or
anti-cheat-bypass behavior was added. The repository security audit passed.

## Hard stop

Phase B is not started. Continue only after the exact acceptance token is
received:

`PHASE_A_ACCEPTED: proceed to Phase B calibration vNext`
