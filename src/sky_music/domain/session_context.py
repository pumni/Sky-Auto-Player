"""Unified playback session state: profile + FPS + timing overrides → effective policy."""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Any, Literal, TYPE_CHECKING

from sky_music.config import (
    AppConfig,
    FrameAlignMode,
    canonical_profile_name,
    display_profile_name,
    load_config,
    normalize_frame_align,
    profile_dict_for,
    spin_threshold_for_profile,
)
from sky_music.domain.scheduler_types import FrameTimingPolicy, TimingPolicy
from sky_music.infrastructure.timing import SleepPolicy

if TYPE_CHECKING:
    from sky_music.orchestration.calibration import CalibrationRecommendation

ConflictPolicy = Literal["degraded", "strict"]


@dataclass(frozen=True, slots=True)
class PlaybackSessionContext:
    """Single source of truth for profile, tempo, FPS, and CLI timing overrides."""

    profile_name: str
    tempo_scale: float = 1.0
    fps: int | None = None
    scan_code_mode: str = "physical"
    same_key_conflict_policy: ConflictPolicy = "degraded"
    frame_align: FrameAlignMode | None = None
    policy_overrides: tuple[tuple[str, Any], ...] = ()

    def __post_init__(self) -> None:
        object.__setattr__(self, "profile_name", canonical_profile_name(self.profile_name))
        if self.tempo_scale <= 0:
            raise ValueError("tempo_scale must be > 0")
        if self.fps is not None and self.fps <= 0:
            object.__setattr__(self, "fps", None)

    @classmethod
    def balanced(
        cls,
        tempo_scale: float = 1.0,
        fps: int | None = None,
        scan_code_mode: str = "physical",
    ) -> PlaybackSessionContext:
        return cls(
            profile_name="balanced",
            tempo_scale=tempo_scale,
            fps=fps,
            scan_code_mode=scan_code_mode,
        )

    @classmethod
    def from_cli_args(cls, args: Any, cfg: AppConfig | None = None) -> PlaybackSessionContext:
        """Build session from argparse namespace after apply_config_defaults."""
        cfg = cfg or load_config()
        profile = canonical_profile_name(args.timing_profile)
        fps_raw = getattr(args, "fps", None)
        fps = int(fps_raw) if fps_raw is not None and int(fps_raw) > 0 else None

        base_dict = profile_dict_for(cfg, profile)
        base_policy = TimingPolicy.from_dict(base_dict)
        conflict: ConflictPolicy = (
            args.same_key_conflict_policy
            if getattr(args, "same_key_conflict_policy", None) is not None
            else base_policy.same_key_conflict_policy  # type: ignore[assignment]
        )

        frame_align_raw = getattr(args, "frame_align", None)
        frame_align: FrameAlignMode | None
        if frame_align_raw is None:
            frame_align = None
        else:
            frame_align = normalize_frame_align(str(frame_align_raw))

        overrides: list[tuple[str, Any]] = []
        if getattr(args, "hold_ms", None) is not None:
            overrides.append(("hold_us", args.hold_ms * 1000))
        if getattr(args, "min_hold_ms", None) is not None:
            overrides.append(("min_hold_us", args.min_hold_ms * 1000))
        if getattr(args, "release_gap_ms", None) is not None:
            overrides.append(("release_gap_us", args.release_gap_ms * 1000))
        if getattr(args, "repeat_release_gap_ms", None) is not None:
            overrides.append(("repeat_release_gap_us", args.repeat_release_gap_ms * 1000))
        if getattr(args, "input_lead_ms", None) is not None:
            overrides.append(("input_lead_us", args.input_lead_ms * 1000))
        if getattr(args, "chord_merge_window_ms", None) is not None:
            overrides.append(("chord_merge_window_us", args.chord_merge_window_ms * 1000))
        if getattr(args, "focus_restore_grace_ms", None) is not None:
            overrides.append(("focus_restore_grace_us", args.focus_restore_grace_ms * 1000))

        return cls(
            profile_name=profile,
            tempo_scale=float(args.tempo_scale),
            fps=fps,
            scan_code_mode=str(args.scan_code_mode),
            same_key_conflict_policy=conflict,
            frame_align=frame_align,
            policy_overrides=tuple(overrides),
        )

    def resolved_frame_align(self, cfg: AppConfig | None = None) -> FrameAlignMode:
        if self.frame_align is not None:
            return self.frame_align
        cfg = cfg or load_config()
        return cfg.frame_timing.frame_align

    def with_profile(self, profile_name: str) -> PlaybackSessionContext:
        return replace(self, profile_name=canonical_profile_name(profile_name))

    def with_tempo(self, tempo_scale: float) -> PlaybackSessionContext:
        if tempo_scale <= 0:
            raise ValueError("tempo_scale must be > 0")
        return replace(self, tempo_scale=tempo_scale)

    def with_fps(self, fps: int | None) -> PlaybackSessionContext:
        normalized = int(fps) if fps is not None and int(fps) > 0 else None
        return replace(self, fps=normalized)

    def with_scan_code_mode(self, mode: str) -> PlaybackSessionContext:
        return replace(self, scan_code_mode=mode)

    def display_profile_label(self) -> str:
        return display_profile_name(self.profile_name, self.fps)

    def metadata_cache_key(self, song_path: Any, cfg: AppConfig | None = None) -> tuple[Any, ...]:
        cfg = cfg or load_config()
        return (
            song_path,
            self.profile_name,
            self.fps,
            self.tempo_scale,
            self.scan_code_mode,
            self.same_key_conflict_policy,
            self.resolved_frame_align(cfg),
            self.policy_overrides,
        )

    def _base_timing_policy(self, cfg: AppConfig | None = None) -> TimingPolicy:
        cfg = cfg or load_config()
        p_dict = dict(profile_dict_for(cfg, self.profile_name))

        # Strict enforcement of timing invariants on the base profile (built-ins +
        # config.json). This catches structurally-broken shipped values such as a
        # profile whose hold_us sits below its own min_hold_us (validate_hold_ordering).
        # CLI/calibration overrides below are an intentional expert escape hatch and are
        # deliberately NOT subjected to the conservative built-in floors, so timing
        # experiments (e.g. --hold-ms 1 to probe the in-game visibility floor) stay
        # possible. The schedule-level validator (validate_key_actions) still flags any
        # resulting risk at playback time.
        from sky_music.domain.validation import validate_builtin_timing_profile
        profile_fields = {
            "hold_us", "min_hold_us", "release_gap_us",
            "repeat_release_gap_us", "input_lead_us", "chord_merge_window_us",
            "hold_frames", "hold_floor_us", "min_hold_frames", "min_hold_floor_us",
            "repeat_release_gap_frames", "repeat_release_gap_floor_us",
        }
        validate_builtin_timing_profile(
            self.profile_name,
            {k: v for k, v in p_dict.items() if k in profile_fields},
            selected_fps=self.fps if self.fps is not None else 60,
        )

        for key, value in self.policy_overrides:
            p_dict[key] = value

        policy = TimingPolicy.from_dict(p_dict)
        if self.same_key_conflict_policy != policy.same_key_conflict_policy:
            return TimingPolicy.from_dict(
                {**p_dict, "same_key_conflict_policy": self.same_key_conflict_policy}
            )
        return policy

    def high_fps_fallback_profile(self) -> str | None:
        """Static safety guard (pure, no I/O).

        ``high_fps_precise`` is only safe above 100 FPS; if selected at <=100 FPS (or with
        FPS unknown) it must fall back to ``local-precise`` to avoid dropped note repeats.
        Returns the fallback profile name when the guard applies, else None. Callers that
        want to inform the user (e.g. main playback setup) should emit a warning once and
        normalize the session; ``resolve_effective_policy`` applies the same fallback
        silently so every call site stays safe.
        """
        from sky_music.config import normalize_profile_name
        if normalize_profile_name(self.profile_name) == "high_fps_precise" and (
            self.fps is None or self.fps <= 100
        ):
            return "local-precise"
        return None

    def resolve_effective_policy(self, cfg: AppConfig | None = None) -> FrameTimingPolicy:
        """Profile dict + CLI overrides + frame-aware scaling (single entry point).

        Pure: no console output. The high-FPS static safety guard is applied silently here;
        user-facing warning is emitted once by the playback setup (see main.play_song).
        """
        cfg = cfg or load_config()

        fallback = self.high_fps_fallback_profile()
        effective_self = self.with_profile(fallback) if fallback else self

        base = effective_self._base_timing_policy(cfg)
        
        has_manual_input_lead_override = any(
            key == "input_lead_us"
            for key, _ in effective_self.policy_overrides
        )

        return FrameTimingPolicy.from_timing_policy(
            base,
            fps=effective_self.fps,
            same_key_conflict_policy=effective_self.same_key_conflict_policy,
            frame_align=effective_self.resolved_frame_align(cfg),
            profile_name=effective_self.profile_name,
            phase_compensate_input_lead=not has_manual_input_lead_override,
            **cfg.frame_timing.as_policy_kwargs(),
        )

    def resolve_sleep_policy(
        self,
        cfg: AppConfig | None = None,
        spin_threshold_us: int | None = None,
    ) -> SleepPolicy:
        cfg = cfg or load_config()
        spin = (
            spin_threshold_us
            if spin_threshold_us is not None
            else spin_threshold_for_profile(cfg, self.profile_name)
        )
        return SleepPolicy(spin_threshold_us=spin, poll_s=0.025)


def merge_session_with_overrides(
    base: PlaybackSessionContext,
    *,
    profile: str | None = None,
    tempo: float | None = None,
    fps: int | None = None,
) -> PlaybackSessionContext:
    """Apply picker / playback overrides while preserving FPS when not overridden."""
    session = base
    if profile is not None:
        session = session.with_profile(profile)
    if tempo is not None:
        session = session.with_tempo(tempo)
    if fps is not None:
        session = session.with_fps(fps)
    return session


def apply_recommendation_to_context(
    session: PlaybackSessionContext,
    recommendation: CalibrationRecommendation,
    *,
    apply_input_lead: bool = True,
) -> PlaybackSessionContext:
    """Apply telemetry calibration advice to an in-memory session (does not persist config)."""
    override_map = dict(session.policy_overrides)
    if apply_input_lead:
        override_map["input_lead_us"] = recommendation.input_lead_us
    return replace(
        session,
        profile_name=canonical_profile_name(recommendation.profile_name),
        tempo_scale=recommendation.tempo_scale,
        policy_overrides=tuple(override_map.items()),
    )
