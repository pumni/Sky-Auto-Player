# Dispatch Core qualification Smoke  - Run: `dispatch-core-smoke-20261109T191101-586c70d5` - Source: `4b1cec7a874ace7030e1b5e47aeccd315f3d9fe4` tree `bf2a1d9c34a55d4885fbde53a661d152b0517bb7` (clean) - Candidate: `4b1cec7a874ace7030e1b5e47aeccd315f3d9fe4` / `dist` / SHA-256 `5cddfc1f32bd3e0b8d3ccb7ea2803f6c34aba929847c6abfcf0c86af7b28e7ae` - Control: not applicable - Sessions: 22; PASS: 22; failed/inconclusive: 0 - Paired order: AB/BA alternating; six pairs per workload/load cell; 15 seconds per timed session. - Contention: two hidden PowerShell CPU workers at BelowNormal priority. Sink: identity-verified ReceiveOnly. - Percentiles use nearest-rank ceiling. No p99.9 is inferred. - Reported: per-chord authored lateness, floor displacement, residual lateness, waiter-to-pre-call, per-syscall sender envelope, per-key realized H/G, Down/Up/Mixed, chord width and authored hot/cold gap groups. - Timer target uses the arm QPC sample plus the relative timeout passed to the timer API; timer return is paired with the arm generation. Missing and negative-order samples remain counted. - Tail screen: candidate p95 worse than max(5 us, 10% control) or p99 worse than max(20 us, 10% control) in >=4/6 pairs in a cell prompts leader review; outliers are retained. - Native RT allocation is not instrumented in the sink runner; see focused no-allocation test result in the accompanying command record.  ## Safety cases  | Scenario | Verdict | Exit | Reason | |---|---:|---:|---|
| canonical-single | PASS | 0 | controlled production-path evidence is clean |
| canonical-max-chord | PASS | 0 | controlled production-path evidence is clean |
| near-minimum-retrigger | PASS | 0 | controlled production-path evidence is clean |
| release-gap-stress | PASS | 0 | controlled production-path evidence is clean |
| mixed-up-down | PASS | 0 | controlled production-path evidence is clean |
| ambiguous-packet | PASS | 0 | controlled production-path evidence is clean |
| preflight-user-held | PASS | 0 | pre-arm UserHeld preflight rejected the session before any key event or cleanup transport |
| modifier-held-final-boundary | PASS | 0 | modifier guard final-boundary evidence passed |
| modifier-held-after-owned | PASS | 0 | modifier guard final-boundary evidence passed |
| focus-loss | PASS | 0 | controlled production-path evidence is clean |
| target-hwnd-change | PASS | 0 | controlled production-path evidence is clean |
| owner-mismatch | PASS | 0 | window-owner authority failure stopped before the prepared sender |
| owner-query-failure | PASS | 0 | window-owner authority failure stopped before the prepared sender |
| owner-process-termination | PASS | 0 | identity-verified ReceiveOnly owner process termination destroyed the foreground window; preroll failed closed before the sender |
| pause-resume | PASS | 0 | controlled production-path evidence is clean |
| pause-resume-release-floor | PASS | 0 | controlled production-path evidence is clean |
| suspend-resume | PASS | 0 | controlled production-path evidence is clean |
| stop-cleanup | PASS | 0 | controlled production-path evidence is clean |
| skip-cleanup | PASS | 0 | controlled production-path evidence is clean |
| supervisor-lease-expiry | PASS | 0 | session watchdog expired after session progress stopped while the unrelated helper remained alive |
| lease-age-watchdog-delayed | PASS | 0 | expired supervisor lease caused the expected terminal block at final Down admission before the sender while the helper remained alive |
| noncanonical-profile | PASS | 0 | controlled production-path evidence is clean |

## Paired performance observations

| Cell | Metric | Available pairs | p95 favorable/unfavorable | p99 favorable/unfavorable | p95/p99 threshold pairs | Leader review prompt |
|---|---|---:|---:|---:|---:|---:|

These are evidence summaries, not an automatic performance-gate verdict. No design, gate, or rollout selection is made by this runner.

