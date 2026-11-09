# Frozen C0 external requalification

- Disposition: **BLOCKED C0 qualification**.
- C0: `b52106b47e4163d0906abbaa59d343935f4fba42` tree `0e1933cc565ad8f9b42239312c996facf2a5126a`, `dist`, `real-input-acceptance`, binary SHA-256 `98017d8e20d938681ba43cc57f62cdb561248583fc52880628e3d36c12d6b569`.
- Original manifest SHA-256: `b9e51334b9169e039db945028bedf725909fb595342d277b41291cda06a1e544`.
- Cases: 21 PASS, 1 BLOCKED, 0 FAIL.
- `original_recorded_status` and `targeted_recorded_status` preserve the original verdicts; `qualification_status` is an external oracle result.
- Compare is not authorized unless the full C0 qualification completes with no blockers.
- Summary SHA-256: `cc0df53f37359e40d893ac195b28e921ecf8c459973c8cdeed6d27b0878f21cb`; oracle script SHA-256: `a34812fa96f594160828a1c281493e4b020e0bfe2cf239af3f5722bb27dfecc3`.

| Scenario | Original status | Targeted recorded status | Qualification | Basis |
|---|---:|---:|---:|---|
| canonical-single | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| canonical-max-chord | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| near-minimum-retrigger | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| release-gap-stress | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| mixed-up-down | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| ambiguous-packet | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| modifier-held-final-boundary | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| modifier-held-after-owned | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| focus-loss | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| target-hwnd-change | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| owner-mismatch | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| owner-query-failure | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| pause-resume | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| pause-resume-release-floor | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| stop-cleanup | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| skip-cleanup | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| supervisor-lease-expiry | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| noncanonical-profile | PASS | not rerun; original report/window reconciled | PASS | unchanged native report PASS, archived runner identity check, ReceiveOnly window reconciliation, and contiguous event stream |
| lease-age-watchdog-delayed | FAIL | FAIL | PASS | corrected external oracle: terminal supervisor_lease_expired with live helper, zero Down attempts, empty identity-bound ReceiveOnly window, zero obligations, and verified clean release outcome; original native FAIL is retained |
| preflight-user-held | INCONCLUSIVE | INCONCLUSIVE | PASS | external sink-window proof: startup preflight rejection before Armed/SendInput; original native telemetry-unavailable INCONCLUSIVE is retained; live sink identity is attested by the hashed archived runner and ready/config/event records |
| owner-process-termination | INCONCLUSIVE | INCONCLUSIVE | BLOCKED | BLOCKED: frozen C0 fixture only invalidated synthetic owner authority; its immutable executable has no OS process-termination fixture. No reclassification as PASS. |
| suspend-resume | INCONCLUSIVE | INCONCLUSIVE | PASS | terminal system_suspend event prefix contains the expected Down and safety Up on the identity-bound ReceiveOnly sink, with verified release and zero residual obligations; the old resume/full-song INCONCLUSIVE remains recorded |

