# Dispatch Core qualification Compare  **Comparison: pinned base vs Scheduler candidate.** - Run: `dispatch-core-compare-20261112T141248-71a8f15e` - Source: `c48cc02d93204980af6a4d8e7b7dc48ec078f32e` tree `710d76b5c841f7a9e38d0c272271c7db2794680d` (clean) - Candidate: `c48cc02d93204980af6a4d8e7b7dc48ec078f32e` / `dist` / SHA-256 `a3f951e42408029cbf30174204a62eb1adeef392f1f96e2661c49052d45d8b46` - Control: C0; `b9ed666e3ddddc4952831626f1654e1cc9071a10` / `dist` / SHA-256 `4bf477726ad738763c6deef95455812f21759824f260bc33afa127248899a611` - Sessions: 49; PASS: 48; failed/inconclusive: 1 - Disposition: STOPPED_AT_FIRST_NATIVE_FAILURE - Paired order: AB/BA alternating; six pairs per workload/load cell; 15 seconds per timed session. - Contention: two hidden PowerShell CPU workers at BelowNormal priority. Sink: identity-verified ReceiveOnly. - Percentiles use nearest-rank ceiling. No p99.9 is inferred. - Reported: per-chord authored lateness, floor displacement, residual lateness, waiter-to-pre-call, per-syscall sender envelope, per-key realized H/G, Down/Up/Mixed, chord width and authored hot/cold gap groups. - Timer target uses the arm QPC sample plus the relative timeout passed to the timer API; timer return is paired with the arm generation. Missing and negative-order samples remain counted. - Tail screen: candidate p95 worse than max(5 us, 10% control) or p99 worse than max(20 us, 10% control) in >=4/6 pairs in a cell prompts leader review; outliers are retained. - Native RT allocation is not instrumented in the sink runner; see focused no-allocation test result in the accompanying command record.  ## Safety cases  | Scenario | Verdict | Exit | Reason | |---|---:|---:|---|

## Paired performance observations

| Cell | Metric | Available pairs | p95 favorable/unfavorable | p99 favorable/unfavorable | p95/p99 threshold pairs | Leader review prompt |
|---|---|---:|---:|---:|---:|---:|
| dense-alternating/Contention | metrics.authored_lateness_per_chord_us | 6/6 | 1/5 | 1/5 | 2/4 | True |
| dense-alternating/Contention | metrics.floor_displacement_us | 6/6 | 1/5 | 1/5 | 2/4 | True |
| dense-alternating/Contention | metrics.realized_gap_G_us | 6/6 | 2/4 | 3/3 | 0/0 | False |
| dense-alternating/Contention | metrics.realized_hold_H_us | 6/6 | 5/1 | 3/3 | 0/0 | False |
| dense-alternating/Contention | metrics.residual_lateness_us | 6/6 | 1/5 | 3/3 | 5/3 | True |
| dense-alternating/Contention | metrics.sender_envelope_per_syscall_us | 6/6 | 1/5 | 2/4 | 2/3 | False |
| dense-alternating/Contention | metrics.timer_arm_to_return_us | 6/6 | 2/4 | 2/4 | 0/0 | False |
| dense-alternating/Contention | metrics.timer_wake_residual_us | 6/6 | 3/3 | 2/4 | 0/2 | False |
| dense-alternating/Contention | metrics.waiter_to_pre_call_us | 0/6 | 0/0 | 0/0 | 0/0 |  |
| dense-alternating/Contention | per_kind.down_only.authored_lateness_per_chord_us | 6/6 | 1/5 | 2/4 | 5/4 | True |
| dense-alternating/Contention | per_kind.down_only.residual_lateness_us | 6/6 | 1/5 | 2/4 | 5/4 | True |
| dense-alternating/Contention | per_kind.down_only.sender_envelope_per_syscall_us | 6/6 | 2/4 | 1/5 | 2/4 | True |
| dense-alternating/Contention | per_kind.mixed.authored_lateness_per_chord_us | 0/6 | 0/0 | 0/0 | 0/0 |  |
| dense-alternating/Contention | per_kind.mixed.residual_lateness_us | 0/6 | 0/0 | 0/0 | 0/0 |  |
| dense-alternating/Contention | per_kind.mixed.sender_envelope_per_syscall_us | 0/6 | 0/0 | 0/0 | 0/0 |  |
| dense-alternating/Contention | per_kind.up_only.authored_lateness_per_chord_us | 6/6 | 1/5 | 1/5 | 3/5 | True |
| dense-alternating/Contention | per_kind.up_only.residual_lateness_us | 6/6 | 5/1 | 3/3 | 1/2 | False |
| dense-alternating/Contention | per_kind.up_only.sender_envelope_per_syscall_us | 6/6 | 2/4 | 2/4 | 3/3 | False |
| dense-alternating/Quiet | metrics.authored_lateness_per_chord_us | 6/6 | 2/4 | 2/4 | 2/2 | False |
| dense-alternating/Quiet | metrics.floor_displacement_us | 6/6 | 2/4 | 3/3 | 2/1 | False |
| dense-alternating/Quiet | metrics.realized_gap_G_us | 6/6 | 2/4 | 4/2 | 0/0 | False |
| dense-alternating/Quiet | metrics.realized_hold_H_us | 6/6 | 4/2 | 3/3 | 0/0 | False |
| dense-alternating/Quiet | metrics.residual_lateness_us | 6/6 | 3/3 | 2/4 | 3/2 | False |
| dense-alternating/Quiet | metrics.sender_envelope_per_syscall_us | 6/6 | 1/5 | 3/3 | 2/1 | False |
| dense-alternating/Quiet | metrics.timer_arm_to_return_us | 6/6 | 3/3 | 2/4 | 0/0 | False |
| dense-alternating/Quiet | metrics.timer_wake_residual_us | 6/6 | 2/4 | 2/4 | 0/1 | False |
| dense-alternating/Quiet | metrics.waiter_to_pre_call_us | 0/6 | 0/0 | 0/0 | 0/0 |  |
| dense-alternating/Quiet | per_kind.down_only.authored_lateness_per_chord_us | 6/6 | 2/4 | 3/3 | 4/2 | True |
| dense-alternating/Quiet | per_kind.down_only.residual_lateness_us | 6/6 | 2/4 | 3/3 | 4/2 | True |
| dense-alternating/Quiet | per_kind.down_only.sender_envelope_per_syscall_us | 6/6 | 3/3 | 1/5 | 2/2 | False |
| dense-alternating/Quiet | per_kind.mixed.authored_lateness_per_chord_us | 0/6 | 0/0 | 0/0 | 0/0 |  |
| dense-alternating/Quiet | per_kind.mixed.residual_lateness_us | 0/6 | 0/0 | 0/0 | 0/0 |  |
| dense-alternating/Quiet | per_kind.mixed.sender_envelope_per_syscall_us | 0/6 | 0/0 | 0/0 | 0/0 |  |
| dense-alternating/Quiet | per_kind.up_only.authored_lateness_per_chord_us | 6/6 | 3/3 | 3/3 | 3/2 | False |
| dense-alternating/Quiet | per_kind.up_only.residual_lateness_us | 6/6 | 4/2 | 3/3 | 1/3 | False |
| dense-alternating/Quiet | per_kind.up_only.sender_envelope_per_syscall_us | 6/6 | 3/3 | 6/0 | 1/0 | False |
| mixed-up-down/Contention | metrics.authored_lateness_per_chord_us | 6/6 | 6/0 | 5/1 | 0/1 | False |
| mixed-up-down/Contention | metrics.floor_displacement_us | 6/6 | 6/0 | 6/0 | 0/0 | False |
| mixed-up-down/Contention | metrics.realized_gap_G_us | 6/6 | 4/2 | 5/1 | 0/0 | False |
| mixed-up-down/Contention | metrics.realized_hold_H_us | 6/6 | 3/3 | 5/1 | 0/0 | False |
| mixed-up-down/Contention | metrics.residual_lateness_us | 6/6 | 6/0 | 5/1 | 0/1 | False |
| mixed-up-down/Contention | metrics.sender_envelope_per_syscall_us | 6/6 | 2/4 | 3/3 | 0/2 | False |
| mixed-up-down/Contention | metrics.timer_arm_to_return_us | 6/6 | 4/2 | 4/2 | 0/0 | False |
| mixed-up-down/Contention | metrics.timer_wake_residual_us | 6/6 | 3/3 | 2/4 | 0/3 | False |
| mixed-up-down/Contention | metrics.waiter_to_pre_call_us | 0/6 | 0/0 | 0/0 | 0/0 |  |
| mixed-up-down/Contention | per_kind.down_only.authored_lateness_per_chord_us | 6/6 | 6/0 | 4/2 | 0/1 | False |
| mixed-up-down/Contention | per_kind.down_only.residual_lateness_us | 6/6 | 6/0 | 4/2 | 0/1 | False |
| mixed-up-down/Contention | per_kind.down_only.sender_envelope_per_syscall_us | 6/6 | 6/0 | 2/4 | 0/3 | False |
| mixed-up-down/Contention | per_kind.mixed.authored_lateness_per_chord_us | 6/6 | 4/2 | 5/1 | 1/1 | False |
| mixed-up-down/Contention | per_kind.mixed.residual_lateness_us | 6/6 | 4/2 | 5/1 | 1/1 | False |
| mixed-up-down/Contention | per_kind.mixed.sender_envelope_per_syscall_us | 6/6 | 2/4 | 2/4 | 1/3 | False |
| mixed-up-down/Contention | per_kind.up_only.authored_lateness_per_chord_us | 6/6 | 6/0 | 4/2 | 0/2 | False |
| mixed-up-down/Contention | per_kind.up_only.residual_lateness_us | 6/6 | 6/0 | 4/2 | 0/2 | False |
| mixed-up-down/Contention | per_kind.up_only.sender_envelope_per_syscall_us | 6/6 | 3/3 | 1/5 | 1/2 | False |
| mixed-up-down/Quiet | metrics.authored_lateness_per_chord_us | 6/6 | 6/0 | 3/3 | 0/3 | False |
| mixed-up-down/Quiet | metrics.floor_displacement_us | 6/6 | 6/0 | 6/0 | 0/0 | False |
| mixed-up-down/Quiet | metrics.realized_gap_G_us | 6/6 | 4/2 | 3/3 | 0/0 | False |
| mixed-up-down/Quiet | metrics.realized_hold_H_us | 6/6 | 5/1 | 4/2 | 0/0 | False |
| mixed-up-down/Quiet | metrics.residual_lateness_us | 6/6 | 6/0 | 3/3 | 0/3 | False |
| mixed-up-down/Quiet | metrics.sender_envelope_per_syscall_us | 6/6 | 6/0 | 1/5 | 0/1 | False |
| mixed-up-down/Quiet | metrics.timer_arm_to_return_us | 6/6 | 2/4 | 3/3 | 0/0 | False |
| mixed-up-down/Quiet | metrics.timer_wake_residual_us | 6/6 | 4/2 | 3/3 | 0/1 | False |
| mixed-up-down/Quiet | metrics.waiter_to_pre_call_us | 0/6 | 0/0 | 0/0 | 0/0 |  |
| mixed-up-down/Quiet | per_kind.down_only.authored_lateness_per_chord_us | 6/6 | 4/2 | 4/2 | 2/1 | False |
| mixed-up-down/Quiet | per_kind.down_only.residual_lateness_us | 6/6 | 4/2 | 4/2 | 2/1 | False |
| mixed-up-down/Quiet | per_kind.down_only.sender_envelope_per_syscall_us | 6/6 | 4/2 | 2/4 | 1/3 | False |
| mixed-up-down/Quiet | per_kind.mixed.authored_lateness_per_chord_us | 6/6 | 4/2 | 3/3 | 2/3 | False |
| mixed-up-down/Quiet | per_kind.mixed.residual_lateness_us | 6/6 | 4/2 | 3/3 | 2/3 | False |
| mixed-up-down/Quiet | per_kind.mixed.sender_envelope_per_syscall_us | 6/6 | 2/4 | 4/2 | 1/2 | False |
| mixed-up-down/Quiet | per_kind.up_only.authored_lateness_per_chord_us | 6/6 | 4/2 | 3/3 | 0/3 | False |
| mixed-up-down/Quiet | per_kind.up_only.residual_lateness_us | 6/6 | 4/2 | 3/3 | 0/3 | False |
| mixed-up-down/Quiet | per_kind.up_only.sender_envelope_per_syscall_us | 6/6 | 5/1 | 3/3 | 0/3 | False |

These are evidence summaries, not an automatic performance-gate verdict. No design, gate, or rollout selection is made by this runner.
