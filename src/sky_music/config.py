"""Sky Music Player — persistent user configuration (config.json schema v2).

The config file is read once at startup and provides *defaults* that can be
overridden by CLI flags.  Saving happens when the user explicitly changes a
value (e.g. switching theme with Ctrl+T).

Schema (all fields are optional; missing keys fall back to the defaults
defined in ``AppDefaults``):

.. code-block:: json

    {
        "schema_version": 2,
        "theme": "aurora",
        "default_timing_profile": "balanced",
        "default_tempo_scale": 1.0,
        "game_fps": 60,
        "telemetry_enabled_by_default": false,
        "verbose_hud": false,
        "hotkeys": {
            "pause":   "f8",
            "skip":    "f9",
            "quit":    "f10",
            "refocus": "f6",
            "panic":   "ctrl+alt+backspace"
        },
        "safety": {
            "prompt_on_medium_risk": true,
            "prompt_on_high_risk":   true
        }
    }
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

CONFIG_PATH: Path = Path("config.json")
SCHEMA_VERSION: int = 2


@dataclass
class HotkeyDefaults:
    pause:   str = "f8"
    skip:    str = "f9"
    quit:    str = "f10"
    refocus: str = "f6"
    panic:   str = "ctrl+alt+backspace"


@dataclass
class SafetyDefaults:
    prompt_on_medium_risk: bool = True
    prompt_on_high_risk:   bool = True


DEFAULT_TIMING_PROFILES: dict[str, dict[str, Any]] = {
    "local_precise": {
        "hold_us": 20000,
        "min_hold_us": 12000,
        "release_gap_us": 3000,
        "repeat_release_gap_us": 2000,
        "min_scheduled_hold_us": 500,
        "input_lead_us": 3000,
        "chord_merge_window_us": 1000,
        "spin_threshold_us": 800,
        "focus_restore_grace_us": 50000
    },
    "balanced": {
        "hold_us": 24000,
        "min_hold_us": 12000,
        "release_gap_us": 3000,
        "repeat_release_gap_us": 2000,
        "min_scheduled_hold_us": 500,
        "input_lead_us": 6000,
        "chord_merge_window_us": 2000,
        "spin_threshold_us": 500,
        "focus_restore_grace_us": 100000
    },
    "remote_safe": {
        "hold_us": 30000,
        "min_hold_us": 15000,
        "release_gap_us": 10000,
        "repeat_release_gap_us": 8000,
        "min_scheduled_hold_us": 500,
        "input_lead_us": 12000,
        "chord_merge_window_us": 4000,
        "spin_threshold_us": 200,
        "focus_restore_grace_us": 150000
    },
    "dense_safe": {
        "hold_us": 24000,
        "min_hold_us": 12000,
        "release_gap_us": 5000,
        "repeat_release_gap_us": 6000,
        "min_scheduled_hold_us": 500,
        "input_lead_us": 6000,
        "chord_merge_window_us": 3000,
        "spin_threshold_us": 500,
        "focus_restore_grace_us": 100000
    }
}


@dataclass
class AppConfig:
    """Typed representation of config.json values.

    Every field has a sensible default so the app works even if the
    config file does not exist or is empty.
    """

    theme:                       str           = "aurora"
    default_timing_profile:      str           = "balanced"
    default_tempo_scale:         float         = 1.0
    game_fps:                    int           = 60
    telemetry_enabled_by_default: bool         = False
    verbose_hud:                 bool          = False
    hotkeys:                     HotkeyDefaults = field(default_factory=HotkeyDefaults)
    safety:                      SafetyDefaults  = field(default_factory=SafetyDefaults)
    timing_profiles:             dict[str, dict[str, Any]] = field(default_factory=dict)
    songs_dir:                   str           = "songs"
    sky_process_names:           list[str]     = field(default_factory=lambda: ["Sky.exe", "Sky Children of the Light.exe"])
    allow_title_fallback:        bool          = False



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


def load_config() -> AppConfig:
    """Load config.json and return a typed ``AppConfig`` with all defaults applied."""
    raw = _load_raw()
    hk_raw = raw.get("hotkeys", {}) if isinstance(raw.get("hotkeys"), dict) else {}
    sf_raw = raw.get("safety", {})   if isinstance(raw.get("safety"),  dict) else {}

    hotkeys = HotkeyDefaults(
        pause   = hk_raw.get("pause",   HotkeyDefaults.pause),
        skip    = hk_raw.get("skip",    HotkeyDefaults.skip),
        quit    = hk_raw.get("quit",    HotkeyDefaults.quit),
        refocus = hk_raw.get("refocus", HotkeyDefaults.refocus),
        panic   = hk_raw.get("panic",   HotkeyDefaults.panic),
    )
    safety = SafetyDefaults(
        prompt_on_medium_risk = bool(sf_raw.get("prompt_on_medium_risk", SafetyDefaults.prompt_on_medium_risk)),
        prompt_on_high_risk   = bool(sf_raw.get("prompt_on_high_risk",   SafetyDefaults.prompt_on_high_risk)),
    )

    timing_profiles_raw = raw.get("timing_profiles")
    timing_profiles = {k: dict(v) for k, v in DEFAULT_TIMING_PROFILES.items()}
    if isinstance(timing_profiles_raw, dict):
        for name, profile_dict in timing_profiles_raw.items():
            if isinstance(profile_dict, dict):
                clean_name = name.lower().replace("-", "_")
                if clean_name not in timing_profiles:
                    timing_profiles[clean_name] = {}
                for k, v in profile_dict.items():
                    timing_profiles[clean_name][k] = v

    spn_raw = raw.get("sky_process_names")
    if isinstance(spn_raw, list):
        sky_process_names = [str(item) for item in spn_raw]
    else:
        sky_process_names = ["Sky.exe", "Sky Children of the Light.exe"]

    default_timing_profile = str(raw.get("default_timing_profile", AppConfig.default_timing_profile))
    if default_timing_profile in ("balanced_120fps", "low_fps_30", "fast", "conservative"):
        default_timing_profile = "balanced"

    return AppConfig(
        theme                        = str(raw.get("theme", AppConfig.theme)),
        default_timing_profile       = default_timing_profile,
        default_tempo_scale          = float(raw.get("default_tempo_scale", AppConfig.default_tempo_scale)),
        game_fps                     = int(raw.get("game_fps", AppConfig.game_fps)),
        telemetry_enabled_by_default = bool(raw.get("telemetry_enabled_by_default", AppConfig.telemetry_enabled_by_default)),
        verbose_hud                  = bool(raw.get("verbose_hud", AppConfig.verbose_hud)),
        hotkeys                      = hotkeys,
        safety                       = safety,
        timing_profiles              = timing_profiles,
        songs_dir                    = str(raw.get("songs_dir", "songs")),
        sky_process_names            = sky_process_names,
        allow_title_fallback         = bool(raw.get("allow_title_fallback", False))
    )


def save_config(cfg: AppConfig) -> None:
    """Persist ``cfg`` to config.json, preserving any unknown keys."""
    raw = _load_raw()
    raw["schema_version"]              = SCHEMA_VERSION
    raw["theme"]                       = cfg.theme
    raw["default_timing_profile"]      = cfg.default_timing_profile
    raw["default_tempo_scale"]         = cfg.default_tempo_scale
    raw["game_fps"]                    = cfg.game_fps
    raw["telemetry_enabled_by_default"] = cfg.telemetry_enabled_by_default
    raw["verbose_hud"]                 = cfg.verbose_hud
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
    raw["timing_profiles"] = cfg.timing_profiles
    raw["songs_dir"] = cfg.songs_dir
    raw["sky_process_names"] = cfg.sky_process_names
    raw["allow_title_fallback"] = cfg.allow_title_fallback
    try:
        with CONFIG_PATH.open("w", encoding="utf-8") as f:
            json.dump(raw, f, indent=4)
    except Exception:
        pass


def apply_config_defaults(args, cfg: AppConfig) -> None:
    """Apply saved config values as defaults for any CLI arg the user did NOT override.

    This is called *before* ``configure_from_args()`` so that explicit CLI
    flags always win.  Only fields with argparse defaults (i.e. the user did
    not supply them explicitly) are updated.
    """
    import argparse  # local import to keep module importable without argparse

    # We need to know which args were explicitly passed vs. came from defaults.
    # argparse doesn't expose this directly, so we compare against known defaults.
    parser_defaults = {
        "timing_profile": "balanced",
        "tempo_scale":    1.0,
        "fps":            60,
        "debug_csv":      False,
        "verbose_hud":    False,
        "pause_key":      "f8",
        "skip_key":       "f9",
        "quit_key":       "f10",
        "refocus_key":    "f6",
        "panic_key":      "ctrl+alt+backspace",
        "theme":          None,
        "songs_dir":      Path("songs"),
        "sky_process_names": "Sky.exe,Sky Children of the Light.exe",
        "allow_title_fallback": False,
    }

    # timing profile
    if getattr(args, "timing_profile", None) == parser_defaults["timing_profile"]:
        args.timing_profile = cfg.default_timing_profile

    # tempo scale
    if getattr(args, "tempo_scale", None) == parser_defaults["tempo_scale"]:
        args.tempo_scale = cfg.default_tempo_scale

    # game fps
    if getattr(args, "fps", None) == parser_defaults["fps"]:
        args.fps = cfg.game_fps

    # telemetry CSV
    if not getattr(args, "debug_csv", True):
        args.debug_csv = cfg.telemetry_enabled_by_default

    # verbose HUD
    if not getattr(args, "verbose_hud", True):
        args.verbose_hud = cfg.verbose_hud

    # hotkeys (only replace if user left default value)
    for attr, saved, default in [
        ("pause_key",   cfg.hotkeys.pause,   parser_defaults["pause_key"]),
        ("skip_key",    cfg.hotkeys.skip,    parser_defaults["skip_key"]),
        ("quit_key",    cfg.hotkeys.quit,    parser_defaults["quit_key"]),
        ("refocus_key", cfg.hotkeys.refocus, parser_defaults["refocus_key"]),
        ("panic_key",   cfg.hotkeys.panic,   parser_defaults["panic_key"]),
    ]:
        if getattr(args, attr, None) == default:
            setattr(args, attr, saved)

    # theme: if not passed on CLI, use saved theme
    if getattr(args, "theme", None) is None:
        args.theme = cfg.theme

    # songs_dir: if not overridden, use configured path
    if getattr(args, "songs_dir", None) == parser_defaults["songs_dir"]:
        args.songs_dir = Path(cfg.songs_dir)

    # sky_process_names: if not overridden, use configured list
    if getattr(args, "sky_process_names", None) == parser_defaults["sky_process_names"]:
        args.sky_process_names = ",".join(cfg.sky_process_names)

    # allow_title_fallback: if not overridden, use configured value
    if getattr(args, "allow_title_fallback", None) == parser_defaults["allow_title_fallback"]:
        args.allow_title_fallback = cfg.allow_title_fallback
