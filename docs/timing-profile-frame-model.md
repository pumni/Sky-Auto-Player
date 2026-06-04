# Frame-Relative Timing Model (reference)

**Status:** Implemented. This is the live model; `timing-principles.md` Appendix A holds the
evidence, `timing-experiments.md` holds the experiments, `timing-architecture-audit.md` records the
June 2026 refactor, and `config.py` / `scheduler_types.py` hold the code.

> **Update (June 2026 refactor).** Four timing knobs were removed: `input_lead_us` (no-op),
> `chord_merge_window_us` (never fired on real songs), `frame_align` (off everywhere), and
> `release_gap_us` (near-zero corpus binding and misleading profile semantics), and
> `repeat_release_gap` (mechanism candidate, but not a reachable production profile lever in the
> audited corpus/model). The live code model now represents `hold` and `min_hold` only.
> `local_precise` uses `hold/min_hold frames = 1.0` — exactly one frame, the measured visibility
> floor — while the other profiles keep `1.2`.

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
  is per-profile overridable (hold/min_hold = 1.0 for local_precise / 1.2 for the others).
- `floor_us` = the profile's **absolute target / wall** — its real character.

Rounding is **`ceil` in both places**, and this is a safety requirement: a visibility floor must
never come out *shorter* than a real frame. The frame period itself is `ceil(1_000_000 / fps)` (not
`round`, which truncated e.g. `1e6/144 = 6944.44 → 6944`, putting a 1.0-frame floor *below* a real
frame), and the outer `ceil(frames × frame_us)` matches the historical `math.ceil`. Both
`FrameTimingPolicy.from_timing_policy` and `validation._frame_coupled_us` must use the identical
computation.

## 2. Why floors are absolute (local vs remote)

- **Local visibility** must survive the *local* client's per-frame sampling → the frame term.
- **Online survivability** depends on the *remote* client's frame sampling + network, which are
  independent of local FPS → an absolute µs floor. A 144 FPS local player must still emit
  durations a ~60 FPS remote listener can capture.
- The ~60 Hz onset cadence (Appendix A) is an absolute-time effect of the same kind — it does
  **not** scale with render FPS.

So `floor_us` is where a profile says "regardless of my local FPS, never go below this."

## 3. Parameter taxonomy

| Parameter             | Representation                                                              |
| --------------------- | -------------------------------------------------------------------------- |
| `hold`                | `hold_frames` + `hold_floor_us`                                            |
| `min_hold`            | `min_hold_frames` + `min_hold_floor_us`                                    |
| `spin_threshold`      | `spin_threshold_us`                                                        |
| `focus_restore_grace` | `focus_restore_grace_us`                                                   |

`*_unframed_us` keys are the conservative fallback used only on the no-FPS / `game_fps = 0`
path (frame-aware disabled).

`spin_threshold` and `focus_restore_grace` are engine infrastructure and should converge to global
policy values after O10.5/O10.6, not profile timing semantics.

## 4. Resolution pipeline (single layer)

```
frame_us = ceil(1_000_000 / fps)        # fps > 0; fps == 0/None disables frame-aware sizing

hold_us       = max(ceil(hold_frames     * frame_us), hold_floor_us)
min_hold_us   = max(ceil(min_hold_frames * frame_us), min_hold_floor_us)

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
- `*_floor_us >= 0`; `ABSOLUTE_MIN_HOLD_US` backstop.
- `validate_audience_safe_profile` enforces the audience floor thresholds (see `validation.py`).

## 7. Current floors

The authoritative per-profile floors live in `config.py::DEFAULT_TIMING_PROFILES` and are
documented with rationale in `timing-principles.md` Appendix A.9. `test_empirical_floors.py`
pins the materialised values at 30/60/144 as a regression. Summary of intent:

| profile        | hold floor | notes                                                       |
| -------------- | ---------- | ----------------------------------------------------------- |
| local_precise  | 0          | pure frame-relative = the measured visibility model; sharpest |
| dense_safe     | 11000      | small body floor for dense local playback                   |
| balanced       | 14000      | default; a little extra body above local                    |
| audience_safe  | 18000      | small remote margin above the registration floor (EXP-4)    |
