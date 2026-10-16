# ADR-0018: Resumable Release Barrier and Final Lease Age

- Status: Accepted
- Date: 2026-10-16
- Supplements: [ADR-0017](ADR-0017-current-down-continuity-and-physical-min-hold.md)
  for resumable lifecycle and final supervisor-lease admission only.
- Tracking: [parent decisions #430](https://github.com/pumni/Sky-Auto-Player/issues/430),
  [P0 #431](https://github.com/pumni/Sky-Auto-Player/issues/431),
  [P1 #432](https://github.com/pumni/Sky-Auto-Player/issues/432),
  [P2 #433](https://github.com/pumni/Sky-Auto-Player/issues/433),
  [documentation phase #434](https://github.com/pumni/Sky-Auto-Player/issues/434).

## Scope and relationship to ADR-0017

ADR-0017 remains authoritative for the materialized hold floor, physical and
authored release gaps, normal current-Down continuity, packet order, musical
ownership, safety release accounting, and the prohibition on schedule rewrite.
This ADR records the accepted D4 resumable Down barrier, D5 bounded final
lease-age proof, D6 focus-loss behavior, and the platform/evidence boundaries
needed to describe them accurately. It does not authorize a runtime behavior
change or replace any ADR-0017 decision.

The P3 change is documentation-only. Current implementation and direct tests
remain the executable authority. P0 is linked as historical characterization;
the accepted P1 and P2 source heads and evidence support the current lifecycle
and lease claims.

## Resumable lifecycle release barrier

After a verified resumable cleanup call returns, the worker takes a fresh QPC
sample `Q` and checked-adds one frame to establish the absolute global Down
barrier:

```text
B = Q + frame_ticks
Down packet wait target = max(authored target, per-key floors, B)
```

The post-cleanup QPC sample is strictly after cleanup returns. The barrier
applies to every subsequent Down-bearing packet, including all Downs in an
atomic mixed packet. It is global and conservative: a completed Down for one
key does not clear it for other keys. Authored targets later than `B` are
preserved. UpOnly packets bypass the barrier and retain their safety timing.

Manual pause, focus restoration, deferred manual suspension, and system-power
resume use the same verified cleanup operation and preserve the barrier. That
cleanup clears canceled musical per-key floors; it does not reset `B`. A new
session may construct a fresh guard. Cleanup that is unknown, partial, or
unverified, a failed post-cleanup QPC read, or checked-add overflow leaves
Down dispatch fail-closed. No usable timestamp is fabricated for an unknown
release state.

### Focus loss and physical release

Focus loss immediately blocks new Downs and pauses musical time. Resumable
release and reconciliation may remain deferred until the exact target is
trustworthily restored or existing terminal cleanup runs. This policy does
not route global Ups to an unfocused window and does not claim that keys were
physically released during a long unfocused interval. Once verified cleanup
completes, the fresh-QPC one-frame barrier above governs resumed Down traffic.

Safety and cleanup Ups bypass the musical hold floor `H`. Musical generation
accounting remains separate from physical release obligations. The tracked
obligation mask is:

```text
release_obligation_mask
= active_mask | possibly_active_mask | failed_release_mask | in_flight_mask
```

An idempotent safety Up need not create a musical `Released` transition.
Physical Up counts can therefore differ from musical Down/Up counts in
abnormal or cleanup sessions. Partial, ambiguous, and clock-uncertain Downs
retain their conservative obligations and are never replayed.

## Bounded final supervisor-lease proof

The watchdog remains enabled and unchanged. Final admission independently
proves lease age for eligible Down-bearing traffic after modifier observation
and final target atomic revalidation, before the last control check and sender
handoff. The proof remains in `sky_player` admission, outside the Win32
`SendInput` sender, and reuses timeout ticks materialized at worker boot.

For a nonzero timeout, the proof takes exactly one fresh QPC age sample and
performs bounded atomic work: at most two state loads and at most one
compare-exchange. It has no retry loop. The lease state retains a monotonic
Expired latch. A heartbeat with a missing progress stamp is Unproven. A stamp
newer than the sampled QPC is treated as fresh concurrent progress without
negative elapsed-time subtraction. Otherwise, `elapsed > timeout` is Expired;
equality passes as Fresh.

An aged snapshot attempts one compare-exchange to latch expiry. CAS success is
Expired. CAS failure permits one state reload: an already-latched state is
Expired, a provably fresh replacement stamp is Fresh, and any missing or still
aged replacement is Unproven. Expired and Unproven reject before `SendInput`.
A QPC failure is terminal before the sender. Disabled timeout preserves the
existing diagnostic-only path without an age sample. UpOnly traffic takes no
new age sample. Descheduling after a successful proof and before the syscall
remains an unavoidable race; this proof does not make the interval atomic.

## Retained timing, scheduling, and receiver limits

The following are retained from ADR-0017 and its implementation:

```text
H = frame_base_hold + timing_margin
authored G = frame + timing_margin
physical same-key G = frame
musical Up not before = successful Down completion + H
```

At 60 FPS and 500 µs margin, `H = 17,167 µs`, authored `G = 17,167 µs`, and
physical same-key `G = 16,667 µs`. The margin contributes once to `H` and
authored `G`; it is not added to the physical one-frame floor.

Authored timestamps and packet order remain immutable. A late current Down is
still eligible for its one send attempt when final non-time gates pass. A late
Down can reduce its physical duration below the authored interval while still
respecting `H`. An earlier physical floor or sequential processing can delay
unrelated Down packets; several overdue packets may consequently be emitted
close together as the worker advances. This is head-of-line delay and overdue
output, not a scheduler rewrite, packet reorder, catch-up, or replay. Custom
native durations need separate duration semantics; the frame-based hold policy
does not promise full authored-duration preservation after a late Down.
Native long-duration behavior and startup memory/time remain characterization
topics; this ADR makes no performance guarantee about either and changes no
startup storage or scheduling policy.

The production waiter requires a high-resolution waitable timer. Its startup
failure is terminal for the production wait backend; the production path does
not fall back to coarse timer polling. The product target is Windows 11 x64.
Microsoft documents `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` as supported from
Windows 10 version 1803. The implementation arms `SetWaitableTimer` using a
negative relative interval in 100-nanosecond units. It does not pass QPC ticks
as FILETIME. `timeBeginPeriod` may affect wait timeout resolution, but does not
improve QPC precision.

`Auto` priority currently attempts MMCSS task `Games` and sets its high task
priority. If acquisition fails, it falls back to `THREAD_PRIORITY_HIGHEST`.
This mode does not join the `Pro Audio` task and is not a real-time scheduling
guarantee. Six startup wake samples initialize a calibration heuristic; they
do not support a percentile or service-level claim.

The evidence layers are separate:

1. `SendInput`'s return value reports an inserted-event count.
2. Sender pre-call and completion QPC describe the sender boundary.
3. A project-owned ReceiveOnly sink can show delivery to that sink.
4. Game sampling, rendered input, and audio onset require independent
   observation and are not proved by the first three layers.

The frame-plus-margin hold model is an application receiver policy, not an
official Windows delivery or game-sampling guarantee.

## Source, regression, and evidence crosswalk

The paths and function names below are source analysis at accepted P2 head
`c9bea5e6593f576cc3c1b72d91633d628c37e900`, reachable from P3 base
`df99c4080c556de163ea9aea7465ca70156b5739`. Accepted phase merge commits are
P0 `aa3c1a97c6a66c4036fed2c27617d0b3069773fd`, P1
`b3324655c1b4be82849fc9b2d858541505d69963`, and P2
`df99c4080c556de163ea9aea7465ca70156b5739`. Evidence revisions refer to the
published P0/P1/P2 archives below; they are not recomputed by this docs-only
phase.

| Rule | Source authority | Direct regression or assertion | Current evidence revision |
| --- | --- | --- | --- |
| Materialized `H`; authored `G=frame+margin`; physical `G=frame`; one successful completion update | `validate_min_hold_and_release_gap_feasibility_ticks`; `PhysicalTimingGuard::observe_successful_packet` and `query` | `native_timing_contract_rejects_mismatched_hold_and_release_floors`; `late_down_completion_does_not_create_a_new_hold_deadline` | P1 head `b28f75126d091b86b40bb41926526a78bed0b73e`, evidence `f6c1c9ffb5cf5e0aaceef77b1c06e23efdc562ed`; P2 head `c9bea5e6593f576cc3c1b72d91633d628c37e900`, evidence `d9478f2e8e9662a9d1bee2d736326ebdec5f8599` |
| Normal Down continuity, immutable offsets, packet order, no replay/rebase; HOL and overdue packets | `dispatch::authored` preparation; `dispatch_loop`; `PhysicalTimingGuard::query` | `normal_prepared_overdue_boundaries_are_sent_once`; `normal_prepared_overdue_boundaries_send_sequentially`; `late_down_completion_does_not_create_a_new_hold_deadline` | P1 `f6c1c9ffb5cf5e0aaceef77b1c06e23efdc562ed`; P2 `d9478f2e8e9662a9d1bee2d736326ebdec5f8599` |
| Verified cleanup starts and preserves the global Down barrier; UpOnly bypasses it; unknown cleanup/QPC/overflow fail closed | `PhysicalTimingGuard::establish_resume_down_barrier`, `query`, and `invalidate`; `cleanup::release_scope` | `r3_late_up_empty_pause_resume_preserves_release_floor`; `r3_active_cleanup_barrier_starts_after_verified_safety_up`; `r3_cleanup_qpc_failure_and_overflow_leave_down_fail_closed`; `r3_inconclusive_cleanup_does_not_establish_or_bypass_barrier`; `r3_focus_and_system_resume_keep_the_verified_cleanup_barrier` | P1 head `b28f75126d091b86b40bb41926526a78bed0b73e`, evidence `f6c1c9ffb5cf5e0aaceef77b1c06e23efdc562ed` |
| Focus loss blocks Downs/pauses; cleanup may be deferred; tracked safety obligation includes active, possibly active, failed-release, and in-flight masks | `admission::enter_focus_pause`, `handle_final_focus_loss`; `TrackedKeyState::release_obligation_mask` | `r3_focus_and_system_resume_keep_the_verified_cleanup_barrier`; `release_obligation_mask_unions_all_conservative_sources`; `focus_loss_probe_cannot_report_cleanup_success` | P1 `f6c1c9ffb5cf5e0aaceef77b1c06e23efdc562ed`; P2 `d9478f2e8e9662a9d1bee2d736326ebdec5f8599` |
| Final lease proof bounds work, rejects Expired/Unproven/QPC failure, and skips age proof for UpOnly | `admission::final_down_lease_age_proof`; `SupervisorLeaseState::prove_down_age` | `r3_overdue_unlatched_lease_blocks_prepared_down`; `r3_unproven_lease_blocks_prepared_down_without_latching_expiry`; `r3_prepared_down_qpc_failure_terminates_before_sender`; `r3_prepared_uponly_does_not_take_lease_age_sample`; `r3_supervisor_lease_down_proof_is_bounded_and_fails_closed` | P2 head `c9bea5e6593f576cc3c1b72d91633d628c37e900`, evidence `d9478f2e8e9662a9d1bee2d736326ebdec5f8599` |
| Production high-resolution wait timer, relative timer units, QPC time domain, and six-sample heuristic | `HybridWaiter::production`; `WaitableTimer::new_with_error` and `arm_relative_us`; `boot::startup_wake_probe`; `CALIBRATION_SAMPLES` | `actual_worker_wait_path_drops_waitable_timer_after_session_exit`; `calibration_deadline_preserves_viable_startup_readiness`; `disabled_waitable_timer_reports_explicit_fallback` (test-support path only) | P1 `f6c1c9ffb5cf5e0aaceef77b1c06e23efdc562ed`; P2 `d9478f2e8e9662a9d1bee2d736326ebdec5f8599`. Windows API semantics are cited below; no new timer experiment was run. |
| `Auto` priority is MMCSS `Games`, then `THREAD_PRIORITY_HIGHEST`; not `Pro Audio` | `MmcssGuard::acquire` | No direct regression covers the fallback ladder; source selection at `mmcss.rs` is the oracle | Source at accepted P2 head `c9bea5e6593f576cc3c1b72d91633d628c37e900`; this documentation phase makes no new priority measurement claim. |
| Inserted-event count, sender QPC, ReceiveOnly observation, and game sampling are distinct | Win32 packet sender; player sender telemetry; native ReceiveOnly runner/sink | `partial_transport_plus_physical_all_up_is_inconclusive`; P1 ReceiveOnly event controls; P2 native controls | P1 evidence `f6c1c9ffb5cf5e0aaceef77b1c06e23efdc562ed`; P2 evidence `d9478f2e8e9662a9d1bee2d736326ebdec5f8599` |

The P2 regression `r3_overdue_unlatched_lease_blocks_prepared_down` was proven
RED on the accepted P1 base before the lease fix and GREEN after it. The P3
change did not alter executable source, tests, or runtime behavior.

## Published phase evidence

- **P0 characterization:** measured source `458ee68aa40ad48a9805b07b62afaec2e0ba58a5`, runtime/base `1574aa267b8364b1930bd6abc7baa36636d33843`, evidence commit
  [`596664e809b69318eb216d25b9bed680446641e3`](https://github.com/pumni/Sky-Auto-Player/tree/596664e809b69318eb216d25b9bed680446641e3/runs/r3-p0-20261005T024258505Z-700aaab7).
  Manifest SHA-256: `ca4904e3114d77f2a7abf769b3a3701954297a803629cd16f6768999a4aa5fbb`.
  This is the pre-fix characterization and is not evidence that P1's barrier
  was already implemented.
- **P1 lifecycle:** source head `b28f75126d091b86b40bb41926526a78bed0b73e`, base
  `aa3c1a97c6a66c4036fed2c27617d0b3069773fd`, evidence commit
  [`f6c1c9ffb5cf5e0aaceef77b1c06e23efdc562ed`](https://github.com/pumni/Sky-Auto-Player/tree/f6c1c9ffb5cf5e0aaceef77b1c06e23efdc562ed/runs/r3-p1-20261016T111711630Z-df42d2fd).
  Manifest SHA-256: `89463e7aaeb2ad13d61eb068c8742aa200fde9e5fe821d70cc8977e042c95fcb`.
  The six ReceiveOnly controls passed 27/27 events. Statistical timing sample
  count was zero; sink evidence does not prove game or audio receipt.
- **P2 final lease age:** source head `c9bea5e6593f576cc3c1b72d91633d628c37e900`, base
  `b3324655c1b4be82849fc9b2d858541505d69963`, evidence commit
  [`d9478f2e8e9662a9d1bee2d736326ebdec5f8599`](https://github.com/pumni/Sky-Auto-Player/tree/d9478f2e8e9662a9d1bee2d736326ebdec5f8599/runs/r3-p2-20261016T132824008Z-03e7028d).
  Manifest SHA-256: `bf5f662a26610f09b5f90be4e86148c8c60512f9aee0552f1737b32f270f1c19`.
  Paired performance qualification captured 1.8 million samples; all 18
  paired budgets and five native controls passed. These measurements remain
  pinned and are not repeated for this docs-only phase.

## External references and provenance

API behavior statements above are external facts; contract equations,
admission ordering, and fallback choices are source analysis at the accepted
P2 head. Primary references:

- Microsoft, [`CreateWaitableTimerExW`](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createwaitabletimerexw), including `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` availability.
- Microsoft, [`SetWaitableTimer`](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-setwaitabletimer), for negative relative due times and 100-nanosecond units.
- Microsoft, [`QueryPerformanceCounter`](https://learn.microsoft.com/en-us/windows/win32/api/profileapi/nf-profileapi-queryperformancecounter), for high-resolution interval timestamps.
- Microsoft, [`timeBeginPeriod`](https://learn.microsoft.com/en-us/windows/win32/api/timeapi/nf-timeapi-timebeginperiod), which states that higher timer resolution does not improve high-resolution performance-counter accuracy.
- Microsoft, [`SendInput`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput), for inserted-event counts and the UIPI boundary.
- Microsoft, [Multimedia Class Scheduler Service](https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service), for the distinct `Games` and `Pro Audio` task names and scheduling behavior.

The historical 1.98.1 pin in older phase notes is not the toolchain used for
P3. The repository pin and this phase's verification use Rust/Cargo 1.99.1
and Bun 1.4.3. Rust's [1.98.1 announcement](https://blog.rust-lang.org/2026/09/03/Rust-1.98.1/)
documents a historical compiler vtable fix; the [1.99.1 announcement](https://blog.rust-lang.org/2026/10/15/Rust-1.99.1/)
records the current point release. These release notes establish toolchain
provenance only, not Windows timing or receiver behavior.
