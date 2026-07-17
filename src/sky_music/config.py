"""Sky Music Player — persistent user configuration (config.json schema v2).

The config file is read once at startup and provides *defaults* that can be
overridden by CLI flags.  Saving happens when the user explicitly changes a
setting in the UI.
"""

import json
import threading
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Literal, cast

# Defined here (not in infrastructure.rt_priority) so that low-level modules like
# platform.win32.inputs can import config without creating an import cycle through the
# platform layer. rt_priority re-exports this name.
RtPriorityMode = Literal["auto", "mmcss", "time_critical", "highest", "off"]

SCHEMA_VERSION: int = 2
DEFAULT_GAME_FPS: int = 60
VALID_FPS: tuple[int, ...] = (30, 60, 90, 120, 144, 165, 240)
CONFIG_PATH: Path = Path(__file__).resolve().parents[2] / "config.json"


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


@dataclass(frozen=True)
class FrameTimingDefaults:
    """Frame-aware scaling ratios (defaults match built-in FrameTimingPolicy formulas)."""

    min_visible_hold_frames: float = 1.25
    # Compression floor for the key-down (visibility): empirically every key-down, even a
    # compressed one, needs >= 1 frame to register (Exp1). 1.25 = one frame + ~25% margin,
    # matching the hold target, and applied at all FPS (not just <60).
    min_hold_min_frame_ratio: float = 1.25

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> FrameTimingDefaults:
        def ratio(key: str, default: float) -> float:
            val = raw.get(key, default)
            try:
                return float(val)
            except (TypeError, ValueError):
                return default

        return cls(
            min_visible_hold_frames=ratio("min_visible_hold_frames", 1.25),
            min_hold_min_frame_ratio=ratio("min_hold_min_frame_ratio", 1.25),
        )

    def as_policy_kwargs(self) -> dict[str, float | int]:
        return {
            "min_visible_hold_frames": self.min_visible_hold_frames,
            "min_hold_min_frame_ratio": self.min_hold_min_frame_ratio,
        }


# ==============================================================================
# ⚠️ ATTENTION DEVELOPERS & AI ASSISTANTS:
# Before modifying or creating new timing profiles, you MUST read and adhere
# to the constraints defined in `docs/timing-principles.md`.
#
# Critical mathematical constraint for game engine input polling:
#   min_hold_us is the visibility floor for every scheduled key-down.
#   Same-key repeats whose interval is below min_hold_us are infeasible.
# ==============================================================================
DEFAULT_TIMING_PROFILES: dict[str, dict[str, Any]] = {
    "local_precise": {
        # hold is intentionally omitted from built-ins and derives from min_hold. Declare hold_*
        # explicitly only as an experiment/escape hatch; see hold-min-hold-unification-plan.md.
        "min_hold_frames": 1,
        "min_hold_unframed_us": 22000,
        "spin_threshold_us": 800,
        "focus_restore_grace_us": 50000,
    },
    "balanced": {
        "min_hold_frames": 1.02,
        "min_hold_unframed_us": 17000,
        "spin_threshold_us": 800,
        "focus_restore_grace_us": 100000,
    },
    "audience_safe": {
        # Deliberately sharp frame-relative audience profile. At high local FPS this no longer
        # guarantees a fixed remote-client visibility wall; see floor-removal-three-profile-plan.
        "min_hold_frames": 1.5,
        "min_hold_unframed_us": 18000,
        "spin_threshold_us": 800,
        "focus_restore_grace_us": 150000,
    },
}


DEFAULT_SKY_PROCESS_NAMES: list[str] = ["Sky.exe", "Sky Children of the Light.exe"]


@dataclass
class UpdateSettings:
    auto_check: bool = False
    auto_apply: bool = False
    skip_version: str = ""
    check_interval_s: int = 86400
    last_check_ts: int = 0
    # Timestamp of the last *failed* fetch. Reset to 0 on the next successful
    # check. Used by ``should_retry_after_error`` to do short backoff retries
    # (independent of the long ``check_interval_s`` throttle for successful
    # checks) so a one-off network blip does not lock the user out of update
    # notifications for a full day.
    last_error_ts: int = 0
    # True once the user has explicitly chosen whether to enable automatic
    # update checks (via the first-run dialog or by toggling the Settings
    # checkbox). Until then, no automatic check fires even if auto_check was
    # left at the legacy default ``True`` in an older config.json — modern
    # best practice is opt-in for "phoning home" to GitHub on the user's
    # behalf. Preserved across rounds so a returning install doesn't re-prompt.
    update_choice_made: bool = False
    pending_update_version: str = ""

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
            
        # Backward compat: pre-this-change installs stored auto_check=True at the
        # dataclass default, but never set ``update_choice_made``. If the user
        # previously had auto_check=True persisted, we treat that as an explicit
        # choice (a returning install) by setting update_choice_made=True.
        # For brand-new installs (no update block at all), the dataclass default
        # (auto_check=False, update_choice_made=False) stands and the first-run
        # dialog offers the choice.
        auto_chk_raw = data.get("auto_check", None)
        had_update_block = "auto_check" in data
        if auto_chk_raw is None and not had_update_block:
            auto_chk = False
            choice_made = False
        elif not isinstance(auto_chk_raw, bool):
            auto_chk = False
            choice_made = True
        else:
            auto_chk = auto_chk_raw
            choice_made = data.get("update_choice_made", True if auto_chk_raw else False)
            if not isinstance(choice_made, bool):
                choice_made = bool(auto_chk_raw)

        auto_app = data.get("auto_apply", False)
        if not isinstance(auto_app, bool):
            auto_app = False

        pending = data.get("pending_update_version", "")
        if not isinstance(pending, str):
            pending = ""

        return cls(
            auto_check=auto_chk,
            auto_apply=auto_app,
            skip_version=skip,
            check_interval_s=interval,
            last_check_ts=last_check,
            last_error_ts=last_err,
            update_choice_made=choice_made,
            pending_update_version=pending,
        )


@dataclass
class AppConfig:
    """Typed representation of config.json values.

    Every field has a sensible default so the app works even if the
    config file does not exist or is empty.
    """

    theme:                       str           = "aurora"
    ui_background_mode:          str           = "transparent"
    default_timing_profile:      str           = "balanced"
    default_tempo_scale:         float         = 1.0
    game_fps:                    int           = DEFAULT_GAME_FPS
    telemetry_enabled_by_default: bool         = False
    verbose_hud:                 bool          = False
    use_dispatch_thread:         bool          = True
    input_path_warn_us:          int           = 3000
    rt_priority_mode:            RtPriorityMode = "auto"  # Replaces dead rt_time_critical. Old "true" maps to "auto", "false" to "off".
    # Graduated 2026-06-11 after live A/B (see docs/perf-baselines/2026-06-baseline.md §3 and
    # the archived rt-pipeline-extreme-optimization-plan): defaults ON in production. CLI
    # --no-adaptive-lead / --no-adaptive-spin are the kill switches.
    enable_adaptive_lead:         bool          = True
    enable_adaptive_spin:         bool          = True
    hotkeys:                     HotkeyDefaults = field(default_factory=HotkeyDefaults)
    safety:                      SafetyDefaults  = field(default_factory=SafetyDefaults)
    frame_timing:                FrameTimingDefaults = field(default_factory=FrameTimingDefaults)
    timing_profiles:             dict[str, dict[str, Any]] = field(default_factory=dict)
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


def normalize_profile_name(name: str) -> str:
    n = name.lower().replace("-", "_")
    if n in ("remote_safe", "audience_safe", "online_audible_safe", "online_audible"):
        return "audience_safe"
    return n


CLI_PROFILE_NAMES: tuple[str, ...] = (
    "balanced",
    "local-precise",
    "audience-safe",
)

_PROFILE_KEY_TO_CLI: dict[str, str] = {
    "balanced": "balanced",
    "local_precise": "local-precise",
    "audience_safe": "audience-safe",
}

def canonical_profile_name(name: str) -> str:
    """Normalize a profile name to picker/CLI form (hyphens, no @fps suffix)."""
    base = name.split("@", 1)[0].strip()
    key = normalize_profile_name(base)
    if key in _PROFILE_KEY_TO_CLI:
        return _PROFILE_KEY_TO_CLI[key]
    return "balanced"


def display_profile_name(base: str, fps: int | None = None) -> str:
    """HUD-friendly profile label; FPS suffix is display-only, never persisted."""
    canonical = canonical_profile_name(base)
    if fps is not None and fps > 0:
        return f"{canonical}@{fps}fps"
    return canonical


def merged_timing_profiles(cfg: AppConfig) -> dict[str, dict[str, Any]]:
    """Built-in profiles with user overrides from config.json."""
    merged = {name: dict(profile) for name, profile in DEFAULT_TIMING_PROFILES.items()}
    for raw_name, override in cfg.timing_profiles.items():
        name = normalize_profile_name(raw_name)
        if name not in merged:
            continue
        base = dict(merged[name])
        base.update(override)
        merged[name] = base
    return merged


def profile_dict_for(cfg: AppConfig, profile_name: str) -> dict[str, Any]:
    """Resolve a timing profile dict by name, falling back to balanced."""
    key = normalize_profile_name(canonical_profile_name(profile_name))
    merged = merged_timing_profiles(cfg)
    return merged.get(key, merged["balanced"])


def spin_threshold_for_profile(cfg: AppConfig, profile_name: str) -> int:
    p_dict = profile_dict_for(cfg, profile_name)
    _default_spin: int = 800  # matches DEFAULT_TIMING_PROFILES["balanced"]["spin_threshold_us"]
    raw = p_dict.get("spin_threshold_us", _default_spin)
    return int(raw) if raw is not None else _default_spin


def sky_process_names_csv(cfg: AppConfig | None = None) -> str:
    names = (cfg or AppConfig()).sky_process_names
    return ",".join(names)


def resolve_game_fps(value: int | None) -> int:
    """Return the effective game FPS; never returns 0/None."""
    if value is None or value <= 0:
        return DEFAULT_GAME_FPS
    return value


def normalize_fps_value(fps: int | None) -> int:
    """Return the persisted FPS value; defaults to 60 when unset or invalid."""
    return resolve_game_fps(fps)


def persist_default_profile(cfg: AppConfig, profile_name: str) -> None:
    cfg.default_timing_profile = canonical_profile_name(profile_name)
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
    profile_name: str,
    tempo_scale: float,
    fps: int | None,
) -> None:
    if tempo_scale <= 0:
        raise ValueError("tempo_scale must be > 0")
    cfg.default_timing_profile = canonical_profile_name(profile_name)
    cfg.default_tempo_scale = tempo_scale
    cfg.game_fps = normalize_fps_value(fps)
    save_config(cfg)


def persist_calibration_defaults(
    cfg: AppConfig,
    *,
    profile_name: str,
    tempo_scale: float,
    fps: int,
) -> None:
    """Persist calibration without storing already frame-scaled hold values."""
    if tempo_scale <= 0:
        raise ValueError("tempo_scale must be > 0")
    canonical = canonical_profile_name(profile_name)
    profile_key = normalize_profile_name(canonical)
    base_profile = dict(profile_dict_for(cfg, profile_key))

    cfg.default_timing_profile = canonical
    cfg.default_tempo_scale = tempo_scale
    cfg.game_fps = normalize_fps_value(fps)
    cfg.timing_profiles[profile_key] = base_profile
    save_config(cfg)


def argparse_base_defaults() -> dict[str, Any]:
    """Generic CLI defaults before ``apply_config_defaults`` applies config.json."""
    hk = HotkeyDefaults()
    return {
        "timing_profile": "balanced",
        "tempo_scale": 1.0,
        "debug_csv": False,
        "verbose_hud": False,
        "no_dispatch_thread": False,
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


def _build_config_from_disk() -> AppConfig:
    raw = _load_raw()
    hk_raw = raw.get("hotkeys", {}) if isinstance(raw.get("hotkeys"), dict) else {}
    sf_raw = raw.get("safety", {})   if isinstance(raw.get("safety"),  dict) else {}
    ft_raw = raw.get("frame_timing", {}) if isinstance(raw.get("frame_timing"), dict) else {}
    up_raw = raw.get("update", {})

    hotkeys = HotkeyDefaults(
        pause   = str(hk_raw.get("pause",   HotkeyDefaults.pause)),
        skip    = str(hk_raw.get("skip",    HotkeyDefaults.skip)),
        quit    = str(hk_raw.get("quit",    HotkeyDefaults.quit)),
        refocus = str(hk_raw.get("refocus", HotkeyDefaults.refocus)),
        panic   = str(hk_raw.get("panic",   HotkeyDefaults.panic)),
    )

    safety = SafetyDefaults(
        prompt_on_medium_risk = bool(sf_raw.get("prompt_on_medium_risk", SafetyDefaults.prompt_on_medium_risk)),
        prompt_on_high_risk   = bool(sf_raw.get("prompt_on_high_risk",   SafetyDefaults.prompt_on_high_risk)),
    )

    frame_timing = FrameTimingDefaults.from_dict(ft_raw)
    update_settings = UpdateSettings.from_dict(up_raw)

    # Validate timing_profiles structure
    timing_profiles_raw = raw.get("timing_profiles", {})
    timing_profiles = (
        {name: profile_dict for name, profile_dict in timing_profiles_raw.items() if isinstance(profile_dict, dict)}
        if isinstance(timing_profiles_raw, dict)
        else {}
    )

    spn_raw = raw.get("sky_process_names")
    if isinstance(spn_raw, list):
        sky_process_names = [str(item) for item in spn_raw]
    else:
        sky_process_names = list(DEFAULT_SKY_PROCESS_NAMES)

    default_timing_profile = canonical_profile_name(
        str(raw.get("default_timing_profile", AppConfig.default_timing_profile))
    )

    return AppConfig(
        theme                        = str(raw.get("theme", AppConfig.theme)),
        ui_background_mode           = str(raw.get("ui_background_mode", AppConfig.ui_background_mode)),
        default_timing_profile       = default_timing_profile,
        default_tempo_scale          = float(raw.get("default_tempo_scale", AppConfig.default_tempo_scale)),
        game_fps                     = resolve_game_fps(raw.get("game_fps", AppConfig.game_fps)),
        telemetry_enabled_by_default = bool(raw.get("telemetry_enabled_by_default", AppConfig.telemetry_enabled_by_default)),
        verbose_hud                  = bool(raw.get("verbose_hud", AppConfig.verbose_hud)),
        use_dispatch_thread          = bool(raw.get("use_dispatch_thread", AppConfig.use_dispatch_thread)),
        input_path_warn_us           = max(0, int(raw.get("input_path_warn_us", AppConfig.input_path_warn_us))),
        # The legacy rt_time_critical flag was DEAD config (never wired to anything), so its value
        # carries no user intent and must not pin the new ladder off: it is ignored entirely and
        # dropped on the next save. Only an explicit rt_priority_mode key overrides the default.
        rt_priority_mode             = cast(RtPriorityMode, str(raw.get("rt_priority_mode", AppConfig.rt_priority_mode))),
        enable_adaptive_lead         = bool(raw.get("enable_adaptive_lead", AppConfig.enable_adaptive_lead)),
        enable_adaptive_spin         = bool(raw.get("enable_adaptive_spin", AppConfig.enable_adaptive_spin)),
        hotkeys                      = hotkeys,
        safety                       = safety,
        frame_timing                 = frame_timing,
        timing_profiles              = timing_profiles,
        songs_dir                    = str(raw.get("songs_dir", AppConfig.songs_dir)),
        sky_process_names            = sky_process_names,
        allow_title_fallback         = bool(raw.get("allow_title_fallback", AppConfig.allow_title_fallback)),
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
    """Persist ``cfg`` to config.json, preserving any unknown keys."""
    raw = _load_raw()
    
    # Update known keys
    raw["theme"]                        = cfg.theme
    raw["ui_background_mode"]           = cfg.ui_background_mode
    raw["default_timing_profile"]       = canonical_profile_name(cfg.default_timing_profile)
    raw["default_tempo_scale"]          = cfg.default_tempo_scale
    raw["game_fps"]                     = cfg.game_fps
    raw["telemetry_enabled_by_default"] = cfg.telemetry_enabled_by_default
    raw["verbose_hud"]                  = cfg.verbose_hud
    raw["use_dispatch_thread"]          = cfg.use_dispatch_thread
    raw["input_path_warn_us"]           = cfg.input_path_warn_us
    raw.pop("rt_time_critical", None)
    raw["rt_priority_mode"]             = cfg.rt_priority_mode
    raw["enable_adaptive_lead"]         = cfg.enable_adaptive_lead
    raw["enable_adaptive_spin"]         = cfg.enable_adaptive_spin
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
    raw["frame_timing"] = {
        "min_visible_hold_frames": cfg.frame_timing.min_visible_hold_frames,
        "min_hold_min_frame_ratio": cfg.frame_timing.min_hold_min_frame_ratio,
    }
    raw["timing_profiles"]              = cfg.timing_profiles
    raw["songs_dir"]                    = cfg.songs_dir
    raw["sky_process_names"]            = cfg.sky_process_names
    raw["allow_title_fallback"]         = cfg.allow_title_fallback
    raw["update"] = {
        "auto_check": cfg.update.auto_check,
        "auto_apply": cfg.update.auto_apply,
        "skip_version": cfg.update.skip_version,
        "check_interval_s": cfg.update.check_interval_s,
        "last_check_ts": cfg.update.last_check_ts,
        "last_error_ts": cfg.update.last_error_ts,
        "update_choice_made": cfg.update.update_choice_made,
        "pending_update_version": cfg.update.pending_update_version,
    }
    raw["schema_version"]               = SCHEMA_VERSION

    global _runtime_cfg
    try:
        with CONFIG_PATH.open("w", encoding="utf-8") as f:
            json.dump(raw, f, indent=4)
        with _runtime_cfg_lock:
            _runtime_cfg = cfg
    except Exception as e:
        print(f"Failed to save config: {e}")


def apply_config_defaults(args: Any, cfg: AppConfig) -> None:
    """Update argparse Namespace with configured defaults for unset flags.
    
    This is called *before* ``configure_from_args()`` so that explicit CLI
    flags always win.  Only fields with argparse defaults (i.e. the user did
    not supply them explicitly) are updated.
    """

    # argparse doesn't expose which flags were explicit; compare to generic CLI defaults.
    parser_defaults = argparse_base_defaults()

    if getattr(args, "timing_profile", None) == parser_defaults["timing_profile"]:
        args.timing_profile = canonical_profile_name(cfg.default_timing_profile)

    if getattr(args, "tempo_scale", None) == parser_defaults["tempo_scale"]:
        args.tempo_scale = cfg.default_tempo_scale

    if getattr(args, "debug_csv", None) == parser_defaults["debug_csv"]:
        args.debug_csv = cfg.telemetry_enabled_by_default

    if getattr(args, "verbose_hud", None) == parser_defaults["verbose_hud"]:
        args.verbose_hud = cfg.verbose_hud

    if getattr(args, "no_dispatch_thread", None) == parser_defaults["no_dispatch_thread"]:
        args.no_dispatch_thread = not cfg.use_dispatch_thread

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


def persist_update_auto_apply(cfg: AppConfig, auto: bool) -> None:
    cfg.update.auto_apply = auto
    save_config(cfg)


def persist_pending_update_version(cfg: AppConfig, version: str) -> None:
    cfg.update.pending_update_version = version
    save_config(cfg)

def persist_update_error_ts(cfg: AppConfig, ts: int) -> None:
    """Persist ``last_error_ts`` so a short-backoff retry can be scheduled.

    Pass ``ts=0`` to clear it (after a successful check).
    """
    cfg.update.last_error_ts = ts
    save_config(cfg)

def persist_update_choice_made(cfg: AppConfig, *, auto_check: bool) -> None:
    """Atomically persist the user's first-run / Settings toggle decision.

    Flips both ``update_choice_made=True`` and ``auto_check`` in a single
    ``save_config`` write so the first-run dialog prompt never re-appears.
    """
    cfg.update.auto_check = auto_check
    cfg.update.update_choice_made = True
    save_config(cfg)
