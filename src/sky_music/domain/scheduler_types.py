from __future__ import annotations

from dataclasses import dataclass
from typing import Literal
import math

from sky_music.domain.domain import NoteKey, ScanCode, Microseconds

from sky_music.config import AppConfig

ActionKind = Literal["down", "up"]


@dataclass(frozen=True, slots=True)
class KeyAction:
    kind: ActionKind
    scan_codes: tuple[ScanCode, ...]
    at_us: Microseconds
    reason: str = "note"


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

    @classmethod
    def from_dict(cls, p_dict: dict, **kwargs) -> "TimingPolicy":
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
            base_value = int(base.get(value_key, base.get(unframed_key, 0)))
            base_unframed_value = int(base.get(unframed_key, base_value))
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
        
        return cls(
            hold_us=hold_us,
            min_hold_us=min_hold_us,
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
    def from_profile_name(cls, name: str, cfg: AppConfig | None = None) -> "TimingPolicy":
        from sky_music.config import load_config, profile_dict_for

        cfg = cfg or load_config()
        return cls.from_dict(profile_dict_for(cfg, name))

    @classmethod
    def local_precise(cls) -> "TimingPolicy":
        return cls.from_profile_name("local_precise")

    @classmethod
    def audience_safe(cls) -> "TimingPolicy":
        return cls.from_profile_name("audience_safe")

    @classmethod
    def balanced(cls) -> "TimingPolicy":
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
    ) -> "FrameTimingPolicy":
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
            eff_hold_us = cls.materialise_frame_us(hold_frames, int(frame_us))
            eff_min_hold_us = cls.materialise_frame_us(min_hold_frames, int(frame_us))

            if policy.hold_override_us is not None:
                eff_hold_us = policy.hold_override_us
            if policy.min_hold_override_us is not None:
                eff_min_hold_us = policy.min_hold_override_us
        else:
            frame_us = Microseconds(0)
            eff_hold_us = policy.hold_us
            eff_min_hold_us = policy.min_hold_us
            
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
        )

    @classmethod
    def from_profile_name(cls, name: str, fps: int | None = None, **kwargs) -> "FrameTimingPolicy":
        policy = TimingPolicy.from_profile_name(name)
        return cls.from_timing_policy(policy, fps=fps, profile_name=name, **kwargs)

    @classmethod
    def local_precise(cls, **kwargs) -> "FrameTimingPolicy":
        return cls.from_profile_name("local_precise", **kwargs)

    @classmethod
    def audience_safe(cls, **kwargs) -> "FrameTimingPolicy":
        return cls.from_profile_name("audience_safe", **kwargs)

    @classmethod
    def balanced(cls, **kwargs) -> "FrameTimingPolicy":
        return cls.from_profile_name("balanced", **kwargs)


@dataclass(frozen=True, slots=True)
class ScheduleDiagnostic:
    source_index: int
    note_key: NoteKey
    scan_code: int
    # Shared diagnostic vocabulary. build_key_actions() only ever emits "impossible_repeat"; the
    # other codes are produced by domain/validation.py against the built schedule.
    code: Literal["negative_timestamp", "duplicate_down", "stuck_keys", "impossible_repeat", "frame_lateness"]
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
    same_key_compressed_holds: int = 0
    infeasible_same_key_repeats: int = 0
    max_polyphony: int = 0
    note_count: int = 0
    shortest_same_key_interval_us: int | None = None
    min_same_key_up_gap_us: int | None = None
    warnings: tuple[str, ...] = ()
    duration_us: Microseconds = Microseconds(0)
    diagnostics: tuple[ScheduleDiagnostic, ...] = ()
    recommended_profile: str | None = None
    recommended_tempo_scale: float | None = None
    frame_us: Microseconds | None = None
    fps: int | None = None
