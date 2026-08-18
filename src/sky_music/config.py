"""Sky Music Player — persistent user configuration (config.json schema v3).

The config file is read once at startup and provides *defaults* that can be
overridden by CLI flags.  Saving happens when the user explicitly changes a
setting in the UI.
"""

import contextlib
import json
import math
import os
import threading
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Literal, cast

from sky_music.domain.hold_timing import (
    DEFAULT_HOLD_FRAMES,
    nearest_hold_frames,
    normalize_hold_frames,
)

SCHEMA_VERSION: int = 3
DEFAULT_GAME_FPS: int = 60
VALID_FPS: tuple[int, ...] = (30, 60, 90, 120, 144, 165, 240)
CONFIG_PATH: Path = Path(__file__).resolve().parents[2] / "config.json"


def _parse_bool(val: Any, default: bool) -> bool:
    if isinstance(val, bool):
        return val
    return default


def _parse_float(val: Any, default: float) -> float:
    if isinstance(val, bool):
        return default
    try:
        f = float(val)
        if not math.isfinite(f):
            return default
        return f
    except (TypeError, ValueError):
        return default


def _parse_int(val: Any, default: int) -> int:
    if isinstance(val, bool):
        return default
    try:
        return int(val)
    except (TypeError, ValueError):
        return default


@dataclass(frozen=True)
class HotkeyDefaults:
    pause:   str = "f8"
    skip:    str = "f9"
    quit:    str = "f10"
    refocus: str = "f6"
    panic:   str = "ctrl+alt+backspace"


@dataclass(frozen=True)
class SafetyDefaults:
    prompt_on_medium_risk: bool = True
    prompt_on_high_risk:   bool = True


DEFAULT_SKY_PROCESS_NAMES: list[str] = ["Sky.exe", "Sky Children of the Light.exe"]


@dataclass
class UpdateSettings:
    auto_check: bool = True
    channel: Literal["stable", "beta"] = "stable"
    skip_version: str = ""
    check_interval_s: int = 86400
    last_check_ts: int = 0
    # Timestamp of the last *failed* fetch. Reset to 0 on the next successful
    # check. Used by ``should_retry_after_error`` to do short backoff retries
    # (independent of the long ``check_interval_s`` throttle for successful
    # checks) so a one-off network blip does not lock the user out of update
    # notifications for a full day.
    last_error_ts: int = 0
    last_notified_version: str = ""
    legacy_old_dir_sweep_pending: bool = False

    @classmethod
    def from_dict(cls, data: Any) -> UpdateSettings:
        if not isinstance(data, dict):
            return cls()
        
        interval = data.get("check_interval_s", 86400)
        if not isinstance(interval, int) or isinstance(interval, bool):
            interval = 86400
        elif interval < 0:
            interval = 0

        last_check = data.get("last_check_ts", 0)
        if not isinstance(last_check, int) or isinstance(last_check, bool):
            last_check = 0

        last_err = data.get("last_error_ts", 0)
        if not isinstance(last_err, int) or isinstance(last_err, bool):
            last_err = 0

        skip = data.get("skip_version", "")
        if not isinstance(skip, str):
            skip = ""
            
        auto_chk = data.get("auto_check", True)
        if not isinstance(auto_chk, bool):
            auto_chk = True

        raw_channel = data.get("channel", "stable")
        if isinstance(raw_channel, str) and raw_channel.lower() in ("stable", "beta"):
            channel = cast(Literal["stable", "beta"], raw_channel.lower())
        else:
            channel = "stable"

        last_notified = data.get("last_notified_version", "")
        if not isinstance(last_notified, str):
            last_notified = ""

        legacy_sweep = data.get("legacy_old_dir_sweep_pending", False)
        if not isinstance(legacy_sweep, bool):
            legacy_sweep = False

        if "pending_update_version" in data or "auto_apply" in data:
            legacy_sweep = True

        return cls(
            auto_check=auto_chk,
            channel=channel,
            skip_version=skip,
            check_interval_s=interval,
            last_check_ts=last_check,
            last_error_ts=last_err,
            last_notified_version=last_notified,
            legacy_old_dir_sweep_pending=legacy_sweep,
        )


@dataclass
class AppConfig:
    """Typed representation of config.json values.

    Every field has a sensible default so the app works even if the
    config file does not exist or is empty.
    """

    theme:                       str           = "aurora"
    ui_background_mode:          str           = "transparent"
    default_hold_frames:         float         = DEFAULT_HOLD_FRAMES
    default_tempo_scale:         float         = 1.0
    game_fps:                    int           = DEFAULT_GAME_FPS
    telemetry_enabled_by_default: bool         = False
    verbose_hud:                 bool          = False
    hotkeys:                     HotkeyDefaults = field(default_factory=HotkeyDefaults)
    safety:                      SafetyDefaults  = field(default_factory=SafetyDefaults)
    songs_dir:                   str           = "songs"
    sky_process_names:           list[str]     = field(default_factory=lambda: list(DEFAULT_SKY_PROCESS_NAMES))
    allow_title_fallback:        bool          = False
    update:                      UpdateSettings = field(default_factory=UpdateSettings)


_runtime_cfg: AppConfig | None = None
_runtime_cfg_lock: threading.Lock = threading.Lock()


def clear_config_cache() -> None:
    """Reset the in-memory config cache (primarily for tests)."""
    global _runtime_cfg
    with _runtime_cfg_lock:
        _runtime_cfg = None


def sky_process_names_csv(cfg: AppConfig | None = None) -> str:
    names = (cfg or AppConfig()).sky_process_names
    return ",".join(names)


def resolve_game_fps(value: int | None) -> int:
    """Return the effective game FPS; never returns 0/None. Rejects unknown FPS."""
    if value is None or value <= 0:
        return DEFAULT_GAME_FPS
    if value not in VALID_FPS:
        return DEFAULT_GAME_FPS
    return value


def normalize_fps_value(fps: int | None) -> int:
    """Return the persisted FPS value; defaults to 60 when unset or invalid."""
    return resolve_game_fps(fps)


def persist_default_hold_frames(cfg: AppConfig, hold_frames: float) -> None:
    cfg.default_hold_frames = normalize_hold_frames(hold_frames)
    save_config(cfg)


def persist_default_tempo(cfg: AppConfig, tempo_scale: float) -> None:
    if tempo_scale <= 0:
        raise ValueError("tempo_scale must be > 0")
    cfg.default_tempo_scale = tempo_scale
    save_config(cfg)


def persist_default_fps(cfg: AppConfig, fps: int | None) -> None:
    cfg.game_fps = normalize_fps_value(fps)
    save_config(cfg)


def persist_playback_defaults(
    cfg: AppConfig,
    *,
    hold_frames: float,
    tempo_scale: float,
    fps: int | None,
) -> None:
    if tempo_scale <= 0:
        raise ValueError("tempo_scale must be > 0")
    cfg.default_hold_frames = normalize_hold_frames(hold_frames)
    cfg.default_tempo_scale = tempo_scale
    cfg.game_fps = normalize_fps_value(fps)
    save_config(cfg)


def persist_calibration_defaults(
    cfg: AppConfig,
    *,
    hold_frames: float,
    tempo_scale: float,
    fps: int,
) -> None:
    """Persist calibration without storing already frame-scaled hold values."""
    if tempo_scale <= 0:
        raise ValueError("tempo_scale must be > 0")
    cfg.default_hold_frames = normalize_hold_frames(hold_frames)
    cfg.default_tempo_scale = tempo_scale
    cfg.game_fps = normalize_fps_value(fps)
    save_config(cfg)


def argparse_base_defaults() -> dict[str, Any]:
    """Generic CLI defaults before ``apply_config_defaults`` applies config.json."""
    hk = HotkeyDefaults()
    return {
        "hold_frames": DEFAULT_HOLD_FRAMES,
        "tempo_scale": 1.0,
        "debug_csv": False,
        "verbose_hud": False,
        "theme": None,
        "ui_background": None,
        "songs_dir": Path(AppConfig.songs_dir),
        "fps": None,
        "allow_title_fallback": False,
        "pause_key": hk.pause,
        "skip_key": hk.skip,
        "quit_key": hk.quit,
        "refocus_key": hk.refocus,
        "panic_key": hk.panic,
        "sky_process_names": sky_process_names_csv(),
    }


def _load_raw() -> dict[str, Any]:
    """Return the raw dict from config.json, or {} on any error."""
    if not CONFIG_PATH.exists():
        return {}
    try:
        with CONFIG_PATH.open("r", encoding="utf-8") as f:
            data = json.load(f)
        if not isinstance(data, dict):
            return {}
        return data
    except Exception:
        return {}


_LEGACY_HOLD_PROFILE_MAP = {
    "local_precise": 1.0,
    "balanced": 1.0,
    "audience_safe": 1.5,
    "remote_safe": 1.5,
    "online_audible_safe": 1.5,
    "online_audible": 1.5,
}


def _migrate_raw_config(raw: dict[str, Any]) -> dict[str, Any]:
    """Convert v2 timing selection to v3 and atomically sanitize the source."""
    migrated = dict(raw)
    fps = resolve_game_fps(_parse_int(raw.get("game_fps"), DEFAULT_GAME_FPS))
    legacy_name = str(raw.get("default_timing_profile", "balanced")).lower().replace("-", "_")
    candidate: float | None = None
    profiles = raw.get("timing_profiles")
    selected = profiles.get(legacy_name, {}) if isinstance(profiles, dict) else {}
    if not selected and isinstance(profiles, dict):
        for profile_key, profile_value in profiles.items():
            if str(profile_key).lower().replace("-", "_") == legacy_name:
                selected = profile_value
                break
    if not isinstance(selected, dict):
        selected = {}
    for key in ("min_hold_frames", "hold_frames"):
        if key in selected:
            try:
                candidate = nearest_hold_frames(float(selected[key]))
            except (TypeError, ValueError, OverflowError):
                candidate = None
            if candidate is not None:
                break
    if candidate is None:
        from sky_music.domain.hold_timing import frame_duration_us
        frame_us = frame_duration_us(fps)
        for key in ("min_hold_us", "hold_us"):
            if key in selected:
                try:
                    candidate = nearest_hold_frames(float(selected[key]) / frame_us)
                except (TypeError, ValueError, OverflowError):
                    candidate = None
                if candidate is not None:
                    break
    if candidate is None:
        candidate = _LEGACY_HOLD_PROFILE_MAP.get(legacy_name, DEFAULT_HOLD_FRAMES)

    migrated["schema_version"] = SCHEMA_VERSION
    migrated["default_hold_frames"] = normalize_hold_frames(
        raw.get("default_hold_frames", candidate), DEFAULT_HOLD_FRAMES
    )
    for key in (
        "default_timing_profile", "timing_profiles", "frame_timing", "hold_us",
        "min_hold_us", "hold_frames", "min_hold_frames", "hold_unframed_us",
        "min_hold_unframed_us",
    ):
        migrated.pop(key, None)
    if migrated != raw:
        _write_raw_atomic(migrated)
    return migrated


def _write_raw_atomic(raw: dict[str, Any]) -> None:
    CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
    tmp = CONFIG_PATH.with_suffix(".json.tmp")
    try:
        with tmp.open("w", encoding="utf-8") as f:
            json.dump(raw, f, indent=4)
        os.replace(tmp, CONFIG_PATH)
    finally:
        with contextlib.suppress(Exception):
            tmp.unlink(missing_ok=True)


def _build_config_from_disk() -> AppConfig:
    raw = _migrate_raw_config(_load_raw())
    hk_raw = raw.get("hotkeys", {}) if isinstance(raw.get("hotkeys"), dict) else {}
    sf_raw = raw.get("safety", {})   if isinstance(raw.get("safety"),  dict) else {}
    up_raw = raw.get("update", {})

    hotkeys = HotkeyDefaults(
        pause   = str(hk_raw.get("pause",   HotkeyDefaults.pause)),
        skip    = str(hk_raw.get("skip",    HotkeyDefaults.skip)),
        quit    = str(hk_raw.get("quit",    HotkeyDefaults.quit)),
        refocus = str(hk_raw.get("refocus", HotkeyDefaults.refocus)),
        panic   = str(hk_raw.get("panic",   HotkeyDefaults.panic)),
    )

    safety = SafetyDefaults(
        prompt_on_medium_risk = _parse_bool(sf_raw.get("prompt_on_medium_risk"), SafetyDefaults.prompt_on_medium_risk),
        prompt_on_high_risk   = _parse_bool(sf_raw.get("prompt_on_high_risk"),   SafetyDefaults.prompt_on_high_risk),
    )

    update_settings = UpdateSettings.from_dict(up_raw)

    spn_raw = raw.get("sky_process_names")
    if isinstance(spn_raw, list):
        sky_process_names = [str(item) for item in spn_raw]
    else:
        sky_process_names = list(DEFAULT_SKY_PROCESS_NAMES)

    return AppConfig(
        theme                        = str(raw.get("theme", AppConfig.theme)),
        ui_background_mode           = str(raw.get("ui_background_mode", AppConfig.ui_background_mode)),
        default_hold_frames          = normalize_hold_frames(raw.get("default_hold_frames")),
        default_tempo_scale          = _parse_float(raw.get("default_tempo_scale"), AppConfig.default_tempo_scale),
        game_fps                     = resolve_game_fps(_parse_int(raw.get("game_fps"), AppConfig.game_fps)),
        telemetry_enabled_by_default = _parse_bool(raw.get("telemetry_enabled_by_default"), AppConfig.telemetry_enabled_by_default),
        verbose_hud                  = _parse_bool(raw.get("verbose_hud"), AppConfig.verbose_hud),
        hotkeys                      = hotkeys,
        safety                       = safety,
        songs_dir                    = str(raw.get("songs_dir", AppConfig.songs_dir)),
        sky_process_names            = sky_process_names,
        allow_title_fallback         = _parse_bool(raw.get("allow_title_fallback"), AppConfig.allow_title_fallback),
        update                       = update_settings,
    )


def load_config(*, force_reload: bool = False) -> AppConfig:
    """Load config.json and return a typed ``AppConfig`` with all defaults applied.

    The result is cached in memory after the first load; call ``save_config`` to
    update the cache, or ``force_reload=True`` to re-read from disk.
    """
    global _runtime_cfg
    if not force_reload:
        with _runtime_cfg_lock:
            if _runtime_cfg is not None:
                return _runtime_cfg
    new_cfg = _build_config_from_disk()
    with _runtime_cfg_lock:
        _runtime_cfg = new_cfg
    return new_cfg


def save_config(cfg: AppConfig) -> None:
    """Persist ``cfg`` to config.json, preserving any unknown keys.

    Concurrency: the full read-modify-write cycle (load raw → overlay known
    keys → write file → swap into place → update in-memory cache) is wrapped
    in ``_runtime_cfg_lock`` AND uses an atomic ``os.replace`` swap. Two
    writers racing would otherwise interleave mid-stream and corrupt
    config.json — the picker launch-time update worker
    (``app.py::check_for_updates_worker``) is the sole automatic caller of
    ``record_successful_check``.
    """
    global _runtime_cfg
    with _runtime_cfg_lock:
        raw = _load_raw()

        # Update known keys
        raw["theme"]                        = cfg.theme
        raw["ui_background_mode"]           = cfg.ui_background_mode
        raw["default_hold_frames"]          = normalize_hold_frames(cfg.default_hold_frames)
        raw["default_tempo_scale"]          = cfg.default_tempo_scale
        raw["game_fps"]                     = cfg.game_fps
        raw["telemetry_enabled_by_default"] = cfg.telemetry_enabled_by_default
        raw["verbose_hud"]                  = cfg.verbose_hud
        raw.pop("rt_time_critical", None)
        raw["hotkeys"] = {
            "pause":   cfg.hotkeys.pause,
            "skip":    cfg.hotkeys.skip,
            "quit":    cfg.hotkeys.quit,
            "refocus": cfg.hotkeys.refocus,
            "panic":   cfg.hotkeys.panic,
        }
        raw["safety"] = {
            "prompt_on_medium_risk": cfg.safety.prompt_on_medium_risk,
            "prompt_on_high_risk":   cfg.safety.prompt_on_high_risk,
        }
        for legacy_key in (
            "default_timing_profile", "timing_profiles", "frame_timing", "hold_us",
            "min_hold_us", "hold_frames", "min_hold_frames", "hold_unframed_us",
            "min_hold_unframed_us",
        ):
            raw.pop(legacy_key, None)
        raw["songs_dir"]                    = cfg.songs_dir
        raw["sky_process_names"]            = cfg.sky_process_names
        raw["allow_title_fallback"]         = cfg.allow_title_fallback
        raw["update"] = {
            "auto_check": cfg.update.auto_check,
            "channel": cfg.update.channel,
            "skip_version": cfg.update.skip_version,
            "check_interval_s": cfg.update.check_interval_s,
            "last_check_ts": cfg.update.last_check_ts,
            "last_error_ts": cfg.update.last_error_ts,
            "last_notified_version": cfg.update.last_notified_version,
            "legacy_old_dir_sweep_pending": cfg.update.legacy_old_dir_sweep_pending,
        }
        raw["schema_version"]               = SCHEMA_VERSION

        try:
            # Write to a sibling tempfile then os.replace into place. On NTFS
            # (and POSIX) ``os.replace`` is atomic: a concurrent reader will
            # observe either the old contents or the new contents, never a
            # truncated/partial write. The tempfile inherits CONFIG_PATH's
            # directory so the rename stays on the same volume.
            CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
            tmp = CONFIG_PATH.with_suffix(".json.tmp")
            try:
                with tmp.open("w", encoding="utf-8") as f:
                    json.dump(raw, f, indent=4)
                os.replace(tmp, CONFIG_PATH)
                _runtime_cfg = cfg
            except Exception:
                # tmp.open()/os.replace may fail (disk full, antivirus, etc.).
                # Clean up the tempfile if it exists; suppress cleanup errors.
                with contextlib.suppress(Exception):
                    tmp.unlink(missing_ok=True)
                raise
        except Exception as e:
            # Failure to persist is non-fatal; we still hold the latest
            # in-memory cfg so the running session keeps working.
            print(f"Failed to save config: {e}")


def apply_config_defaults(args: Any, cfg: AppConfig) -> None:
    """Update argparse Namespace with configured defaults for unset flags.
    
    This is called *before* ``configure_from_args()`` so that explicit CLI
    flags always win.  Only fields with argparse defaults (i.e. the user did
    not supply them explicitly) are updated.
    """

    # argparse doesn't expose which flags were explicit; compare to generic CLI defaults.
    parser_defaults = argparse_base_defaults()

    if (
        not getattr(args, "_hold_frames_explicit", False)
        and getattr(args, "hold_frames", None) == parser_defaults["hold_frames"]
    ):
        args.hold_frames = normalize_hold_frames(cfg.default_hold_frames)

    if getattr(args, "tempo_scale", None) == parser_defaults["tempo_scale"]:
        args.tempo_scale = cfg.default_tempo_scale

    if getattr(args, "debug_csv", None) == parser_defaults["debug_csv"]:
        args.debug_csv = cfg.telemetry_enabled_by_default

    if getattr(args, "verbose_hud", None) == parser_defaults["verbose_hud"]:
        args.verbose_hud = cfg.verbose_hud

    if getattr(args, "theme", None) == parser_defaults["theme"]:
        args.theme = cfg.theme

    if getattr(args, "ui_background", None) == parser_defaults["ui_background"]:
        args.ui_background = cfg.ui_background_mode

    if getattr(args, "songs_dir", None) == parser_defaults["songs_dir"]:
        args.songs_dir = Path(cfg.songs_dir)

    if getattr(args, "allow_title_fallback", None) == parser_defaults["allow_title_fallback"]:
        args.allow_title_fallback = cfg.allow_title_fallback

    if getattr(args, "fps", None) == parser_defaults["fps"]:
        args.fps = resolve_game_fps(cfg.game_fps)

    if getattr(args, "pause_key", None) == parser_defaults["pause_key"]:
        args.pause_key = cfg.hotkeys.pause

    if getattr(args, "skip_key", None) == parser_defaults["skip_key"]:
        args.skip_key = cfg.hotkeys.skip

    if getattr(args, "quit_key", None) == parser_defaults["quit_key"]:
        args.quit_key = cfg.hotkeys.quit

    if getattr(args, "refocus_key", None) == parser_defaults["refocus_key"]:
        args.refocus_key = cfg.hotkeys.refocus

    if getattr(args, "panic_key", None) == parser_defaults["panic_key"]:
        args.panic_key = cfg.hotkeys.panic

    if getattr(args, "sky_process_names", None) == parser_defaults["sky_process_names"]:
        args.sky_process_names = sky_process_names_csv(cfg)


def persist_update_skip_version(cfg: AppConfig, version: str) -> None:
    cfg.update.skip_version = version
    save_config(cfg)


def persist_update_check_ts(cfg: AppConfig, ts: int) -> None:
    cfg.update.last_check_ts = ts
    save_config(cfg)


def persist_update_auto_check(cfg: AppConfig, auto: bool) -> None:
    cfg.update.auto_check = auto
    save_config(cfg)


def persist_update_channel(cfg: AppConfig, channel: Literal["stable", "beta"]) -> None:
    cfg.update.channel = channel
    save_config(cfg)


def persist_update_last_notified(cfg: AppConfig, version: str) -> None:
    cfg.update.last_notified_version = version
    save_config(cfg)


def persist_legacy_old_dir_sweep_pending(cfg: AppConfig, pending: bool) -> None:
    cfg.update.legacy_old_dir_sweep_pending = pending
    save_config(cfg)


def persist_update_error_ts(cfg: AppConfig, ts: int) -> None:
    """Persist ``last_error_ts`` so a short-backoff retry can be scheduled.

    Pass ``ts=0`` to clear it (after a successful check).
    """
    cfg.update.last_error_ts = ts
    save_config(cfg)
