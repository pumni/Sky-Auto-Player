# Timing Principles for Sky Music Player

This document is the engineering source of truth for designing, reviewing, and calibrating timing profiles for Sky Music Player.

It is intentionally a principles document, not an implementation document. It should describe what timing profiles must guarantee, why those guarantees matter, and how future changes should be evaluated before they are exposed to users.

The goal is not to make playback appear fast in logs. The goal is reliable note registration inside the game and, when playing online, reliable audibility for other players.

**Related docs:** `timing-experiments.md` (the experiments that prove Appendix A and the open
calibration work), `timing-profile-frame-model.md` (the frame-relative representation in code),
`timing-architecture-audit.md` (the 2026-06 audit + 3-phase refactor that removed dead knobs), and
`scheduler-core-architecture-plan.md` (the follow-up scheduler contract/refactor plan).

> ⚠️ **ARCHITECTURE UPDATE (June 2026).** A measurement-driven audit removed four timing knobs
> that were stated here but had no real, necessary effect (see `timing-architecture-audit.md`):
> **`input_lead`** (architectural no-op — the player generates the whole timeline with no external
> reference, so a uniform shift is unobservable; proven, then removed), **`chord_merge`** (effectively
> never fires on real songs), and **`frame_align`** (off in every profile, and pointless because the
> game resamples on its own frames), **`release_gap`** (near-zero real-song binding; removed as a
> profile/scheduler lever), and **`repeat_release_gap`** (real mechanism candidate, but not a
> reachable production playback lever in the current corpus/policy model; removed from profile,
> CLI, and runtime policy semantics). The principles tied to the removed knobs — **§9 General
> Release Gap, §10 Chord Batching, §11 Input
> Lead, §15 Frame Alignment** — are now **RETIRED** (kept with a banner, not deleted, to preserve
> section numbering and cross-references). The current production timing model exposes
> **hold/min_hold** only; same-key repeats are feasible when authored spacing is at least
> `min_hold_us`, otherwise strict mode rejects and degraded mode reports the overlap.

---

## 1. Scope

This document defines the timing principles for:

- local playback on the same machine;
- dense song playback with high note pressure;
- online room playback where other players need to hear the notes correctly;
- frame-aware timing behavior;
- profile review and calibration rules.

The values used by actual profiles are conservative engineering defaults. They are starting points, not universal truths. Any change to a profile must still be validated through real gameplay.

Sky Music Player should assume that the game samples input on frame boundaries and that online playback is less forgiving than local playback. A timing profile that sounds correct locally may still be too aggressive for remote listeners.

---

## 2. Core Terms

All timing values are expressed in microseconds.

| Term                        | Meaning                                                                                               |
| --------------------------- | ----------------------------------------------------------------------------------------------------- |
| hold_us                     | Effective key-down duration for a normal note after profile materialisation and overrides.            |
| hold_frames                 | Local frame-visibility margin for normal holds.                                                       |
| hold_floor_us               | Absolute lower wall/target for normal holds.                                                          |
| min_hold_us                 | Effective minimum key-down duration after compression.                                                |
| min_hold_frames             | Local frame-visibility margin for compressed holds.                                                   |
| min_hold_floor_us           | Absolute lower wall/target for compressed holds.                                                      |
| ~~repeat_release_gap_us~~       | **REMOVED (June 2026)** — was a requested same-key up-time target; not reachable as a production lever. |
| ~~repeat_release_gap_frames~~   | **REMOVED (June 2026)** — former frame margin for same-key repeat up-time.                           |
| ~~repeat_release_gap_floor_us~~ | **REMOVED (June 2026)** — former absolute lower wall/target for same-key repeat up-time.             |
| ~~input_lead_us~~           | **REMOVED (June 2026)** — was "how early input is sent"; proven a no-op, deleted from code.            |
| ~~chord_merge_window_us~~   | **REMOVED (June 2026)** — was the chord-snap window; never fired on real songs, deleted from code.     |
| same_key_interval_us        | Time between two down events for the same scan code; below `min_hold_us`, the repeat is infeasible.   |
| frame_us                    | Duration of one game frame. At 60 FPS, one frame is about 16.67 ms.                                   |
| game_fps                    | The FPS value selected or calibrated by the user. A value of 0 means frame-aware scaling is disabled. |
| tempo_scale                 | Playback speed multiplier. Values above 1.0 increase scheduling pressure.                             |

---

## 3. Design Assumptions

> Note: As of June 2026 the central assumption below — that the game samples input
> on frame boundaries — is no longer just an assumption. It has been confirmed by
> controlled in-game measurement, together with concrete timing floors. See
> **Appendix A — Empirical Validation** for the method, results, and the floors that
> now govern profile design. Where a measured floor differs from an earlier
> rule-of-thumb in this document, the measured value governs.

Sky Music Player timing should be designed around these assumptions:

1. The game may not observe every injected input event immediately.
2. The game may sample input state only once per frame.
3. A note can be logged by the scheduler but still fail to appear in-game if the down or up state is too short.
4. Same-key repeats are more fragile than different-key transitions.
5. Online listeners are less forgiving than the local player because remote audibility may depend on replication timing, network jitter, client-side batching, and frame sampling on another machine.
6. Dense songs create scheduling pressure and may require safer timing rather than shorter timing.
7. Safety floors should be raised when reliability is poor; they should not be lowered just to make playback faster.

---

## 4. Profile Classes

Profiles are not all validated under the same assumptions. A profile must clearly state what kind of playback it is meant for.

### 4.1 General 60 FPS-Safe Profiles

A general 60 FPS-safe profile may be used at normal 60 FPS and must remain safe when frame-aware scaling is disabled.

These profiles prioritize reliable local capture while keeping playback responsive.

Examples:

- local_precise
- balanced
- dense_safe
- audience_safe

A general 60 FPS-safe profile must keep `min_hold_us` above one game frame after materialisation.
At 60 FPS, one frame is approximately 16.67 ms. Same-key repeats authored closer than `min_hold_us`
are not fixable by a profile gap knob; they require slower tempo, different arrangement, or explicit
strict/degraded handling.

### 4.2 Local Precise Profiles

Local precise profiles optimize for sharper local playback.

They may use shorter holds and smaller gaps than audience profiles, but they must still preserve reliable local note registration.

A local precise profile is not automatically suitable for online rooms. If a profile is tuned mainly by what the local player hears, it should be treated as local-first.

### 4.3 Dense Playback Profiles

Dense playback profiles are designed for songs with high scheduling pressure.

They should reduce collapse and missed notes by balancing:

- sufficient same-key release time;
- reasonable visible hold duration;
- a safer profile or lower tempo when the authored note pressure exceeds the same-key cycle.

Dense playback should not be solved by blindly lowering min_hold_us. If density causes misses, safer spacing is usually better than shorter spacing.

### 4.4 Online Audience Profiles

Online audience profiles optimize for what other players hear, not only what the local player hears.

Online audience playback should use:

- larger visible holds;
- stronger protection against dense timing collapse.

audience_safe is the recommended profile class for online rooms. After the June 2026 refactor it
differs from local profiles **only** through higher hold/min_hold floors (no longer
through input lead or chord merge, which were removed). Whether those wider floors are actually
needed remote is still open — see `timing-experiments.md` O3/O4.

If local playback sounds correct but other players miss notes, the timing should be treated as insufficient for online audience playback.

### 4.5 Why there is no dedicated high-FPS profile

An earlier experimental `high_fps_precise` profile assumed that higher render FPS lets timing
go uniformly sharper (shorter holds, shorter gaps, sub-frame margins). Measurement disproved
that premise (Appendix A.10 / EXP-2): higher render FPS sharpens **single-note visibility
only**; onset cadence can show phase-dependent internal bucket jumps that do not scale with render
FPS. A separate high-FPS profile therefore bought nothing the frame
model does not already provide, and was removed.

High-FPS users are served by the normal profiles: every profile is frame-aware, so its
visibility holds already shorten with FPS via the `*_frames` term, while audience remote margins
correctly stay constant. `local_precise` in particular is pure frame-relative (zero hold floor) and
is the sharpest option at high FPS.

---

## 5. Principle 1 — Frame Capture Matters More Than Scheduler Logs

A scheduler log can show that a key was pressed and released correctly, while the game still fails to observe the state change.

This happens because the game may only sample input state at frame boundaries. If the key-down or key-up state exists for too short a time, it can fall between samples.

Therefore, timing profiles must be judged by in-game behavior, not by logs alone.

Reliable playback requires that important state transitions survive long enough to be observed by the game.

---

## 6. Principle 2 — Same-Key Repeats Must Not Overlap

Same-key repeats require a complete state transition:

DOWN, visible hold, UP, DOWN again.

The critical production invariant is:

same_key_interval_us must be at least min_hold_us.

If the authored same-key interval is below `min_hold_us`, the scheduler cannot both preserve the
visibility floor and release before the next down. Degraded mode keeps `min_hold_us` and reports the
overlap; strict mode may reject instead.

A same-key repeat can be dropped, merged, or heard as incomplete if the game does not observe a complete down-up-down sequence.

For any profile that may be selected at the current FPS:

min_hold_us must be greater than frame_us.

For profiles exposed as generally safe at 60 FPS:

min_hold_us must be greater than 16.67 ms.

For online audience playback, `min_hold_us` should usually be more conservative than the local
minimum. Online reliability should target remote survivability, not merely one-frame local capture.

---

## 7. Principle 3 — The Visibility Rule

min_hold_us must be long enough for the game client to observe the key as down.

A short hold may feel attractive for dense songs, but it can make notes vanish if the game does not sample the down state in time.

Do not lower min_hold_us only to make dense songs faster.

If notes vanish, first consider:

1. increasing min_hold_us;
2. using a safer profile;
3. reducing tempo_scale.

For online audience playback, min_hold_us should be more conservative than local-only playback.

---

## 8. Principle 4 — Same-Key Release Gap Is Not A Production Knob

The game may require an up state before repeated notes on the same key. However, the former
`repeat_release_gap_us` field did not prove itself as a reachable production schedule lever and has
been removed from profile/CLI/runtime policy semantics.

Same-key repeats are fragile because the game must see the key return to an up state before it can treat the next down state as a new note.

The current scheduler handles same-key repeats by shortening the previous hold only when the next
same-key down arrives before the target hold. It never shortens below `min_hold_us`.

If repeated notes drop under the current policy, first reduce tempo or inspect the authored same-key
interval. There is no production repeat-gap knob to tune.

---

## 9. Principle 5 — General Release Gap ~~Still Matters~~ **[RETIRED — June 2026]**

> **RETIRED.** `release_gap_us` was removed after corpus/timeline audit showed near-zero real-song
> binding and no reliable production value. Dense Sky songs are dense through chords, 80–150 ms note
> motion, and same-key repeats, not through cross-key downs a few milliseconds after release.
> Same-key behavior is audited separately in O10.4; general release spacing is no longer a profile
> field, CLI flag, or scheduler branch.

The historical text below is kept for context only:

release_gap_us protected general scheduling after a key-up event. It was less critical than
repeat_release_gap_us for same-key repeats, but it was thought to help avoid overly compressed event
streams. The corpus audit did not support keeping it.

---

## 10. Principle 6 — Chord Batching ~~Reduces Pressure~~ **[RETIRED — June 2026]**

> **RETIRED.** `chord_merge_window_us` was removed. Measurement (`timing-experiments.md` O2) showed
> real songs never contain the 5–20 ms note clusters the window targeted — notes are either exactly
> simultaneous or ≥ ~100 ms apart — so the window effectively never fired. Exactly-simultaneous chords
> are still grouped into one SendInput at the final event-grouping step, so chords replicate as before.
> Notes a few ms apart now go out at their own time (a frame-sampler sees them on the same frame
> anyway). There is no chord-merge knob to tune; do not reintroduce one without new evidence.

The historical text below is kept for context only:

chord_merge_window_us controlled how nearby notes were grouped into a chord. Small windows preserved
expressive timing; larger windows reduced scheduling pressure but could flatten intended arpeggios.

---

## 11. Principle 7 — Input Lead ~~Compensates for Delay~~ **[RETIRED — June 2026]**

> **RETIRED.** `input_lead_us` was removed after being proven an **architectural no-op**
> (`timing-architecture-audit.md` §1, `timing-experiments.md` O1). The player generates the entire
> timeline and the playback clock is zero-based to the moment you press play, so a uniform earlier
> shift has nothing to be early *relative to* — it is unobservable, except for a small artifact that
> compressed the very first interval (`max(0, source − lead)`). Sweeping the old `--input-lead-ms`
> 0/8/20 produced identical measured offset. The real cause of any "off-beat" feel is relative scatter
> (the game's ~60 Hz tick jitter, A.10), which a mean shift cannot fix. There is no lead knob; consistent
> lateness is not a player-side tunable.

The historical text below is kept for context only:

input_lead_us sent input before the musical timestamp to compensate for scheduler / injection / OS /
frame / online delay. It was a by-ear value; too little sounded late, too much sounded early.

---

## 12. Principle 8 — Tempo Scale Increases Timing Pressure

tempo_scale affects how much timing pressure the scheduler must handle.

Values above 1.0 make notes closer together. This increases the risk of:

- missed notes;
- dropped same-key repeats;
- collapsed dense passages;
- incomplete remote audibility.

If a song becomes unreliable at a higher tempo scale, do not immediately lower safety floors.

First try a safer profile or reduce tempo_scale.

A profile that is stable at tempo_scale 1.0 may not remain stable at higher speed.

---

## 13. Principle 9 — Online Reliability Wins Over Local Sharpness

Online audience playback must prioritize remote audibility over local sharpness.

A locally sharp profile can still be too aggressive for online rooms.

Symptoms of insufficient online timing include:

- other players missing notes that sound correct locally;
- repeated notes sounding incomplete to other players;
- chords sounding rattly, broken, or uneven remotely;
- dense passages collapsing for listeners while local playback appears acceptable.

When online reliability is the goal, choose audience_safe before trying to manually optimize a sharper local profile.

Online mode should not silently use a local-only profile when the user expects other players to hear the music clearly.

---

## 14. Principle 10 — Frame-Aware Scaling Should Raise Safety, Not Hide Risk

Frame-aware materialisation adapts local visibility margins to the configured game FPS.
For frame-coupled parameters, profiles declare both pieces together:

```
effective_us = max(ceil(frames * frame_us), floor_us)
```

The frame term protects local frame-boundary sampling. The floor term preserves absolute
timing intent such as the same-key wall and online survivability margins.

It should be used to improve stability, not to hide unsafe profile design.

Important rules:

1. If game_fps is 0 or unknown, frame-aware scaling is disabled.
2. Built-in profiles should declare frame-coupled timing as `*_frames` plus `*_floor_us`.
3. Absolute `_us` values are overrides for CLI, calibration, legacy profiles, and targeted experiments.
4. Persist the base profile, calibrated FPS, tempo scale, and calibration values separately.
5. A blocked or experimental profile should not become selectable only because scaling exists.
6. Higher FPS may reduce frame boundary delay, but it does not automatically make all short hold or release values safe.
7. Safety durations should not be reduced just because FPS is higher unless that behavior has been validated in gameplay.

Frame-aware materialisation should protect users from unstable timing, not encourage fragile timing.

---

## 15. Principle 11 — Frame Alignment ~~Must Be Conservative~~ **[RETIRED — June 2026]**

> **RETIRED.** `frame_align` (and `down_only`) were removed. The mode was off in every profile and is
> conceptually pointless: the game samples input on *its own* render frames, which are not synchronised
> to the player's clock, so snapping the send timestamps to the player's frame grid aligns to the wrong
> reference. The safest default ("no frame alignment") is now the only behavior. Adequate hold and
> release durations remain the real protection — see §6, §7, §8.

The historical text below is kept for context only:

Frame alignment aimed to place events closer to expected frame boundaries; aggressive alignment added
bias and could make playback feel late, so the safe default was always "none".

---

## 16. Recommended Profile Intent

The project ships exactly four profiles:

| Profile       | Intended Use             | Online Audience Use | Notes                                                                          |
| ------------- | ------------------------ | ------------------- | ------------------------------------------------------------------------------ |
| local_precise | Sharp local playback     | No                  | Reference profile = the measured floors themselves; pure frame-relative holds. |
| balanced      | General default playback | Limited             | local_precise + a little body. Good default for normal use.                    |
| dense_safe    | Dense local playback     | Limited             | Slightly stronger body floor for note pressure.                                |
| audience_safe | Online room playback     | Yes                 | A little above balanced for remote audibility; currently carried by higher hold floors. |

balanced should remain the general default profile.

audience_safe should be the recommended profile for online audience playback.

dense_safe should be used when density causes collapse but online audience safety is not the main requirement.

local_precise should be used only when local responsiveness matters more than remote reliability.

---

## 17. Symptom-Based Tuning

| Symptom                                                  | Likely Cause                                        | First Adjustment                                          |
| -------------------------------------------------------- | --------------------------------------------------- | --------------------------------------------------------- |
| Same-key repeats drop locally                            | Authored same-key interval is too short             | Reduce tempo; audit repeat-gap reachability before tuning. |
| Notes vanish locally                                     | Hold is too short or FPS is lower than expected     | Increase min_hold_us or use balanced.                     |
| Local playback sounds fine, but other players miss notes | Online timing is too aggressive                     | Use audience_safe.                                        |
| Other players hear repeated notes as incomplete          | Same-key transition may not survive replication     | Use audience_safe/reduce tempo; validate remotely.        |
| Playback sounds consistently late or early               | Game-side bucket/phase behavior or network jitter   | Not a player-side lead fix — see A.10 (input lead was removed). |
| Dense passages collapse                                  | Scheduling pressure is too high                     | Use dense_safe, use audience_safe, or reduce tempo_scale. |
| Local playback feels too soft or mushy                   | Holds or gaps are too large for the use case        | Use balanced or local_precise.                            |

---

## 18. Tuning Order

When playback is unreliable, tune in this order:

1. Confirm the playback intent: local, dense, or online audience.
2. Confirm the selected profile matches that intent.
3. Confirm the configured FPS and whether FPS is stable.
4. Confirm tempo_scale is not creating unrealistic density.
5. For dropped same-key repeats, reduce tempo and inspect O10.4 reachability/binding.
6. For vanished notes, increase min_hold_us.
7. For online audience misses, switch to audience_safe.
8. Only after those steps, consider changing hold_us or defining a new profile.

(Consistent lateness, chord spread, and general post-release spacing are no longer tunable:
input lead, chord merge, and release_gap were removed.
Residual onset scatter is game-side bucket/phase behavior — see A.10 — not a player-side knob.)

Do not reduce safety floors to make playback appear faster.

The priority is reliable registration in the game and reliable audibility for listeners.

---

## 19. Validation Rules for Profile Changes

Any profile change must be reviewed against these questions:

1. What playback intent is this profile for?
2. Is it local-only, dense-safe, general default, or online audience-safe?
3. Does it remain safe at the FPS values where it can be selected?
4. Does it preserve same-key repeat reliability?
5. Does it preserve visible key-down capture?
6. Does it behave acceptably at tempo_scale 1.0?
7. Does it remain reasonable when tempo_scale is increased?
8. Has it been tested in real gameplay, not only scheduler logs?
9. If it is meant for online rooms, have remote listeners confirmed reliability?

A profile should not be exposed as production-ready until these questions have acceptable answers.

---

## 20. Non-Negotiable Rules

1. balanced remains the general default profile.
2. audience_safe is the recommended online audience profile.
3. Online reliability wins over local sharpness when the selected intent is audience playback.
4. Same-key repeat safety claims must demonstrate a reachable schedule change, not only a configured gap value.
5. Any profile exposed at normal 60 FPS must satisfy the 60 FPS cycle rule.
6. Frame-aware scaling must not persist already-scaled hold or gap values as profile defaults.
7. Future profiles must state their class and validation assumptions.
8. Any change that lowers min_hold_us or changes repeat-gap behavior must include a gameplay reason and a validation plan.
9. Logs are not enough. Real in-game behavior is the source of truth.
10. A profile that sounds correct locally is not automatically safe for online audience playback.
11. Render FPS sharpens single-note visibility only; it does not prove onset cadence or same-key repeat mechanisms become arbitrarily faster, so no profile may assume higher FPS makes short holds/gaps safe.

---

## 21. Final Principle

Timing profiles should be conservative where the game is fragile and precise only where reliability has already been proven.

For local playback, responsiveness matters.

For dense playback, pressure control matters.

For online playback, audibility for other players matters most.

When in doubt, choose the profile that makes the game and the audience receive the notes reliably, not the profile that makes the scheduler look fastest.

---

## Appendix A — Empirical Validation (Measured In-Game Behavior)

This appendix records controlled in-game measurements (June 2026) that upgrade the
frame-sampling model from assumption (Sections 3, 5, 6) to measured fact, and that
fix concrete timing floors. Where a measured floor differs from an earlier
rule-of-thumb in this document, **the measured value governs**.

### A.1 Method

Ground truth is the **recorded game audio**, not the scheduler log: `--debug-csv` verifies the
_sent_ side, onset counting on the WAV verifies _registration_. The full method, tooling
(`tests/make_test_song.py`, `tests/analyze_onsets.py`), and critical controls (lock FPS
externally, explicit `--fps 0` when measuring game intrinsics, percussive instrument, count onsets) are
documented in **`timing-experiments.md` §0**, which also proves each result below (Part 1).

### A.2 Result 1 — The game samples input once per render frame (confirmed)

The minimum hold needed for reliable single-note registration scales **linearly with
the frame period**: about one frame at every FPS tested.

| Game FPS | One frame | Measured hold floor |
| -------- | --------- | ------------------- |
| 30       | 33.3 ms   | ≈ 33 ms             |
| 60       | 16.7 ms   | ≈ 17 ms             |
| 144      | 6.9 ms    | ≈ 7 ms              |

Sub-frame holds register **probabilistically** (e.g. a 0.72-frame hold registered
about 72% of notes), exactly as a per-frame rising-edge sampler predicts. Conclusion:
the game reads key **state** on frame boundaries. Frame-aware timing is therefore
correct in principle, not merely a useful heuristic.

### A.3 Result 2 — Visibility floor (hold) = one frame, purely frame-relative

Reliable key-down capture requires `hold ≥ 1 frame`. No fixed-millisecond component
was observed (7 ms suffices at 144 FPS). The measured reliable point is ≈0.96–1.01 frame at
30/60/144 (result.md T1). Encoded standard: built-ins keep a margin above one frame. **Update
(June 2026):** `local_precise` is intentionally the sharp local profile and now sits at exactly
**1.0** frame — the measured floor itself, validated in-game — while the other profiles keep wider
margins (1.2). To guarantee a 1.0-frame floor never lands *below* a real frame, the frame period is
rounded up (`frame_us = ceil(1e6/fps)`), so `local_precise` materialises 33334 / 16667 / 6945 µs at
30 / 60 / 144 FPS.

### A.4 Result 3 — Same-key release-gap floor = max(~1.4 frame, ~17 ms fixed) **[HISTORICAL]**

> **REACHABILITY CORRECTION (June 2026 O10 audit).** The measurements below describe a plausible
> game mechanism under synthetic policies where `hold > min_hold`. They do **not** prove the current
> frame-aware profile field changes playback. Current frame-aware profiles materialise
> `hold == min_hold`, making the scheduler's repeat-gap compression band empty in degraded mode.
> Corpus audit found zero schedule-changing positive real-song intervals through tempo 3.0x.
> Follow-up architecture removed `repeat_release_gap` from profile/CLI/runtime policy semantics.
> Keep this section as mechanism history and counterfactual experiment context only.

The smallest same-key release gap that re-triggered reliably in the recorded synthetic runs:

| Game FPS | One frame | Reliable gap in recorded runs | In frames |
| -------- | --------- | -------------------------- | --------- |
| 60       | 16.7 ms   | ≈ 22–24 ms                 | ~1.3–1.4  |
| 144      | 6.9 ms    | ≈ 14–17 ms                 | ~2–2.5    |

The observed boundary is **neither purely constant in milliseconds nor purely constant in frames**.
It fits a "larger of the two" mechanism model: `gap ≥ max(~1.4 × frame, ~16 ms fixed)`. At high FPS a
fixed-time wall around 16–17 ms appears to dominate (plausibly an internal note/animation cadence near
60 Hz, independent of render FPS — hypothesised, not proven). At a gap of exactly one frame,
reliability was not repeatable in the recorded runs, so mechanism experiments need margin.

Former encoded value: `repeat_release_gap ≥ max(1.5 × frame, ~17000–18000 µs)`. This value has been
removed from profile semantics.

### A.5 The asymmetry between hold and gap

- **Hold (visibility)** floor is a _pure frame multiple_ — no fixed component.
- **Historical repeat gap** measurements looked like a frame multiple plus a fixed-time wall around
  16–17 ms, but
  that field is no longer part of production profile semantics.

Practical consequence at the game-mechanism level: higher FPS sharpens single-note capture, but does
not prove same-key re-trigger can become arbitrarily fast. The current production scheduler no longer
has a repeat-gap field; infeasible same-key intervals are handled through hold/min_hold validation and
tempo/profile choice.

### A.6 How the standard is currently enforced

- Frame-coupled values materialise as `max(ceil(frames × frame_us), floor_us)` whenever
  `game_fps > 0`; when `game_fps = 0` the raw `*_unframed_us` values are kept (the intentional
  expert/experiment escape hatch).
- Profiles currently differentiate playback through **hold/min_hold floors** (input lead,
  chord merge, and release_gap were removed June 2026).

### A.7 Measurement pitfalls

Recorded in `timing-experiments.md` §0 (count onsets not loudness; keep hold ≥ 1 frame when
sweeping the gap; use explicit `--fps 0` during intrinsic measurement). Kept there so they live next to the
experiment procedures.

### A.8 Open / unconfirmed

- The exact mechanism of the fixed-time wall around 16–20 ms (hypothesised internal cadence near
  60 Hz) is still not proven. A.10 shows a related phase-dependent bucket jump in single-note onset
  timing, so the model is plausible at two measurement points, but it remains a model rather than a
  direct measurement of the game's internal clock.
- A clean 30 FPS gap point (hold ≥ 42 ms) has not yet been taken; the model predicts a
  ~46 ms reliable gap there.
- Onset counts are noisy (±1–2); thresholds above are read as trends, not exact values.

### A.9 Profile differentiation lives in explicit floors and frame margins

> ⚠️ **CORRECTION (June 2026 audit).** This section originally called `input_lead_us` "the real
> audience lever". That is now known to be **wrong**: input lead is an architectural no-op (it shifts a
> self-generated timeline with no external reference — `timing-architecture-audit.md` §1) and was
> removed, along with `chord_merge_window_us` and later `release_gap_us`. Audience differentiation now
> rests **entirely** on the hold/min_hold floors. The removed-lever rows and prose below are
> kept struck through for history; read the floors, ignore those levers.

A direct consequence of the measured floors (A.3, A.4): frame-coupled profile values are
materialised as the larger of a local frame term and an absolute floor:

```
effective_us = max(ceil(frames * frame_us), floor_us)
```

At 60 FPS the shared local margins materialise `min_hold` to at least about 20 ms for the non-local
profiles. Therefore a profile whose absolute floor is at or below that value does not differentiate
that parameter at 60 FPS; the frame term wins.

A profile _could_ be made more conservative than the local minimum by setting its
`min_hold_floor_us` **above** the local materialised value.
The earlier `audience_safe` did exactly that (≈2-frame floors). **As of the EXP-4 review
(June 2026) it no longer does**: a wide hold/gap was found to trade away articulation,
repeat speed, and chord expressiveness _without_ a demonstrated remote benefit (frame-test
with the floors removed was on par with the floored profile for a remote listener). The
audience margin is therefore carried **entirely by the hold/min floors**, which sit just above
the registration floor a typical remote (~60 FPS) client needs (~~plus input lead and chord merge —
both removed June 2026~~):

| Value                       | audience_safe | In frames @60 | Rationale                                                                                                                 |
| --------------------------- | ------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------- |
| hold_floor_us               | 20000         | ~1.2          | visible hold for a remote ~60fps client; no wide 2-frame margin                                                           |
| min_hold_floor_us           | 18000         | ~1.1          | compressed notes survive ~1 remote frame                                                                                  |
| ~~repeat_release_gap_floor_us~~ | ~~24000~~  | —             | **REMOVED** — not a reachable production lever in corpus/policy audit                                                     |
| ~~input_lead_us~~           | ~~10000~~     | —             | **REMOVED** — proven no-op (audit §1); was not actually compensating anything                                             |
| ~~chord_merge_window_us~~   | ~~5000~~      | —             | **REMOVED** — never fired on real songs (O2)                                                                              |

At 60 FPS these floors sit at/below the local frame terms. At 144 FPS the frame terms shrink and the
absolute floors take over (hold 18000 / min 18000), holding a remote minimum without inflating to two
host frames.

The local profiles (`balanced`, `local_precise`, `dense_safe`) intentionally leave
`min_hold_floor_us` at or below the shared local floors;
physics caps same-key reliability identically for all of them at low/normal FPS, so they
differentiate through `hold_floor_us`/`min_hold_floor_us` instead (input lead, chord merge, and
release_gap were removed June 2026).

As a follow-up tuning after the frame/floor representation landed, local profiles now use
lower `hold_floor_us` and `min_hold_floor_us` values when FPS is known:

| Profile class | hold/min floor   | Why                                                                             |
| ------------- | ---------------- | ------------------------------------------------------------------------------- |
| local_precise | 0 us             | pure frame-relative = the measured visibility model; sharpest at high FPS       |
| dense_safe    | 11000 us         | a small body floor above local while staying under the 144 FPS frame term       |
| balanced      | 14000 us         | default profile keeps extra high-FPS body over local-precise                    |
| audience_safe | 20000 / 18000 us | remote ~60fps registration floor + small margin (EXP-4); no wide 2-frame margin |

Built-ins keep conservative `*_unframed_us` fallback values so the no-FPS/experiment path
does not silently inherit the sharper local floors.

> Caveat: the audience values are extrapolated from LOCAL measurements plus the principles
> in this document. They have not yet been validated in a populated online room; that
> validation is the remaining open item for the audience profile.

> Update (EXP-4, June 2026): a first remote-listener A/B (a second player recorded the room
> audio over two sessions) compared `audience_safe` (absolute floors on) against
> `audience_frame_test` (floors off). On `TEST_metro_same_200` both registered all 64 onsets
> with near-identical same-key valley depth (−15.27 vs −15.25 dB); on `TEST_repeat_staircase`
> the floor-off version trailed by only 1–2 onsets (within detector noise), with no systematic
> note loss or broken down-up-down. So under the network conditions sampled, the absolute
> audience floors are **not yet shown to be necessary**. This is not proof they are redundant:
> the floors exist for adverse remote conditions (high ping / jitter / replication stalls) that
> these two clean sessions did not stress. Treat as "lower the floors only with deliberate,
> staged validation under worse network", not "remove the floors".

### A.10 Result 4 — Onset bucket-jumps are game-side, phase-dependent, not player-tunable

> Heading note: the original framing ("input lead must NOT scale with render FPS") is now moot —
> input lead was removed entirely (audit §1). The underlying finding is that sender timing is clean
> while game-audio onsets can jump by roughly 20 ms in a phase-dependent way; no player-side knob can
> fix that relative scatter.

A second measurement round (June 2026; recorded WAV via Audacity, onset counting per A.1)
tested whether _onset timing_ tracks the render frame or a coarser game-side cadence/bucket.

**EXP-1 — the player's send side is clean.** Per-event telemetry (`--debug-csv`) on a steady
stream gave send-interval std of **0.05–0.07 ms** at both 60 and 144 FPS, and p95 send
lateness ~**0.13 ms**. Any audible rhythm problem is therefore _not_ the player's
scheduler/sleep timing — it must originate in the game.

**EXP-2 — onset jitter does not shrink with render FPS.** With the player forced to `--fps 0` (no
frame-aware rescaling) and the game FPS locked externally, a steady alternating-key stream showed
jitter that does not improve at 144 FPS. **Correction (result.md, vs the earlier "≈13/≈12 ms
constant" estimate):** the jitter is **bimodal / phase-dependent**, not a stable floor — some runs are
clean (residual std ~0.02 ms) and some carry **±~20 ms bucket-jumps** (std 5–8 ms; e.g. a 200 ms
cadence splitting into 181/219 ms pairs). A pure render-frame sampler would have roughly halved the
jitter (16.7 → 6.9 ms frame); it did not. The ±20 ms jumps are consistent with a coarser internal
cadence beating against the send stream, not render-frame-relative random jitter.

Conclusion: onset registration is not governed by the player's send jitter and does not simply improve
with higher render FPS. Some runs show a game-side bucket jump around **~20 ms**, consistent with an
internal cadence near 60 Hz, but the run-to-run phase dependence means this should be treated as a
model of the observed behavior rather than direct proof of a fixed game clock. The architecture
decision still follows: lead/frame-align cannot remove relative onset scatter.

**Consequence for input lead (two code changes).** *First* (mid-2026) the high-FPS lead "phase
compensation" (`lead − frame/2 + frame'/2`) was removed because EXP-2 showed the phase behavior was
not render-frame-scaled — scaling the lead down at high FPS biased notes late ("lạc nhịp" at 144 FPS).
*Then* (the June 2026 audit) **input lead was removed entirely**: it was shown to be an architectural
no-op (the player generates the whole timeline against no external reference, so a uniform shift is
unobservable — `timing-architecture-audit.md` §1, `timing-experiments.md` O1). So there is no lead to
hold, scale, or compensate anymore.

Caveat: the residual onset jitter (phase-dependent bucket-jumps up to ~20 ms) is game-side in the
recorded data and **cannot be removed from the player side** — no lead, frame-align, or chord-merge
knob ever addressed it, which is exactly why all three were removed.
