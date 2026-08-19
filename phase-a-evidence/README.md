# Phase A evidence

Local acceptance evidence for the precision-sender Phase A correction pass.

Phase B calibration work is intentionally excluded. This directory records
only Phase A implementation, validation, and benchmark evidence.

The required raw benchmark contract is present as
`baseline_bench_run_01..05.json` and `candidate_bench_run_01..05.json`.
Their scope/mode fields are explicit because the sender-only A/B excludes
waiter/coordinator scheduling; the full real-wait benchmark remains the
default harness mode.
