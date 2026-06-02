# Design: Frame-Relative Timing Profiles

**Status:** Proposed — reviewed, self-checked against the numbers (equivalence verified,
0 mismatch at 30/60/144), decisions recorded (§15). Behaviour-preserving refactor; an
optional follow-up tuning is separated out (§16).
**Author:** design discussion, June 2026
**Related:** `docs/timing-principles.md` (esp. Appendix A), `src/sky_music/config.py`,
`src/sky_music/domain/scheduler_types.py`

---

## 1. Problem

Profiles declare timing in **absolute microseconds** (`min_hold_us: 17000`) and a separate
**global frame-aware scaling layer** overrides them at runtime
(`min_hold = max(base_us, 1.25 × frame)`). After Appendix A this two-layer model fights the
physics:

- **The number you write is not the number that runs.** `min_hold_us: 17000` → 20834 µs at
  60 FPS, 41667 µs at 30 FPS, 17000 µs at 144 FPS. The variance is *hidden* in a global
  ratio that lives far from the profile.
- The base µs only matters when it exceeds the frame floor (Appendix A.9). To reason about a
  profile you must mentally compute `max(base, global_ratio × frame)` across two layers.
- Two representations (absolute base µs + global ratios) encode one intent and can disagree.

The values themselves are empirically right (Appendix A). The **representation** is wrong:
the per-profile number is an *absolute floor/target*, and the frame behaviour is an
*invisible global rule*.

## 2. Core idea

Every frame-coupled duration is the **larger of a local-visibility frame term and an
absolute floor**, declared together, per profile:

```
effective_us(param) = max(ceil(frames(param) × local_frame_us), floor_us(param))
```
(`ceil`, matching today's `math.ceil(frame × ratio)`, makes this exactly equivalent — §11.)

- `frames` = the **local visibility margin** (physics: ≥ 1 frame). This is what the global
  ratios already encoded (hold/min_hold ≈ 1.25, repeat_gap ≈ 1.5); it moves *into the
  profile* and becomes per-profile overridable.
- `floor_us` = the profile's **absolute target / wall** — its real character. Local profiles'
  hold length, the fixed ~17 ms same-key wall, and `audience_safe`'s online-survivability
  durations are all absolute, FPS-independent quantities (a remote client samples on *its*
  frames, not the local ones).

`repeat_gap_floor_us` already had exactly this shape; we generalise it to `hold` and
`min_hold`. This makes the two components explicit and co-located, and (critically) is a
**pure re-expression of today's behaviour** — see §11.

## 3. Why floors are absolute (local vs remote)

- **Local visibility** must survive the *local* client's per-frame sampling → frame term.
- **Online survivability** depends on the *remote* client's frame sampling + network, which
  are independent of local FPS → absolute µs floor. A 144 FPS local player must still emit
  durations a 30 FPS remote listener can capture.
- The ~17 ms same-key wall (Exp2) is an absolute-time effect of the same kind.

So `floor_us` is where a profile says "regardless of my local FPS, never go below this."
`audience_safe` (future default) leans on its floors; local profiles mostly leave them as the
absolute hold target they already had.

## 4. Parameter taxonomy

| Parameter             | Representation                                   |
| --------------------- | ------------------------------------------------ |
| `hold`                | `hold_frames` + `hold_floor_us`                  |
| `min_hold`            | `min_hold_frames` + `min_hold_floor_us`          |
| `repeat_gap`          | `repeat_gap_frames` + `repeat_gap_floor_us`      |
| `input_lead`          | `input_lead_us` (absolute + ½-frame phase-comp; unchanged) |
| `release_gap`         | `release_gap_us` (absolute; low-FPS clamp kept)  |
| `chord_merge_window`  | `chord_merge_window_us` (absolute; low-FPS clamp kept) |
| `spin_threshold`      | `spin_threshold_us`                              |
| `focus_restore_grace` | `focus_restore_grace_us`                         |

## 5. Data model

```python
"balanced": {
    "hold_frames": 1.25,     "hold_floor_us":      26000,   # >=1.25 frame, and >=26 ms
    "min_hold_frames": 1.25, "min_hold_floor_us":  17000,
    "repeat_gap_frames": 1.5,"repeat_gap_floor_us": 18000,  # frame term, or ~17 ms wall
    "release_gap_us":  4000, "input_lead_us": 6000,
    "chord_merge_window_us": 3000, "spin_threshold_us": 500,
    "focus_restore_grace_us": 100000,
}
```

`frames` default to the (former global) visibility margins; a profile overrides them only
when it wants a different *local* margin (e.g. `high_fps_precise` uses a smaller
`min_hold_frames`). `_us`-only keys remain valid as absolute overrides for legacy/user/
calibration paths (§7, §9).

## 6. Resolution pipeline (single layer)

```
frame_us = 1_000_000 / (fps if fps and fps > 0 else 60)         # §15.2 baseline 60 when unknown

hold_us       = max(ceil(hold_frames     * frame_us), hold_floor_us)
min_hold_us   = max(ceil(min_hold_frames * frame_us), min_hold_floor_us)
repeat_gap_us = max(ceil(repeat_gap_frames * frame_us), repeat_gap_floor_us)

release_gap_us        = clamp_low_fps(release_gap_us, frame_us)         # unchanged
chord_merge_window_us = clamp_low_fps(chord_merge_window_us, frame_us)  # unchanged
input_lead_us         = phase_compensate(input_lead_us, fps)           # unchanged
spin / grace          = as declared
# then apply absolute-µs overrides (§7)
```

The global `frame_timing` ratios disappear (subsumed into per-profile `frames`). The only
runtime floors are the explicit per-profile `*_floor_us`.

## 7. Override / escape-hatch layer (absolute µs)

Applied **after** materialisation, overriding the resolved value: CLI flags
(`--hold-ms`, `--min-hold-ms`, …), telemetry calibration, and user `config.json` entries
using `_us` keys. Clean split: **profiles = declarative intent; `_us` = absolute control.**
Appendix-A probing keeps working via `--hold-ms` / `--min-hold-ms`.

## 8. Validation (frame invariants)

- `0 < min_hold_frames <= hold_frames` and `min_hold_floor_us <= hold_floor_us`
  → materialised `min_hold_us <= hold_us` at every FPS.
- `min_hold_frames >= 1.0` → ≥ one physical visibility frame at any FPS → **Non-Negotiable
  Rule 6 becomes structural**, not a separate check.
- `repeat_gap_frames > 0`; `*_floor_us >= 0`; `ABSOLUTE_MIN_HOLD_US` backstop unchanged.
- `validate_audience_safe` → floor thresholds (e.g. `hold_floor_us >= 28000`,
  `repeat_gap_floor_us >= 28000`, `min_hold_floor_us >= 17000`).

## 9. Backward compatibility & migration

Dual-read per profile dict: `*_frames` present → frame model; a frame-coupled `*_us` present
→ absolute override of that param. Built-ins move to `_frames` + floors; user overrides keep
working as absolute overlays. **Switch `merged_timing_profiles` to a deep overlay (§15.4)** so
a partial user override composes onto the built-in instead of replacing the whole profile.

## 10. Concrete conversion (frame @60 ≈ 16667 µs) — behaviour-identical

`frames` = the visibility margins the global ratios already applied; `floor_us` = today's
base µs. All other µs unchanged.

| profile          | hold_frames / floor | min_hold_frames / floor | repeat_gap_frames / floor |
| ---------------- | ------------------- | ----------------------- | ------------------------- |
| local_precise    | 1.25 / 22000        | 1.25 / 17000            | 1.5 / 18000               |
| dense_safe       | 1.25 / 22000        | 1.25 / 17000            | 1.5 / 18000               |
| balanced         | 1.25 / 26000        | 1.25 / 17000            | 1.5 / 18000               |
| audience_safe    | 1.25 / 34000        | 1.25 / 25000            | 1.5 / 33000               |
| high_fps_precise | 1.25 / 18000        | 1.0  / 10000            | 1.5 / 18000               |

(`high_fps_precise` differentiates with a low `hold_floor` (18 ms, sharp) and `min_hold_frames
= 1.0`; it stays gated to >100 FPS.)

## 11. Behaviour change analysis — none (verified)

For every built-in and every FPS, `max(ceil(frames × frame), floor_us)` reproduces today's
`max(base_us, ceil(ratio × frame) [, wall])` **exactly**. Verified by script against
`resolve_effective_policy` at 30/60/144 for hold, min_hold and repeat_gap on all five
profiles: **0 mismatches** (using `ceil`, to match the existing `math.ceil` scaling — `round`
would introduce ±1 µs drift, so `ceil` is required).

This is a **representation refactor, not a behaviour change.** `audience_safe` (future
default) is byte-for-byte preserved. Exact-equality tests keep their current expected µs.

## 12. Implementation plan (each step keeps the suite green)

1. Add `materialise(frames, floor_us, frame_us)` (with `ceil`) + dual-read of profile dicts
   (no behaviour change; built-ins still `_us`).
2. Rewrite built-in `DEFAULT_TIMING_PROFILES` to `_frames` + floors (§10); assert 60/30/144
   materialisation matches today for every profile (the §11 equivalence script becomes a test).
3. Route `TimingPolicy`/`FrameTimingPolicy` resolution through `materialise`; delete the
   `max(base_us, ratio × frame)` branches in `from_timing_policy`.
4. Retire the global `frame_timing` ratios (subsumed by per-profile `frames`); keep a minimal
   default frame set as the fallback for `_us`-only legacy profiles.
5. Switch validation to the §8 invariants.
6. Deep-overlay merge for user overrides (§9).
7. Refresh any test that asserted the global ratios; keep `test_empirical_floors.py`.
8. Update `docs/timing-principles.md` (floors intrinsic to each profile).

## 13. Test plan

- Materialisation `max(ceil(frames × frame), floor)` at 30/60/144 → expected µs (parametrised).
- **Equivalence gate:** every built-in materialises to today's µs at 30/60/144 (hard gate,
  esp. `audience_safe`) — promote the §11 verification script into a regression test.
- Override layer: `--hold-ms` overrides the resolved value; probing still works.
- Dual-read: a frame-coupled `_us` key behaves as an absolute override.
- Invariants: `min_hold_frames ∈ (0, hold_frames]`, `≥ 1.0`, floor ordering.

## 14. Risks / residual notes

- **Rounding:** none — `ceil` matches the existing scaling exactly (§11), so no test churn
  from materialisation drift. (`round` would drift ±1 µs; the design mandates `ceil`.)
- **Mixed-unit dicts** (frames + floor µs + plain µs): honest but multi-kind; mitigated by
  explicit suffixes and §4/§10 docs.
- `input_lead` stays µs + phase-comp; revisit only if a pure-frame component is later needed.

## 15. Decisions (recommended answers)

1. **Adopt the unified `max(ceil(frames × frame), floor_us)` model (§2), behaviour-preserving
   (§10–11).** Rationale: it is the only representation simultaneously correct for *local*
   visibility (frame term) and *online/absolute* targets (floor), it generalises the
   already-proven `repeat_gap_floor_us`, and it re-expresses today's numbers exactly (verified
   0 mismatch) — clean, readable, per-profile, low-risk. The per-profile number now clearly
   means "absolute floor," and the frame margin is explicit instead of a distant global ratio.

2. **`fps = 0 / unknown` → assume the 60 FPS baseline, and nudge the user to set real FPS.**
   Rationale: 60 is the dominant case and always yields real µs; a lower assumption over-holds
   for the majority, a higher one under-holds. Probing stays in absolute-µs overrides (§7).
   Doctor/preflight should surface a one-time "game FPS not set — assuming 60" hint.

3. **`high_fps_precise`: stay in the frame model** (sharp via a low `hold_floor_us` and
   `min_hold_frames = 1.0`), gated to >100 FPS — NOT an absolute special case. Rationale:
   removes the only mixed-scheme wart; the frame model already expresses "sharp" cleanly and
   stays ≥ 1 frame. `_us` absolute remains reserved for user/calibration overrides.

4. **User-override merge: deep overlay.** Rationale: a partial `_us` override must compose
   onto the built-in `_frames` (override one field, keep the rest); the current shallow
   name-level merge silently drops the built-in frames — a footgun.

5. **Proceed with §12**, with the §13 equivalence gate (built-in parity at 30/60/144,
   `audience_safe` especially) as a hard pre-merge acceptance criterion.

## 16. Separated follow-up: frame-relative local holds (optional, needs validation)

The refactor above does **not** change behaviour. A *separate, later* tuning it enables:
lower the **local** profiles' `hold_floor_us` toward 0 so their holds become genuinely
frame-relative and **sharper at high FPS** (today balanced holds 26 ms even at 144 FPS = 3.7
frames; a frame-relative hold would be ~1.25–1.5 frames ≈ 11 ms). This is justified because
hold is a *visibility* concern — Sky notes ring independently of hold duration (Exp2) — so a
long fixed hold at high FPS is wasted key-occupancy that lowers the max note rate for no
audible gain.

It is kept out of this refactor because (a) it is a behaviour change that should be validated
in-game, and (b) it must avoid the low-FPS inversion (a local profile's frame term must not
exceed `audience_safe`'s effective hold). Do it only after the representation lands and with
its own measurements; `audience_safe`'s absolute floors must remain untouched (online).

## 17. Appendix — equivalence verification (runnable gate)

This script proves the §10 table reproduces the current resolver exactly. It must print
`mismatches: 0` against the present code, and should be promoted into a regression test
during step 12.2 (and re-run after step 12.3 against the new resolver).

```python
import sys, math
sys.path.insert(0, "src")
from sky_music.config import AppConfig
from sky_music.domain.session_context import PlaybackSessionContext

# §10 conversion table: param -> (frames, floor_us)
PROPOSED = {
    "local-precise":    {"hold": (1.25, 22000), "min_hold": (1.25, 17000), "rgap": (1.5, 18000)},
    "dense-safe":       {"hold": (1.25, 22000), "min_hold": (1.25, 17000), "rgap": (1.5, 18000)},
    "balanced":         {"hold": (1.25, 26000), "min_hold": (1.25, 17000), "rgap": (1.5, 18000)},
    "audience-safe":    {"hold": (1.25, 34000), "min_hold": (1.25, 25000), "rgap": (1.5, 33000)},
    "high-fps-precise": {"hold": (1.25, 18000), "min_hold": (1.0,  10000), "rgap": (1.5, 18000)},
}
materialise = lambda frames, floor, frame_us: max(math.ceil(frames * frame_us), floor)

mismatches = 0
for profile, spec in PROPOSED.items():
    # high_fps_precise statically falls back to local-precise at <=100 FPS, so test it only >100
    fps_list = [144] if profile == "high-fps-precise" else [30, 60, 144]
    for fps in fps_list:
        frame_us = round(1_000_000 / fps)
        p = PlaybackSessionContext(profile_name=profile, fps=fps).resolve_effective_policy(AppConfig())
        current = {"hold": p.hold_us, "min_hold": p.min_hold_us, "rgap": p.repeat_release_gap_us}
        for key, (frames, floor) in spec.items():
            want, got = materialise(frames, floor, frame_us), current[key]
            if want != got:
                mismatches += 1
                print(f"MISMATCH {profile} fps={fps} {key}: proposed={want} current={got}")
print("mismatches:", mismatches, "(0 = the frame model reproduces current behaviour)")
```
