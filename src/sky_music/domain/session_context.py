"""Unified playback session state: selected hold frames, FPS, and tempo."""

from __future__ import annotations

from copy import replace as copy_replace
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, Literal

from sky_music.config import DEFAULT_GAME_FPS, AppConfig, resolve_game_fps
from sky_music.domain.domain import Microseconds
from sky_music.domain.hold_timing import (
    DEFAULT_HOLD_FRAMES,
    format_hold_frames,
    validate_hold_frames,
)
from sky_music.domain.scheduler_types import (
    DEFAULT_CHORD_STAGGER_MAX_US,
    DEFAULT_CHORD_STAGGER_US,
    DEFAULT_FOCUS_RESTORE_GRACE_US,
    FrameTimingPolicy,
    TimingPolicy,
)

if TYPE_CHECKING:
    from sky_music.orchestration.calibration import CalibrationRecommendation

ConflictPolicy = Literal["degraded", "drop_chord", "strict"]


@dataclass(frozen=True, slots=True)
class PlaybackSessionContext:
    hold_frames: float = DEFAULT_HOLD_FRAMES
    tempo_scale: float = 1.0
    fps: int = DEFAULT_GAME_FPS
    scan_code_mode: str = "physical"
    same_key_conflict_policy: ConflictPolicy = "drop_chord"
    focus_restore_grace_us: int = DEFAULT_FOCUS_RESTORE_GRACE_US
    chord_stagger_us: int = DEFAULT_CHORD_STAGGER_US
    chord_stagger_max_us: int = DEFAULT_CHORD_STAGGER_MAX_US

    def __post_init__(self) -> None:
        object.__setattr__(self, "hold_frames", validate_hold_frames(self.hold_frames))
        if self.tempo_scale <= 0:
            raise ValueError("tempo_scale must be > 0")
        if not 0 <= self.focus_restore_grace_us <= 60_000_000:
            raise ValueError("focus_restore_grace_us must be in 0..=60000000")
        object.__setattr__(self, "fps", resolve_game_fps(self.fps))

    @classmethod
    def default(
        cls,
        hold_frames: float = DEFAULT_HOLD_FRAMES,
        tempo_scale: float = 1.0,
        fps: int | None = None,
        scan_code_mode: str = "physical",
    ) -> PlaybackSessionContext:
        return cls(
            hold_frames=hold_frames,
            tempo_scale=tempo_scale,
            fps=resolve_game_fps(fps),
            scan_code_mode=scan_code_mode,
        )

    @classmethod
    def from_cli_args(cls, args: Any, cfg: AppConfig | None = None) -> PlaybackSessionContext:
        del cfg
        return cls(
            hold_frames=validate_hold_frames(getattr(args, "hold_frames", DEFAULT_HOLD_FRAMES)),
            tempo_scale=float(args.tempo_scale),
            fps=resolve_game_fps(getattr(args, "fps", DEFAULT_GAME_FPS)),
            scan_code_mode=str(args.scan_code_mode),
            same_key_conflict_policy=getattr(args, "same_key_conflict_policy", "drop_chord"),
            focus_restore_grace_us=DEFAULT_FOCUS_RESTORE_GRACE_US,
            chord_stagger_us=max(0, int(getattr(args, "chord_stagger_us", None) or DEFAULT_CHORD_STAGGER_US)),
            chord_stagger_max_us=max(0, int(getattr(args, "chord_stagger_max_us", None) or DEFAULT_CHORD_STAGGER_MAX_US)),
        )

    def with_hold_frames(self, hold_frames: float) -> PlaybackSessionContext:
        return copy_replace(self, hold_frames=validate_hold_frames(hold_frames))

    def with_tempo(self, tempo_scale: float) -> PlaybackSessionContext:
        if tempo_scale <= 0:
            raise ValueError("tempo_scale must be > 0")
        return copy_replace(self, tempo_scale=tempo_scale)

    def with_fps(self, fps: int | None) -> PlaybackSessionContext:
        return copy_replace(self, fps=resolve_game_fps(fps))

    def with_scan_code_mode(self, mode: str) -> PlaybackSessionContext:
        return copy_replace(self, scan_code_mode=mode)

    def display_hold_label(self) -> str:
        return format_hold_frames(self.hold_frames)

    def metadata_cache_key(self, song_path: Any, cfg: AppConfig | None = None) -> tuple[Any, ...]:
        del cfg
        return (
            song_path,
            self.hold_frames,
            self.fps,
            self.tempo_scale,
            self.scan_code_mode,
            self.same_key_conflict_policy,
            self.chord_stagger_us,
            self.chord_stagger_max_us,
        )

    def resolve_effective_policy(
        self,
        cfg: AppConfig | None = None,
        *,
        calibrated_margin_us: int | None = None,
        calibrated_margin_source: str = "default_500",
    ) -> FrameTimingPolicy:
        del cfg
        policy = TimingPolicy(
            hold_frames=self.hold_frames,
            focus_restore_grace_us=Microseconds(self.focus_restore_grace_us),
            same_key_conflict_policy=self.same_key_conflict_policy,
            chord_stagger_us=Microseconds(self.chord_stagger_us),
            chord_stagger_max_us=Microseconds(self.chord_stagger_max_us),
            min_hold_margin_us=Microseconds(max(0, calibrated_margin_us if calibrated_margin_us is not None else 500)),
            min_hold_margin_source=calibrated_margin_source if calibrated_margin_us is not None else "default_500",
        )
        return FrameTimingPolicy.from_timing_policy(policy, fps=self.fps)

def merge_session_with_overrides(
    base: PlaybackSessionContext,
    *,
    hold_frames: float | None = None,
    tempo: float | None = None,
    fps: int | None = None,
) -> PlaybackSessionContext:
    session = base
    if hold_frames is not None:
        session = session.with_hold_frames(hold_frames)
    if tempo is not None:
        session = session.with_tempo(tempo)
    if fps is not None:
        session = session.with_fps(fps)
    return session


def apply_recommendation_to_context(
    session: PlaybackSessionContext,
    recommendation: CalibrationRecommendation,
) -> PlaybackSessionContext:
    return copy_replace(
        session,
        hold_frames=validate_hold_frames(recommendation.hold_frames),
        tempo_scale=recommendation.tempo_scale,
    )
