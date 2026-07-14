from __future__ import annotations

from dataclasses import dataclass
from enum import StrEnum


class PlaybackMode(StrEnum):
    PICKER = "picker"
    PLAYING = "playing"
    RISK = "risk"
    ERROR = "error"
    COUNTDOWN = "countdown"


_VALID_TRANSITIONS: dict[PlaybackMode, frozenset[PlaybackMode]] = {
    PlaybackMode.PICKER: frozenset({
        PlaybackMode.PLAYING, PlaybackMode.RISK, PlaybackMode.ERROR, PlaybackMode.COUNTDOWN,
    }),
    PlaybackMode.PLAYING: frozenset({PlaybackMode.PICKER}),
    PlaybackMode.RISK: frozenset({PlaybackMode.PICKER, PlaybackMode.PLAYING}),
    PlaybackMode.ERROR: frozenset({PlaybackMode.PICKER}),
    PlaybackMode.COUNTDOWN: frozenset({PlaybackMode.PICKER, PlaybackMode.PLAYING}),
}


def validate_transition(current: PlaybackMode, next_mode: PlaybackMode) -> None:
    valid = _VALID_TRANSITIONS[current]
    if next_mode not in valid:
        allowed = ", ".join(sorted(m.value for m in valid))
        raise ValueError(
            f"Invalid transition: {current.value} -> {next_mode.value} "
            f"(allowed from {current.value}: {allowed})"
        )


@dataclass(frozen=True, slots=True)
class PickerConfigState:
    """Read-only configuration snapshot shared between App and PickerScreen."""
    active_theme: str = "textual"
    background_mode: str = "win32"
    profile_name: str = "default"
    tempo_scale: float = 1.0
    fps: int = 30
    dry_run: bool = False
    scan_code_mode: str = "default"
    verbose_hud: bool = False
    telemetry_enabled: bool = False
    dispatch_lead_us: int = 0
