from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from sky_music.config import AppConfig
from sky_music.domain import Song
from sky_music.domain.analyzer import ScheduleRiskReport, analyze_schedule
from sky_music.domain.scheduler import (
    ScheduleBuildError,
    ScheduleMetadata,
    build_key_actions,
)
from sky_music.domain.scheduler_types import FrameTimingPolicy, KeyAction
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.domain.song_repository import get_shared_song_repository
from sky_music.domain.validation import ScheduleInvariantViolation, validate_key_actions
from sky_music.orchestration.calibrated_policy import resolve_calibrated_policy


@dataclass(frozen=True, slots=True)
class PlaybackPlan:
    actions: tuple[KeyAction, ...]
    sched_meta: ScheduleMetadata
    session: PlaybackSessionContext
    active_policy: FrameTimingPolicy
    song: Song
    risk_report: ScheduleRiskReport
    cfg: AppConfig
    violations: tuple[ScheduleInvariantViolation, ...] = ()


@dataclass(frozen=True, slots=True)
class PlaybackError:
    code: str
    message: str
    recommended_tempo_scale: float | None = None
    recommended_hold_frames: float | None = None


def prepare_playback(
    song_path_or_song: Path | Song,
    session: PlaybackSessionContext,
    cfg: AppConfig,
    is_dry_run: bool = False,
) -> PlaybackPlan | PlaybackError:
    if isinstance(song_path_or_song, Path):
        try:
            song = get_shared_song_repository().load(song_path_or_song)
        except Exception as exc:
            return PlaybackError(code="parse_failed", message=str(exc))
    else:
        song = song_path_or_song

    current_tempo = session.tempo_scale
    active_policy = resolve_calibrated_policy(session, cfg)

    try:
        sched_meta = build_key_actions(
            song,
            policy=active_policy,
            scan_code_mode=session.scan_code_mode,
            resolver=None,
            tempo_scale=current_tempo,
        )
    except (ScheduleBuildError, ValueError) as exc:
        return PlaybackError(
            code="build_failed",
            message=str(exc),
            recommended_tempo_scale=getattr(exc, "recommended_tempo_scale", None),
            recommended_hold_frames=getattr(exc, "recommended_hold_frames", None),
        )

    actions = sched_meta.actions

    # Native dispatch is fail-closed and has no runtime drop/degraded conflict
    # mode. Reject infeasible repeats before quiescing the picker or focusing
    # a target. Dry-run keeps the diagnostic schedule for inspection.
    if not is_dry_run and sched_meta.impossible_same_key_repeats > 0:
        return PlaybackError(
            code="validation_failed",
            message=(
                f"Detected {sched_meta.impossible_same_key_repeats} infeasible same-key repeat(s): "
                "the authored interval is shorter than the configured hold."
            ),
            recommended_tempo_scale=sched_meta.recommended_tempo_scale,
            recommended_hold_frames=sched_meta.recommended_hold_frames,
        )

    violations = validate_key_actions(actions, policy=active_policy)
    if violations:
        fatal_violations = [v for v in violations if getattr(v, "severity", "fatal") == "fatal"]
        if fatal_violations and not is_dry_run:
            msg = "; ".join(f"[{v.code}] {v.message}" for v in fatal_violations)
            return PlaybackError(code="validation_failed", message=msg)

    report = analyze_schedule(
        sched_meta,
        raw_notes=song.notes,
        current_hold_frames=session.hold_frames,
        current_tempo_scale=session.tempo_scale,
    )

    return PlaybackPlan(
        actions=actions,
        sched_meta=sched_meta,
        session=session,
        active_policy=active_policy,
        song=song,
        risk_report=report,
        cfg=cfg,
        violations=violations,
    )


def rebuild_with(
    plan_or_session: PlaybackPlan | PlaybackSessionContext,
    *,
    hold_frames: float | None = None,
    tempo: float | None = None,
    is_dry_run: bool = False,
    cfg: AppConfig | None = None,
    song: Song | None = None,
) -> PlaybackPlan | PlaybackError:
    if isinstance(plan_or_session, PlaybackPlan):
        session = plan_or_session.session
        resolved_song = song or plan_or_session.song
        resolved_cfg = cfg or plan_or_session.cfg
    else:
        session = plan_or_session
        if song is None or cfg is None:
            raise ValueError("song and cfg must be provided if plan_or_session is a PlaybackSessionContext")
        resolved_song = song
        resolved_cfg = cfg

    if hold_frames is not None:
        session = session.with_hold_frames(hold_frames)
    if tempo is not None:
        session = session.with_tempo(tempo)

    return prepare_playback(
        song_path_or_song=resolved_song,
        session=session,
        cfg=resolved_cfg,
        is_dry_run=is_dry_run,
    )
