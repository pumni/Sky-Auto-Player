# Timing Principles for Sky Music Player

This document is the engineering source of truth for explicit hold-frame timing in Sky Music Player. It defines sender-side timing contracts; game registration remains subject to uninstrumented frame-sampling evidence.

---

## 0. Hierarchy of Truth & Ground Truth

### Ground Truth
* **Frame-Bound Sampling:** The game samples input state once per render frame. For a key-down event to be registered, the key must remain held down for **at least 1 game frame**. This is the only hard timing constraint.
* **No Arbitrary Margins:** Same-key feasibility is determined strictly by the key's minimum hold duration (`min_hold_us`). The scheduler does not add a separate scheduling-time latency guess (such as the legacy `release_latency_margin_us`). Note that since 2026-07 `min_hold_us` itself *includes* a small constant **device-delivery margin** (`min_hold_margin_us`, default 500 µs — see §2): that margin models a measured physical effect (post-`SendInput` kernel delivery latency), not a scheduling fudge factor, and setting it to 0 restores the pure frame-ratio model.

For the Rust worker, game polling is not a proof boundary. The proved runtime sequence ends at
`SendInput` return; an app-owned Raw Input receipt is an optional delivery proxy and must never be
called game-observed latency. A session may report `finished` only after the complete clean ledger
and backend-mask contract described in `rt-dispatch-architecture.md` §1; cancellation is not a
successful finish.

### Evidence Hierarchy
When resolving conflicts, the following hierarchy applies:
1. **Observed Game Behavior** (audio/onsets recorded in-game) — wins over everything.
2. **Deterministic Measurements** (telemetry CSV, coordinator/scheduler simulator) — wins over "experience/intuition".
3. **Current Codebase** (`src/`) — wins over descriptions in any document.
4. **Documentation** — only interpretive; if a document conflicts with 1, 2, or 3, it is outdated/incorrect and must be corrected.

> [!NOTE]
> [AGENTS.md](../AGENTS.md) remains the single source of truth for overall project rules and coding constraints.

---

## 1. Core Terms
Authored schedules, hold selection, estimator values, and human-readable reports use
microseconds ($\mu\text{s}$). In the native production worker, scheduling and state decisions
use typed QPC-derived `QpcTicks`, `TimelineTicks`, and `DurationTicks`; conversion occurs once at
the API/configuration boundary and again only when publishing telemetry. The worker does not use
ticks → microseconds → ticks as a control-path intermediary.

| Term | Meaning |
| :--- | :--- |
| `hold_us` | Effective key-down duration for a normal note. Equal to `min_hold_us` in every production session. |
| `min_hold_us` | The visibility floor. The absolute minimum key-down duration allowed after compression. |
| `hold_frames` | The selected ratio; only `1.0`, `1.25`, and `1.5` are accepted. |
| `same_key_interval_us` | Time between two down events on the same scan code. If below `min_hold_us`, the repeat is infeasible. |
| `frame_us` | Duration of one game frame. Calculated as `ceil(1,000,000 / game_fps)`. |
| `game_fps` | The game FPS explicitly selected by the user to match Sky. FPS is never auto-detected. |
| `tempo_scale` | Playback speed multiplier. Values above 1.0 increase scheduling pressure. |

---

## 2. Timing and Feasibility Model

### Hold Model
Every production hold materializes from the selected frame ratio plus a constant device-delivery margin:
$$\text{hold\_us} = \text{min\_hold\_us} = \text{round}(\text{hold\_frames} \times \text{frame\_us}) + \text{min\_hold\_margin\_us}$$
Where:
$$\text{frame\_us} = \lceil 1,000,000 / \text{game\_fps} \rceil$$

`min_hold_margin_us` (default **500 µs**, `0` restores the pure ratio model) covers the residual kernel delivery latency after `SendInput` returns. It is an internal device-delivery allowance and is not user-selectable in the picker.

The optional Windows calibration cache is evidence of kind
`injected_raw_input_delivery_proxy`: the app-owned calibration window receives keys injected
through `SendInput`, and the harness correlates its `WM_INPUT` receipt with the native-call
completion timestamp. This is a host delivery proxy only; it does not observe Sky polling, render
frames, or audio onset. Its UTC `sampled_at` and evidence label are diagnostic metadata, and the
loader applies no freshness TTL until repeated measurements establish a conservative policy. Warm-up
injections are excluded from both measured classes; Hot uses a short gap and Cold uses an explicit
idle gap. Down/Up and polyphony channels remain independent. Partial injection and uncertain
cleanup invalidate a run rather than becoming zero-latency evidence.

### FPS Assumption vs Real Game FPS
The selected `game_fps` determines the hold duration. It must match Sky's configured FPS. The scheduler never detects or corrects the game's FPS; if the selected value is too high, a hold can be shorter than one real game frame.

### Same-Key Feasibility
The authored feasibility floor remains:
$$\text{same\_key\_interval\_us} \ge \text{min\_hold\_us}$$

The native sender applies a stricter runtime condition because the release floor is anchored at
the actual down completion:
$$\text{repeat\_interval} \ge \text{min\_hold\_us} + \text{delivery\_budget} + \text{off\_gap\_budget}$$

`delivery_budget` is a conservative sender-side estimate, not a claim about when the game
observes the key. `off_gap_budget` is profile- and frame-dependent and is intentionally not a
universal constant. A schedule can therefore be authored-feasible while still producing a
runtime conflict under a slow or contended input path; fidelity-oriented tooling should warn or
reject such repeats instead of silently treating them as guaranteed.

The native session requires a resolved `game_fps` in the inclusive range
15..=240. At runtime its physical floor is at least one nominal frame plus the
initial 500 µs jitter guard, unless the configured floor is larger. This is a
sender-side visibility safeguard, not proof that the game observed the key.
A packet that arrives one or more frames late shifts the effective timeline
forward by its lateness, preserving relative packet spacing and avoiding
overdue catch-up bursts. Note-on timestamps are never rounded to frame phases.

If the authored interval is smaller than `min_hold_us`:
1. **Strict Mode:** The scheduler rejects the playback and recommends a lower tempo.
2. **Runtime invariant failure:** The compiler rejects authored overlap; if a conflict appears at
   runtime anyway, the worker aborts fail-closed. It never reports `finished` after dropping a
   chord or a late pulse.

---

## 3. The Completion-Anchor Contract
To enforce the sender-side minimum-hold floor despite measured OS dispatch latency, key releases are scheduled relative to down-dispatch completion rather than down-dispatch start. This does not prove when the game sampled the key.

The authored/API representation of the runtime visibility contract is:
$$\text{release\_not\_before\_us} = \text{down\_dispatch\_completed\_us} + \text{min\_hold\_us}$$
$$\text{effective\_release\_us} = \max(\text{scheduled\_release\_us}, \text{release\_not\_before\_us})$$
The native implementation evaluates the same contract in checked tick arithmetic, using the
exact SendInput completion boundary and a precomputed `DurationTicks` hold floor.

### Rationale
The sender-side completion-to-completion proxy preserves the intended hold floor: measuring from down-dispatch start would subtract the down injection latency from the sender timeline. For `local_precise` at 144 FPS (6.94 ms hold), this avoided the previously observed sender-side shortfall. Completion-anchoring does not establish a game-observed hold, because game sampling and kernel delivery are not instrumented here. The constant `min_hold_margin_us` models residual sender-to-device delivery latency; it is a margin, not game-onset evidence.

### Interaction with Adaptive Dispatch Lead (2026-08)
Dispatch targets a **sender-completion timing proxy**: events are admitted at a
logical deadline and carry an absolute QPC target computed from the same clock
sample. The lead is the nearest-rank p95 of a fixed rolling window of
`dispatch_cost_us`, bucketed by physical send path (`down_only`, `up_only`, or
`mixed`) and event count. It is monotonic by event count, capped by the
configured maximum, and has no runtime Hot/Cold classification or correction
loop. This is not a claim about game input sampling, rendering, or audio onset.
The lead is symmetric for down and up paths; the hold/release floor still wins.
Completion before the physical target is a terminal timing error rather than a
negative training sample. See [rt-dispatch-architecture.md](rt-dispatch-architecture.md).

One worker loop epoch resolves exactly one immutable `NextDispatchPlan`
(`worker/planning.rs`): it computes the path-aware authored lead once, freezes
health budgets, forms the pending-release cohort lead once, and derives a single
wait deadline and physical QPC target from those same plans — so prepare-due and
wait-until can never disagree on lead selection. A normal
dispatch deadline wake reuses that plan directly; it does not restart the full
worker epoch, drain observers, recalculate lead, or rebuild orchestration before
physical dispatch. Interrupts, commands, focus/pause transitions, backend calls,
and release-recovery changes still invalidate it. The planner does not mutate
the coordinator, sample QPC itself, allocate, or format strings on success.
Pending releases use a bounded cohort fixed point: the Up lead is selected from the releases that
share the next effective deadline, rather than from all currently pending keys. The resulting
deadline/lead/event-count-cohort plan is reused for waiting and popping. The native accuracy-first path
also requires `chord_stagger_us == 0`; staggered chords remain an explicit Python diagnostic path.
A future physical anchor is created before the worker loop; the first authored action is gated at
`startup_anchor + scheduled_us - lead_us`, including the negative offset for a note at `t=0`.
The exact QPC boundary established by that startup wait is carried through the first physical
authored send; it is not reconstructed from a later epoch sample or replaced with the wake time.
Because the logical timeline is unsigned, later authored timestamps smaller than the requested
lead do not saturate to the same zero deadline: the coordinator temporarily applies no early
lead to those sub-lead actions, preserving their order. Only the first action receives the
startup-anchor negative offset.
The worker records a separate Up completion residual only from clean, non-deferred, single-source
release cohorts.
Normal estimator operation uses the clamped rolling p95; native strict timing
uses the clamped rolling maximum so an observed upper-tail sample remains
visible. Sparse buckets fall back to the nearest seeded lower-cardinality bucket,
then the nearest seeded higher-cardinality bucket, then zero; there is no
cross-path contamination or path prior. The separate lead-saturation diagnostic
arrays use exact event-count indices 1..14 and an explicit `15_plus` bucket at
index 15; this diagnostic compression does not change the estimator's exact
1..30 event-count buckets. Repeated positive residual at the lead cap is a
controlled timing error rather than an unreported
tail.
For authored observations, `applied_lead_ticks` comes from the coordinator's prepared batch,
not from the requested estimator lead. For release observations, it is derived from the
scheduled release minus the effective physical deadline. A completion/ownership hold floor or
retry floor therefore reports applied lead zero. `lead_up_saturated` is true only when the
lead-adjusted deadline itself controls the physical target and the applied lead still equals the
estimator-capped maximum; `saturated_positive` is derived separately from the effective SendInput
completion error, not from authored-timestamp lateness. This prevents floor-deferred work from
building a false positive-at-cap saturation streak.

Native `strict_timing` is a completion contract in addition to a dispatch decision. The Rust
boundary forces same-key conflicts to `AbortPlayback`, regardless of the caller's parsed
legacy conflict-policy input. A clean, single-attempt Down and a clean, non-deferred, single-source Up
must satisfy `strict_down_completion_late_us` and `strict_up_completion_late_us` (2,000 µs by
default) after `SendInput` returns. A violation is recorded as
`strict_completion_slo_exceeded`, followed by full cleanup and a controlled error. Deferred or
mixed release cohorts are not compared with their authored timestamp because their effective
release target is different.

Runtime lead selection has no Hot/Cold state. The independent calibration
harness may still report Hot/Cold delivery-proxy classes for diagnostic
comparison, but those classes do not enter production planning, estimator
state, health budgets, or telemetry semantics.

At a normal timer deadline, the worker keeps the immutable plan and enters the
physical path directly. The last-mile order is fresh command/lease admission,
optional Down target/focus admission, `SendInput`, ownership reconciliation,
and fixed raw observation enqueue. UpOnly and pending-release traffic uses
control plus lease admission but never a focus gate. Interrupts and lease-only
wakes replan instead of dispatching the stale plan.

The production controller has one target: `SendInput` completion at the
effective scheduled timestamp. It learns only clean sender-side dispatch cost;
it does not apply integral correction. Raw Input calibration remains an
app-owned host delivery proxy for diagnostics only; it is not part of the
adaptive lead, scheduler timestamps, hold duration, or any game-observed timing
claim. The current estimator state is version 12 and v11 or other versions are
rejected without migration. Its exact JSON shape is
`{version:12,max_events:30,down:[{samples:[]}...31],up:[...31],mixed:[...31]}`.

---

## 4. Hold Selection Guidance

The supported selections are exactly `1.0`, `1.25`, and `1.5` frames. `1.0` is the default and
maximizes same-key repeat room; `1.25` adds a moderate visibility cushion; `1.5` is the longest
supported hold and can help when registration reliability matters more than compact repeats.
Select the same FPS configured inside Sky. The app does not inspect or correct Sky's FPS.

### Production vs Strict Timing Diagnostic

The native production dispatch mode prioritizes continuity when the sender still reports
complete input operations, while recording timing degradation in telemetry.
`strict_timing` is a diagnostic mode: it prioritizes completion-SLO enforcement
and ends the session after the configured timing contract is exceeded. It is
not the production default and must not be treated as game-observed receipt.

---

## 5. Investigation Findings & Historical Validation

### 2026-06-06 Investigation Summary
1. **Sender Dispatch is Clean:** Extended test sweeps (88 real songs under varying FPS and send durations) resulted in **0 notes dropped** on the sender side (`dropped_conflict`). Note drops only occur on synthetic test cases deliberately authored below the frame duration.
2. **Real Songs Do Not Hit the Same-Key Floor:** The minimum same-key interval across the entire song corpus is **76 ms** (in the song `blue`), with a P50 of ~996 ms. Zero transitions occur below 70 ms. Consequently, same-key floor compression is not a cause of note loss in normal gameplay.
3. **Consistency of Hold Selection:** Reloading, changing, or persisting hold selections results in identical round-trip calibration values (e.g., exactly 6945 $\mu\text{s}$ at 144 FPS), proving that config persistence is robust.
4. **Game FPS Toggle is Not a Workaround:** Early reports suggested toggling the game FPS (e.g. 144 $\rightarrow$ 60 $\rightarrow$ 144) resolved missing notes. Controlled testing showed this is not a reliable fix and likely only resets a volatile game focus/timing state. Missed notes at high FPS are due to game-side sampling phase alignment or runtime thread scheduling delays, not scheduler math.
5. **Hardened Input Path:** Robustness changes include re-acquiring the active game window handle on play, enforcing a 1 ms timer guard in the dispatch thread, and enabling diagnostic startup telemetry under `PLAYBACK_DEBUG`.

---

## 6. Metric Honesty (2026-07-18)

Telemetry metrics are sender-side proxies, **not** game-onset ground truth:

| Metric | Means | Does **not** mean |
|--------|--------|---------|
| `actual_us` | Timeline when backend call began (`T_call_entry`) | Game sampled the key (`T_game_observed`) |
| `send_completed_us` / `dispatch_completed_us` | `perf_counter` after `SendInput` returned (`T_call_return`) | Kernel delivered key; game polled (`T_game_observed`) |
| `visible_lateness_us` | `send_completed_us − scheduled_us` (sender-side completion error) | Game-onset error |
| `observed_hold_us` | Completion-to-completion on sender timeline | Game-visible hold |

The summary JSON includes `timing_semantics.onset_definition = "sendinput_return"` and
`game_observed.available = false` until WASAPI/onset evidence is explicitly attached (Phase J).
Do **not** treat `visible_lateness_us ≈ 0` as proof the game received the note on time.

---

## 7. Accuracy Improvements (2026-08 dispatch-cost estimator)

The native `DispatchCostEstimator` persists exact sender-side completion-cost
windows to `.cache/lead_estimator.json` through a Python envelope schema 2.
The native state is schema 11 with three path-isolated arrays (`down`, `up`, and
`mixed`), 31 buckets for the default maximum event count of 30, and a fixed
rolling window of 32 samples. Each bucket starts with five seed samples; the
nearest-rank p95 is used for the lead. Live observations are clamped to the
maximum sample bound, while persisted values are rejected if invalid. Corrupt,
version-mismatched, or structurally incompatible state is discarded without
entering playback; there is no v10 migration.

The estimator is deliberately separate from health budgets. Health uses its
fixed sender floor and hysteresis windows, while the estimator learns only
clean, canonical dispatch observations. A missing post-send metric therefore
cannot become a zero-cost training sample.

### Wait and spin policy
High-resolution waitable timers and the bounded final spin remain the complete
production wait mechanism. The worker computes an absolute physical target QPC
before either due return and carries that target through admission, `SendInput`,
and deferred observation. The startup spin probe remains separate from dispatch
cost estimation.

### Mid-song spin re-probe
The native worker performs only the bounded startup probe. Mid-song reprobe was removed from the
native precision window; the legacy compatibility option is ignored by that worker. The Python
oracle may retain its diagnostic-only path for rollback tests, but it is not part of the Rust
production timing contract.

Python does not own native sleep, wait, or spin tuning. Supervisor polling is an application-side
control concern; the Rust worker owns its precision wait policy and its effective spin threshold.

---

## 8. Appendix: Retired Knobs
To clean up the codebase and reduce scheduling overhead, several historical timing knobs were completely removed in June 2026 after empirical testing proved they had no beneficial impact on real playback.

For historical context and audit details of these knobs, refer to the archived documents in [archive/](archive/):
* **`input_lead_us`:** Retired because the player generates its own timeline with no external clock reference. A uniform shift is unobservable. See [timing-architecture-audit.md](archive/2026-06_timing-architecture-audit.md).
* **`chord_merge_window_us`:** Retired because real songs do not contain notes clustered within 5–20 ms; they are either simultaneous or $\ge 100\text{ ms}$ apart. See [timing-experiments.md](archive/2026-06_timing-experiments.md).
* **`frame_align` & `down_only`:** Snapping events to the player's frame grid is useless because the game samples on its own unsynchronized render loop. Snapping introduced offset errors without increasing capture.
* **`release_gap_us` & `repeat_release_gap_us`:** Removed after corpus audits showed they did not bind on real songs and only inflated scheduler complexity. Same-key repeats are now governed purely by the `min_hold_us` constraint.
