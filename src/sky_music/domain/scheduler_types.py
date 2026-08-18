from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum
from typing import Literal

from sky_music.domain.domain import Microseconds, NoteKey, ScanCode
from sky_music.domain.hold_timing import (
    DEFAULT_HOLD_FRAMES,
    frame_duration_us,
    materialize_hold_us,
    validate_hold_frames,
)

DEFAULT_FOCUS_RESTORE_GRACE_US = 100_000
DEFAULT_SAME_KEY_CONFLICT_POLICY = "drop_chord"
DEFAULT_DOWN_LATE_GRACE_US = 500


class ActionKind(StrEnum):
    DOWN = "down"
    UP = "up"


@dataclass(frozen=True, slots=True)
class KeyAction:
    kind: ActionKind
    scan_codes: tuple[ScanCode, ...]
    at_us: Microseconds
    reason: str = "note"


ConflictPolicy = Literal["degraded", "drop_chord", "strict"]


@dataclass(frozen=True, slots=True)
class TimingPolicy:
    hold_frames: float = DEFAULT_HOLD_FRAMES
    focus_restore_grace_us: Microseconds = Microseconds(DEFAULT_FOCUS_RESTORE_GRACE_US)
    same_key_conflict_policy: ConflictPolicy = DEFAULT_SAME_KEY_CONFLICT_POLICY
    min_hold_margin_us: Microseconds = Microseconds(500)
    min_hold_margin_source: str = "default_500"
    down_late_grace_us: Microseconds = Microseconds(DEFAULT_DOWN_LATE_GRACE_US)

    def __post_init__(self) -> None:
        validate_hold_frames(self.hold_frames)
        if self.focus_restore_grace_us < 0:
            raise ValueError("focus_restore_grace_us must be non-negative")
        if (
            not isinstance(self.down_late_grace_us, int)
            or isinstance(self.down_late_grace_us, bool)
            or self.down_late_grace_us < 0
        ):
            raise ValueError("down_late_grace_us must be a non-negative integer")


@dataclass(frozen=True, slots=True)
class FrameTimingPolicy:
    fps: int
    frame_us: Microseconds
    hold_frames: float
    hold_us: Microseconds
    min_hold_us: Microseconds
    focus_restore_grace_us: Microseconds
    same_key_conflict_policy: ConflictPolicy = DEFAULT_SAME_KEY_CONFLICT_POLICY
    min_hold_margin_us: Microseconds = Microseconds(500)
    min_hold_margin_source: str = "default_500"
    down_late_grace_us: Microseconds = Microseconds(DEFAULT_DOWN_LATE_GRACE_US)

    @classmethod
    def from_timing_policy(
        cls,
        policy: TimingPolicy,
        fps: int,
        same_key_conflict_policy: ConflictPolicy | None = None,
    ) -> FrameTimingPolicy:
        ratio = validate_hold_frames(policy.hold_frames)
        frame_us = frame_duration_us(fps)
        effective = materialize_hold_us(ratio, fps, policy.min_hold_margin_us)
        conflict = same_key_conflict_policy or policy.same_key_conflict_policy
        if conflict not in ("strict", "drop_chord", "degraded"):
            conflict = DEFAULT_SAME_KEY_CONFLICT_POLICY
        return cls(
            fps=fps,
            frame_us=Microseconds(frame_us),
            hold_frames=ratio,
            hold_us=Microseconds(effective),
            min_hold_us=Microseconds(effective),
            focus_restore_grace_us=policy.focus_restore_grace_us,
            same_key_conflict_policy=conflict,
            min_hold_margin_us=Microseconds(max(0, int(policy.min_hold_margin_us))),
            min_hold_margin_source=policy.min_hold_margin_source,
            down_late_grace_us=Microseconds(policy.down_late_grace_us),
        )

    @classmethod
    def from_hold_frames(
        cls,
        hold_frames: float,
        fps: int,
        *,
        margin_us: int = 500,
        margin_source: str = "default_500",
        down_late_grace_us: int = DEFAULT_DOWN_LATE_GRACE_US,
        focus_restore_grace_us: int = DEFAULT_FOCUS_RESTORE_GRACE_US,
        same_key_conflict_policy: ConflictPolicy = DEFAULT_SAME_KEY_CONFLICT_POLICY,
    ) -> FrameTimingPolicy:
        return cls.from_timing_policy(
            TimingPolicy(
                hold_frames=hold_frames,
                focus_restore_grace_us=Microseconds(focus_restore_grace_us),
                same_key_conflict_policy=same_key_conflict_policy,
                min_hold_margin_us=Microseconds(margin_us),
                min_hold_margin_source=margin_source,
                down_late_grace_us=Microseconds(down_late_grace_us),
            ),
            fps,
        )


@dataclass(frozen=True, slots=True)
class ScheduleDiagnostic:
    source_index: int
    note_key: NoteKey
    scan_code: int
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
    recommended_hold_frames: float | None = None
    recommended_tempo_scale: float | None = None
    sub_60fps_frame_notes: int = 0
    gap_below_frame_repeats: int = 0
