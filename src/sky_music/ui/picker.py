from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from sky_music.config import VALID_FPS
from sky_music.ui.picker_helpers import (
    load_saved_theme,
    save_theme,
)

ACTIVE_THEME: str = load_saved_theme()

__all__ = [
    "ACTIVE_THEME",
    "FPS_OPTIONS",
    "HOLD_OPTIONS",
    "TEMPO_OPTIONS",
    "SongPickerResult",
    "get_hold_options",
    "save_theme",
]

@dataclass(frozen=True, slots=True)
class SongPickerResult:
    """Carries the user's confirmed decision from the song picker."""
    song_path: Path
    action: Literal["play", "dry_run"]
    hold_frames: float = 1.0
    tempo_scale: float = 1.0
    fps: int = 60
    verbose_hud: bool | None = None
    telemetry_enabled: bool | None = None

HOLD_OPTIONS = [
    (1.0, "1.0 frame — default, sharpest timing"),
    (1.25, "1.25 frames — moderate visibility cushion"),
    (1.5, "1.5 frames — longest visibility"),
]

def get_hold_options() -> list[tuple[float, str]]:
    return list(HOLD_OPTIONS)

TEMPO_OPTIONS = [
    (0.90, "safer for listeners"),
    (0.95, "recommended for medium/high risk songs"),
    (1.00, "original speed"),
    (1.05, "faster"),
    (1.10, "high risk"),
]

FPS_OPTIONS = [
    (fps, f"{fps} FPS" + (" (Standard)" if fps == 60 else ""))
    for fps in VALID_FPS
]

