"""Unified playback session state: profile + FPS + timing overrides → effective policy."""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Any, Literal, TYPE_CHECKING

from sky_music.config import (
    AppConfig,
    canonical_profile_name,
    display_profile_name,
    load_config,
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

        def ms_to_us(value: float | int) -> int:
            return int(round(float(value) * 1000))

        base_dict = profile_dict_for(cfg, profile)
        base_policy = TimingPolicy.from_dict(base_dict)
        conflict: ConflictPolicy = (
            args.same_key_conflict_policy
            if getattr(args, "same_key_conflict_policy", None) is not None
            else base_policy.same_key_conflict_policy  # type: ignore[assignment]
        )

        overrides: list[tuple[str, Any]] = []
        if getattr(args, "hold_ms", None) is not None:
            overrides.append(("hold_us", ms_to_us(args.hold_ms)))
        if getattr(args, "min_hold_ms", None) is not None:
            overrides.append(("min_hold_us", ms_to_us(args.min_hold_ms)))
        if getattr(args, "focus_restore_grace_ms", None) is not None:
            overrides.append(("focus_restore_grace_us", ms_to_us(args.focus_restore_grace_ms)))

        return cls(
            profile_name=profile,
            tempo_scale=float(args.tempo_scale),
            fps=fps,
            scan_code_mode=str(args.scan_code_mode),
            same_key_conflict_policy=conflict,
            policy_overrides=tuple(overrides),
        )

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
            "hold_us", "min_hold_us",
            "hold_frames", "hold_floor_us", "min_hold_frames", "min_hold_floor_us",
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

    def resolve_effective_policy(self, cfg: AppConfig | None = None) -> FrameTimingPolicy:
        """Profile dict + CLI overrides + frame-aware scaling (single entry point).

        Pure: no console output.
        """
        cfg = cfg or load_config()

        effective_self = self

        base = effective_self._base_timing_policy(cfg)

        return FrameTimingPolicy.from_timing_policy(
            base,
            fps=effective_self.fps,
            same_key_conflict_policy=effective_self.same_key_conflict_policy,
            profile_name=effective_self.profile_name,
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
) -> PlaybackSessionContext:
    """Apply telemetry calibration advice to an in-memory session (does not persist config)."""
    override_map = dict(session.policy_overrides)
    return replace(
        session,
        profile_name=canonical_profile_name(recommendation.profile_name),
        tempo_scale=recommendation.tempo_scale,
        policy_overrides=tuple(override_map.items()),
    )
