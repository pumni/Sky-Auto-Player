from __future__ import annotations

import math
from dataclasses import dataclass
from enum import StrEnum
from typing import Literal

from sky_music.config import AppConfig
from sky_music.domain.domain import Microseconds, NoteKey, ScanCode


class ActionKind(StrEnum):
    DOWN = "down"
    UP = "up"


@dataclass(frozen=True, slots=True)
class KeyAction:
    kind: ActionKind
    scan_codes: tuple[ScanCode, ...]
    at_us: Microseconds
    reason: str = "note"


def get_calibrated_margin_recommendation() -> int | None:
    import json
    from pathlib import Path
    path = Path(".cache/input_latency.json")
    if not path.exists():
        return None
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        if not isinstance(data, dict) or data.get("version") != 1:
            return None
        down_us = data.get("down_us")
        up_us = data.get("up_us")
        if not isinstance(down_us, dict) or not isinstance(up_us, dict):
            return None
        p99_down = down_us.get("p99")
        p50_up = up_us.get("p50")
        if not isinstance(p99_down, (int, float)) or not isinstance(p50_up, (int, float)):
            return None
        if p99_down < 0 or p50_up < 0 or p99_down > 100_000 or p50_up > 100_000:
            return None
        # Recommended formula: margin_rec = clamp(300, 2000, p99(down_delivery) - p50(up_delivery) + 100)
        # Put this comment for human review: margin_rec = clamp(300, 2000, p99(down_delivery) - p50(up_delivery) + 100)
        margin_rec = p99_down - p50_up + 100
        margin_rec = max(300.0, min(2000.0, margin_rec))
        return round(margin_rec)
    except Exception:
        return None


@dataclass(frozen=True, slots=True)
class TimingPolicy:
    hold_us: Microseconds
    min_hold_us: Microseconds

    focus_restore_grace_us: Microseconds = Microseconds(100_000) # Default is overridden in from_dict

    same_key_conflict_policy: Literal["degraded", "strict"] = "degraded"
    hold_frames: float = 1.25
    min_hold_frames: float = 1.25
    hold_override_us: Microseconds | None = None
    min_hold_override_us: Microseconds | None = None
    hold_uses_frame_model: bool = False
    min_hold_uses_frame_model: bool = False

    # Intra-chord micro-stagger (remote-reliability knob, onset-only). 0 = OFF (one SendInput per
    # chord, the local-optimal default). When > 0 the scheduler spreads a chord's key-downs by this
    # per-key step (capped at chord_stagger_max_us total) so each note lands in its own game tick /
    # network update — mitigates remote-listener note drops on dense chords. See
    # docs/chord-stagger-remote-drops.md. Never shifts a chord earlier: the first key keeps the
    # authored onset; later keys are pushed forward only.
    chord_stagger_us: Microseconds = Microseconds(0)
    chord_stagger_max_us: Microseconds = Microseconds(15_000)

    # Device-delivery margin added on top of the frame-model materialisation of hold/min_hold
    # (frame branch only; explicit *_override_us values win verbatim; the unframed fallback gets
    # no margin). Covers the residual kernel delivery latency after SendInput returns (<~0.5ms,
    # previously acknowledged but unaccounted) plus down-vs-up delivery asymmetry, which is the
    # only sender-side mechanism that can SHORTEN the game-observed hold. 0 restores the pure
    # ratio model bit-for-bit. See docs/timing-principles.md §2 and the round-2 overhaul plan.
    min_hold_margin_us: Microseconds = Microseconds(500)

    @classmethod
    def from_dict(cls, p_dict: dict, **kwargs) -> TimingPolicy:
        from sky_music.config import DEFAULT_TIMING_PROFILES
        base = DEFAULT_TIMING_PROFILES["balanced"]
        declares_hold = any(
            key in p_dict
            for key in ("hold_us", "hold_frames", "hold_unframed_us")
        )

        def int_value(key: str, default: int) -> int:
            return int(p_dict.get(key, default))

        def float_value(key: str, default: float) -> float:
            return float(p_dict.get(key, default))

        def frame_coupled(
            *,
            value_key: str,
            frames_key: str,
            unframed_key: str,
            default_frames: float,
            fallback_value: int | None = None,
            fallback_frames: float | None = None,
        ) -> tuple[Microseconds, float, Microseconds | None, bool]:
            _bv_raw = base.get(value_key, base.get(unframed_key, 0))
            base_value = int(_bv_raw) if _bv_raw is not None else 0
            _buv_raw = base.get(unframed_key, base_value)
            base_unframed_value = int(_buv_raw) if _buv_raw is not None else base_value
            has_frame_model = frames_key in p_dict or (
                fallback_frames is not None and value_key in p_dict
            )
            default_value = (
                fallback_value if fallback_value is not None else base_unframed_value
            )
            value = int_value(value_key, int_value(unframed_key, default_value))
            override = Microseconds(value) if has_frame_model and value_key in p_dict else None
            frames = float_value(
                frames_key,
                fallback_frames if fallback_frames is not None else default_frames,
            )
            return Microseconds(value), frames, override, has_frame_model

        min_hold_us, min_hold_frames, min_hold_override_us, min_hold_uses_frame_model = frame_coupled(
            value_key="min_hold_us",
            frames_key="min_hold_frames",
            unframed_key="min_hold_unframed_us",
            default_frames=1.25,
        )

        if declares_hold:
            hold_us, hold_frames, hold_override_us, hold_uses_frame_model = frame_coupled(
                value_key="hold_us",
                frames_key="hold_frames",
                unframed_key="hold_unframed_us",
                default_frames=1.25,
                fallback_value=int(min_hold_us),
                fallback_frames=min_hold_frames,
            )
        else:
            hold_us = min_hold_us
            hold_frames = min_hold_frames
            hold_override_us = min_hold_override_us
            hold_uses_frame_model = min_hold_uses_frame_model
        
        chord_stagger_us = max(0, int_value("chord_stagger_us", 0))
        chord_stagger_max_us = max(0, int_value("chord_stagger_max_us", 15_000))
        
        if "min_hold_margin_us" in p_dict:
            min_hold_margin_us = max(0, int(p_dict["min_hold_margin_us"]))
        else:
            calibrated = get_calibrated_margin_recommendation()
            min_hold_margin_us = calibrated if calibrated is not None else 500



        return cls(
            hold_us=hold_us,
            min_hold_us=min_hold_us,
            chord_stagger_us=Microseconds(chord_stagger_us),
            chord_stagger_max_us=Microseconds(chord_stagger_max_us),
            min_hold_margin_us=Microseconds(min_hold_margin_us),
            focus_restore_grace_us=Microseconds(int_value("focus_restore_grace_us", int(base["focus_restore_grace_us"]))),
            same_key_conflict_policy=(
                p_dict.get("same_key_conflict_policy", "degraded")
                if p_dict.get("same_key_conflict_policy", "degraded") in ("degraded", "strict")
                else "degraded"
            ),
            hold_frames=hold_frames,
            min_hold_frames=min_hold_frames,
            hold_override_us=hold_override_us,
            min_hold_override_us=min_hold_override_us,
            hold_uses_frame_model=hold_uses_frame_model,
            min_hold_uses_frame_model=min_hold_uses_frame_model,
            **kwargs
        )

    @classmethod
    def from_profile_name(cls, name: str, cfg: AppConfig | None = None) -> TimingPolicy:
        from sky_music.config import load_config, profile_dict_for

        cfg = cfg or load_config()
        return cls.from_dict(profile_dict_for(cfg, name))

    @classmethod
    def local_precise(cls) -> TimingPolicy:
        return cls.from_profile_name("local_precise")

    @classmethod
    def audience_safe(cls) -> TimingPolicy:
        return cls.from_profile_name("audience_safe")

    @classmethod
    def balanced(cls) -> TimingPolicy:
        return cls.from_profile_name("balanced")


@dataclass(frozen=True, slots=True)
class FrameTimingPolicy:
    fps: int
    frame_us: Microseconds

    hold_us: Microseconds
    min_hold_us: Microseconds

    focus_restore_grace_us: Microseconds

    same_key_conflict_policy: Literal["degraded", "strict"] = "degraded"
    profile_name: str | None = None

    # Carried unchanged from TimingPolicy; consumed by build_key_actions. See TimingPolicy docstring.
    chord_stagger_us: Microseconds = Microseconds(0)
    chord_stagger_max_us: Microseconds = Microseconds(15_000)

    # The device-delivery margin that was APPLIED during materialisation (frame branch only).
    # Recorded on the resolved policy so telemetry/diagnostics can state what was added; 0 when
    # the frame model was inactive or the value came from an explicit override.
    min_hold_margin_us: Microseconds = Microseconds(0)

    @staticmethod
    def materialise_frame_us(frames: float, frame_us: int) -> Microseconds:
        return Microseconds(round(frames * frame_us))

    @classmethod
    def from_timing_policy(
        cls,
        policy: TimingPolicy,
        fps: int | None = None,
        min_visible_hold_frames: float = 1.25,
        same_key_conflict_policy: Literal["degraded", "strict"] | None = None,
        min_hold_min_frame_ratio: float = 1.25,
        *,
        profile_name: str | None = None,
    ) -> FrameTimingPolicy:
        if fps is not None and fps > 0:
            # Round the frame period UP: a visibility/safety floor must never be shorter than a
            # real frame. round() truncates (e.g. 1e6/144 = 6944.44 -> 6944, which is below a real
            # frame), so a 1.0-frame floor would silently drop into the sub-frame probabilistic
            # zone. ceil() keeps `ceil(frames * frame_us) >= frames * real_frame`. Must match the
            # identical computation in domain/validation.py:_frame_coupled_us.
            frame_us = Microseconds(math.ceil(1_000_000 / fps))
            hold_frames = policy.hold_frames if policy.hold_uses_frame_model else min_visible_hold_frames
            min_hold_frames = (
                policy.min_hold_frames
                if policy.min_hold_uses_frame_model
                else min_hold_min_frame_ratio
            )
            # Device-delivery margin (see TimingPolicy.min_hold_margin_us): added AFTER the
            # ceil-materialisation, to BOTH hold and min_hold so the min_hold <= hold ordering
            # invariant survives (built-ins derive hold from min_hold with equal frames), and
            # BEFORE the override check so explicit *_override_us values win verbatim without
            # margin. The unframed branch below gets no margin — those fallbacks already carry
            # ample slack. Must match domain/validation.py:_frame_coupled_us exactly.
            applied_margin_us = max(0, int(policy.min_hold_margin_us))
            eff_hold_us = Microseconds(
                int(cls.materialise_frame_us(hold_frames, int(frame_us))) + applied_margin_us
            )
            eff_min_hold_us = Microseconds(
                int(cls.materialise_frame_us(min_hold_frames, int(frame_us))) + applied_margin_us
            )

            if policy.hold_override_us is not None:
                eff_hold_us = policy.hold_override_us
            if policy.min_hold_override_us is not None:
                eff_min_hold_us = policy.min_hold_override_us
                applied_margin_us = 0
        else:
            frame_us = Microseconds(0)
            eff_hold_us = policy.hold_us
            eff_min_hold_us = policy.min_hold_us
            applied_margin_us = 0
            
        conflict_policy = same_key_conflict_policy if same_key_conflict_policy is not None else policy.same_key_conflict_policy
        if conflict_policy not in ("strict", "degraded"):
            conflict_policy = "degraded"

        return cls(
            fps=fps if fps is not None else 0,
            frame_us=frame_us,
            hold_us=eff_hold_us,
            min_hold_us=eff_min_hold_us,
            focus_restore_grace_us=policy.focus_restore_grace_us,
            same_key_conflict_policy=conflict_policy,
            profile_name=profile_name,
            chord_stagger_us=policy.chord_stagger_us,
            chord_stagger_max_us=policy.chord_stagger_max_us,
            min_hold_margin_us=Microseconds(applied_margin_us),
        )

    @classmethod
    def from_profile_name(cls, name: str, fps: int | None = None, **kwargs) -> FrameTimingPolicy:
        policy = TimingPolicy.from_profile_name(name)
        return cls.from_timing_policy(policy, fps=fps, profile_name=name, **kwargs)

    @classmethod
    def local_precise(cls, **kwargs) -> FrameTimingPolicy:
        return cls.from_profile_name("local_precise", **kwargs)

    @classmethod
    def audience_safe(cls, **kwargs) -> FrameTimingPolicy:
        return cls.from_profile_name("audience_safe", **kwargs)

    @classmethod
    def balanced(cls, **kwargs) -> FrameTimingPolicy:
        return cls.from_profile_name("balanced", **kwargs)


@dataclass(frozen=True, slots=True)
class ScheduleDiagnostic:
    source_index: int
    note_key: NoteKey
    scan_code: int
    # Shared diagnostic vocabulary. build_key_actions() only ever emits "impossible_repeat"; the
    # other codes are produced by domain/validation.py against the built schedule.
    code: Literal["negative_timestamp", "duplicate_down", "stuck_keys", "impossible_repeat", "frame_lateness", "gap_below_frame"]
    message: str


@dataclass(frozen=True, slots=True)
class ScheduleMetadata:
    actions: tuple[KeyAction, ...]
    source_duration_us: Microseconds
    playback_duration_us: Microseconds
    compressed_holds: int = 0
    impossible_same_key_repeats: int = 0
    risky_same_key_repeats: int = 0
    deduplicated_note_count: int = 0
    duplicate_note_count: int = 0
    max_polyphony: int = 0
    note_count: int = 0
    shortest_same_key_interval_us: int | None = None
    min_same_key_up_gap_us: int | None = None
    warnings: tuple[str, ...] = ()
    duration_us: Microseconds = Microseconds(0)
    diagnostics: tuple[ScheduleDiagnostic, ...] = ()
    recommended_profile: str | None = None
    recommended_tempo_scale: float | None = None
    sub_60fps_frame_notes: int = 0
    gap_below_frame_repeats: int = 0
