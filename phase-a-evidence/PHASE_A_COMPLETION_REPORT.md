# Phase A completion report

## Status

`IMPLEMENTED — HARD STOP ACTIVE`

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
- Benchmark status: `benchmark_comparison.md`
- Baseline SHA: `base_sha.txt`
- Candidate SHA: `candidate_sha.txt`

Benchmark status is `INCONCLUSIVE` because both prescribed runs aborted before
producing JSON. Real Windows evidence is `AWAITING_HUMAN_REVIEW`.

## Security boundary

The implementation retains Windows `SendInput` as the only input simulation
mechanism. No game tampering, memory access, hooks, injection, or
anti-cheat-bypass behavior was added. The repository security audit passed.

## Hard stop

Phase B is not started. Continue only after the exact acceptance token is
received:

`PHASE_A_ACCEPTED: proceed to Phase B calibration vNext`
