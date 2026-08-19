# Phase A completion report

## Status

`CORRECTION EVIDENCE COMPLETE — AWAITING HUMAN ACCEPTANCE`

Phase B is still locked. No cleanup has been executed.

## Implementation

The target-crossing sample is fused into the trusted prepared sender. Down
cutoff strictness, equality, Up-only behavior, one-send behavior, completion
timing, and the direct in-grace crossing case are covered by tests.
Calibration files, calibration schema, calibration binary behavior, and Phase
B code were not changed.

## Acceptance evidence

- QPC ordering: `qpc_read_proof.txt`
- Code path: `codepath_before.md`, `codepath_after.md`
- Validation: `gate_results.md`
- Targeted tests: `targeted_tests.txt`
- Rust workspace tests: `rust_workspace_tests.txt`
- Benchmark authority and comparison: `benchmark_comparison.md`
- Raw A/B runs: `baseline_bench_run_01..05.json`, `candidate_bench_run_01..05.json`
- Baseline SHA: `base_sha.txt`
- Exact implementation/evidence provenance: `candidate_sha.txt`

The benchmark authority uses the full coordinator dispatch/admission/commit
path with `test_started_ticks=None` at the sender boundary. It covers all nine
required scenarios and five 5,000-iteration runs per side. Deadline misses and
other non-dispatches are recorded in JSON instead of aborting the run.

## Security boundary

The implementation retains Windows `SendInput` as the only input simulation
mechanism. No game tampering, memory access, hooks, injection, or
anti-cheat-bypass behavior was added. The repository security audit remains
required and is recorded in `gate_results.md`.

## Hard stop

Phase B must not start until the exact acceptance token is received:

`PHASE_A_ACCEPTED: proceed to Phase B calibration vNext`
