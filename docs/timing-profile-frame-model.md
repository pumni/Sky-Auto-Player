# Pure Frame-Relative Timing Model

**Status:** Implemented June 2026. This is the live timing-profile reference.

Related evidence and decisions:
* [timing-principles.md](timing-principles.md): in-game measurements and historical context.
* [timing-experiments.md](timing-experiments.md): experiments and A/B observations.

---

## 1. Core Model

When FPS is known and positive, every built-in hold is materialized only from its declared frame ratio:
```text
frame_us = ceil(1_000_000 / fps)
effective_us = round(frames * frame_us)
```

There is no absolute floor and no `max(frame_term, floor_us)` step. The frame period is rounded up so a declared 1.0-frame hold never becomes shorter than a real frame. The final duration is rounded to the nearest microsecond. No fixed scheduling margins or latency buffers are added to this model.

When FPS is `None` or disabled, the profile uses `*_unframed_us`. Explicit `_us` values supplied by CLI or config remain absolute overrides and are applied after frame materialization.

---

## 2. Hold Unification

Built-ins declare only `min_hold_frames` and `min_hold_unframed_us`. Normal `hold` derives from `min_hold`, so built-in effective policies satisfy:
```text
hold_us == min_hold_us
```

An explicit `hold_us`, `hold_frames`, or `hold_unframed_us` remains an escape hatch and may separate normal hold from minimum compressed hold.

---

## 3. Current Profiles

| Profile | `min_hold_frames` | `min_hold_unframed_us` | Intent |
| :--- | :---: | :---: | :--- |
| `local_precise` | 1.0 | 22000 | Sharpest local visibility profile (no margin) |
| `audience_safe` | 1.1 | 18000 | Audience-tested sharp profile |
| `balanced` | 1.02 | 17000 | General default with more local-frame body |

`dense_safe` was removed. Schedule-stress recommendations (fast repeats / dense polyphony / infeasible same-key cycles without delivery-timing failure) select `local_precise` together with tempo reduction; severe delivery-timing failures (panic releases, p99 > 15 ms, late > 10 ms count > 5) select `audience_safe` instead, since the failure mode there is missed notes from short holds, not crowded cycles.

---

## 4. Validation Invariants

* `min_hold_frames >= 1.0`.
* If explicitly declared, `0 < min_hold_frames <= hold_frames`.
* Materialized `min_hold_us` must be greater than the real frame duration.
* Explicit hold/min-hold ordering remains `0 < min_hold_us <= hold_us`.
* Unknown legacy keys, including former `*_floor_us` keys, are ignored.

There is no audience-specific absolute-duration validator. `audience_safe` is validated by the same frame-relative rules as the other profiles.

---

## 5. Accepted Audience Risk

Removing the absolute floor makes high-FPS local holds shorter in absolute time. At 144 FPS, `balanced` materializes to 7084 $\mu\text{s}$, which may be missed by a remote client sampling around 60 FPS. This is intentional and must not be silently counteracted by introducing another absolute wall under a different name.

---

## 6. Regression Baselines

The required `hold_us == min_hold_us` values are:

| Profile | None | 30 FPS | 60 FPS | 144 FPS |
| :--- | :---: | :---: | :---: | :---: |
| `local_precise` | 22000 | 33334 | 16667 | 6945 |
| `audience_safe` | 18000 | 36667 | 18334 | 7640 |
| `balanced` | 17000 | 34001 | 17000 | 7084 |

Golden schedules use `TimingPolicy.from_dict({})` and must not be regenerated for this change.
