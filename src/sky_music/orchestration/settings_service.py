"""Presentation-neutral access to normalized application settings."""

from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path

from sky_music.config import (
    DEFAULT_GAME_FPS,
    DEFAULT_SKY_PROCESS_NAMES,
    VALID_FPS,
    AppConfig,
    load_config,
    normalize_fps_value,
    normalize_hold_frames,
    persist_playback_defaults,
    save_config,
)
from sky_music.domain.hold_timing import DEFAULT_HOLD_FRAMES

THEME_IDS: frozenset[str] = frozenset({"aurora", "minimalist", "slate", "cyberpunk", "classic"})
BACKGROUND_MODES: frozenset[str] = frozenset({"transparent", "painted"})


@dataclass(frozen=True, slots=True)
class HotkeySettings:
    pause: str
    skip: str
    quit: str
    refocus: str
    panic: str


@dataclass(frozen=True, slots=True)
class SafetySettings:
    prompt_on_medium_risk: bool
    prompt_on_high_risk: bool


@dataclass(frozen=True, slots=True)
class UpdatePreferences:
    auto_check: bool
    channel: str
    skip_version: str
    check_interval_s: int


@dataclass(frozen=True, slots=True)
class PlaybackDefaults:
    hold_frames: float
    tempo_scale: float
    fps: int


@dataclass(frozen=True, slots=True)
class ApplicationSettings:
    """Normalized settings exposed to application adapters.

    This view intentionally does not expose config.json keys or its raw
    migration layout. ``AppConfig`` remains private to the service boundary.
    """

    theme: str
    ui_background_mode: str
    default_hold_frames: float
    default_tempo_scale: float
    game_fps: int
    telemetry_enabled: bool
    verbose_hud: bool
    songs_dir: Path
    sky_process_names: tuple[str, ...]
    allow_title_fallback: bool
    hotkeys: HotkeySettings
    safety: SafetySettings
    update_preferences: UpdatePreferences

    @property
    def playback_defaults(self) -> PlaybackDefaults:
        return PlaybackDefaults(
            hold_frames=self.default_hold_frames,
            tempo_scale=self.default_tempo_scale,
            fps=self.game_fps,
        )


def _normalized_theme(value: object) -> str:
    candidate = str(value).strip().casefold()
    return candidate if candidate in THEME_IDS else "aurora"


def _normalized_background_mode(value: object) -> str:
    candidate = str(value).strip().casefold()
    return candidate if candidate in BACKGROUND_MODES else "transparent"


def _normalized_tempo(value: object) -> float:
    try:
        candidate = float(value)  # type: ignore[arg-type]
    except (TypeError, ValueError, OverflowError):
        return 1.0
    return candidate if math.isfinite(candidate) and candidate > 0 else 1.0


def _normalized_process_names(value: object) -> tuple[str, ...]:
    if not isinstance(value, list):
        return tuple(DEFAULT_SKY_PROCESS_NAMES)
    names = tuple(item.strip() for item in value if isinstance(item, str) and item.strip())
    return names or tuple(DEFAULT_SKY_PROCESS_NAMES)


def _normalized_interval(value: object) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        return 86400
    return max(0, value)


def normalize_application_settings(cfg: AppConfig) -> ApplicationSettings:
    """Convert an ``AppConfig`` into the validated adapter-facing view."""
    hotkeys = cfg.hotkeys
    safety = cfg.safety
    update = cfg.update
    return ApplicationSettings(
        theme=_normalized_theme(cfg.theme),
        ui_background_mode=_normalized_background_mode(cfg.ui_background_mode),
        default_hold_frames=normalize_hold_frames(cfg.default_hold_frames, DEFAULT_HOLD_FRAMES),
        default_tempo_scale=_normalized_tempo(cfg.default_tempo_scale),
        game_fps=normalize_fps_value(cfg.game_fps if cfg.game_fps in VALID_FPS else DEFAULT_GAME_FPS),
        telemetry_enabled=bool(cfg.telemetry_enabled_by_default),
        verbose_hud=bool(cfg.verbose_hud),
        songs_dir=Path(cfg.songs_dir),
        sky_process_names=_normalized_process_names(cfg.sky_process_names),
        allow_title_fallback=bool(cfg.allow_title_fallback),
        hotkeys=HotkeySettings(
            pause=str(hotkeys.pause),
            skip=str(hotkeys.skip),
            quit=str(hotkeys.quit),
            refocus=str(hotkeys.refocus),
            panic=str(hotkeys.panic),
        ),
        safety=SafetySettings(
            prompt_on_medium_risk=bool(safety.prompt_on_medium_risk),
            prompt_on_high_risk=bool(safety.prompt_on_high_risk),
        ),
        update_preferences=UpdatePreferences(
            auto_check=bool(update.auto_check),
            channel=update.channel if update.channel in {"stable", "beta"} else "stable",
            skip_version=str(update.skip_version),
            check_interval_s=_normalized_interval(update.check_interval_s),
        ),
    )


class SettingsService:
    """Wrap config persistence while exposing only normalized settings."""

    def __init__(self, cfg: AppConfig | None = None) -> None:
        self._cfg = cfg or load_config()

    def snapshot(self) -> ApplicationSettings:
        return normalize_application_settings(self._cfg)

    def reload(self) -> ApplicationSettings:
        self._cfg = load_config(force_reload=True)
        return self.snapshot()

    def update_playback_defaults(
        self,
        *,
        hold_frames: float,
        tempo_scale: float,
        fps: int,
    ) -> ApplicationSettings:
        persist_playback_defaults(
            self._cfg,
            hold_frames=hold_frames,
            tempo_scale=tempo_scale,
            fps=fps,
        )
        return self.snapshot()

    def set_theme(self, theme: str) -> ApplicationSettings:
        normalized = _normalized_theme(theme)
        self._cfg.theme = normalized
        save_config(self._cfg)
        return self.snapshot()

    def set_telemetry_enabled(self, enabled: bool) -> ApplicationSettings:
        self._cfg.telemetry_enabled_by_default = bool(enabled)
        save_config(self._cfg)
        return self.snapshot()

    def set_verbose_hud(self, enabled: bool) -> ApplicationSettings:
        self._cfg.verbose_hud = bool(enabled)
        save_config(self._cfg)
        return self.snapshot()


__all__ = [
    "BACKGROUND_MODES",
    "THEME_IDS",
    "ApplicationSettings",
    "HotkeySettings",
    "PlaybackDefaults",
    "SafetySettings",
    "SettingsService",
    "UpdatePreferences",
    "normalize_application_settings",
]
