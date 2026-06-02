from __future__ import annotations

from dataclasses import dataclass
from typing import Literal, NewType
import math

from sky_music.domain.domain import NoteKey

from sky_music.config import AppConfig

Microseconds = NewType("Microseconds", int)
ScanCode = NewType("ScanCode", int)

ActionKind = Literal["down", "up"]
FrameAlignMode = Literal["none", "down_only"]


def align_frame_down_us(at_us: int, frame_us: int, mode: FrameAlignMode) -> int:
    """Optional snap of key-down timestamps to frame boundaries (down_only mode)."""
    if mode != "down_only" or frame_us <= 0:
        return at_us
    return max(0, round(at_us / frame_us) * frame_us)


def frame_us_for(fps: int) -> float:
    if fps <= 0:
        raise ValueError("fps must be > 0")
    return 1_000_000 / fps


def compensated_input_lead_us(
    *,
    base_input_lead_us: int,
    reference_fps: int,
    runtime_fps: int,
    min_lead_us: int,
    max_lead_us: int,
) -> int:
    reference_frame_us = frame_us_for(reference_fps)
    runtime_frame_us = frame_us_for(runtime_fps)

    reference_effective_lead = base_input_lead_us - reference_frame_us / 2
    runtime_lead = reference_effective_lead + runtime_frame_us / 2

    return int(round(max(min_lead_us, min(runtime_lead, max_lead_us))))


def is_audience_safe_profile_name(name: str | None) -> bool:
    if name is None:
        return False
    normalized = name.lower().replace("-", "_")
    return normalized in {
        "audience_safe",
        "remote_safe",
        "online_audible_safe",
        "online_audible",
    }


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
    release_gap_us: Microseconds
    repeat_release_gap_us: Microseconds

    input_lead_us: Microseconds = Microseconds(0)
    chord_merge_window_us: Microseconds = Microseconds(0)
    focus_restore_grace_us: Microseconds = Microseconds(100_000) # Default is overridden in from_dict

    same_key_conflict_policy: Literal["degraded", "strict"] = "degraded"

    @classmethod
    def from_dict(cls, p_dict: dict, **kwargs) -> "TimingPolicy":
        from sky_music.config import DEFAULT_TIMING_PROFILES
        base = DEFAULT_TIMING_PROFILES["balanced"]
        
        return cls(
            hold_us=Microseconds(p_dict.get("hold_us", base["hold_us"])),
            min_hold_us=Microseconds(p_dict.get("min_hold_us", base["min_hold_us"])),
            release_gap_us=Microseconds(p_dict.get("release_gap_us", base["release_gap_us"])),
            repeat_release_gap_us=Microseconds(p_dict.get("repeat_release_gap_us", base["repeat_release_gap_us"])),
            input_lead_us=Microseconds(p_dict.get("input_lead_us", base["input_lead_us"])),
            chord_merge_window_us=Microseconds(p_dict.get("chord_merge_window_us", base["chord_merge_window_us"])),
            focus_restore_grace_us=Microseconds(p_dict.get("focus_restore_grace_us", base["focus_restore_grace_us"])),
            same_key_conflict_policy=(
                p_dict.get("same_key_conflict_policy", "degraded")
                if p_dict.get("same_key_conflict_policy", "degraded") in ("degraded", "strict")
                else "degraded"
            ),
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

    input_lead_us: Microseconds
    chord_merge_window_us: Microseconds
    focus_restore_grace_us: Microseconds

    min_visible_hold_frames: float = 1.25
    chord_merge_max_frame_ratio: float = 0.25
    same_key_conflict_policy: Literal["degraded", "strict"] = "degraded"
    frame_align: FrameAlignMode = "none"
    profile_name: str | None = None
    base_input_lead_us: int | None = None
    phase_compensated: bool = False

    @classmethod
    def from_timing_policy(
        cls,
        policy: TimingPolicy,
        fps: int | None = None,
        min_visible_hold_frames: float = 1.25,
        chord_merge_max_frame_ratio: float = 0.25,
        same_key_conflict_policy: Literal["degraded", "strict"] | None = None,
        input_lead_min_frame_ratio: float = 0.5,
        release_gap_min_frame_ratio: float = 0.15,
        repeat_release_gap_min_frame_ratio: float = 1.5,
        repeat_release_gap_floor_us: int = 18000,
        min_hold_min_frame_ratio: float = 1.25,
        frame_align: FrameAlignMode = "none",
        *,
        profile_name: str | None = None,
        phase_compensate_input_lead: bool = True,
    ) -> "FrameTimingPolicy":
        if fps is not None and fps > 0:
            frame_us = Microseconds(round(1_000_000 / fps))
            if fps < 60:
                # low-fps upscale only
                eff_hold_us = Microseconds(max(policy.hold_us, math.ceil(frame_us * min_visible_hold_frames)))
                eff_min_hold_us = Microseconds(max(policy.min_hold_us, math.ceil(frame_us * min_hold_min_frame_ratio)))
                eff_input_lead_us = Microseconds(max(policy.input_lead_us, math.ceil(frame_us * input_lead_min_frame_ratio)))
                eff_release_gap_us = Microseconds(max(policy.release_gap_us, math.ceil(frame_us * release_gap_min_frame_ratio)))
                # Empirical same-key release floor (Exp2): reliable repeats need a gap of
                # max(~1.5 frame, ~17ms fixed). The fixed floor dominates at high FPS where
                # one frame is tiny; the frame term dominates at low FPS.
                eff_repeat_release_gap_us = Microseconds(
                    max(
                        policy.repeat_release_gap_us,
                        math.ceil(frame_us * repeat_release_gap_min_frame_ratio),
                        repeat_release_gap_floor_us,
                    )
                )

                # Safety margin Cycle rule clamp (min_hold + repeat_gap >= frame + 5% or 1ms margin).
                # Now largely superseded by the repeat-gap floor above (which alone keeps the
                # cycle >= ~2.1 frame), but kept as a harmless backstop for unusual custom ratios.
                safety_margin_us = max(1000, math.ceil(frame_us * 0.05))
                required_cycle_us = frame_us + safety_margin_us
                current_cycle = eff_min_hold_us + eff_repeat_release_gap_us
                if current_cycle < required_cycle_us:
                    deficit = required_cycle_us - current_cycle
                    eff_repeat_release_gap_us = Microseconds(eff_repeat_release_gap_us + deficit)

                # Prevent chord merge window from collapsing too small at low FPS.
                is_remote_like = policy.repeat_release_gap_us >= 15000 or policy.input_lead_us >= 12000
                min_chord_merge_us = 4000 if is_remote_like else 2000
                scaled_chord_merge = math.floor(frame_us * chord_merge_max_frame_ratio)
                eff_chord_merge = Microseconds(min(policy.chord_merge_window_us, max(min_chord_merge_us, scaled_chord_merge)))
            else:
                # fps >= 60: keep base safety durations, but still apply the visibility
                # floor (Exp1: every key-down needs >= 1 frame). At >=60 the base values
                # usually dominate, but flooring here keeps the model uniform across all FPS
                # and protects low custom holds at exactly 60 FPS.
                eff_hold_us = Microseconds(max(policy.hold_us, math.ceil(frame_us * min_visible_hold_frames)))
                eff_min_hold_us = Microseconds(max(policy.min_hold_us, math.ceil(frame_us * min_hold_min_frame_ratio)))

                # Phase offset high-fps compensation for audience_safe
                if phase_compensate_input_lead and is_audience_safe_profile_name(profile_name) and fps > 75:
                    eff_input_lead_us = Microseconds(
                        compensated_input_lead_us(
                            base_input_lead_us=int(policy.input_lead_us),
                            reference_fps=60,
                            runtime_fps=fps,
                            min_lead_us=8000,
                            max_lead_us=int(policy.input_lead_us),
                        )
                    )
                else:
                    eff_input_lead_us = policy.input_lead_us
                    
                eff_release_gap_us = policy.release_gap_us
                # Apply the same empirical same-key release floor at >=60 FPS. The fixed
                # ~17ms floor (Exp2) is a hard physical limit independent of render FPS, so
                # high-FPS profiles must respect it rather than keeping a smaller base gap.
                eff_repeat_release_gap_us = Microseconds(
                    max(
                        policy.repeat_release_gap_us,
                        math.ceil(frame_us * repeat_release_gap_min_frame_ratio),
                        repeat_release_gap_floor_us,
                    )
                )
                eff_chord_merge = policy.chord_merge_window_us
        else:
            frame_us = Microseconds(0)
            eff_hold_us = policy.hold_us
            eff_min_hold_us = policy.min_hold_us
            eff_chord_merge = policy.chord_merge_window_us
            eff_input_lead_us = policy.input_lead_us
            eff_release_gap_us = policy.release_gap_us
            eff_repeat_release_gap_us = policy.repeat_release_gap_us
            
        conflict_policy = same_key_conflict_policy if same_key_conflict_policy is not None else policy.same_key_conflict_policy
        if conflict_policy not in ("strict", "degraded"):
            conflict_policy = "degraded"

        align_mode: FrameAlignMode = frame_align if frame_align in ("none", "down_only") else "none"
        
        is_comp = (
            phase_compensate_input_lead
            and is_audience_safe_profile_name(profile_name)
            and fps is not None
            and fps > 75
            and int(eff_input_lead_us) != int(policy.input_lead_us)
        )
            
        return cls(
            fps=fps if fps is not None else 0,
            frame_us=frame_us,
            hold_us=eff_hold_us,
            min_hold_us=eff_min_hold_us,
            release_gap_us=eff_release_gap_us,
            repeat_release_gap_us=eff_repeat_release_gap_us,
            input_lead_us=eff_input_lead_us,
            chord_merge_window_us=eff_chord_merge,
            focus_restore_grace_us=policy.focus_restore_grace_us,
            min_visible_hold_frames=min_visible_hold_frames,
            chord_merge_max_frame_ratio=chord_merge_max_frame_ratio,
            same_key_conflict_policy=conflict_policy,
            frame_align=align_mode,
            profile_name=profile_name,
            base_input_lead_us=int(policy.input_lead_us),
            phase_compensated=is_comp,
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
    max_polyphony: int = 0
    note_count: int = 0
    shortest_same_key_interval_us: int | None = None
    warnings: tuple[str, ...] = ()
    duration_us: Microseconds = Microseconds(0)
    diagnostics: tuple[ScheduleDiagnostic, ...] = ()
    recommended_profile: str | None = None
    recommended_tempo_scale: float | None = None
    frame_us: Microseconds | None = None
    fps: int | None = None
    base_input_lead_us: int | None = None
    runtime_input_lead_us: int | None = None
    phase_compensated: bool | None = None
    chord_merge_window_us: int | None = None
    frame_align: str | None = None
