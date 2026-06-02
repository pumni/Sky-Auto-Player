# Timing Principles for Sky Music Player

This document is the engineering source of truth for designing, reviewing, and calibrating timing profiles for Sky Music Player.

It is intentionally a principles document, not an implementation document. It should describe what timing profiles must guarantee, why those guarantees matter, and how future changes should be evaluated before they are exposed to users.

The goal is not to make playback appear fast in logs. The goal is reliable note registration inside the game and, when playing online, reliable audibility for other players.

**Related docs:** `timing-experiments.md` (the experiments that prove Appendix A and the open
calibration work) and `timing-profile-frame-model.md` (the frame-relative representation in code).

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

| Term                  | Meaning                                                                                               |
| --------------------- | ----------------------------------------------------------------------------------------------------- |
| hold_us               | Effective key-down duration for a normal note after profile materialisation and overrides.            |
| hold_frames           | Local frame-visibility margin for normal holds.                                                       |
| hold_floor_us         | Absolute lower wall/target for normal holds.                                                          |
| min_hold_us           | Effective minimum key-down duration after compression.                                                |
| min_hold_frames       | Local frame-visibility margin for compressed holds.                                                   |
| min_hold_floor_us     | Absolute lower wall/target for compressed holds.                                                      |
| release_gap_us        | Minimum gap after a key is released before general scheduling continues.                              |
| repeat_release_gap_us | Effective up-time before pressing the same key again. This is the critical same-key repeat gap.       |
| repeat_release_gap_frames | Local frame margin for same-key repeat up-time.                                                  |
| repeat_release_gap_floor_us | Absolute lower wall/target for same-key repeat up-time.                                      |
| input_lead_us         | How early input is sent relative to the musical timestamp.                                            |
| chord_merge_window_us | Window used to snap nearby notes into the same chord.                                                 |
| cycle_us              | min_hold_us plus repeat_release_gap_us. This is the critical same-key repeat cycle.                   |
| frame_us              | Duration of one game frame. At 60 FPS, one frame is about 16.67 ms.                                   |
| game_fps              | The FPS value selected or calibrated by the user. A value of 0 means frame-aware scaling is disabled. |
| tempo_scale           | Playback speed multiplier. Values above 1.0 increase scheduling pressure.                             |

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

A general 60 FPS-safe profile must satisfy the 60 FPS cycle rule:

min_hold_us plus repeat_release_gap_us must be greater than one 60 FPS frame.

At 60 FPS, one frame is approximately 16.67 ms. For production defaults, the practical cycle should usually be at least 18–22 ms, depending on the profile’s purpose.

### 4.2 Local Precise Profiles

Local precise profiles optimize for sharper local playback.

They may use shorter holds and smaller gaps than audience profiles, but they must still preserve reliable local note registration.

A local precise profile is not automatically suitable for online rooms. If a profile is tuned mainly by what the local player hears, it should be treated as local-first.

### 4.3 Dense Playback Profiles

Dense playback profiles are designed for songs with high scheduling pressure.

They should reduce collapse and missed notes by balancing:

- sufficient same-key release time;
- reasonable visible hold duration;
- slightly larger chord merge windows;
- enough input lead to compensate for scheduling and frame delay.

Dense playback should not be solved by blindly lowering min_hold_us. If density causes misses, safer spacing is usually better than shorter spacing.

### 4.4 Online Audience Profiles

Online audience profiles optimize for what other players hear, not only what the local player hears.

Online audience playback should use:

- larger visible holds;
- larger same-key release gaps;
- larger chord merge windows;
- more conservative input lead;
- stronger protection against dense timing collapse.

audience_safe is the recommended profile class for online rooms.

If local playback sounds correct but other players miss notes, the timing should be treated as insufficient for online audience playback.

### 4.5 Why there is no dedicated high-FPS profile

An earlier experimental `high_fps_precise` profile assumed that higher render FPS lets timing
go uniformly sharper (shorter holds, shorter gaps, sub-frame margins). Measurement disproved
that premise (Appendix A.10 / EXP-2): higher render FPS sharpens **single-note visibility
only**; onset cadence and same-key repeats are capped by a fixed ~60 Hz internal tick that
does not scale with render FPS. A separate high-FPS profile therefore bought nothing the frame
model does not already provide, and was removed.

High-FPS users are served by the normal profiles: every profile is frame-aware, so its
visibility holds already shorten with FPS via the `*_frames` term, while the fixed floors
(same-key wall, audience remote margin) correctly stay constant. `local_precise` in
particular is pure frame-relative (zero hold floor) and is the sharpest option at high FPS.

---

## 5. Principle 1 — Frame Capture Matters More Than Scheduler Logs

A scheduler log can show that a key was pressed and released correctly, while the game still fails to observe the state change.

This happens because the game may only sample input state at frame boundaries. If the key-down or key-up state exists for too short a time, it can fall between samples.

Therefore, timing profiles must be judged by in-game behavior, not by logs alone.

Reliable playback requires that important state transitions survive long enough to be observed by the game.

---

## 6. Principle 2 — The Cycle Rule

Same-key repeats require a complete state transition:

DOWN, visible hold, UP, visible release, DOWN again.

The critical cycle is:

cycle_us equals min_hold_us plus repeat_release_gap_us.

A same-key repeat can be dropped, merged, or heard as incomplete if the game does not observe a complete down-up-down sequence.

For any profile that may be selected at the current FPS:

cycle_us must be greater than frame_us.

For profiles exposed as generally safe at 60 FPS:

cycle_us must be greater than 16.67 ms.

For production defaults at 60 FPS, prefer a practical cycle of at least 18–22 ms.

For online audience playback, the cycle should usually be more conservative than the local minimum. Online reliability should target multi-frame survivability, not merely one-frame local capture.

---

## 7. Principle 3 — The Visibility Rule

min_hold_us must be long enough for the game client to observe the key as down.

A short hold may feel attractive for dense songs, but it can make notes vanish if the game does not sample the down state in time.

Do not lower min_hold_us only to make dense songs faster.

If notes vanish, first consider:

1. increasing min_hold_us;
2. increasing repeat_release_gap_us;
3. using a safer profile;
4. reducing tempo_scale;
5. increasing chord_merge_window_us slightly if the issue is chord spread.

For online audience playback, min_hold_us should be more conservative than local-only playback.

---

## 8. Principle 4 — Same-Key Release Is Critical

repeat_release_gap_us is the most important value for repeated notes on the same key.

Same-key repeats are fragile because the game must see the key return to an up state before it can treat the next down state as a new note.

release_gap_us is not enough protection for same-key repeats. Same-key repeats must use repeat_release_gap_us.

If repeated notes drop locally, increase repeat_release_gap_us before lowering min_hold_us.

If repeated notes sound correct locally but are missed by other players, switch to an online audience-safe profile or increase repeat_release_gap_us.

---

## 9. Principle 5 — General Release Gap Still Matters

release_gap_us protects general scheduling after a key-up event.

It is less critical than repeat_release_gap_us for same-key repeats, but it still helps avoid overly compressed event streams.

A release gap that is too small can make playback more fragile under load, especially in dense songs or online rooms.

release_gap_us should be tuned for general stability, while repeat_release_gap_us should be tuned for same-key reliability.

---

## 10. Principle 6 — Chord Batching Reduces Pressure

chord_merge_window_us controls how nearby notes are grouped into a chord.

Small windows preserve expressive timing, strums, and intentional offsets.

Larger windows reduce scheduling pressure and can make chords more coherent for remote listeners.

The window should not be too large. Excessive batching can flatten intended arpeggios and make expressive passages sound unnatural.

For local expressive playback, use smaller chord merge windows.

For dense or online audience playback, slightly larger chord merge windows are usually safer.

---

## 11. Principle 7 — Input Lead Compensates for Delay

input_lead_us sends input before the musical timestamp.

It compensates for:

- scheduler delay;
- input injection delay;
- OS-level timing variation;
- frame boundary delay;
- online perceived delay.

Input lead should be adjusted gradually.

Too little lead makes playback sound late.

Too much lead makes playback feel early, especially for the local player.

When the only symptom is consistent lateness, prefer calibrating input_lead_us before changing hold or gap values.

For online audience playback, prefer using an audience-safe profile before manually pushing input lead too far.

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

## 15. Principle 11 — Frame Alignment Must Be Conservative

Frame alignment can improve capture consistency by placing events closer to expected frame boundaries.

However, aggressive alignment can add timing bias and make playback feel late or uneven.

The safest default is no frame alignment.

Down-only alignment may be useful as an opt-in calibration mode, but it should be easy to disable.

Frame alignment should never be used as a substitute for adequate hold and release durations.

---

## 16. Recommended Profile Intent

The project ships exactly four profiles:

| Profile          | Intended Use                            | Online Audience Use | Notes                                                                    |
| ---------------- | --------------------------------------- | ------------------- | ------------------------------------------------------------------------ |
| local_precise    | Sharp local playback                    | No                  | Reference profile = the measured floors themselves; pure frame-relative holds. |
| balanced         | General default playback                | Limited             | local_precise + a little body/lead. Good default for normal use.        |
| dense_safe       | Dense local playback                    | Limited             | Body like balanced + larger chord merge / release gap for note pressure. |
| audience_safe    | Online room playback                    | Yes                 | A little above balanced for remote audibility; carried mainly by input lead. |

balanced should remain the general default profile.

audience_safe should be the recommended profile for online audience playback.

dense_safe should be used when density causes collapse but online audience safety is not the main requirement.

local_precise should be used only when local responsiveness matters more than remote reliability.

---

## 17. Symptom-Based Tuning

| Symptom                                                  | Likely Cause                                        | First Adjustment                                          |
| -------------------------------------------------------- | --------------------------------------------------- | --------------------------------------------------------- |
| Same-key repeats drop locally                            | Same-key cycle is too short                         | Increase repeat_release_gap_us.                           |
| Notes vanish locally                                     | Hold is too short or FPS is lower than expected     | Increase min_hold_us or use balanced.                     |
| Local playback sounds fine, but other players miss notes | Online timing is too aggressive                     | Use audience_safe.                                        |
| Other players hear repeated notes as incomplete          | Same-key release is too short for online audibility | Increase repeat_release_gap_us or use audience_safe.      |
| Other players hear chords as broken or rattly            | Chord events are too spread out                     | Increase chord_merge_window_us slightly.                  |
| Playback sounds consistently late                        | Input lead is too small                             | Increase input_lead_us gradually.                         |
| Playback sounds early locally                            | Input lead is too large                             | Decrease input_lead_us.                                   |
| Dense passages collapse                                  | Scheduling pressure is too high                     | Use dense_safe, use audience_safe, or reduce tempo_scale. |
| Local playback feels too soft or mushy                   | Holds or gaps are too large for the use case        | Use balanced or local_precise.                            |

---

## 18. Tuning Order

When playback is unreliable, tune in this order:

1. Confirm the playback intent: local, dense, or online audience.
2. Confirm the selected profile matches that intent.
3. Confirm the configured FPS and whether FPS is stable.
4. Confirm tempo_scale is not creating unrealistic density.
5. For dropped same-key repeats, increase repeat_release_gap_us.
6. For vanished notes, increase min_hold_us.
7. For online audience misses, switch to audience_safe.
8. For broken or rattly chords, increase chord_merge_window_us slightly.
9. For consistent lateness, adjust input_lead_us.
10. Only after those steps, consider changing hold_us or defining a new profile.

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
6. Does it avoid excessive chord flattening?
7. Does it avoid excessive input lead?
8. Does it behave acceptably at tempo_scale 1.0?
9. Does it remain reasonable when tempo_scale is increased?
10. Has it been tested in real gameplay, not only scheduler logs?
11. If it is meant for online rooms, have remote listeners confirmed reliability?

A profile should not be exposed as production-ready until these questions have acceptable answers.

---

## 20. Non-Negotiable Rules

1. balanced remains the general default profile.
2. audience_safe is the recommended online audience profile.
3. Online reliability wins over local sharpness when the selected intent is audience playback.
4. Same-key repeats must be protected by repeat_release_gap_us.
5. release_gap_us must not be treated as sufficient protection for same-key repeats.
6. Any profile exposed at normal 60 FPS must satisfy the 60 FPS cycle rule.
7. Frame-aware scaling must not persist already-scaled hold or gap values as profile defaults.
8. Future profiles must state their class and validation assumptions.
9. Any change that lowers min_hold_us or repeat_release_gap_us must include a gameplay reason and a validation plan.
10. Logs are not enough. Real in-game behavior is the source of truth.
11. A profile that sounds correct locally is not automatically safe for online audience playback.
12. Render FPS sharpens single-note visibility only; it does not speed up onset cadence or same-key repeats (fixed ~60 Hz tick), so no profile may assume higher FPS makes short holds/gaps safe.

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
*sent* side, onset counting on the WAV verifies *registration*. The full method, tooling
(`tests/make_test_song.py`, `tests/analyze_onsets.py`), and critical controls (lock FPS
externally, `--fps` off when measuring game intrinsics, percussive instrument, count onsets) are
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
was observed (7 ms suffices at 144 FPS). Encoded standard: `min_visible_hold_frames =
1.25` (one frame plus ~25% phase margin). This value is kept.

### A.4 Result 3 — Same-key release-gap floor = max(~1.4 frame, ~17 ms fixed)

The smallest same-key release gap that still re-triggers the note **100% of the time**:

| Game FPS | One frame | Measured 100%-reliable gap | In frames |
| -------- | --------- | -------------------------- | --------- |
| 60       | 16.7 ms   | ≈ 22–24 ms                 | ~1.3–1.4  |
| 144      | 6.9 ms    | ≈ 14–17 ms                 | ~2–2.5    |

The floor is **neither constant in milliseconds nor constant in frames**. It fits a
"larger of the two" model: `gap ≥ max(~1.4 × frame, ~16 ms fixed)`. At high FPS a
**fixed ~17 ms wall** dominates (plausibly a fixed internal note/animation tick near
60 Hz, independent of render FPS — hypothesised, not proven). At a gap of exactly one
frame, reliability is only ~80% (phase-dependent), so margin is mandatory.

Encoded standard: `repeat_release_gap ≥ max(1.5 × frame, 18000 µs)`. (The previous
`0.5 × frame` with no fixed floor was too low on both counts and dropped fast same-key
repeats.)

### A.5 The asymmetry between hold and gap

- **Hold (visibility)** floor is a *pure frame multiple* — no fixed component.
- **Repeat gap** floor is a frame multiple **plus** a fixed ~17 ms wall.

Practical consequence: higher FPS sharpens single-note capture without bound, but does
**not** let same-key repeats go below ~17 ms of release. Higher render FPS is not a
licence for arbitrarily fast repeated notes.

### A.6 How the standard is currently enforced

- Frame-coupled values materialise as `max(ceil(frames × frame_us), floor_us)` whenever
  `game_fps > 0`; when `game_fps = 0` the raw `*_unframed_us` values are kept (the intentional
  expert/experiment escape hatch).
- The same-key gap uses `repeat_release_gap_frames = 1.5` + `repeat_release_gap_floor_us = 18000`
  on **all** profiles, so they converge on this floor at a given FPS (30 → 50000 µs, 60 → 25001 µs,
  144 → 18000 µs) and differentiate through hold, input lead, and chord merge instead.

### A.7 Measurement pitfalls

Recorded in `timing-experiments.md` §0 (count onsets not loudness; keep hold ≥ 1 frame when
sweeping the gap; `--fps` off during intrinsic measurement). Kept there so they live next to the
experiment procedures.

### A.8 Open / unconfirmed

- The exact mechanism of the fixed ~17 ms wall (hypothesised ~60 Hz internal tick) is
  still not proven. However, A.10 (Result 4) now shows the same fixed cadence governs
  single-note **onset phase** as well, not just same-key repeats — so the fixed-tick model
  is observed at two independent measurement points, not one.
- A clean 30 FPS gap point (hold ≥ 42 ms) has not yet been taken; the model predicts a
  ~46 ms reliable gap there.
- Onset counts are noisy (±1–2); thresholds above are read as trends, not exact values.

### A.9 Profile differentiation lives in explicit floors and frame margins

A direct consequence of the measured floors (A.3, A.4): frame-coupled profile values are
materialised as the larger of a local frame term and an absolute floor:

```
effective_us = max(ceil(frames * frame_us), floor_us)
```

At 60 FPS the shared local margins materialise to at least `min_hold = 20834` and
`repeat_gap = 25001`. Therefore a profile whose absolute floor is at or below those values
does not differentiate that parameter at 60 FPS; the frame term wins.

A profile *could* be made more conservative than the local minimum by setting its
`min_hold_floor_us`/`repeat_release_gap_floor_us` **above** the local materialised values.
The earlier `audience_safe` did exactly that (≈2-frame floors). **As of the EXP-4 review
(June 2026) it no longer does**: a wide hold/gap was found to trade away articulation,
repeat speed, and chord expressiveness *without* a demonstrated remote benefit (frame-test
with the floors removed was on par with the floored profile for a remote listener). The
audience margin is therefore carried mainly by `input_lead_us` — which compensates remote
perceived delay as a harmless uniform shift — and a modest `chord_merge_window_us`, while the
hold/min/repeat floors sit just above the registration floor a typical remote (~60 FPS)
client needs:

| Value                       | audience_safe | In frames @60 | Rationale                                                  |
| --------------------------- | ------------- | ------------- | ---------------------------------------------------------- |
| hold_floor_us               | 20000         | ~1.2          | visible hold for a remote ~60fps client; no wide 2-frame margin |
| min_hold_floor_us           | 18000         | ~1.1          | compressed notes survive ~1 remote frame                   |
| repeat_release_gap_floor_us | 24000         | ~1.4          | top of the measured 100%-reliable @60 band (A.4) + remote jitter margin |
| input_lead_us               | 10000         | —             | remote perceived-delay compensation (uniform shift; the real audience lever) — still by-ear, see timing-experiments.md O1 |
| chord_merge_window_us       | 5000          | —             | chord coherence for listeners without flattening arpeggios |

At 60 FPS these floors sit at/below the local frame terms, so the same-key cycle materialises
to ~45.8 ms (~2.75 frames) — **equal to the local minimum**, by design. At 144 FPS the frame
terms shrink and the absolute floors take over (hold 20000 / min 18000 / repeat 24000),
holding a remote minimum without inflating to two host frames. `repeat_release_gap_floor_us`
keeps slightly more margin than `hold_floor_us` because same-key re-trigger is the most
jitter-fragile transition; if adverse-network testing later shows dropped repeats, raise that
floor alone rather than the holds.

The local profiles (`balanced`, `local_precise`, `dense_safe`) intentionally leave
`min_hold_floor_us`/`repeat_release_gap_floor_us` at or below the shared local floors;
physics caps same-key reliability identically for all of them at low/normal FPS, so they
differentiate through `hold_floor_us`, `input_lead_us` and
`chord_merge_window_us` instead.

As a follow-up tuning after the frame/floor representation landed, local profiles now use
lower `hold_floor_us` and `min_hold_floor_us` values when FPS is known:

| Profile class                         | hold/min floor | Why                                                        |
| ------------------------------------- | -------------- | ---------------------------------------------------------- |
| local_precise                         | 0 us           | pure frame-relative = the measured visibility model; sharpest at high FPS |
| dense_safe                            | 11000 us       | a small body floor above local while staying under the 144 FPS frame term |
| balanced                              | 14000 us       | default profile keeps extra high-FPS body over local-precise |
| audience_safe                         | 20000 / 18000 us | remote ~60fps registration floor + small margin (EXP-4); no wide 2-frame margin |

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

### A.10 Result 4 — Onset cadence is a fixed internal tick; input lead must NOT scale with render FPS

A second measurement round (June 2026; recorded WAV via Audacity, onset counting per A.1)
tested whether *onset timing* tracks the render frame or a fixed internal cadence.

**EXP-1 — the player's send side is clean.** Per-event telemetry (`--debug-csv`) on a steady
stream gave send-interval std of **0.05–0.07 ms** at both 60 and 144 FPS, and p95 send
lateness ~**0.13 ms**. Any audible rhythm problem is therefore *not* the player's
scheduler/sleep timing — it must originate in the game.

**EXP-2 — onset jitter does not shrink with render FPS.** With the player's `--fps` off (no
frame-aware rescaling) and the game FPS locked externally, a steady alternating-key stream
gave game-side onset jitter of **≈13 ms at 60 FPS and ≈12 ms at 144 FPS** — essentially
unchanged. A pure render-frame sampler would have roughly halved the jitter (16.7 → 6.9 ms
frame). The 144 FPS residuals also showed periodic **±20 ms jumps** — the signature of a
fixed cadence beating against the send stream, not render-frame-relative random jitter.

Conclusion: onset registration is governed by a **fixed internal cadence (~60 Hz tick),
independent of render FPS** — the same fixed wall first seen for same-key repeats (A.4/A.5),
now confirmed to also govern single-note onset phase. The render frame only governs when it
is *coarser* than this tick, i.e. below ~60 FPS.

**Consequence for input lead (code change).** The earlier high-FPS lead "phase compensation"
(`lead − frame/2 + frame'/2`, applied above 75 FPS for audience profiles) assumed the phase
term scales with the render frame. EXP-2 contradicts that: the phase term is fixed. Scaling
the lead *down* at high FPS biased notes systematically late and beat against the fixed tick —
the observed "lạc nhịp" at 144 FPS that was absent at 60. The compensation was therefore
**removed**: input lead is held at its base value for all FPS ≥ 60. The low-FPS clamp is
unchanged and consistent with the model (below 60 FPS the render frame is the coarser
quantiser, so the lead is still raised toward ~½ frame).

Caveat: the residual ~12 ms onset jitter is the game's own cadence quantisation. It is present
at every render FPS (including 60) and cannot be removed from the player side. Removing the bad
scaling makes 144 FPS behave like 60 FPS — no better, no worse — it does not make onsets
sample-accurate.
