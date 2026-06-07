from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from sky_music.ui.picker_helpers import (
    load_saved_theme,
    save_theme,
    SONG_DIR,
)

ACTIVE_THEME: str = load_saved_theme()

@dataclass(frozen=True, slots=True)
class SongPickerResult:
    """Carries the user's confirmed decision from the song picker."""
    song_path: Path
    action: Literal["play", "dry_run"]
    profile_name: str
    tempo_scale: float
    fps: int | None = None
    verbose_hud: bool | None = None
    telemetry_enabled: bool | None = None

PROFILES_INFO = [
    ("local-precise", "Local Precise: sharp local play, less safe for remote listeners"),
    ("balanced", "Balanced: default setting for local or online play"),
    ("audience-safe", "Audience Safe: helps online players hear notes clearly"),
]

def get_profiles_info(fps: int | None) -> list[tuple[str, str]]:
    return list(PROFILES_INFO)

TEMPO_OPTIONS = [
    (0.90, "safer for listeners"),
    (0.95, "recommended for medium/high risk songs"),
    (1.00, "original speed"),
    (1.05, "faster"),
    (1.10, "high risk"),
]

FPS_OPTIONS = [
    (None, "Auto (No forced sync)"),
    (30, "30 FPS (Mobile/Emulator)"),
    (60, "60 FPS (Standard)"),
    (90, "90 FPS (High Performance)"),
    (120, "120 FPS (High Refresh)"),
    (144, "144 FPS (High Refresh)"),
    (165, "165 FPS (High Refresh)"),
    (240, "240 FPS (Ultra Refresh)"),
]

