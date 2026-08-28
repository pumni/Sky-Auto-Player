# Native Dispatch Follow-up Optimization Evidence (2026-08-28)

This report records the follow-up implementation and measurements for the
remaining hot-60-FPS completion-width finding, lease-only spin, and the proven
dead inner precision wait. It is host- and revision-specific evidence. The
sender-side harness cannot prove that Sky sampled, rendered, or produced audio
for every injected transition.

## Status

The implementation work is complete for the defined low-risk scope. The
coordinator Phase-A qualification and the real-wait core A/B qualification are
clean. The final-HEAD single-key hot sender workload is clean. An earlier
single-key cold run was clean, but two final-HEAD cold reruns exposed
intermittent pre-call hold-shrink violations, so cold is not signed off below.

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
| Hot, paired, polyphony 1 at final HEAD | 10,240 | Clean/statistically eligible | Completion diagnostic: 229 below-frame samples; hard counters zero. |
| Cold, paired, polyphony 1 at final HEAD | 256–9,216 attempted | Not qualified | Two 40-suite reruns stopped on 3 and 1 pre-call hold-shrink violations; traces remained complete. |
| Cold, paired, polyphony 1 at `45151f9` | 10,240 | Clean/statistically eligible | Earlier clean artifact; native production code is unchanged by later benchmark/docs commits. |
| Hot, paired, polyphony 15 | 272 | Not qualified | 4,080 keys delivered; release-gap hard counter 2,025 and matching anomaly count. |

Final-HEAD hot single-key artifact:
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup-candidate-hot-p1-a5baa2c.json`

Earlier clean cold single-key artifact:
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup-candidate-cold-p1-45151f9.json`

Final-HEAD cold failed artifacts:
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup-candidate-cold-p1-a5baa2c-failed-run-36.json`
and
`C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup-candidate-cold-p1-a5baa2c-v2-failed-run-2.json`.

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
acceptance on this host. The cold rerun failures are also retained as a hard
qualification result; they show host scheduling jitter beyond the 500 µs
pre-call hold-shrink allowance, not missing or duplicated input.

None of these artifacts observes Sky's Raw Input, frame registration, render,
or audio onset. A remaining “HUD clean but note not heard” report therefore
requires downstream observability; sender telemetry alone cannot distinguish
game sampling from game-side rejection or audio behavior.

## Follow-up: acceptance-fidelity repair (schema 9)

This section is a successor to the earlier evidence above. The earlier failed
results remain historical evidence; they are not rewritten as passes.

### Scope and implementation

The starting checkout was `455c8adb52f11a25ac3f22ebef66aa0b4602f461` with a
clean working tree. The follow-up implementation commits were:

```text
6279af7599f24ded0b9b2c290cd41939849d9095  fix: enforce custom sender down cutoff
c58679a74b66ed64d3cb81b4ef5272f6b3976b7c  fix: separate release visibility forensics
06bacaa29c5be791cc0a2e73d192cdceaf7c327c  bench: separate acceptance qualification dimensions
ea3afdeb9a52b20fd53eafe1f244283bb8236a56  bench: record overdue real-wait samples
e8e826f1fb8a3718dea4b4f37013cc3c9e9c9883  bench: expose failed qualification dimensions
a3cb345fbb6aaf0c06b90e67fc76107ba2d8ff0c  test: verify failed acceptance dimension reports
a4f5404688326a2acadfa1228eaa8c5082722290  bench: classify anomaly ring overflow as diagnostic
0b9283720a6169f4b38fab3c97039ad03e8f7104  refactor: isolate test-support dispatch seams
cd7541763981fe82dd098825a3f8169358b57287  bench: report aggregate real wait cost
```

The custom test-support sender now samples its authoritative QPC immediately
before the custom emitter, applies the shared production Down-only cutoff, and
returns `DeadlineMissedBeforeSend` without invoking the emitter when the sample
is one tick beyond the boundary. Equality is allowed and Up-only packets are
exempt. The deterministic tests are
`prepared_down_cutoff_exact_boundary_sends_once`,
`prepared_down_cutoff_one_tick_late_never_calls_emitter`,
`prepared_custom_emitter_samples_cutoff_before_invocation`, and
`prepared_custom_emitter_keeps_up_only_exempt_from_down_cutoff` in
`rust/crates/sky_dispatch_win32/src/input/tests/tracked.rs`.

Production behavior was intentionally frozen: authored timestamps, the
500 µs Down grace, 300 µs default transport margin, hold policy, authored
release-gap validation, focus/target/lease gates, future authorization,
single-wait architecture, one `SendInput` attempt, cleanup, and RT allocation
behavior did not change.

### Corrected release-gap contract

The old forensic counter compared
`next_down_pre_call - previous_up_completion` with the complete authored
`min_release_gap_us`. That required transport to consume none of the headroom
that the policy intentionally reserved. At 60 FPS the unchanged authored
policy is:

```text
frame floor       = 16,667 µs
sender headroom   = 500 + 300 = 800 µs
authored gap      = 17,467 µs
```

Static schedule validation still hard-requires the authored target gap of
17,467 µs. Sender forensics now compares the conservative completion-to-next-
pre-call interval with the base 16,667 µs visibility floor. The consumed
800 µs headroom is reported separately. Negative timestamp ordering and all
ownership/transport/trace/cleanup invariants remain hard failures. Structural
anomalies are hard qualification counters; timing observations, headroom
consumption, and bounded anomaly-ring overwrite are visible diagnostics and
are no longer accepted only through one undifferentiated anomaly total.

The acceptance report now separates `hard_correctness`,
`hard_scheduler_cutoff`, `transport_diagnostics`, and
`headroom_consumption_diagnostics`. `rt_handoff_bench` separately reports
`waiter_timing_clean`, `dispatch_path_clean`, `sender_cutoff_clean`, and
`statistics_eligible`; its test-support wait path is not silently presented as
production `SendInput` qualification.

### Environment and provenance

All acceptance runs below were on:

```text
Windows-11-10.0.26200-SP0
AMD64 Family 23 Model 104 Stepping 1, AuthenticAMD
QPC 10,000,000 Hz
rustc 1.98.0 (88d9e12ae 2026-08-18)
```

The schema-9 acceptance artifacts record `dirty_worktree=false` and their
`native_build_commit` equals their candidate SHA. The test-support wheel used
for the final source verification was built at `0b9283720a6169f4b38fab3c97039ad03e8f7104`;
its SHA256 was
`6E63F9F087853DEC5A8E5383A85CFAD4C2CD4B6C80544227615078537265AF56`.
The earlier schema-9 JSON runs at `ea3afde` and `a4f5404` preserve native
commit provenance in their own reports; their historical runner did not emit a
wheel SHA field. No stale wheel was accepted: each run passed the build
commit guard before execution. A final production wheel is built and verified
at the final documentation commit after this section is committed.

### Corrected evidence

| Evidence | Command/workload | Candidate/native commit | Samples | Result |
| --- | --- | --- | ---: | --- |
| E1 cold single-key | `bench_native_acceptance.py --dispatch-repeats 40 --actions 128 --polyphony 1 --scenario paired --gap-profile cold --start-delay-us 100000 --skip-command-samples --continue-after-failure --budget-seconds 600` | `ea3afde` / `ea3afde` | 10,880 requested boundaries; 15/40 suites completed | **REAL CUTOFF FAILURE**; faithful seam recorded Down cutoff misses, so not statistically eligible |
| E2 hot single-key | same paired p1 shape with `--gap-profile hot` | `ea3afde` / `ea3afde` | 40 suites requested; 23 completed | Not qualified; failure artifacts retained, including cutoff/lease failures |
| E3 hot polyphony 15 | paired `--polyphony 15 --gap-profile hot` | `a4f5404` / `a4f5404` | 272 boundaries, 4,080 keys | **REAL RELEASE-VISIBILITY FAILURE** under corrected floor: 1,815 hard floor violations; old 2,025 full-gap count was over-constrained |
| E4 real-wait core | `RT_HANDOFF_BENCH_ITERATIONS=10000 RT_HANDOFF_BENCH_SCOPE=real_wait_core RT_HANDOFF_BENCH_MODE=real_wait` with `DUE_US=1000` | cargo binary at `ea3afde` | 10,000 iterations/mode | Not qualified: `statistics_eligible=false`; dimensions and raw counters retained |
| E5 negative cutoff | deterministic Rust sender tests listed above | `0b92837` test-support source | equality/+1 tick/Up-only cases | Pass; late Down is rejected before successful mock transport |

Artifacts:

```text
C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup2-ea3afde-cold-p1-start100ms.json
C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup2-ea3afde-hot-p1.json
C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup2-a4f5404-hot-p15.json
C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup2-ea3afde-rt-real-wait-core.json
```

The release-mode rerun after the aggregate-cost report change is preserved at:

```text
C:\Users\PE4CE_~1\AppData\Local\Temp\sky-auto-player-followup-final-75aaf2a-rt-real-wait-core.json
```

It used the same 1 ms due interval and the clean final source checkout at
`75aaf2afa231a6c8bf3b456f02b67d98010800f8`. It collected 60,000 waits per
mode (10,000 in each of six scenarios), with zero early or overdue events.
The aggregate report now retains total spin and wall time:

| Mode | Spin threshold | Cutoff/non-dispatch | Dispatch p99/p99.9/max | Spin p99/p99.9/max | Total spin / wall (µs) | Spin duty | CPU duty |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Fixed 400 µs | 400 µs | 238 | 506 / 1,553 / 24,561 µs | 370 / 379 / 747 µs | 5,302,081 / 96,050,557 | 5.52% | 11.51% |
| Fixed 700 µs | 700 µs | 89 | 139 / 620 / 28,407 µs | 656 / 672 / 1,083 µs | 23,541,061 / 93,063,424 | 25.30% | 31.44% |
| Fixed 1,000 µs | 1,000 µs | 10 | 46 / 1,252 / 21,595 µs | 998 / 1,133 / 12,591 µs | 52,131,199 / 91,972,264 | 56.68% | 60.01% |
| Production calibrated | 909 µs | 32 | 63 / 615 / 13,160 µs | 878 / 886 / 1,824 µs | 25,665,346 / 92,823,221 | 27.65% | 33.65% |

The four modes are not statistically eligible because their faithful
test-support sender cutoff path recorded Down deadline misses/non-dispatches.
The benchmark therefore proves the waiter/dispatch measurements and preserves
the cutoff evidence, but does not certify production `SendInput`; its
qualification dimensions remain explicitly separated.

The cold result is a real failure of the unchanged 500 µs cutoff on this
host, not evidence that the mock transported an out-of-grace Down. The p15
result shows that the old 2,025 count was inflated by comparing against the
full authored gap, but the corrected base-frame floor still has real observed
violations in this run. The real-wait report contains 10,000 iterations per
mode and reports wait count, overdue count, p99/p99.9 tails, spin time/duty,
wall time, and process CPU duty. Process CPU duty is benchmark-fixture CPU,
not application CPU. The 1 ms due interval is a bounded waiter workload and
not a 60-FPS game claim.

The corrected evidence does not justify changing calibration in this task.
Because the real-wait Down modes still show cutoff misses and the cold run
shows host sensitivity at the unchanged grace, the recommendation is
**SEPARATE CALIBRATION INVESTIGATION REQUIRED**; that is a separate task, not
a production change here. No runtime adaptation or scheduler tuning was
introduced.

Finally, sender-side evidence still does not prove that Sky sampled, rendered,
or produced audio for every injected transition. Game receipt and audio remain
outside this application's observability boundary.

## CI and production-equivalent scheduling follow-up

The final documentation commit `b67cf79107e6bb11c8653350a2aa7ed26f8dd284`
was independently observed red in GitHub Actions run `33147483496`. Static,
packaged-smoke, and other validation jobs passed, but the Windows
`uv run python scripts/check.py rust` step failed one timing-sensitive engine
test. The failure was `engine::tests::trusted_pre_call_deadline_miss_finishes_with_clean_session_health`:
worker startup preemption made its authored zero-time first Down cross the
unchanged 500 µs cutoff before the test's injected second-call fault could be
observed. GitHub reported 235 player tests passed and 1 failed in that run.

Commit `1ad42ae9338a4342197a505d298776087a2cf89d` fixes only the test setup.
The completion-latency release-gap test now uses the existing 500 ms test
pre-roll, and the injected-deadline-miss test starts its authored schedule at
2 seconds after the frozen epoch. These tests now exercise their intended
behavior instead of an incidental startup race. No production sender,
scheduler, authored timestamp policy, hold/release policy, focus/target gate,
lease, or 500 µs Down grace changed.

The corrected local canonical gates at that source commit were:

```text
check.py static: PASS
check.py rust: PASS — 50 core, 1 golden, 3 property, 223 Win32,
  236 player, 20 no-allocation, 52 updater, 1 packaged E2E, 1 PEP 440,
  5 updater-safety tests
check.py tests with test-support wheel: PASS — 944 passed, 1 skipped,
  1 xfailed
check.py tests with production wheel: PASS — 936 passed, 9 skipped,
  1 xfailed
```

The production-equivalent test-support scheduling rerun used
`--rt-priority-mode auto`, the same `40 × 128` p1 workloads, and retained
separate JSON artifacts:

```text
.benchmarks/followup-final-auto-cold-p1.json
.benchmarks/followup-final-auto-hot-p1.json
.benchmarks/followup-final-auto-hot-p15.json
.benchmarks/followup-final-real-wait-core.json
```

The p1 cold and hot runs each completed 40/40 suites with 10,880 and 10,240
physical boundaries respectively, zero missed Down/cutoff events, zero hard
correctness counters, and `statistics_eligible=true`. These are still
test-support/host scheduling measurements, not shipping `SendInput` proof.
The p15 run retained complete trace/transport evidence and reproduced 1,815
corrected base-frame release-visibility violations under the default mock
transport model; it did not prove an equivalent duration for production
`SendInput`.

The real-wait core rerun used:

```text
RT_HANDOFF_BENCH_ITERATIONS=10000
RT_HANDOFF_BENCH_SCOPE=real_wait_core
RT_HANDOFF_BENCH_MODE=real_wait
RT_HANDOFF_BENCH_DUE_US=1000
```

It collected 10,000 iterations in each of the four required modes. The run
remained `statistics_eligible=false` because the test-support real-wait
scenarios recorded non-dispatch/cutoff evidence; the report retains all raw
counts and separates waiter timing from sender-cutoff qualification.

An actual production-wheel `SendInput` run was not issued from this execution
environment. The project-owned sink started successfully, but
`GetForegroundWindow()` returned `0` (no interactive foreground desktop), so
using `--no-require-focus` would not have been a valid sink-receipt
qualification. Actual `SendInput` qualification is therefore
**INCONCLUSIVE**, not green by omission. The production wheel was nevertheless
built and verified from a clean tree with `native_build_commit` equal to the
HEAD used for its build; its exact SHA256 and build metadata are recorded in
the final command output and acceptance handoff.

The calibration recommendation remains:

**SEPARATE PRODUCTION-EQUIVALENT SCHEDULING / CALIBRATION ROOT-CAUSE
INVESTIGATION REQUIRED.**

No calibration constants, runtime adaptation, 500 µs grace, authored hold or
release gap, focus policy, target policy, lease policy, one-wait architecture,
SendInput count, or RT allocation behavior was changed by this follow-up.

## Production-equivalent scheduling rerun (schema 10, source `7c3cff6`)

This section is a later, source-pinned rerun of the scheduling follow-up. It
does not rewrite the historical failures above. The benchmark source and the
test-support wheel were both built from the clean checkout at:

```text
7c3cff6341b89e2e00f162fb7ae3deb9805f0eac
```

The two implementation commits in this rerun are:

```text
85a21299ceacc7f1a00d20ce255497c99944b87a  bench: add production-equivalent scheduling provenance
7c3cff6341b89e2e00f162fb7ae3deb9805f0eac  bench: use production waiter constructor for real-wait evidence
```

The production behavior remains frozen. No authored timestamps, hold or
release-gap policy, 500 µs Down grace, focus/target/lease gate, future
authorization, one-wait architecture, SendInput count, retry policy, or RT
allocation behavior changed. The new mode only makes the test-support harness
use the production waiter constructor, production startup calibration formula,
and explicit scheduling provenance. Its transport remains the deterministic
mock; it is not a production SendInput substitute.

### Build and host provenance

The host for every artifact in this section was:

```text
Windows-11-10.0.26200-SP0
AMD64 Family 23 Model 104 Stepping 1, AuthenticAMD
QPC 10,000,000 Hz
rustc 1.98.0 (88d9e12ae 2026-08-18)
```

The test-support wheel was built with `scripts/build_rust_wheel.py --test-support`.
Its SHA256 was:

```text
5A9E501E6568D8D7C946EB1C3B3937BB6FF6B57622C9E07E7C8180EB3701C8A9
```

The production wheel was built with `scripts/build_rust_wheel.py` and was not
used for mock acceptance. Its SHA256 was:

```text
8A1138B5EAB8793C1DBE946ED9FE97D77AB2DB4C2B7851203976266C9439BC86
```

Both wheel `build_info()` results reported:

```text
native_build_commit = 7c3cff6341b89e2e00f162fb7ae3deb9805f0eac
native_abi          = cp314t-win_amd64
free_threaded       = true
win32_backend       = true
```

The production-equivalent runs requested `--rt-priority-mode auto`. The host
acquired `off` rather than the requested elevated mode, so these artifacts are
explicitly test-support/off-profile evidence. The harness did not force a
priority class or relabel the fallback as production scheduling.

### Corrected acceptance reruns

Commands for E1–E3 were the same shape, with the indicated workload changed:

```text
uv run --offline --env-file .env python scripts/bench_native_acceptance.py \
  --backend mock --wait-policy production_calibrated --rt-priority-mode auto \
  --scenario paired --actions 128 --dispatch-repeats 40 --warmup-cycles 8 \
  --start-delay-us 100000 --skip-command-samples --continue-after-failure \
  --budget-seconds 600 --expected-native-build-commit 7c3cff6341b89e2e00f162fb7ae3deb9805f0eac
```

E1 used `--polyphony 1 --gap-profile cold`, E2 used
`--polyphony 1 --gap-profile hot`, and E3 used
`--polyphony 15 --gap-profile hot`. Each requested 40 suites and preserved
failed-run JSON files rather than excluding them.

| Evidence | Requested samples | Completed | Hard correctness counters | Hard cutoff/release counters | Diagnostic counters | Eligibility/result |
| --- | ---: | ---: | --- | --- | --- | --- |
| E1 cold p1 | 10,880 authored boundaries | 35/40 suites | trace mismatch 282; other structural counters 0 | missed Down 6 boundaries/6 keys; release floor 0 | completion below frame 0 | `statistics_eligible=false`; test-support cutoff failure, production relevance unproven |
| E2 hot p1 control | 10,880 authored boundaries | 29/40 suites | trace mismatch 24; other structural counters 0 | missed Down 12 boundaries/12 keys; release floor 0 | completion below frame 64 | `statistics_eligible=false`; no clean control on this host |
| E3 hot p15 | 10,880 authored boundaries / 163,200 requested keys | 0/40 suites clean | trace mismatch 26; other structural counters 0 | missed Down 13 boundaries/195 keys; corrected release-floor count 73,905 | completion below frame 1,110; headroom consumed 80,610; ring overwrites 72,625 | `statistics_eligible=false`; real mock-model release-visibility failure |

The per-run p15 workload is 272 boundaries and 4,080 keys, matching the
historical stress shape. The aggregate is larger because the required rerun
used 40 suites. Static authored targets still used the unchanged 17,467 µs
release gap and the unchanged 16,667 µs base visibility floor. The old full-gap
proxy is not used as the sender-side hard observation anymore. The remaining
p15 failures therefore describe the current mock transport model
(`80 + 40 × key_count` µs plus Windows scheduling), not measured production
`SendInput` duration.

E5 is the deterministic cutoff artifact supplied by the Rust sender tests:
equality at the cutoff sends once, one QPC tick late returns
`DeadlineMissedBeforeSend` without invoking the custom emitter, and Up-only
remains exempt. The tests are in
`rust/crates/sky_dispatch_win32/src/input/tests/tracked.rs` and cover the
shared production/test-support predicate.

Artifacts:

```text
.benchmarks/production-equivalent-auto-cold-p1-final7c.json
.benchmarks/production-equivalent-auto-hot-p1-final7c.json
.benchmarks/production-equivalent-auto-hot-p15-final7c.json
```

The E1/E2 labels must be read as **test-support cutoff failures under the
current host profile**, not as proof that shipping production scheduling or
real `SendInput` has the same distribution. All raw failed-run artifacts are
retained beside their report files.

### Real-wait core rerun

The command was:

```text
RT_HANDOFF_BENCH_ITERATIONS=10000
RT_HANDOFF_BENCH_SCOPE=real_wait_core
RT_HANDOFF_BENCH_MODE=real_wait
RT_HANDOFF_BENCH_DUE_US=1000
cargo run --manifest-path rust/Cargo.toml -p sky_player_rs \
  --example rt_handoff_bench --features test-support -- \
  .benchmarks/production-equivalent-real-wait-core-10k.json
```

It used `HybridWaiter::production()` and collected 10,000 iterations in each
of six scenarios, or 60,000 waits per mode. The binary was compiled from the
clean source SHA above with the test-support feature; unlike wheel-backed
acceptance, this direct Cargo artifact has no Python wheel SHA. The command,
source SHA, compiler, QPC and output path above are the complete provenance.

| Mode | Selected spin | Cutoff/non-dispatch | Overdue | Worst dispatch p99/p99.9 | Worst spin p99/p99.9 | Total spin / wall (µs) | CPU duty |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Production calibrated | 1,000 µs | 14 | 0 | 92 / 1,673 µs | 992 / 1,137 µs | 50,132,370 / 97,501,075 | 61.82% |
| Fixed 400 µs | 400 µs | 239 | 11 | 1,386 / 3,928 µs | 370 / 1,608 µs | 7,041,473 / 104,767,029 | 21.32% |
| Fixed 700 µs | 700 µs | 204 | 6 | 852 / 3,010 µs | 669 / 1,998 µs | 22,456,542 / 101,382,593 | 37.16% |
| Fixed 1,000 µs | 1,000 µs | 81 | 6 | 1,268 / 13,820 µs | 999 / 1,404 µs | 49,764,300 / 100,282,744 | 61.16% |

The production-calibrated mode executed six startup samples and selected the
1,000 µs cap on this run. Every mode has `statistics_eligible=false` because
the test-support sender seam recorded deadline/non-dispatch or overdue
evidence. The report keeps the dimensions separate:
`waiter_timing_clean`, `dispatch_path_clean`, and `sender_cutoff_clean`; the
last one is never claimed by this benchmark as production `SendInput`
qualification. Process CPU duty includes benchmark-fixture overhead and is
not application CPU usage.

Artifact:

```text
.benchmarks/production-equivalent-real-wait-core-10k.json
```

### Qualification decision and remaining uncertainty

The cold result is **INCONCLUSIVE** for shipping production: the faithful
test-support seam observed real cutoff failures under an `Auto→off` host
profile, but no production scheduling/MMCSS or actual `SendInput` run was
available. The hot p15 result is **INCONCLUSIVE** for shipping production and
is a **REAL RELEASE-VISIBILITY FAILURE UNDER THE CURRENT MOCK P15 TRANSPORT
MODEL**. It must not be used to increase the authored release gap.

No safe interactive foreground sink was available (`GetForegroundWindow()` was
zero), so an actual production-wheel `SendInput` qualification was not run.
Using `--no-require-focus` would not have been valid evidence. Sender-side
evidence does not prove that Sky sampled, rendered, or produced audio for every
injected transition.

The calibration recommendation is:

**SEPARATE PRODUCTION-EQUIVALENT SCHEDULING / CALIBRATION ROOT-CAUSE
INVESTIGATION REQUIRED.**

This is not authorization to change calibration constants, add runtime
adaptation, change the 500 µs grace, or retune authored timing. The next
investigation must first obtain production-equivalent scheduling and isolated
real-`SendInput` evidence.
