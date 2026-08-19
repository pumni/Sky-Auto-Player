# Phase A evidence

Local acceptance evidence for the precision-sender Phase A correction pass.

Phase B calibration work is intentionally excluded. This directory records
only Phase A implementation, validation, and benchmark evidence.

The required raw benchmark contract is present as
`baseline_bench_run_01..05.json` and `candidate_bench_run_01..05.json`.
Their scope/mode fields identify the corrected production-boundary A/B:
coordinator dispatch/admission/commit is included, while only kernel waiter
scheduling is excluded by a test-only direct boundary. The sender-only mode is
supplemental and is not acceptance authority.
