from dataclasses import dataclass
from typing import Literal, NewType
import math

from sky_music.domain.domain import NoteKey

Microseconds = NewType("Microseconds", int)
ScanCode = NewType("ScanCode", int)

ActionKind = Literal["down", "up"]
ActionReason = Literal["note", "release", "repeat_release", "final_release"]

@dataclass(frozen=True, slots=True)
class KeyAction:
    at_us: Microseconds
    scan_codes: tuple[ScanCode, ...]
    kind: ActionKind
    reason: ActionReason

@dataclass(frozen=True, slots=True)
class TimingPolicy:
    hold_us: Microseconds = Microseconds(20_000)
    min_hold_us: Microseconds = Microseconds(12_000)
    release_gap_us: Microseconds = Microseconds(3_000)
    repeat_release_gap_us: Microseconds = Microseconds(2_000)
    min_scheduled_hold_us: Microseconds = Microseconds(500)
    input_lead_us: Microseconds = Microseconds(0)
    chord_merge_window_us: Microseconds = Microseconds(0)
    focus_restore_grace_us: Microseconds = Microseconds(100_000)
    same_key_conflict_policy: Literal["degraded", "strict"] = "degraded"

    @classmethod
    def from_dict(cls, p_dict: dict, **kwargs) -> "TimingPolicy":
        return cls(
            hold_us=Microseconds(p_dict.get("hold_us", 24_000)),
            min_hold_us=Microseconds(p_dict.get("min_hold_us", 12_000)),
            release_gap_us=Microseconds(p_dict.get("release_gap_us", 3_000)),
            repeat_release_gap_us=Microseconds(p_dict.get("repeat_release_gap_us", 2_000)),
            min_scheduled_hold_us=Microseconds(p_dict.get("min_scheduled_hold_us", 500)),
            input_lead_us=Microseconds(p_dict.get("input_lead_us", 6_000)),
            chord_merge_window_us=Microseconds(p_dict.get("chord_merge_window_us", 2_000)),
            focus_restore_grace_us=Microseconds(p_dict.get("focus_restore_grace_us", 100_000)),
            same_key_conflict_policy=p_dict.get("same_key_conflict_policy", "degraded"),
            **kwargs
        )

    @classmethod
    def from_profile_name(cls, name: str) -> "TimingPolicy":
        from sky_music.config import load_config, DEFAULT_TIMING_PROFILES
        name_clean = name.lower().replace("-", "_")
        try:
            user_cfg = load_config()
            timing_profiles = user_cfg.timing_profiles
        except Exception:
            timing_profiles = DEFAULT_TIMING_PROFILES

        p_dict = timing_profiles.get(name_clean) or DEFAULT_TIMING_PROFILES.get(name_clean, DEFAULT_TIMING_PROFILES["balanced"])
        return cls.from_dict(p_dict)

    @classmethod
    def local_precise(cls) -> "TimingPolicy":
        return cls.from_profile_name("local_precise")

    @classmethod
    def remote_safe(cls) -> "TimingPolicy":
        return cls.from_profile_name("remote_safe")

    @classmethod
    def dense_safe(cls) -> "TimingPolicy":
        return cls.from_profile_name("dense_safe")

    @classmethod
    def balanced(cls) -> "TimingPolicy":
        return cls.from_profile_name("balanced")


@dataclass(frozen=True, slots=True)
class FrameTimingPolicy:
    fps: int
    frame_us: Microseconds

    hold_us: Microseconds
    min_hold_us: Microseconds
    release_gap_us: Microseconds
    repeat_release_gap_us: Microseconds
    min_scheduled_hold_us: Microseconds

    input_lead_us: Microseconds
    chord_merge_window_us: Microseconds
    focus_restore_grace_us: Microseconds

    min_visible_hold_frames: float = 1.25
    chord_merge_max_frame_ratio: float = 0.25
    same_key_conflict_policy: Literal["degraded", "strict", "adaptive"] = "degraded"

    @classmethod
    def from_timing_policy(
        cls,
        policy: TimingPolicy,
        fps: int | None = None,
        min_visible_hold_frames: float = 1.25,
        chord_merge_max_frame_ratio: float = 0.25,
        same_key_conflict_policy: Literal["degraded", "strict", "adaptive"] | None = None,
        input_lead_min_frame_ratio: float = 0.5,
        release_gap_min_frame_ratio: float = 0.15
    ) -> "FrameTimingPolicy":
        if fps is not None and fps > 0:
            frame_us = Microseconds(round(1_000_000 / fps))
            eff_hold_us = Microseconds(max(policy.hold_us, math.ceil(frame_us * min_visible_hold_frames)))
            eff_chord_merge = Microseconds(min(policy.chord_merge_window_us, math.floor(frame_us * chord_merge_max_frame_ratio)))
            eff_input_lead_us = Microseconds(max(policy.input_lead_us, math.floor(frame_us * input_lead_min_frame_ratio)))
            eff_release_gap_us = Microseconds(max(policy.release_gap_us, math.floor(frame_us * release_gap_min_frame_ratio)))
        else:
            frame_us = Microseconds(0)
            eff_hold_us = policy.hold_us
            eff_chord_merge = policy.chord_merge_window_us
            eff_input_lead_us = policy.input_lead_us
            eff_release_gap_us = policy.release_gap_us
            
        conflict_policy = same_key_conflict_policy if same_key_conflict_policy is not None else policy.same_key_conflict_policy
        if conflict_policy not in ("strict", "degraded", "adaptive"):
            conflict_policy = "degraded"
            
        return cls(
            fps=fps if fps is not None else 0,
            frame_us=frame_us,
            hold_us=eff_hold_us,
            min_hold_us=policy.min_hold_us,
            release_gap_us=eff_release_gap_us,
            repeat_release_gap_us=policy.repeat_release_gap_us,
            min_scheduled_hold_us=policy.min_scheduled_hold_us,
            input_lead_us=eff_input_lead_us,
            chord_merge_window_us=eff_chord_merge,
            focus_restore_grace_us=policy.focus_restore_grace_us,
            min_visible_hold_frames=min_visible_hold_frames,
            chord_merge_max_frame_ratio=chord_merge_max_frame_ratio,
            same_key_conflict_policy=conflict_policy
        )

    @classmethod
    def from_profile_name(cls, name: str, fps: int | None = None, **kwargs) -> "FrameTimingPolicy":
        policy = TimingPolicy.from_profile_name(name)
        return cls.from_timing_policy(policy, fps=fps, **kwargs)

    @classmethod
    def local_precise(cls, **kwargs) -> "FrameTimingPolicy":
        return cls.from_profile_name("local_precise", **kwargs)

    @classmethod
    def remote_safe(cls, **kwargs) -> "FrameTimingPolicy":
        return cls.from_profile_name("remote_safe", **kwargs)

    @classmethod
    def dense_safe(cls, **kwargs) -> "FrameTimingPolicy":
        return cls.from_profile_name("dense_safe", **kwargs)

    @classmethod
    def balanced(cls, **kwargs) -> "FrameTimingPolicy":
        return cls.from_profile_name("balanced", **kwargs)


@dataclass(frozen=True, slots=True)
class ScheduleDiagnostic:
    source_index: int
    note_key: NoteKey
    scan_code: int
    source_time_us: Microseconds
    scheduled_down_us: Microseconds
    scheduled_up_us: Microseconds
    hold_us: Microseconds
    reason: str
    risk: Literal["ok", "compressed", "risky_repeat", "impossible_repeat"]


@dataclass(frozen=True, slots=True)
class ScheduleResult:
    actions: tuple[KeyAction, ...]
    compressed_holds: int
    impossible_same_key_repeats: int
    max_polyphony: int
    note_count: int
    duration_us: Microseconds
    warnings: tuple[str, ...]
    risky_same_key_repeats: int = 0
    shortest_same_key_interval_us: int | None = None
    source_duration_us: Microseconds = Microseconds(0)
    playback_duration_us: Microseconds = Microseconds(0)
    diagnostics: tuple[ScheduleDiagnostic, ...] = ()
    recommended_profile: str | None = None
    recommended_tempo_scale: float | None = None
    frame_us: Microseconds | None = None
    fps: int | None = None

