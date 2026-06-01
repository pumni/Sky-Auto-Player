# Timing Principles for Sky Music Player

This document is the engineering source of truth for designing, reviewing, and
calibrating timing profiles for Sky Music Player.

It defines:

- the timing vocabulary used by the scheduler;
- the mathematical constraints that prevent dropped or merged notes;
- the difference between general 60 FPS-safe profiles and FPS-gated profiles;
- the expected behavior for local playback, dense MIDI playback, and online
  audience playback;
- validation rules that future profiles must pass before they are exposed in the
  UI or CLI.

The numbers in this document are conservative engineering defaults. They model a
game client that samples input on frame boundaries and an online environment where
remote listeners may be less forgiving than local self-hearing. They should be
validated with real gameplay, but profile changes must start from these rules.

---

## 1. Core Terms

All timing values are expressed in microseconds.

| Term                    | Meaning                                                                                       |
| ----------------------- | --------------------------------------------------------------------------------------------- |
| `hold_us`               | Preferred key-down duration for normal notes before compression.                              |
| `min_hold_us`           | Minimum allowed key-down duration after compression. This is the visible down-state floor.    |
| `release_gap_us`        | Minimum gap after releasing a key before general scheduling continues.                        |
| `repeat_release_gap_us` | Minimum up-time before pressing the same key again. This is the critical same-key repeat gap. |
| `input_lead_us`         | How early input is sent relative to the musical timestamp.                                    |
| `chord_merge_window_us` | Window used to snap nearby notes into the same chord.                                         |
| `cycle_us`              | `min_hold_us + repeat_release_gap_us`; the critical same-key repeat cycle.                    |
| `frame_us`              | Duration of one game frame: `1_000_000 / fps`.                                                |
| `game_fps`              | The FPS value selected or calibrated by the user. `0` disables frame-aware scaling.           |
| `tempo_scale`           | Playback speed multiplier. Values above `1.0` increase scheduling pressure.                   |

---

## 2. Profile Classes

Profiles are not all validated with the same assumptions.

### 2.1 General 60 FPS-safe profiles

These profiles may be used at normal 60 FPS and must remain safe when
frame-aware scaling is disabled:

- `local_precise`
- `balanced`
- `dense_safe`
- `audience_safe`

They must satisfy the 60 FPS cycle rule:

```text
min_hold_us + repeat_release_gap_us > 16667
```

For production defaults, prefer:

```text
min_hold_us + repeat_release_gap_us >= 18000
```

### 2.2 FPS-gated high-FPS profiles

`high_fps_precise` is not a general 60 FPS-safe profile. It is a gated local-only
profile and must only be selectable when:

```text
game_fps > 100
```

If `game_fps <= 100`, the UI/CLI must refuse `high_fps_precise` or fall back to a
safe profile such as `balanced`.

The current `high_fps_precise` profile has:

```text
cycle_us = 10000 + 6000 = 16000
```

That is intentionally below the preferred 60 FPS production margin, but it is
valid for the gated high-FPS use case:

| FPS | `frame_us` | `high_fps_precise` cycle | Status                                               |
| --: | ---------: | -----------------------: | ---------------------------------------------------- |
|  60 |   16.67 ms |                 16.00 ms | Invalid; must not be selectable.                     |
| 100 |   10.00 ms |                 16.00 ms | Still blocked by policy because the gate is `> 100`. |
| 101 |    9.90 ms |                 16.00 ms | Valid if FPS is stable.                              |
| 120 |    8.33 ms |                 16.00 ms | Valid if FPS is stable.                              |
| 144 |    6.94 ms |                 16.00 ms | Valid if FPS is stable.                              |

This profile should not be used for online audience playback.

### 2.3 Online audience-safe profiles

Online audience playback optimizes for what other players hear, not only what the
local machine hears. It should use larger holds, larger same-key release gaps,
larger chord merge windows, and more input lead.

`audience_safe` is the default online/audience profile. Aliases such as
`remote_safe`, `online_audible_safe`, and `online_audible` should resolve to
`audience_safe` unless a separate remote/cloud transport profile is implemented.

---

## 3. Principle 1 — The Cycle Rule

Same-key repeats require a complete state transition:

```text
DOWN -> visible hold -> UP -> visible release -> DOWN again
```

The critical cycle is:

```text
cycle_us = min_hold_us + repeat_release_gap_us
```

A same-key repeat can be dropped or merged if the game client does not observe a
complete down/up/down sequence across its input sampling frames.

### 3.1 General rule

For any profile that may be selected at the current FPS:

```text
cycle_us > frame_us
```

For profiles exposed as generally safe at 60 FPS:

```text
cycle_us > 16667
```

Recommended practical minimums:

| Target FPS | Frame duration | Practical minimum cycle |
| ---------: | -------------: | ----------------------: |
|     30 FPS |       33.33 ms |                36–40 ms |
|     60 FPS |       16.67 ms |                18–22 ms |
|     90 FPS |       11.11 ms |                13–16 ms |
|   101+ FPS |     <= 9.90 ms |                14–18 ms |
|    120 FPS |        8.33 ms |                10–16 ms |

### 3.2 Why `repeat_release_gap_us` matters more than `release_gap_us`

`release_gap_us` protects general scheduling after a key-up event.

`repeat_release_gap_us` protects repeated notes on the same key. It must be large
enough for the game, OS input layer, and possible online replication path to see
the key as released before the next press.

For repeat-heavy songs, tune `repeat_release_gap_us` before reducing
`min_hold_us`.

---

## 4. Principle 2 — The Visibility Rule

`min_hold_us` must be long enough for the game client to observe the key as down.
A short hold can look correct in scheduler logs but still fail in the game.

Recommended ranges:

| Mode                                   | Recommended `min_hold_us` |
| -------------------------------------- | ------------------------: |
| Gated high-FPS local, `game_fps > 100` |                  10–12 ms |
| Sharp local 60 FPS                     |                  12–14 ms |
| Balanced local 60 FPS                  |                  14–16 ms |
| Dense local MIDI                       |                  12–16 ms |
| Online audience-safe                   |                  17–21 ms |
| Remote/cloud input transport           |                  20–24 ms |

Do not lower `min_hold_us` only to make dense songs faster. If density causes
misses, first try:

1. increasing `repeat_release_gap_us`;
2. increasing `chord_merge_window_us` slightly;
3. using `dense_safe` or `audience_safe`;
4. reducing `tempo_scale`.

---

## 5. Principle 3 — The Same-Key Release Rule

`repeat_release_gap_us` is the most important value for same-key repeated notes.

Recommended ranges:

| Mode                                   | Recommended `repeat_release_gap_us` |
| -------------------------------------- | ----------------------------------: |
| Gated high-FPS local, `game_fps > 100` |                              6–7 ms |
| Sharp local 60 FPS                     |                              6–7 ms |
| Balanced local 60 FPS                  |                              7–8 ms |
| Dense local MIDI                       |                             8–10 ms |
| Online audience-safe                   |                            12–16 ms |
| Remote/cloud input transport           |                            15–20 ms |

Rules:

- Same-key repeats should never use `release_gap_us` as their only protection.
- Online audience playback should not use tiny same-key release gaps.
- If local playback sounds fine but remote listeners miss repeated notes,
  increase `repeat_release_gap_us` or switch to `audience_safe`.

---

## 6. Principle 4 — The Chord Batching Rule

`chord_merge_window_us` controls how nearby notes are snapped into one chord.

Recommended ranges:

| Mode                         | Recommended `chord_merge_window_us` |
| ---------------------------- | ----------------------------------: |
| Expressive local play        |                              2–3 ms |
| Balanced local play          |                                3 ms |
| Dense MIDI local play        |                              3–4 ms |
| Online audience-safe         |                              5–7 ms |
| Remote/cloud input transport |                              6–8 ms |

Small windows preserve intentional strums and MIDI imperfections. Larger windows
reduce scheduling pressure and make chords more coherent for remote listeners.

Do not make this too large for local expressive play. Excessive batching can turn
intended arpeggios into flat chords.

---

## 7. Principle 5 — The Input Lead Rule

`input_lead_us` sends input slightly before the musical timestamp.

Recommended ranges:

| Mode                         | Recommended `input_lead_us` |
| ---------------------------- | --------------------------: |
| Strong local PC              |                      3–4 ms |
| Normal local PC              |                      4–6 ms |
| Dense local playback         |                      5–8 ms |
| Online audience-safe         |                    10–14 ms |
| Remote/cloud input transport |                    12–18 ms |

Input lead compensates for scheduler delay, driver/input injection delay, frame
boundary delay, and extra perceived delay in online rooms.

Rules:

- Increase gradually; too much lead can make local playback feel early.
- Prefer calibrating `input_lead_us` over changing hold/gap values when the only
  symptom is consistent lateness.
- For online audience playback, use `audience_safe` before manually increasing
  lead beyond the recommended range.

---

## 8. Frame-Aware Scaling

Frame-aware scaling adapts timing floors to the configured game FPS.

Important rules:

1. If `game_fps <= 0`, frame-aware scaling is disabled.
2. Do not persist already-scaled profile values.
3. Persist the base profile, calibrated FPS, tempo scale, and input lead
   separately.
4. Profile selection guards run before playback. `high_fps_precise` must be
   rejected unless `game_fps > 100`.
5. Scaling may raise floors for stability, but it should not silently make a
   blocked profile selectable.
6. Frame alignment must be conservative. Aligning press-down events can improve
   capture consistency, but aggressive alignment can increase latency and harm
   musical timing.

Suggested frame policy floors:

```text
min_visible_hold_frames >= 1.25
input_lead_min_frame_ratio >= 0.50
release_gap_min_frame_ratio >= 0.15
repeat_release_gap_min_frame_ratio >= 0.50
min_hold_min_frame_ratio >= 0.60
```

These are floors, not targets. Profile-specific values may intentionally be
larger.

### 8.1 Frame alignment

Supported values:

| Value       | Meaning                                                                                          |
| ----------- | ------------------------------------------------------------------------------------------------ |
| `none`      | Do not align events to frame boundaries. Best default.                                           |
| `down_only` | Align press-down events conservatively. Can improve capture consistency but may add timing bias. |

`down_only` should be opt-in, calibrated, and easy to disable.

---

## 9. Recommended Built-In Profiles

The following profiles should match `DEFAULT_TIMING_PROFILES`.

### 9.1 `local_precise`

Sharp local playback on a stable PC.

Eligibility:

- local playback;
- not optimized for online audience audibility;
- general 60 FPS-safe.

```python
"local_precise": {
    "hold_us": 20000,
    "min_hold_us": 12000,
    "release_gap_us": 3000,
    "repeat_release_gap_us": 6000,
    "min_scheduled_hold_us": 500,
    "input_lead_us": 3000,
    "chord_merge_window_us": 2000,
    "spin_threshold_us": 800,
    "focus_restore_grace_us": 50000
}
```

Derived values:

```text
cycle_us = 12000 + 6000 = 18000
```

---

### 9.2 `balanced`

Recommended default.

Eligibility:

- normal local playback;
- safe default when the user has not calibrated anything;
- general 60 FPS-safe.

```python
"balanced": {
    "hold_us": 24000,
    "min_hold_us": 14000,
    "release_gap_us": 4000,
    "repeat_release_gap_us": 7000,
    "min_scheduled_hold_us": 500,
    "input_lead_us": 6000,
    "chord_merge_window_us": 3000,
    "spin_threshold_us": 500,
    "focus_restore_grace_us": 100000
}
```

Derived values:

```text
cycle_us = 14000 + 7000 = 21000
```

---

### 9.3 `dense_safe`

Dense local MIDI playback.

Eligibility:

- dense songs;
- local-first playback;
- general 60 FPS-safe;
- not the first choice for online audience sessions.

```python
"dense_safe": {
    "hold_us": 20000,
    "min_hold_us": 12000,
    "release_gap_us": 5000,
    "repeat_release_gap_us": 8000,
    "min_scheduled_hold_us": 500,
    "input_lead_us": 6000,
    "chord_merge_window_us": 3000,
    "spin_threshold_us": 500,
    "focus_restore_grace_us": 100000
}
```

Derived values:

```text
cycle_us = 12000 + 8000 = 20000
```

---

### 9.4 `audience_safe`

> [!NOTE]
> The `audience_safe` profile replaces and consolidates the legacy `remote_safe` profile (along with other legacy aliases like `online_audible_safe` and `online_audible`) to maintain strict timing and naming consistency across the codebase.

Optimized for online room sessions where other players must hear the notes
reliably.

Eligibility:

- online/audience playback;
- remote audibility first;
- repeated-note safety first;
- general 60 FPS-safe.

```python
"audience_safe": {
    "hold_us": 32000,
    "min_hold_us": 18000,
    "release_gap_us": 7000,
    "repeat_release_gap_us": 13000,
    "min_scheduled_hold_us": 500,
    "input_lead_us": 13000,
    "chord_merge_window_us": 5000,
    "spin_threshold_us": 500,
    "focus_restore_grace_us": 150000
}
```

Derived values:

```text
cycle_us = 18000 + 13000 = 31000
```

---

### 9.5 `high_fps_precise`

Verified high-FPS local playback.

Eligibility:

- local-only playback;
- `game_fps > 100` is required;
- FPS must be stable and not frequently dipping below the configured value;
- not safe as a 60 FPS general profile;
- not recommended for online audience playback.

```python
"high_fps_precise": {
    "hold_us": 18000,
    "min_hold_us": 10000,
    "release_gap_us": 3000,
    "repeat_release_gap_us": 6000,
    "min_scheduled_hold_us": 500,
    "input_lead_us": 3000,
    "chord_merge_window_us": 2000,
    "spin_threshold_us": 500,
    "focus_restore_grace_us": 50000
}
```

Derived values:

```text
cycle_us = 10000 + 6000 = 16000
```

This profile is valid because selection is gated by `game_fps > 100`. If that
guard is removed, this profile becomes unsafe for normal 60 FPS assumptions and
must be changed or hidden.

---

## 10. Profile Selection Policy

Selection must be explicit, predictable, and safe by default.

Recommended policy:

```python
GENERAL_60FPS_SAFE_PROFILES: set[str] = {
    "local_precise",
    "balanced",
    "dense_safe",
    "audience_safe",
}

FPS_GATED_PROFILES: dict[str, int] = {
    # Strictly greater than this FPS.
    "high_fps_precise": 100,
}

ONLINE_AUDIENCE_PROFILE = "audience_safe"
DEFAULT_PROFILE = "balanced"
```

Profile resolution:

```python
def resolve_profile(
    requested_profile: str,
    *,
    game_fps: int,
    online_audience_mode: bool,
) -> str:
    profile = requested_profile.lower().replace("-", "_")

    if profile in {"remote_safe", "online_audible_safe", "online_audible"}:
        profile = "audience_safe"

    if online_audience_mode:
        return "audience_safe"

    if profile == "high_fps_precise" and game_fps <= 100:
        raise ValueError("high_fps_precise requires game_fps > 100")

    if profile not in GENERAL_60FPS_SAFE_PROFILES and profile not in FPS_GATED_PROFILES:
        return DEFAULT_PROFILE

    return profile
```

Do not silently run `high_fps_precise` at 60 FPS. Either block it with a clear
message or fall back to `balanced` with a visible warning.

---

## 11. Symptom-Based Tuning

| Symptom                                        | Likely cause                                                                      | First adjustment                                                            |
| ---------------------------------------------- | --------------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| Same-key repeats drop locally                  | Cycle too short                                                                   | Increase `repeat_release_gap_us`.                                           |
| Notes randomly vanish locally                  | Hold too short or FPS lower than expected                                         | Increase `min_hold_us` or use `balanced`.                                   |
| `high_fps_precise` fails intermittently        | FPS is not stable above 100, or game/input layer is sampling slower than expected | Use `balanced` or `local_precise`; do not lower gaps further.               |
| Local sounds fine, other players miss notes    | Online replication/audience path is not surviving dense timing                    | Use `audience_safe`.                                                        |
| Other players hear incomplete repeated notes   | Same-key release too short for online audibility                                  | Increase `repeat_release_gap_us` or use `audience_safe`.                    |
| Other players hear chords as rattly/incomplete | Chord events are too spread out                                                   | Increase `chord_merge_window_us`.                                           |
| Playback sounds late locally and remotely      | Lead too small                                                                    | Increase `input_lead_us` gradually.                                         |
| Playback sounds early locally                  | Lead too large                                                                    | Decrease `input_lead_us`.                                                   |
| Dense song collapses                           | Too much scheduling pressure                                                      | Use `dense_safe`, use `audience_safe`, or reduce `tempo_scale`.             |
| Local play feels mushy                         | Hold/gaps too large                                                               | Use `balanced`, `local_precise`, or gated `high_fps_precise` when eligible. |

---

## 12. Validation Rules for Future Profiles

Every new profile must declare its class:

- general 60 FPS-safe;
- FPS-gated local-only;
- online/audience-safe;
- experimental hidden profile.

Do not validate all classes with the same assumptions.

### 12.1 Typed validation helpers

```python
from typing import Final, Literal, TypedDict

ProfileClass = Literal["general_60fps", "fps_gated", "audience_safe", "experimental"]

class TimingProfile(TypedDict):
    hold_us: int
    min_hold_us: int
    release_gap_us: int
    repeat_release_gap_us: int
    min_scheduled_hold_us: int
    input_lead_us: int
    chord_merge_window_us: int
    spin_threshold_us: int
    focus_restore_grace_us: int

GENERAL_60FPS_SAFE_PROFILES: Final[set[str]] = {
    "local_precise",
    "balanced",
    "dense_safe",
    "audience_safe",
}

FPS_GATED_MIN_EXCLUSIVE_FPS: Final[dict[str, int]] = {
    "high_fps_precise": 100,
}

AUDIENCE_SAFE_PROFILES: Final[set[str]] = {"audience_safe"}


def frame_us_for(fps: int) -> float:
    if fps <= 0:
        raise ValueError("fps must be > 0")
    return 1_000_000 / fps


def cycle_us(profile: TimingProfile) -> int:
    return profile["min_hold_us"] + profile["repeat_release_gap_us"]
```

### 12.2 General profile validation

```python
def validate_general_60fps_profile(name: str, profile: TimingProfile) -> None:
    cycle = cycle_us(profile)

    if cycle <= 16_667:
        raise ValueError(f"{name}: cycle_us must be > 16667us for 60 FPS safety")

    if cycle < 18_000:
        raise ValueError(f"{name}: cycle_us should be >= 18000us for production margin")

    if profile["min_hold_us"] < 10_000:
        raise ValueError(f"{name}: min_hold_us below 10000us is not allowed")

    if profile["repeat_release_gap_us"] < 6_000:
        raise ValueError(f"{name}: repeat_release_gap_us below 6000us is not allowed")

    if profile["input_lead_us"] < 0:
        raise ValueError(f"{name}: input_lead_us must be non-negative")

    if profile["chord_merge_window_us"] < 0:
        raise ValueError(f"{name}: chord_merge_window_us must be non-negative")
```

### 12.3 FPS-gated profile validation

```python
def validate_fps_gated_profile(
    name: str,
    profile: TimingProfile,
    *,
    selected_fps: int,
) -> None:
    min_exclusive_fps = FPS_GATED_MIN_EXCLUSIVE_FPS[name]

    if selected_fps <= min_exclusive_fps:
        raise ValueError(f"{name}: requires selected_fps > {min_exclusive_fps}")

    frame_us = frame_us_for(selected_fps)
    cycle = cycle_us(profile)

    if cycle <= frame_us:
        raise ValueError(
            f"{name}: unsafe cycle {cycle}us <= frame {frame_us:.0f}us at {selected_fps} FPS"
        )

    if profile["min_hold_us"] < 10_000:
        raise ValueError(f"{name}: min_hold_us below 10000us is not allowed")

    if profile["repeat_release_gap_us"] < 6_000:
        raise ValueError(f"{name}: repeat_release_gap_us below 6000us is not allowed")

    if profile["input_lead_us"] < 0:
        raise ValueError(f"{name}: input_lead_us must be non-negative")

    if profile["chord_merge_window_us"] < 0:
        raise ValueError(f"{name}: chord_merge_window_us must be non-negative")
```

### 12.4 Audience-safe validation

```python
def validate_audience_safe_profile(name: str, profile: TimingProfile) -> None:
    cycle = cycle_us(profile)

    if cycle < 28_000:
        raise ValueError(f"{name}: audience-safe cycle_us should be >= 28000us")

    if profile["min_hold_us"] < 17_000:
        raise ValueError(f"{name}: audience-safe min_hold_us must be >= 17000us")

    if profile["repeat_release_gap_us"] < 12_000:
        raise ValueError(f"{name}: audience-safe repeat_release_gap_us must be >= 12000us")

    if profile["input_lead_us"] < 10_000:
        raise ValueError(f"{name}: audience-safe input_lead_us must be >= 10000us")

    if profile["chord_merge_window_us"] < 5_000:
        raise ValueError(f"{name}: audience-safe chord_merge_window_us must be >= 5000us")
```

### 12.5 Dispatcher

```python
def validate_timing_profile(
    name: str,
    profile: TimingProfile,
    *,
    selected_fps: int | None = None,
) -> None:
    if name in FPS_GATED_MIN_EXCLUSIVE_FPS:
        if selected_fps is None:
            raise ValueError(f"{name}: selected_fps is required for FPS-gated profiles")
        validate_fps_gated_profile(name, profile, selected_fps=selected_fps)
        return

    validate_general_60fps_profile(name, profile)

    if name in AUDIENCE_SAFE_PROFILES:
        validate_audience_safe_profile(name, profile)
```

---

## 13. Current Profile Matrix

| Profile            | Class                | `cycle_us` | 60 FPS general-safe | Online audience-safe | Notes                                   |
| ------------------ | -------------------- | ---------: | ------------------- | -------------------- | --------------------------------------- |
| `local_precise`    | general 60 FPS-safe  |      18 ms | Yes                 | No                   | Sharp local playback.                   |
| `balanced`         | general 60 FPS-safe  |      21 ms | Yes                 | Limited              | Recommended default.                    |
| `dense_safe`       | general 60 FPS-safe  |      20 ms | Yes                 | Limited              | Dense local playback.                   |
| `audience_safe`    | audience-safe        |      31 ms | Yes                 | Yes                  | Best built-in profile for online rooms. |
| `high_fps_precise` | FPS-gated local-only |      16 ms | No                  | No                   | Valid only when `game_fps > 100`.       |

---

## 14. Tuning Order

When playback is unreliable, tune in this order:

1. Confirm the correct profile class.
2. Confirm the configured FPS and whether FPS is stable.
3. Confirm `tempo_scale` is not creating unrealistic density.
4. For dropped same-key repeats, increase `repeat_release_gap_us`.
5. For vanished notes, increase `min_hold_us`.
6. For remote/audience misses, switch to `audience_safe`.
7. For rattly chords, increase `chord_merge_window_us` slightly.
8. For consistent lateness, adjust `input_lead_us`.
9. Only after those steps, consider changing `hold_us` or adding a new profile.

Do not reduce safety floors to make logs look faster. The goal is reliable
registration in the game and, when online, reliable audibility for other players.

---

## 15. Non-Negotiable Rules

1. `balanced` remains the default profile.
2. `audience_safe` is the recommended online/audience profile.
3. `high_fps_precise` must require `game_fps > 100`.
4. `high_fps_precise` must not be auto-selected for online audience playback.
5. Same-key repeats must use `repeat_release_gap_us`, not only `release_gap_us`.
6. Frame-aware scaling must not persist already-scaled hold/gap values.
7. Future profiles must state their class and validation assumptions.
8. Any profile exposed at normal 60 FPS must satisfy the 60 FPS cycle rule.
9. Any change that lowers `min_hold_us` or `repeat_release_gap_us` must include a
   gameplay reason and a validation plan.
10. Online reliability wins over local sharpness when the selected mode is
    audience playback.
