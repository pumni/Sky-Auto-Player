from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum
from typing import Literal

from sky_music.domain.domain import Microseconds, NoteKey, ScanCode
from sky_music.domain.hold_timing import (
    DEFAULT_HOLD_FRAMES,
    frame_base_hold_us,
    frame_duration_us,
    validate_hold_frames,
)

DEFAULT_FOCUS_RESTORE_GRACE_US = 100_000
DEFAULT_SAME_KEY_CONFLICT_POLICY = "drop_chord"
DEFAULT_DOWN_LATE_GRACE_US = 500
DEFAULT_TRANSPORT_MARGIN_US = 300
DEFAULT_TRANSPORT_MARGIN_SOURCE = "default_transport_300"


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
    # Compatibility input name: this is the transport component, not the
    # final additive margin exposed by FrameTimingPolicy.
    min_hold_margin_us: Microseconds = Microseconds(DEFAULT_TRANSPORT_MARGIN_US)
    min_hold_margin_source: str = DEFAULT_TRANSPORT_MARGIN_SOURCE
    down_late_grace_us: Microseconds = Microseconds(DEFAULT_DOWN_LATE_GRACE_US)

    def __post_init__(self) -> None:
        validate_hold_frames(self.hold_frames)
        if self.focus_restore_grace_us < 0:
            raise ValueError("focus_restore_grace_us must be non-negative")
        if (
            not isinstance(self.min_hold_margin_us, int)
            or isinstance(self.min_hold_margin_us, bool)
            or self.min_hold_margin_us < 0
        ):
            raise ValueError("min_hold_margin_us must be a non-negative integer")
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
    # Compatibility alias for down-late grace + transport margin.
    min_hold_margin_us: Microseconds = Microseconds(
        DEFAULT_DOWN_LATE_GRACE_US + DEFAULT_TRANSPORT_MARGIN_US
    )
    min_hold_margin_source: str = DEFAULT_TRANSPORT_MARGIN_SOURCE
    down_late_grace_us: Microseconds = Microseconds(DEFAULT_DOWN_LATE_GRACE_US)
    frame_base_hold_us: Microseconds | None = None
    transport_margin_us: Microseconds | None = None
    min_release_gap_us: Microseconds | None = None

    def __post_init__(self) -> None:
        if self.min_hold_margin_us < self.down_late_grace_us:
            raise ValueError("min_hold_margin_us must be at least down_late_grace_us")
        if self.frame_base_hold_us is None:
            object.__setattr__(
                self,
                "frame_base_hold_us",
                Microseconds(max(0, int(self.min_hold_us) - int(self.min_hold_margin_us))),
            )
        elif self.frame_base_hold_us < 0:
            raise ValueError("frame_base_hold_us must be non-negative")
        if self.transport_margin_us is None:
            object.__setattr__(
                self,
                "transport_margin_us",
                Microseconds(max(0, int(self.min_hold_margin_us) - int(self.down_late_grace_us))),
            )
        elif self.transport_margin_us < 0:
            raise ValueError("transport_margin_us must be non-negative")
        if self.min_release_gap_us is None:
            object.__setattr__(self, "min_release_gap_us", Microseconds(self.frame_us))
        elif self.min_release_gap_us < 0:
            raise ValueError("min_release_gap_us must be non-negative")

    @classmethod
    def from_timing_policy(
        cls,
        policy: TimingPolicy,
        fps: int,
        same_key_conflict_policy: ConflictPolicy | None = None,
    ) -> FrameTimingPolicy:
        ratio = validate_hold_frames(policy.hold_frames)
        frame_us = frame_duration_us(fps)
        transport_margin_us = int(policy.min_hold_margin_us)
        down_late_grace_us = int(policy.down_late_grace_us)
        base_hold_us = frame_base_hold_us(ratio, fps)
        effective = base_hold_us + down_late_grace_us + transport_margin_us
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
            min_hold_margin_us=Microseconds(down_late_grace_us + transport_margin_us),
            min_hold_margin_source=policy.min_hold_margin_source,
            down_late_grace_us=Microseconds(down_late_grace_us),
            frame_base_hold_us=Microseconds(base_hold_us),
            transport_margin_us=Microseconds(transport_margin_us),
            min_release_gap_us=Microseconds(frame_us),
        )

    @classmethod
    def from_hold_frames(
        cls,
        hold_frames: float,
        fps: int,
        *,
        margin_us: int = DEFAULT_TRANSPORT_MARGIN_US,
        margin_source: str = DEFAULT_TRANSPORT_MARGIN_SOURCE,
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
