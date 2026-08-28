# Native Dispatch Follow-up Optimization Evidence (2026-08-28)

This report records the follow-up implementation and measurements for the
remaining hot-60-FPS completion-width finding, lease-only spin, and the proven
dead inner precision wait. It is host- and revision-specific evidence. The
sender-side harness cannot prove that Sky sampled, rendered, or produced audio
for every injected transition.

## Status

The implementation work is complete for the defined low-risk scope. The
coordinator Phase-A qualification and the real-wait core A/B qualification are
clean. The default single-key hot/cold sender workloads are also clean.

The deliberately tighter hot 15-key stress workload remains unqualified: its
hard release-gap counter detects that a large mock `SendInput` completion can
consume part of the authored release gap. All 272 authored boundaries and all
4,080 requested keys in that run were delivered, but this is not evidence that
the physical release-gap policy is satisfied. The counter was not weakened.

## Provenance and changes

- Native production behavior: `45151f95168fe0286cb7131d4d562702179ea695`
  (`45151f9`); test-support benchmark/harness changes: `96881bcc638fac1586c2d73f66ce2221fd249089`.
- Host: Windows 11 build `10.0.26200`, AMD64 Family 23 Model 104 Stepping 1.
- Rust: `rustc 1.98.0 (88d9e12ae 2026-08-18)`.
- QPC: `10,000,000 Hz`.
- Native acceptance transport: deterministic test-support mock transport;
  real `SendInput` and game receipt are not measured.

The follow-up preserves authored timestamps, the 500 µs Down grace, final
focus/target/control/lease gates, future Down authorization, one physical wait,
one production `SendInput`, cutoff semantics, packet ordering, and fail-closed
cleanup.

Implemented changes:

- Startup calibration now computes an absolute probe deadline bounded by the
  20 ms calibration budget and the existing 2 ms readiness reserve. Regression
  tests cover skipping a probe that would consume the reserve and preserving a
  viable startup deadline.
- `production_completion_hold_below_frame_count` is retained as a bounded
  transport diagnostic. Hard acceptance still checks pre-call hold grace,
  release gap, retrigger/anchor/unmatched state, transport integrity, target
  timing, and cleanup. The hot evidence algebra is:
  `completion_hold = authored_hold + (up_start_late - down_start_late) +
  (up_send_dur - down_send_dur)`.
- A lease-only bounded wake uses zero spin; an exact physical-target wake keeps
  the frozen calibrated threshold, including deterministic equality precedence.
- Static call-graph and dispatch-loop review proved the old precision inner wait
  was unreachable on shipping paths. The fallback and its production plumbing
  were removed; test-support direct-boundary seams remain explicit.
- `poll_s` remains because the runtime loop and tests use it as an active
  cadence seam; it was not removed for aesthetics.
- The real-wait benchmark now records spin time, wall time, and spin duty. Its
  `real_wait_core` scope qualifies only calibrated, fixed 400, fixed 700, and
  fixed 1,000 µs modes; the full scope retains 250 µs negative control and
  mixed diagnostic scenarios.

## Phase-A coordinator qualification

Artifact:
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-rt-handoff-followup-45151f9-phase-a-10k.json`

The production-boundary matrix ran 10,000 iterations in each of nine
Down/Up/mixed packet scenarios (90,000 observations). It reported
`acceptance_clean=true` and `statistics_eligible=true`; early dispatches and
non-dispatches were both zero. The maximum dispatch-start p99/p99.9 was
`120/163 µs`. Calibration metadata recorded six samples and a 681 µs selected
threshold for that run. This is deterministic direct-boundary evidence, not
real waiter or game evidence.

## Real-wait A/B qualification

Artifact:
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-rt-handoff-followup-45151f9-real-wait-core-10k-due1ms.json`

The `real_wait_core` run used 10,000 waits per mode across six Down/Up packet
size scenarios, for 60,000 waits per mode. It used a 1 ms due interval to keep
the host run bounded; this is a valid HybridWaiter/QPC timing workload, not a
claim about a 60-FPS game frame. Every mode was clean and statistically
eligible.

| Mode | Selected spin | Process CPU duty | Worst dispatch-start p99 | Worst p99.9 | Worst spin p99 | Max spin duty |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Calibrated | 415 µs | 10.95% | 613 µs | 809 µs | 380 µs | 7.40% |
| Fixed 400 µs | 400 µs | 10.53% | 566 µs | 763 µs | 364 µs | 4.91% |
| Fixed 700 µs | 700 µs | 30.71% | 116 µs | 516 µs | 639 µs | 43.81% |
| Fixed 1,000 µs | 1,000 µs | 59.90% | 16 µs | 186 µs | 998 µs | 99.12% |

No mode had an early dispatch or non-dispatch in this core run. The host data
does not justify changing the frozen production calibration formula: lowering
CPU with 400 µs costs tail margin, while 1,000 µs buys lower tail at very high
CPU. Production remains startup-calibrated and session-frozen.

The full diagnostic matrix was also run at 10,000 iterations/mode. Its 250 µs
negative control and mixed/very-tight due workload are intentionally not used
as production qualification; their failures remain visible rather than being
converted into a pass.

## Native acceptance evidence

| Workload | Boundaries | Result | Notes |
| --- | ---: | --- | --- |
| Hot, paired, polyphony 1 | 10,240 | Clean/statistically eligible | Completion diagnostic: 196 below-frame samples; hard counters zero. |
| Cold, paired, polyphony 1 | 10,240 | Clean/statistically eligible | Completion diagnostic zero; hard counters zero. |
| Hot, paired, polyphony 15 | 272 | Not qualified | 4,080 keys delivered; release-gap hard counter 2,025 and matching anomaly count. |

Hot single-key artifact:
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup-candidate-hot-p1-45151f9.json`

Cold single-key artifact:
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup-candidate-cold-p1-45151f9.json`

The hot 15-key artifact is:
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup-candidate-hot-45151f9-failed-run-0.json`.
Its sender trace is complete and all requested keys were inserted, but the
completion-to-next-Down release interval is below the fixed policy for many
same-key transitions. This is a real hard-policy finding, not a reason to
remove the release-gap gate. Mixed packet cardinality and coordinator ordering
are covered cleanly by the Phase-A 90,000-observation matrix.

## Interpretation and residual risk

The hot single-key completion failures are explained by transport asymmetry,
not by an early Down or an authored-target rewrite. The pre-call boundary is
the scheduler cutoff; completion is sampled after `SendInput` returns and is
therefore retained as transport forensics. The separate release-gap result for
large packets remains hard and prevents claiming a full all-polyphony hot
acceptance on this host.

None of these artifacts observes Sky's Raw Input, frame registration, render,
or audio onset. A remaining “HUD clean but note not heard” report therefore
requires downstream observability; sender telemetry alone cannot distinguish
game sampling from game-side rejection or audio behavior.
