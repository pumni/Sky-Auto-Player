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
    hold_frames: float = 1.25
    hold_floor_us: Microseconds | None = None
    min_hold_frames: float = 1.25
    min_hold_floor_us: Microseconds | None = None
    repeat_release_gap_frames: float = 1.5
    repeat_release_gap_floor_us: Microseconds | None = None
    hold_override_us: Microseconds | None = None
    min_hold_override_us: Microseconds | None = None
    repeat_release_gap_override_us: Microseconds | None = None
    hold_uses_frame_model: bool = False
    min_hold_uses_frame_model: bool = False
    repeat_release_gap_uses_frame_model: bool = False

    @classmethod
    def from_dict(cls, p_dict: dict, **kwargs) -> "TimingPolicy":
        from sky_music.config import DEFAULT_TIMING_PROFILES
        base = DEFAULT_TIMING_PROFILES["balanced"]

        def int_value(key: str, default: int) -> int:
            return int(p_dict.get(key, default))

        def float_value(key: str, default: float) -> float:
            return float(p_dict.get(key, default))

        def frame_coupled(
            *,
            value_key: str,
            frames_key: str,
            floor_key: str,
            unframed_key: str,
            default_frames: float,
        ) -> tuple[Microseconds, float, Microseconds, Microseconds | None, bool]:
            base_value = int(base.get(floor_key, base.get(value_key, 0)))
            base_unframed_value = int(base.get(unframed_key, base_value))
            has_frame_model = frames_key in p_dict or floor_key in p_dict
            floor = int_value(floor_key, base_value) if has_frame_model else int_value(value_key, base_value)
            default_value = floor if has_frame_model else base_unframed_value
            value = int_value(value_key, int_value(unframed_key, default_value))
            override = Microseconds(value) if has_frame_model and value_key in p_dict else None
            return Microseconds(value), float_value(frames_key, default_frames), Microseconds(floor), override, has_frame_model

        hold_us, hold_frames, hold_floor_us, hold_override_us, hold_uses_frame_model = frame_coupled(
            value_key="hold_us",
            frames_key="hold_frames",
            floor_key="hold_floor_us",
            unframed_key="hold_unframed_us",
            default_frames=1.25,
        )
        min_hold_us, min_hold_frames, min_hold_floor_us, min_hold_override_us, min_hold_uses_frame_model = frame_coupled(
            value_key="min_hold_us",
            frames_key="min_hold_frames",
            floor_key="min_hold_floor_us",
            unframed_key="min_hold_unframed_us",
            default_frames=1.25,
        )
        repeat_gap_us, repeat_gap_frames, repeat_gap_floor_us, repeat_gap_override_us, repeat_gap_uses_frame_model = frame_coupled(
            value_key="repeat_release_gap_us",
            frames_key="repeat_release_gap_frames",
            floor_key="repeat_release_gap_floor_us",
            unframed_key="repeat_release_gap_unframed_us",
            default_frames=1.5,
        )
        
        return cls(
            hold_us=hold_us,
            min_hold_us=min_hold_us,
            release_gap_us=Microseconds(int_value("release_gap_us", int(base["release_gap_us"]))),
            repeat_release_gap_us=repeat_gap_us,
            input_lead_us=Microseconds(int_value("input_lead_us", int(base["input_lead_us"]))),
            chord_merge_window_us=Microseconds(int_value("chord_merge_window_us", int(base["chord_merge_window_us"]))),
            focus_restore_grace_us=Microseconds(int_value("focus_restore_grace_us", int(base["focus_restore_grace_us"]))),
            same_key_conflict_policy=(
                p_dict.get("same_key_conflict_policy", "degraded")
                if p_dict.get("same_key_conflict_policy", "degraded") in ("degraded", "strict")
                else "degraded"
            ),
            hold_frames=hold_frames,
            hold_floor_us=hold_floor_us,
            min_hold_frames=min_hold_frames,
            min_hold_floor_us=min_hold_floor_us,
            repeat_release_gap_frames=repeat_gap_frames,
            repeat_release_gap_floor_us=repeat_gap_floor_us,
            hold_override_us=hold_override_us,
            min_hold_override_us=min_hold_override_us,
            repeat_release_gap_override_us=repeat_gap_override_us,
            hold_uses_frame_model=hold_uses_frame_model,
            min_hold_uses_frame_model=min_hold_uses_frame_model,
            repeat_release_gap_uses_frame_model=repeat_gap_uses_frame_model,
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

    @staticmethod
    def materialise_frame_floor(frames: float, floor_us: int, frame_us: int) -> Microseconds:
        return Microseconds(max(math.ceil(frames * frame_us), floor_us))

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
    ) -> "FrameTimingPolicy":
        if fps is not None and fps > 0:
            frame_us = Microseconds(round(1_000_000 / fps))
            hold_frames = policy.hold_frames if policy.hold_uses_frame_model else min_visible_hold_frames
            min_hold_frames = (
                policy.min_hold_frames
                if policy.min_hold_uses_frame_model
                else min_hold_min_frame_ratio
            )
            repeat_gap_frames = (
                policy.repeat_release_gap_frames
                if policy.repeat_release_gap_uses_frame_model
                else repeat_release_gap_min_frame_ratio
            )
            repeat_gap_floor_us = (
                int(policy.repeat_release_gap_floor_us)
                if policy.repeat_release_gap_uses_frame_model and policy.repeat_release_gap_floor_us is not None
                else max(int(policy.repeat_release_gap_us), repeat_release_gap_floor_us)
            )
            eff_hold_us = cls.materialise_frame_floor(
                hold_frames,
                int(policy.hold_floor_us if policy.hold_floor_us is not None else policy.hold_us),
                int(frame_us),
            )
            eff_min_hold_us = cls.materialise_frame_floor(
                min_hold_frames,
                int(policy.min_hold_floor_us if policy.min_hold_floor_us is not None else policy.min_hold_us),
                int(frame_us),
            )
            eff_repeat_release_gap_us = cls.materialise_frame_floor(
                repeat_gap_frames,
                repeat_gap_floor_us,
                int(frame_us),
            )

            if fps < 60:
                eff_input_lead_us = Microseconds(max(policy.input_lead_us, math.ceil(frame_us * input_lead_min_frame_ratio)))
                eff_release_gap_us = Microseconds(max(policy.release_gap_us, math.ceil(frame_us * release_gap_min_frame_ratio)))

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
                # At FPS >= 60 the render frame is finer than the game's fixed onset
                # cadence (~60 Hz internal tick, measured June 2026 — see Appendix A,
                # Result 4). The lead must therefore NOT be scaled with render FPS:
                # reducing it by frame'/2 biased notes late and beat against the fixed
                # tick (the "lạc nhịp" observed at 144 FPS). Lead is held at base.
                eff_input_lead_us = policy.input_lead_us
                eff_release_gap_us = policy.release_gap_us
                eff_chord_merge = policy.chord_merge_window_us

            if policy.hold_override_us is not None:
                eff_hold_us = policy.hold_override_us
            if policy.min_hold_override_us is not None:
                eff_min_hold_us = policy.min_hold_override_us
            if policy.repeat_release_gap_override_us is not None:
                eff_repeat_release_gap_us = policy.repeat_release_gap_override_us
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
    chord_merge_window_us: int | None = None
    frame_align: str | None = None
