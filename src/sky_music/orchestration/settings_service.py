"""Presentation-neutral access to normalized application settings."""

from __future__ import annotations

import math
from collections.abc import Mapping
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
    save_config,
)
from sky_music.domain.hold_timing import (
    DEFAULT_HOLD_FRAMES,
    validate_hold_frames,
)

THEME_IDS: frozenset[str] = frozenset({"aurora", "minimalist", "slate", "cyberpunk", "classic"})
BACKGROUND_MODES: frozenset[str] = frozenset({"transparent", "painted"})
HOLD_FRAME_OPTIONS: tuple[float, ...] = (1.0, 1.25, 1.5)
TEMPO_SCALE_OPTIONS: tuple[float, ...] = (0.90, 0.95, 1.00, 1.05, 1.10)
PATCHABLE_SETTINGS: frozenset[str] = frozenset(
    {
        "default_hold_frames",
        "default_tempo_scale",
        "game_fps",
        "theme",
        "telemetry_enabled",
        "verbose_hud",
        "update_auto_check",
        "update_channel",
        "update_skip_version",
    }
)


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


def _normalized_bool(value: object, default: bool = False) -> bool:
    return value if type(value) is bool else default


def _validate_write_tempo(value: object) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError("tempo_scale must be a finite positive number")
    candidate = float(value)
    if not math.isfinite(candidate) or candidate <= 0:
        raise ValueError("tempo_scale must be a finite positive number")
    return candidate


def _validate_write_hold_frames(value: object) -> float:
    try:
        return validate_hold_frames(value)
    except ValueError as exc:
        raise ValueError("hold_frames must be one of 1.0, 1.25, or 1.5") from exc


def _validate_write_fps(value: object) -> int:
    if type(value) is not int or value not in VALID_FPS:
        raise ValueError(f"fps must be one of {VALID_FPS}")
    return value


def _validate_write_bool(value: object, field: str) -> bool:
    if type(value) is not bool:
        raise ValueError(f"{field} must be a boolean")
    return value


def _validate_write_theme(value: object) -> str:
    if not isinstance(value, str):
        raise ValueError("theme must be a known theme ID")
    normalized = value.strip().casefold()
    if normalized not in THEME_IDS:
        raise ValueError("theme must be a known theme ID")
    return normalized


def _validate_write_update_channel(value: object) -> str:
    if not isinstance(value, str) or value.strip().casefold() not in {"stable", "beta"}:
        raise ValueError("update_channel must be stable or beta")
    return value.strip().casefold()


def _validate_write_skip_version(value: object) -> str:
    if not isinstance(value, str) or len(value.encode("utf-8")) > 128 or "\x00" in value:
        raise ValueError("update_skip_version must be bounded text")
    return value.strip()


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
        telemetry_enabled=_normalized_bool(cfg.telemetry_enabled_by_default),
        verbose_hud=_normalized_bool(cfg.verbose_hud),
        songs_dir=Path(cfg.songs_dir),
        sky_process_names=_normalized_process_names(cfg.sky_process_names),
        allow_title_fallback=_normalized_bool(cfg.allow_title_fallback),
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
            auto_check=_normalized_bool(update.auto_check, default=True),
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

    def config_snapshot(self) -> AppConfig:
        """Return a detached config for trusted read-only orchestration work."""
        from copy import deepcopy

        return deepcopy(self._cfg)

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
        normalized_hold_frames = _validate_write_hold_frames(hold_frames)
        normalized_tempo_scale = _validate_write_tempo(tempo_scale)
        normalized_fps = _validate_write_fps(fps)
        self._cfg.default_hold_frames = normalized_hold_frames
        self._cfg.default_tempo_scale = normalized_tempo_scale
        self._cfg.game_fps = normalized_fps
        save_config(self._cfg)
        return self.snapshot()

    def set_hold_frames(self, hold_frames: float) -> ApplicationSettings:
        settings = self.snapshot()
        return self.update_playback_defaults(
            hold_frames=hold_frames,
            tempo_scale=settings.default_tempo_scale,
            fps=settings.game_fps,
        )

    def set_tempo_scale(self, tempo_scale: float) -> ApplicationSettings:
        settings = self.snapshot()
        return self.update_playback_defaults(
            hold_frames=settings.default_hold_frames,
            tempo_scale=tempo_scale,
            fps=settings.game_fps,
        )

    def set_fps(self, fps: int) -> ApplicationSettings:
        settings = self.snapshot()
        return self.update_playback_defaults(
            hold_frames=settings.default_hold_frames,
            tempo_scale=settings.default_tempo_scale,
            fps=fps,
        )

    def set_theme(self, theme: str) -> ApplicationSettings:
        normalized = _validate_write_theme(theme)
        self._cfg.theme = normalized
        save_config(self._cfg)
        return self.snapshot()

    def set_telemetry_enabled(self, enabled: bool) -> ApplicationSettings:
        self._cfg.telemetry_enabled_by_default = _validate_write_bool(
            enabled, "telemetry_enabled"
        )
        save_config(self._cfg)
        return self.snapshot()

    def set_verbose_hud(self, enabled: bool) -> ApplicationSettings:
        self._cfg.verbose_hud = _validate_write_bool(enabled, "verbose_hud")
        save_config(self._cfg)
        return self.snapshot()

    def patch(self, values: Mapping[str, object]) -> ApplicationSettings:
        """Atomically validate and persist the supported application settings."""
        if not isinstance(values, Mapping):
            raise ValueError("settings patch must be an object")
        unknown = [
            key for key in values if not isinstance(key, str) or key not in PATCHABLE_SETTINGS
        ]
        if unknown:
            labels = ", ".join(sorted(str(key) for key in unknown))
            raise ValueError(f"unsupported settings: {labels}")

        normalized: dict[str, object] = {}
        if "default_hold_frames" in values:
            normalized["default_hold_frames"] = _validate_write_hold_frames(
                values["default_hold_frames"]
            )
        if "default_tempo_scale" in values:
            normalized["default_tempo_scale"] = _validate_write_tempo(
                values["default_tempo_scale"]
            )
        if "game_fps" in values:
            normalized["game_fps"] = _validate_write_fps(values["game_fps"])
        if "theme" in values:
            normalized["theme"] = _validate_write_theme(values["theme"])
        if "telemetry_enabled" in values:
            normalized["telemetry_enabled"] = _validate_write_bool(
                values["telemetry_enabled"], "telemetry_enabled"
            )
        if "verbose_hud" in values:
            normalized["verbose_hud"] = _validate_write_bool(
                values["verbose_hud"], "verbose_hud"
            )
        if "update_auto_check" in values:
            normalized["update_auto_check"] = _validate_write_bool(
                values["update_auto_check"], "update_auto_check"
            )
        if "update_channel" in values:
            normalized["update_channel"] = _validate_write_update_channel(
                values["update_channel"]
            )
        if "update_skip_version" in values:
            normalized["update_skip_version"] = _validate_write_skip_version(
                values["update_skip_version"]
            )

        if "default_hold_frames" in normalized:
            self._cfg.default_hold_frames = normalized["default_hold_frames"]  # type: ignore[assignment]
        if "default_tempo_scale" in normalized:
            self._cfg.default_tempo_scale = normalized["default_tempo_scale"]  # type: ignore[assignment]
        if "game_fps" in normalized:
            self._cfg.game_fps = normalized["game_fps"]  # type: ignore[assignment]
        if "theme" in normalized:
            self._cfg.theme = normalized["theme"]  # type: ignore[assignment]
        if "telemetry_enabled" in normalized:
            self._cfg.telemetry_enabled_by_default = normalized["telemetry_enabled"]  # type: ignore[assignment]
        if "verbose_hud" in normalized:
            self._cfg.verbose_hud = normalized["verbose_hud"]  # type: ignore[assignment]
        if "update_auto_check" in normalized:
            self._cfg.update.auto_check = normalized["update_auto_check"]  # type: ignore[assignment]
        if "update_channel" in normalized:
            self._cfg.update.channel = normalized["update_channel"]  # type: ignore[assignment]
        if "update_skip_version" in normalized:
            self._cfg.update.skip_version = normalized["update_skip_version"]  # type: ignore[assignment]
        if normalized:
            save_config(self._cfg)
        return self.snapshot()


__all__ = [
    "BACKGROUND_MODES",
    "HOLD_FRAME_OPTIONS",
    "PATCHABLE_SETTINGS",
    "TEMPO_SCALE_OPTIONS",
    "THEME_IDS",
    "ApplicationSettings",
    "HotkeySettings",
    "PlaybackDefaults",
    "SafetySettings",
    "SettingsService",
    "UpdatePreferences",
    "normalize_application_settings",
]
