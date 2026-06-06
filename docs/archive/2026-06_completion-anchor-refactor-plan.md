> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: chứa mô tả release_latency_margin_us = 500µs và start-anchor đã cũ, nay trái code (đã gỡ margin).

# Refactor Plan — Completion-Anchor Visibility Floor (Plan C′)

> Status: APPROVED for implementation. Author of plan = reviewing AI (acceptance/nghiệm thu).
> Implementer = a separate AI executing this spec cold. Read this whole file before editing.
> Single source of project rules: `AGENTS.md`. Timing rationale: `docs/timing-principles.md`.

## Locked decisions (do not relitigate)
- Anchor: single **completion-anchor**. No dual-floor, no forced-early-release. (Owner approved.)
- `release_latency_margin_us` = **fixed 500µs constant** (configurable field, default 500). Do NOT
  derive it dynamically from telemetry — no real song is anywhere near the cutoff, so dynamic logic
  buys nothing. (Owner approved.)
- `local_precise` profile value stays **1.0 frame**. Do not bump to 1.05/round().

## 0. TL;DR

Today the runtime anchors a note's release floor to the **down dispatch START**
(`runtime_dispatch.py:236`: `release_not_before_us = dispatch_started_us + min_hold_us`).
The game does not observe start-to-start; it observes **inject-to-inject** (completion-to-completion).
Because each `SendInput` call has its own latency, the game-observed hold is
`min_hold + (up_send_dur − down_send_dur)`, centered exactly on 1 frame, so **~50% of notes land
below 1 frame** and drop probabilistically in-game (continuous light note loss).

**Fix:** anchor the release floor to the down dispatch **COMPLETION**:
`release_not_before_us = dispatch_completed_us + min_hold_us`. Then observed hold =
`min_hold + up_send_dur ≥ 1 frame` for every note, with minimal excess (~one up-latency, ≈90µs).
To keep the contract *scheduler-feasible ⇒ runtime-feasible* honest under nonzero injection latency,
raise the scheduler's same-key feasibility cutoff from `interval < min_hold` to
`interval < min_hold + release_latency_margin_us`.

Do **not** build the dual-floor / forced-early-release machinery from the earlier draft plan — it is
overengineering (see §2 corpus evidence; it would never fire on real songs).

## 1. Why (measured, not theoretical)

Real telemetry, 4 runs of `songs/blue.json` (WinSendInput, 2026-06-06, `logs/playback_telemetry_0243*`):

- `send_duration_us` (host injection cost per SendInput call): mean **104µs**, p50 91, p95 **228**,
  p99 245, max **250µs**; right-skewed; independent of game FPS.
- `lateness_us` (wake error, spin-precise): mean 0.7, p95 0, max 211 (rare).
- Game-observed hold (down_completed → up_completed) @144fps: min **6727µs = 0.969 frame**,
  median 6945, **49.6% of notes below 1 frame**. @60fps shallower (min 0.987 frame) → that is why
  144 is hit harder: the same absolute jitter is a larger fraction of the shorter 6.945ms frame.

`local_precise @144` materialises `hold == min_hold == ceil(1×6945) = 6945µs` (1.0 frame, zero
headroom): `config.py` `local_precise` has `min_hold_frames: 1` and omits `hold` (so hold inherits
min_hold via `scheduler_types.py` `from_dict`).

## 2. Corpus evidence (why C′, not dual-floor)

Ran the real scheduler over all 115 files in `songs/` at `local_precise @144`, tempo 1.0:

- `impossible_same_key_repeats = 0`, `risky = 0`, `compressed = 0` for **every real song**.
- Shortest same-key interval in any **real** song = **75ms** (Flower Dance), ~11× min_hold. Next:
  blue 76ms, Counting Star 80ms, all others ≥85ms. Even at tempo 2× the shortest real interval is
  ~38ms (>5× min_hold). Reaching the fragile band (`interval ≈ min_hold`) needs tempo >10×.
- The only sub-75ms entries are synthetic `TEST_repeat_*` probes (7/17/20/24ms) built to stress the
  same-key floor. Exactly one (`TEST_repeat_floor_144` = 7ms) sits in the band, and it is a fixture.

Conclusion: the same-key conflict path is irrelevant to production content. C′ treats
`interval < min_hold + margin` as **honestly infeasible** (with nonzero injection latency you cannot
both hold a full frame AND release before the next press) and routes it through the existing
`impossible_same_key_repeats` degraded/strict handling. No new hot-path branching.

## 3. Priorities (in order) the change must satisfy

1. **Timeline correctness** — every `down` still dispatches at its authored time; releases never push
   any `down` late; no per-note slowdown accumulation.
2. **No note loss** — game-observed hold ≥ 1 frame for every feasible note (visibility), and no
   feasible same-key repeat is dropped.
3. **Hug 1 frame** — keep `min_hold` profile value at exactly 1.0 frame (do NOT bump to 1.05). The
   headroom comes from the completion anchor (≈ one up-latency, ~90µs median), not from inflating the
   profile. Excess hold must stay minimal.

## 4. Implementation steps

### 4.1 Runtime: switch the anchor (core change)
File `src/sky_music/orchestration/runtime_dispatch.py`, `activate_sent_downs` (~line 199-238).

- Change the floor:
  ```
  release_not_before_us = dispatch_completed_us + self.min_hold_us
  ```
  (was `dispatch_started_us + self.min_hold_us`.)
- Keep both `down_dispatch_started_us` and `down_dispatch_completed_us` on `ActiveKeyGeneration`
  (already stored) — `started` is still needed for the telemetry label rule in §4.4.
- Rewrite the long comment block (currently lines ~221-236) that justifies the START anchor. New
  rationale: the game observes completion-to-completion; the down's own SendInput latency would
  otherwise be subtracted from the observed hold, pushing ~50% of notes below one frame (measured,
  §1). Completion-anchor makes observed hold = `min_hold + up_send_dur ≥ min_hold`. Same-key
  feasibility is preserved because the scheduler now requires `interval ≥ min_hold + margin` (§4.3),
  which bounds the extra `down_send_dur` the completion anchor adds. Cross-reference
  `docs/timing-principles.md §7` (which must also be updated, §4.6).

`PendingRelease.effective_release_us` (`max(scheduled_release_us, release_not_before_us)`) is
unchanged structurally; it now carries the completion floor.

### 4.2 No dual-floor, no forced-early-release
Do not add a second floor, `force_release_for_due_downs`, or a `forced_early_release` outcome. The
genuinely-infeasible tail keeps the existing degraded `dropped_conflict` behavior in
`split_down_intents` / `_dispatch_down_batch`.

### 4.3 Scheduler: honest feasibility margin
File `src/sky_music/domain/scheduler.py` + `scheduler_types.py`.

- Add a margin used only for the same-key **feasibility cutoff** (NOT added to the hold value):
  `release_latency_margin_us`, default **500** (covers measured max send_duration 250µs + lateness
  slack ~211µs, rounded up). Make it a field on `FrameTimingPolicy` (default 500) so it is
  configurable; thread it from config like other policy fields. Do not change `min_hold`/`hold`.
- In `plan_same_key_hold`: the "severe/infeasible" cutoff becomes
  `max_hold_us < min_hold_us + feasibility_margin_us` (was `< min_hold_us`). The returned hold value
  still never goes below `min_hold_us`; the moderate-compression band logic is unchanged.
- In `_recommended_tempo_scale_for_repeats`: use `min_cycle_us = min_hold_us + feasibility_margin_us`
  as the cycle so the strict-mode tempo recommendation matches the new cutoff.
- `build_key_actions` keeps emitting `impossible_same_key_repeats` + the `impossible_repeat`
  diagnostic for intervals now below the raised cutoff; degraded preserves min_hold and reports the
  overlap; strict raises `ScheduleBuildError` (unchanged mechanism).

### 4.4 Telemetry ripple: keep the `deferred_release` label honest
File `src/sky_music/orchestration/engine.py`, `_dispatch_pending_releases` (~line 394-441) and the
`deferred_by_us`/`runtime_outcome` computation (~411-435).

Problem: with completion-anchor, `effective_release_us − scheduled_release_us ≈ down_send_dur (~90µs)`
for **every** note (because authored hold == min_hold for local_precise). Naively this would label
every release `deferred_release` and skip its counter in `observe_result` (engine.py:591-600),
flooding telemetry.

Fix: decide the `deferred_release` label from whether the note was **genuinely compressed** — i.e.
its authored hold was below the visibility floor — using the START-anchored comparison:
`is_deferred = scheduled_release_us < (down_dispatch_started_us + min_hold_us)`.
For a normal local_precise note `scheduled_release_us == down_start + min_hold` (not strictly less)
→ label `sent`. Only authored-sub-min_hold notes get `deferred_release`. Keep using the
completion-anchored `effective_release_us` for the actual release **timing**. Expose
`down_dispatch_started_us` to this computation (carry it on `PendingRelease` if not already reachable).

### 4.5 Telemetry: record the game-observed (completion-to-completion) hold
File `src/sky_music/orchestration/telemetry.py` (around the hold metrics, ~line 194-277).

Add (or repurpose) a metric `observed_hold_us = up_dispatch_completed_us − down_dispatch_completed_us`
per matched generation, and a `observed_hold_below_frame_count` = number of feasible notes whose
`observed_hold_us < frame_us`. This is the **primary acceptance metric** (§6). Keep existing
`note_hold_duration_us` and `confirmed_hold_lower_bound_us`/`confirmed_hold_shortfall_count` for
continuity.

### 4.6 Docs
- `docs/timing-principles.md §7` — update the "Anchor correction (2026-06-05)" box: the floor is now
  measured from down **completion**, with the measured reason (the down's own injection latency is not
  cancelled per-note; start-anchor left ~50% of notes below one frame on real songs). State that
  same-key feasibility is preserved by the scheduler margin (`interval ≥ min_hold + margin`), not by
  the anchor. Note that `interval == min_hold` is now honestly infeasible.
- `docs/runtime-hold-refactor-plan.md` — add a short superseding note pointing here.

## 5. Tests (write/adjust)

Add a RED-first visibility test before implementing, then make it green:

1. **NEW `test_observed_hold_never_below_one_frame_under_asymmetric_send_latency`**
   (`tests/test_runtime_dispatch.py`). Use a backend whose `key_down` SendInput is slow (~250µs) and
   `key_up` is fast (~20µs). Play a non-repeat sequence + a chord. Assert every matched
   `up_completed − down_completed ≥ min_hold`. MUST be RED on current start-anchor, GREEN after.
2. **UPDATE `test_scheduler_feasible_repeat_is_runtime_feasible_invariant`** (line 275). Change the
   `extra` sweep so all intervals are `≥ margin` (e.g. `(500, 700, 1000, 2000)`); add an assertion
   that `observed_hold (comp→comp) ≥ min_hold` for both notes. Contract is now
   "scheduler-feasible (interval ≥ min_hold + margin) ⇒ never dropped AND observed ≥ 1 frame".
3. **RETIRE/REPLACE `test_same_key_repeat_at_min_hold_floor_presses_on_time_with_send_latency`**
   (line 244). Its premise (`interval == min_hold` must press on time) is intentionally dropped in C′.
   Replace with `test_repeat_at_exactly_min_hold_is_flagged_infeasible`: assert the scheduler reports
   `impossible_same_key_repeats >= 1` (degraded) / `ScheduleBuildError` (strict) for `interval == min_hold`.
4. **KEEP `test_repeat_clean_ground_truth_song_never_drops_end_to_end`** (line 313). Its intervals
   (≥20ms) remain feasible under margin 500µs. Strengthen: also assert
   `observed_hold_below_frame_count == 0`.
5. **KEEP `test_deferred_release_does_not_delay_unrelated_down`** (line 354) — verifies priority 1
   (timeline). Confirm it still passes; the unrelated key's down must fire at its authored time.
6. **AUDIT** every test asserting `runtime_outcome in {sent, deferred_release}` or counting
   `deferred_release` / `confirmed_hold_shortfall_count`; reconcile with the §4.4 label rule. Grep:
   `deferred_release`, `runtime_outcome`, `confirmed_hold`, `note_hold_duration`.

Run the full suite: `uv run pytest -q`. Lint/type per `AGENTS.md` (`uv run` only).

## 6. Acceptance gates (the reviewer will run these)

A. **Corpus visibility gate (centerpiece).** A script loads every file in `songs/` via
   `parse_song_file`, builds with `FrameTimingPolicy.local_precise(fps=144)` and `(fps=60)` tempo 1.0,
   runs the real `PlaybackEngine` against a backend injecting **worst-case asymmetric** latency
   (down ~250µs, up ~20µs) on a `FakeClock`, and asserts for EVERY song:
   - `observed_hold (down_comp→up_comp) ≥ frame_us` for all feasible notes (0 below frame);
   - `runtime_conflict_dropped_down_count == 0`;
   - `sent_down_count == feasible note count`.
   This must pass for all 115 files (real songs have huge slack; none should be flagged infeasible at
   tempo 1.0).
B. **Invariant gate.** Updated test #2 green: feasible repeats never dropped; observed ≥ 1 frame.
C. **Honest-infeasible gate.** Test #3 green: `interval == min_hold` flagged infeasible (not silently
   dropped, not falsely "pressed on time").
D. **Timeline gate.** Test #5 green + a check that no `down`'s `actual_us` exceeds its `scheduled_us`
   by more than send latency (no release-induced down delay; no cumulative drift).
E. **Minimal-excess gate (hug 1 frame).** Median `observed_hold_us` ≤ `frame_us + p95(send_duration)`
   (≈ frame + 230µs). I.e. the anchor adds only ~one up-latency, not a frame of body.
F. **Full suite green**, no type/lint regressions.

## 7. Out of scope (do not do)
- Do not change profile numbers (`local_precise` stays 1.0 frame). Do not reintroduce 1.05/round().
- Do not add dual floors, forced-early-release, or a `forced_early_release` telemetry outcome.
- Do not touch the SendInput batching (`platform/win32/inputs.py`) or chord grouping — chords are
  already atomic and correct.
- Do not modify game files / read game memory / change CLI behavior (AGENTS.md hard constraints).
