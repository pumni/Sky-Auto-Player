# Frame-Relative Timing Model (reference)

**Status:** Implemented. This is the live model; `timing-principles.md` Appendix A holds the
evidence, `timing-experiments.md` holds the experiments, and `config.py` /
`scheduler_types.py` hold the code.

> History: profiles used to declare absolute microseconds with a *separate* global frame-aware
> scaling layer (`min_hold = max(base_us, 1.25 × frame)`). The number you wrote was not the
> number that ran, and one intent lived in two places. That two-layer model was replaced by the
> single per-profile model below (a behaviour-preserving representation change, verified 0
> mismatch at 30/60/144 at the time; floors were retuned afterwards). The migration scaffolding
> is in git history and `test_empirical_floors.py`.

## 1. Core model

Every frame-coupled duration is the **larger of a local-visibility frame term and an absolute
floor**, declared together per profile:

```
effective_us(param) = max(ceil(frames(param) × local_frame_us), floor_us(param))
```

- `frames` = the **local visibility margin** (physics: ≥ 1 frame). Moves with the profile and
  is per-profile overridable (hold/min_hold ≈ 1.25, repeat_gap ≈ 1.5).
- `floor_us` = the profile's **absolute target / wall** — its real character.

`ceil` is required (matches the historical `math.ceil(frame × ratio)`; `round` drifts ±1 µs).

## 2. Why floors are absolute (local vs remote)

- **Local visibility** must survive the *local* client's per-frame sampling → the frame term.
- **Online survivability** depends on the *remote* client's frame sampling + network, which are
  independent of local FPS → an absolute µs floor. A 144 FPS local player must still emit
  durations a ~60 FPS remote listener can capture.
- The fixed ~17 ms same-key wall and the ~60 Hz onset cadence (Appendix A) are absolute-time
  effects of the same kind — they do **not** scale with render FPS.

So `floor_us` is where a profile says "regardless of my local FPS, never go below this."

## 3. Parameter taxonomy

| Parameter             | Representation                                                              |
| --------------------- | -------------------------------------------------------------------------- |
| `hold`                | `hold_frames` + `hold_floor_us`                                            |
| `min_hold`            | `min_hold_frames` + `min_hold_floor_us`                                    |
| `repeat_gap`          | `repeat_gap_frames` + `repeat_gap_floor_us`                               |
| `input_lead`          | `input_lead_us` (absolute; raised at <60 FPS only — no high-FPS scaling, Appendix A.10) |
| `release_gap`         | `release_gap_us` (absolute; low-FPS clamp)                                |
| `chord_merge_window`  | `chord_merge_window_us` (absolute; low-FPS clamp)                         |
| `spin_threshold`      | `spin_threshold_us`                                                        |
| `focus_restore_grace` | `focus_restore_grace_us`                                                   |

`*_unframed_us` keys are the conservative fallback used only on the no-FPS / `game_fps = 0`
path (frame-aware disabled).

## 4. Resolution pipeline (single layer)

```
frame_us = 1_000_000 / (fps if fps and fps > 0 else 60)        # baseline 60 when unknown

hold_us       = max(ceil(hold_frames     * frame_us), hold_floor_us)
min_hold_us   = max(ceil(min_hold_frames * frame_us), min_hold_floor_us)
repeat_gap_us = max(ceil(repeat_gap_frames * frame_us), repeat_gap_floor_us)

release_gap_us        = clamp_low_fps(release_gap_us, frame_us)
chord_merge_window_us = clamp_low_fps(chord_merge_window_us, frame_us)
input_lead_us         = clamp_low_fps(input_lead_us, frame_us)   # raise at <60 FPS only
spin / grace          = as declared
# then apply absolute-µs overrides (§5)
```

## 5. Override / escape-hatch layer (absolute µs)

Applied **after** materialisation: CLI flags (`--hold-ms`, `--min-hold-ms`, …), telemetry
calibration, and `config.json` `_us` keys. Clean split: **profiles = declarative intent;
`_us` = absolute control.** Appendix-A probing keeps working via `--hold-ms` / `--min-hold-ms`.
User overrides compose onto the built-in via a deep overlay (override one field, keep the rest).

## 6. Validation invariants

- `0 < min_hold_frames <= hold_frames` and `min_hold_floor_us <= hold_floor_us`
  → materialised `min_hold_us <= hold_us` at every FPS.
- `min_hold_frames >= 1.0` → ≥ one visibility frame at any FPS (makes the 60 FPS cycle rule
  structural rather than a separate check).
- `repeat_gap_frames > 0`; `*_floor_us >= 0`; `ABSOLUTE_MIN_HOLD_US` backstop.
- `validate_audience_safe_profile` enforces the audience floor thresholds (see `validation.py`).

## 7. Current floors

The authoritative per-profile floors live in `config.py::DEFAULT_TIMING_PROFILES` and are
documented with rationale in `timing-principles.md` Appendix A.9. `test_empirical_floors.py`
pins the materialised values at 30/60/144 as a regression. Summary of intent:

| profile        | hold floor | notes                                                       |
| -------------- | ---------- | ----------------------------------------------------------- |
| local_precise  | 0          | pure frame-relative = the measured visibility model; sharpest |
| dense_safe     | 11000      | small body floor + larger chord merge / release for density |
| balanced       | 14000      | default; a little body/lead above local                     |
| audience_safe  | 20000      | small remote margin above the registration floor (EXP-4)    |
