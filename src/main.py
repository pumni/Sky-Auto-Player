import argparse
import os
import sys
import time
from pathlib import Path
from dataclasses import dataclass

# Import từ các mô-đun chuyên biệt
from sky_music.platform.win32 import inputs
from sky_music.config import (
    load_config,
    apply_config_defaults,
    HotkeyDefaults,
    AppConfig,
    merged_timing_profiles,
    persist_calibration_defaults,
    persist_default_profile,
    persist_playback_defaults,
    spin_threshold_for_profile,
    sky_process_names_csv,
    canonical_profile_name,
    display_profile_name,
    CLI_PROFILE_NAMES,
)
from sky_music.domain.session_context import (
    PlaybackSessionContext,
    merge_session_with_overrides,
    apply_recommendation_to_context,
)
from sky_music.platform.win32.inputs import (
    enable_high_precision_timers,
    disable_high_precision_timers
)
from sky_music.ui.hud import (
    PLAYBACK_SKIPPED,
    PLAYBACK_QUIT,
    ProgressRenderer,
    clear_terminal
)
from sky_music.infrastructure.hotkeys import (
    PlaybackControls,
    parse_hotkey,
    hotkey_conflicts_with_note_keys
)
from sky_music.ui.picker_helpers import (
    SONG_DIR,
    SUPPORTED_EXTENSIONS,
    get_song_choices,
    resolve_song_selection,
    countdown_before_playback,
)

PLAYBACK_DEBUG = False
CURRENT_SCAN_CODE_MODE = "physical"
DEBUG_LOG_PATH = None
DEBUG_START_PERF = None
DEBUG_LOG_BUFFER = []
TIMING_POLICY = None
SLEEP_POLICY = None
PLAYBACK_SESSION: PlaybackSessionContext | None = None
TELEMETRY_CSV_ENABLED = False
DRY_RUN_MODE = False
TEMPO_SCALE = 1.0
TIMING_PROFILE_NAME = "balanced"
VERBOSE_HUD = False
USE_DISPATCH_THREAD = True
ENABLE_TIMER_GUARD = True
ENABLE_WAITABLE_TIMER = True
ENABLE_GC_PAUSE = True

def init_debug_log() -> None:
    global DEBUG_LOG_PATH, DEBUG_START_PERF
    DEBUG_START_PERF = time.perf_counter()
    debug_log_dir = Path("logs")
    debug_log_dir.mkdir(parents=True, exist_ok=True)
    DEBUG_LOG_PATH = debug_log_dir / f"playback_debug_{time.strftime('%Y%m%d_%H%M%S')}.log"
    with DEBUG_LOG_PATH.open("w", encoding="utf-8") as log_file:
        log_file.write(f"[{time.strftime('%Y-%m-%d %H:%M:%S')}] Debug playback log started\n")

def debug_log(message: str) -> None:
    if not PLAYBACK_DEBUG:
        return
    now = time.perf_counter()
    rel = 0.0 if DEBUG_START_PERF is None else now - DEBUG_START_PERF
    DEBUG_LOG_BUFFER.append(f"[{time.strftime('%Y-%m-%d %H:%M:%S')} +{rel:.6f}s] {message}")

def flush_debug_log() -> None:
    global DEBUG_LOG_BUFFER
    if not PLAYBACK_DEBUG or DEBUG_LOG_PATH is None or not DEBUG_LOG_BUFFER:
        return
    try:
        with DEBUG_LOG_PATH.open("a", encoding="utf-8") as log_file:
            log_file.write("\n".join(DEBUG_LOG_BUFFER) + "\n")
    except Exception as e:
        print(f"Failed to write logs: {e}")
    finally:
        DEBUG_LOG_BUFFER.clear()

# Kết nối hàm debug_log của main.py sang inputs.py để đồng bộ logging
inputs._debug_log_callback = debug_log

def _handle_risk_analysis(report, song, is_dry_run: bool, controls, policy_override_fn=None) -> tuple[bool, str | None, float | None]:
    """Display risk analysis, prompt user for action if severity is medium/high.

    Returns (should_continue, new_profile_name_or_None, new_tempo_scale_or_None).
    """

    severity = report.severity.upper()
    # Single source of truth for the profile recommendation: the analyzer already computed
    # report.suggested_profile from the same risk signals (was duplicated by _recommended_profile).
    recommended = report.suggested_profile

    print()
    print(f"  ┌─ Schedule Risk: {severity} " + "─" * max(0, 38 - len(severity)))
    for rec in report.recommendations:
        print(f"  │  * {rec}")
    print(f"  │  Recommended profile: {recommended}")
    print(f"  └{'─' * 44}")
    print()

    if is_dry_run:
        # In dry-run mode just show the warning, don't block
        return True, None, None

    print("  What would you like to do?")
    print(f"  [1] Switch to '{recommended}' profile")
    print( "  [2] Scale tempo down to 0.92x")
    print( "  [3] Dry-run first (simulate, no keystrokes)")
    print( "  [4] Proceed with current settings")
    print( "  [5] Cancel")
    print()

    try:
        choice = input("  Choice [1-5]: ").strip()
    except (EOFError, KeyboardInterrupt):
        return False, None, None

    if choice == "1":
        print(f"  → Switched to profile: {recommended}")
        # Cross-session persistence for risk-based profile change
        try:
            user_cfg = load_config()
            persist_default_profile(user_cfg, recommended)
        except Exception:
            pass
        return True, recommended, None
    elif choice == "2":
        print( "  → Tempo scaled to 0.92x")
        return True, None, 0.92
    elif choice == "3":
        print( "  → Running dry-run simulation first...")
        return True, None, None  # caller handles dry-run flag
    elif choice == "5":
        return False, None, None
    else:
        print( "  → Proceeding with current settings.")
        return True, None, None


def _mini_preflight(is_dry_run: bool, profile: str = "balanced", tempo: float = 1.0, controls = None) -> bool:
    """Preflight check before real playback — uniform premium TUI panel output."""
    if is_dry_run:
        return True

    import sky_music.infrastructure.doctor as doctor
    checks: list[tuple[bool, str]] = []

    # Constants for ANSI styling
    ANSI_RESET = "\033[0m"
    ANSI_BOLD = "\033[1m"
    ANSI_CYAN = "\033[36m"
    ANSI_GREEN = "\033[32m"
    ANSI_RED = "\033[31m"
    ANSI_YELLOW = "\033[33m"
    
    # Share the picker's width clamp and box renderer so every panel renders at
    # the same width with cell-width-correct borders (no len()-based drift).
    import shutil
    from sky_music.ui.text_render import clamp_terminal_width, ansi_box
    terminal_width = shutil.get_terminal_size((80, 24)).columns
    width = clamp_terminal_width(terminal_width)

    def print_ansi_box(title: str, lines: list[str], border_color: str = ANSI_CYAN) -> None:
        for rendered in ansi_box(title, lines, width=width, border_color=border_color):
            print(rendered)

    # 1. Sky window
    win = doctor.check_sky_window()
    checks.append((win["ok"], "Sky window detected" if win["ok"] else f"Sky not found: {win['msg']}"))
    
    if not win["ok"]:
        while True:
            dry_str = "ON" if is_dry_run else "OFF"
            header_line = f"Readiness │ profile {ANSI_CYAN}{profile}{ANSI_RESET} │ tempo {ANSI_CYAN}{tempo:.2f}x{ANSI_RESET} │ dry {ANSI_CYAN}{dry_str}{ANSI_RESET}"
            col1 = f"{ANSI_RED}✗{ANSI_RESET} Sky not found: {win['msg']}"
            status_line = f"{ANSI_YELLOW}Waiting for Sky focus. Playback has not started yet.{ANSI_RESET}"
            controls_line = f"{ANSI_BOLD}R{ANSI_RESET} retry │ {ANSI_BOLD}D{ANSI_RESET} dry-run │ {ANSI_BOLD}Enter{ANSI_RESET} cancel"
            
            print()
            print_ansi_box("SKY MUSIC HELPER", [header_line], border_color=ANSI_CYAN)
            print()
            print_ansi_box("Checks", [col1], border_color=ANSI_CYAN)
            print()
            print_ansi_box("Status", [status_line, controls_line], border_color=ANSI_YELLOW)
            print()
            
            try:
                choice = input("  Choice: ").strip().casefold()
            except (EOFError, KeyboardInterrupt):
                return False
            if choice == "r":
                win = doctor.check_sky_window()
                if win["ok"]:
                    checks[0] = (True, "Sky window detected")
                    break
            elif choice == "d":
                print("  → Use --dry-run to simulate without Sky.")
                return False
            else:
                return False

    # 2. Focus strict validation & wait delay
    from sky_music.platform.win32 import inputs as _inputs
    _inputs.focusWindow()
    import time
    time.sleep(0.25)
    
    focus_ok = _inputs.is_sky_active()
    if not focus_ok:
        while True:
            dry_str = "ON" if is_dry_run else "OFF"
            header_line = f"Readiness │ profile {ANSI_CYAN}{profile}{ANSI_RESET} │ tempo {ANSI_CYAN}{tempo:.2f}x{ANSI_RESET} │ dry {ANSI_CYAN}{dry_str}{ANSI_RESET}"
            
            check_lines = []
            for ok, msg in checks:
                icon = "✓" if ok else "✗"
                color = ANSI_GREEN if ok else ANSI_RED
                check_lines.append(f"{color}{icon}{ANSI_RESET} {msg}")
            
            col1 = f"{ANSI_RED}✗{ANSI_RESET} Focus failed"
            status_line = f"{ANSI_YELLOW}Waiting for Sky focus. Playback has not started yet.{ANSI_RESET}"
            controls_line = f"{ANSI_BOLD}R{ANSI_RESET} retry │ {ANSI_BOLD}D{ANSI_RESET} dry-run │ {ANSI_BOLD}Enter{ANSI_RESET} cancel"
            
            print()
            print_ansi_box("SKY MUSIC HELPER", [header_line], border_color=ANSI_CYAN)
            print()
            print_ansi_box("Checks", check_lines + [col1], border_color=ANSI_CYAN)
            print()
            print_ansi_box("Status", [status_line, controls_line], border_color=ANSI_YELLOW)
            print()
            
            try:
                choice = input("  Choice: ").strip().casefold()
            except (EOFError, KeyboardInterrupt):
                return False
            if choice == "r":
                _inputs.focusWindow()
                time.sleep(0.25)
                if _inputs.is_sky_active():
                    break
            elif choice == "d":
                print("  → Use --dry-run to simulate without Sky.")
                return False
            else:
                return False
                
    checks.append((True, "Focus confirmed"))

    # 3. Timer
    timer = doctor.check_timer_resolution()
    checks.append((timer["ok"], "Timer active" if timer["ok"] else timer["msg"]))

    # 4. Key conflicts
    keys = doctor.check_physical_keys_held()
    checks.append((keys["ok"], "No note keys held" if keys["ok"] else f"Held: {', '.join(keys.get('held_keys', []))}"))

    # Render gorgeous preflight panels!
    dry_str = "ON" if is_dry_run else "OFF"
    header_line = f"Readiness │ profile {ANSI_CYAN}{profile}{ANSI_RESET} │ tempo {ANSI_CYAN}{tempo:.2f}x{ANSI_RESET} │ dry {ANSI_CYAN}{dry_str}{ANSI_RESET}"
    
    row_parts = []
    for ok, msg in checks:
        icon = "✓" if ok else "✗"
        color = ANSI_GREEN if ok else ANSI_RED
        row_parts.append((ok, icon, color, msg))
        
    lines = []
    for i in range(0, len(row_parts), 2):
        part1 = row_parts[i]
        col1 = f"{part1[2]}{part1[1]}{ANSI_RESET} {part1[3]}"
        col1_len = 2 + len(part1[3])
        col1_pad = col1 + " " * (34 - col1_len)
        
        if i + 1 < len(row_parts):
            part2 = row_parts[i+1]
            col2 = f"{part2[2]}{part2[1]}{ANSI_RESET} {part2[3]}"
            col2_len = 2 + len(part2[3])
            col2_pad = col2 + " " * (34 - col2_len)
            lines.append(f"{col1_pad}   {col2_pad}")
        else:
            lines.append(col1_pad)

    status_line1 = f"{ANSI_GREEN}Readiness checks passed. Starting playback...{ANSI_RESET}"
    if controls is not None and controls.enabled:
        ctrls_str = (
            f"{ANSI_BOLD}{controls.panic.display}{ANSI_RESET} panic │ "
            f"{ANSI_BOLD}{controls.pause.display}{ANSI_RESET} pause/resume │ "
            f"{ANSI_BOLD}{controls.skip.display}{ANSI_RESET} skip │ "
            f"{ANSI_BOLD}{controls.quit.display}{ANSI_RESET} quit │ "
            f"{ANSI_BOLD}{controls.refocus.display}{ANSI_RESET} refocus"
        )
        status_lines = [status_line1, ctrls_str]
    else:
        status_lines = [status_line1]
    
    print()
    print_ansi_box("SKY MUSIC HELPER", [header_line], border_color=ANSI_CYAN)
    print()
    print_ansi_box("Checks", lines, border_color=ANSI_CYAN)
    print()
    print_ansi_box("Status", status_lines, border_color=ANSI_GREEN)
    print()
    return True


def _print_profile_comparison_table(cfg: AppConfig | None = None) -> None:
    """Print a rich ANSI side-by-side timing comparison table for all profiles."""
    ANSI_RESET  = "\033[0m"
    ANSI_BOLD   = "\033[1m"
    ANSI_CYAN   = "\033[36m"
    ANSI_YELLOW = "\033[33m"
    ANSI_DIM    = "\033[2m"

    cfg = cfg or load_config()
    profiles = merged_timing_profiles(cfg)

    def frame_coupled_ms(
        data: dict,
        *,
        value_key: str,
        unframed_key: str,
        fallback_unframed_key: str | None = None,
    ) -> str:
        value = data.get(value_key)
        if value is None:
            value = data.get(unframed_key)
        if value is None and fallback_unframed_key is not None:
            value = data.get(fallback_unframed_key, 0)
        return f"{int(value) // 1000}"

    COLS = [
        ("Profile",              lambda n, d: n),
        ("hold_ms",              lambda n, d: frame_coupled_ms(
            d,
            value_key="hold_us",
            unframed_key="hold_unframed_us",
            fallback_unframed_key="min_hold_unframed_us",
        )),
        ("min_hold_ms",          lambda n, d: frame_coupled_ms(d, value_key="min_hold_us", unframed_key="min_hold_unframed_us")),
        ("grace_ms",             lambda n, d: f"{d.get('focus_restore_grace_us', 0) // 1000}"),
        ("conflict_policy",      lambda n, d: d.get("same_key_conflict_policy", "degraded")),
    ]

    rows: list[list[str]] = []
    for name, data in sorted(profiles.items()):
        rows.append([fmt(name, data) for _, fmt in COLS])

    col_widths = [max(len(header), max(len(r[i]) for r in rows)) for i, (header, _) in enumerate(COLS)]

    def _fmt_row(cells: list[str], header: bool = False) -> str:
        parts = []
        for i, cell in enumerate(cells):
            padded = cell.ljust(col_widths[i])
            if header:
                parts.append(f"{ANSI_BOLD}{ANSI_CYAN}{padded}{ANSI_RESET}")
            elif i == 0:
                parts.append(f"{ANSI_YELLOW}{padded}{ANSI_RESET}")
            else:
                parts.append(padded)
        return "  │  ".join(parts)

    sep = "  ┼──".join("─" * w for w in col_widths)

    print()
    print(f"  {ANSI_BOLD}{ANSI_CYAN}Timing Profile Comparison{ANSI_RESET}")
    print(f"  {'─' * (sum(col_widths) + 5 * (len(COLS) - 1))}")
    print(f"  {_fmt_row([h for h, _ in COLS], header=True)}")
    print(f"  {sep}")
    for row in rows:
        print(f"  {_fmt_row(row)}")
    print()
    print(f"  {ANSI_DIM}All time values in milliseconds. Use --timing-profile <name> to select.{ANSI_RESET}")
    print()


def _apply_calibration_from_telemetry(
    cfg: AppConfig,
    *,
    persist: bool = False,
    summary_path: Path | str | None = None,
) -> bool:
    """Apply the latest telemetry calibration recommendation to the session, optionally saving it."""
    global PLAYBACK_SESSION, TIMING_POLICY, SLEEP_POLICY, TIMING_PROFILE_NAME, TEMPO_SCALE

    ANSI_RESET  = "\033[0m"
    ANSI_BOLD   = "\033[1m"
    ANSI_CYAN   = "\033[36m"
    ANSI_YELLOW = "\033[33m"
    ANSI_GREEN  = "\033[32m"
    ANSI_DIM    = "\033[2m"

    from sky_music.orchestration.calibration import (
        calibrate_profile,
        calibration_input_from_summary,
        load_telemetry_summary,
    )

    summary = load_telemetry_summary(summary_path)
    if summary is None:
        target = summary_path if summary_path is not None else "logs/"
        print(f"\n  {ANSI_YELLOW}No telemetry summary found at {target}.{ANSI_RESET}")
        print("  Run a playback with --debug-csv first to generate telemetry.")
        print()
        return False

    inp = calibration_input_from_summary(summary)
    rec = calibrate_profile(inp)
    base = PLAYBACK_SESSION or PlaybackSessionContext.balanced(
        tempo_scale=cfg.default_tempo_scale,
        fps=cfg.game_fps if cfg.game_fps > 0 else None,
    )
    updated = apply_recommendation_to_context(base, rec)
    if inp.fps > 0:
        updated = updated.with_fps(inp.fps)
    RUNTIME_STATE.apply_session(updated, cfg)
    RUNTIME_STATE.telemetry_csv_enabled = TELEMETRY_CSV_ENABLED
    RUNTIME_STATE.dry_run = DRY_RUN_MODE
    RUNTIME_STATE.verbose_hud = VERBOSE_HUD
    _sync_legacy_runtime_globals()

    print()
    print(f"  {ANSI_BOLD}{ANSI_CYAN}Applied calibration to session{ANSI_RESET}")
    print(f"    Profile     : {rec.profile_name}")
    print(f"    Tempo scale : {rec.tempo_scale:.2f}x")
    print(f"    Hold target : {rec.hold_us / 1000:.1f} ms ({ANSI_DIM}via FrameTimingPolicy{ANSI_RESET})")
    print(f"    Severity    : {rec.severity.upper()}")
    print(f"    Reason      : {rec.reason}")
    if persist:
        persist_calibration_defaults(
            cfg,
            profile_name=rec.profile_name,
            tempo_scale=rec.tempo_scale,
            fps=inp.fps,
        )
        print(f"  {ANSI_GREEN}Saved calibration defaults to config.json.{ANSI_RESET}")
    else:
        print(f"  {ANSI_GREEN}In-memory only — config.json not modified.{ANSI_RESET}")
    print()
    return True


def _run_auto_calibrate(summary_path: Path | str | None = None) -> None:
    """Read the most recent telemetry summary and print calibration recommendations."""
    ANSI_RESET  = "\033[0m"
    ANSI_BOLD   = "\033[1m"
    ANSI_CYAN   = "\033[36m"
    ANSI_YELLOW = "\033[33m"

    from sky_music.orchestration.calibration import (
        calibrate_profile,
        calibration_input_from_summary,
        load_telemetry_summary,
    )

    summary = load_telemetry_summary(summary_path)
    if summary is None:
        target = summary_path if summary_path is not None else "logs/"
        print(f"\n  {ANSI_YELLOW}No telemetry summary found at {target}.{ANSI_RESET}")
        print("  Run a playback with --debug-csv first to generate telemetry.")
        return

    print()
    label = str(summary_path) if summary_path is not None else "latest telemetry"
    print(f"  {ANSI_BOLD}{ANSI_CYAN}Auto-Calibrate — analysing: {label}{ANSI_RESET}")
    print()

    inp = calibration_input_from_summary(summary)
    rec = calibrate_profile(inp)
    lat = summary.get("lateness_us", {})
    dur = summary.get("send_duration_us", {})
    print(f"  Song          : {summary.get('song', 'unknown')}")
    print(f"  Profile used  : {inp.profile_name}")
    print(f"  FPS           : {inp.fps}")
    print(f"  p95 lateness  : {lat.get('p95_us', 0) / 1000:.1f} ms")
    print(f"  p99 lateness  : {lat.get('p99_us', 0) / 1000:.1f} ms")
    print(f"  p95 send      : {dur.get('p95_us', 0) / 1000:.1f} ms")
    print()
    print("  Calibration Recommendation:")
    print(f"    Suggested Profile : {rec.profile_name}")
    print(f"    Suggested Tempo   : {rec.tempo_scale:.2f}x")
    print(f"    Hold Target       : {rec.hold_us / 1000:.1f} ms")
    print(f"    Severity          : {rec.severity.upper()}")
    print(f"    Reason            : {rec.reason}")
    print()
    print()


@dataclass
class PlaybackOverrides:
    dry_run: bool = False
    profile: str | None = None
    tempo: float | None = None
    fps: int | None = None


@dataclass
class RuntimeSessionState:
    session: PlaybackSessionContext | None = None
    timing_policy: object | None = None
    sleep_policy: object | None = None
    scan_code_mode: str = "physical"
    telemetry_csv_enabled: bool = False
    dry_run: bool = False
    tempo_scale: float = 1.0
    timing_profile_name: str = "balanced"
    verbose_hud: bool = False
    use_dispatch_thread: bool = True
    enable_timer_guard: bool = True
    enable_waitable_timer: bool = True
    enable_gc_pause: bool = True

    def apply_session(self, session: PlaybackSessionContext, cfg: AppConfig, *, spin_threshold_us: int | None = None) -> None:
        self.session = session
        self.timing_policy = session.resolve_effective_policy(cfg)
        self.sleep_policy = session.resolve_sleep_policy(cfg, spin_threshold_us=spin_threshold_us)
        self.scan_code_mode = session.scan_code_mode
        self.tempo_scale = session.tempo_scale
        self.timing_profile_name = session.display_profile_label()


RUNTIME_STATE = RuntimeSessionState()


def _sync_legacy_runtime_globals() -> None:
    """Keep historical module globals in sync while runtime state is centralized."""
    global CURRENT_SCAN_CODE_MODE, TIMING_POLICY, SLEEP_POLICY, PLAYBACK_SESSION
    global TELEMETRY_CSV_ENABLED, DRY_RUN_MODE, TEMPO_SCALE, TIMING_PROFILE_NAME, VERBOSE_HUD
    global USE_DISPATCH_THREAD, ENABLE_TIMER_GUARD, ENABLE_WAITABLE_TIMER, ENABLE_GC_PAUSE

    CURRENT_SCAN_CODE_MODE = RUNTIME_STATE.scan_code_mode
    TIMING_POLICY = RUNTIME_STATE.timing_policy
    SLEEP_POLICY = RUNTIME_STATE.sleep_policy
    PLAYBACK_SESSION = RUNTIME_STATE.session
    TELEMETRY_CSV_ENABLED = RUNTIME_STATE.telemetry_csv_enabled
    DRY_RUN_MODE = RUNTIME_STATE.dry_run
    TEMPO_SCALE = RUNTIME_STATE.tempo_scale
    TIMING_PROFILE_NAME = RUNTIME_STATE.timing_profile_name
    VERBOSE_HUD = RUNTIME_STATE.verbose_hud
    USE_DISPATCH_THREAD = RUNTIME_STATE.use_dispatch_thread
    ENABLE_TIMER_GUARD = RUNTIME_STATE.enable_timer_guard
    ENABLE_WAITABLE_TIMER = RUNTIME_STATE.enable_waitable_timer
    ENABLE_GC_PAUSE = RUNTIME_STATE.enable_gc_pause

def play_selected_song(
    selected_song: Path,
    countdown_seconds: int,
    controls: PlaybackControls | None = None,
    overrides: PlaybackOverrides | None = None,
) -> str:
    from sky_music.domain.song_repository import get_shared_song_repository
    from sky_music.domain.scheduler import build_key_actions, ScheduleBuildError
    from sky_music.infrastructure.backend import WinSendInputBackend, DryRunBackend
    from sky_music.orchestration.engine import PlaybackEngine
    from sky_music.ui.hud import ProgressRenderer

    try:
        song = get_shared_song_repository().load(selected_song)
    except Exception as exc:
        print(f"Failed to parse song: {exc}")
        return PLAYBACK_QUIT

    # Extract overrides into a unified session context
    force_dry_run = overrides.dry_run if overrides else False
    force_profile = overrides.profile if overrides else None
    force_tempo = overrides.tempo if overrides else None
    force_fps = overrides.fps if overrides else None
    if force_profile is not None:
        force_profile = canonical_profile_name(force_profile)

    user_cfg = load_config()
    base_session = PLAYBACK_SESSION or PlaybackSessionContext.balanced(
        tempo_scale=TEMPO_SCALE,
        fps=user_cfg.game_fps if user_cfg.game_fps > 0 else None,
        scan_code_mode=CURRENT_SCAN_CODE_MODE,
    )
    session = merge_session_with_overrides(
        base_session,
        profile=force_profile,
        tempo=force_tempo,
        fps=force_fps,
    )

    is_dry_run = DRY_RUN_MODE or force_dry_run
    current_profile = session.display_profile_label()
    current_tempo = session.tempo_scale

    active_policy = session.resolve_effective_policy(user_cfg)
    active_sleep_policy = session.resolve_sleep_policy(user_cfg)

    # build_key_actions builds DefaultNoteResolver(profile) when resolver is None; that
    # single resolver now handles both physical and mapped scan-code modes.
    resolver = None

    def check_and_abort_violations(violations_tuple, is_dry_run_flag) -> bool:
        if not violations_tuple:
            return True
        fatal_violations = [v for v in violations_tuple if getattr(v, "severity", "fatal") == "fatal"]
        if fatal_violations and not is_dry_run_flag:
            print("\n[FATAL] Real Playback aborted due to severe schedule invariant violations:")
            for violation in fatal_violations:
                print(f"  - [{violation.code}] {violation.message}")
            print("  Please try choosing a safer Timing Profile or decrease --tempo-scale.")
            return False
        return True

    def build_schedule(session_ctx, policy, tempo):
        try:
            return build_key_actions(
                song,
                policy=policy,
                scan_code_mode=session_ctx.scan_code_mode,
                resolver=resolver,
                tempo_scale=tempo,
            )
        except ScheduleBuildError as exc:
            print(f"\n[FATAL] Schedule build failed: {exc}")
            if exc.recommended_tempo_scale is not None:
                print(f"  Try a slower tempo: --tempo-scale {exc.recommended_tempo_scale:.2f}")
            if exc.recommended_profile:
                print(f"  Or switch to a safer profile: --timing-profile {exc.recommended_profile}")
            return None

    sched_meta = build_schedule(session, active_policy, current_tempo)
    if sched_meta is None:
        return PLAYBACK_QUIT
    actions = sched_meta.actions

    # Run Schedule Invariant Validator
    from sky_music.domain.validation import validate_key_actions
    violations = validate_key_actions(actions, policy=active_policy)
    if violations:
        print("\n[Warning] Schedule Invariant Violations detected:")
        for violation in violations:
            print(f"  - [{violation.code}] {violation.message}")
        if not check_and_abort_violations(violations, is_dry_run):
            return PLAYBACK_QUIT

    # Pre-playback schedule risk analysis (advisory only — do NOT auto-apply)
    from sky_music.domain.analyzer import analyze_schedule
    report = analyze_schedule(sched_meta, raw_notes=song.notes)

    # If picker already decided (force_profile/tempo supplied), skip the prompt
    if report.severity != "low" and force_profile is None and force_tempo is None:
        should_prompt = True
        if report.severity == "medium" and not user_cfg.safety.prompt_on_medium_risk:
            should_prompt = False
        elif report.severity == "high" and not user_cfg.safety.prompt_on_high_risk:
            should_prompt = False

        if should_prompt:
            should_continue, new_profile, new_tempo = _handle_risk_analysis(
                report, song, is_dry_run, controls
            )
            if not should_continue:
                return PLAYBACK_QUIT
        else:
            # Still print the advisory report to the console for user awareness, but do not block
            print(f"\n[Advisory Warning] Playback risk is {report.severity.upper()}:")
            for rec in report.recommendations:
                print(f"  * {rec}")
            print("Proceeding automatically as configured by safety rules.\n")
            should_continue = True
            new_profile, new_tempo = None, None
        if new_profile is not None and canonical_profile_name(new_profile) != session.profile_name:
            session = session.with_profile(new_profile)
            active_policy = session.resolve_effective_policy(user_cfg)
            active_sleep_policy = session.resolve_sleep_policy(user_cfg)
            current_profile = session.display_profile_label()

            sched_meta = build_schedule(session, active_policy, current_tempo)
            if sched_meta is None:
                return PLAYBACK_QUIT
            actions = sched_meta.actions

            violations = validate_key_actions(actions, policy=active_policy)
            if violations:
                print("\n[Warning] Schedule Invariant Violations detected after profile change:")
                for violation in violations:
                    print(f"  - [{violation.code}] {violation.message}")
                if not check_and_abort_violations(violations, is_dry_run):
                    return PLAYBACK_QUIT
                    
        if new_tempo is not None:
            session = session.with_tempo(new_tempo)
            current_tempo = session.tempo_scale
            active_policy = session.resolve_effective_policy(user_cfg)
            sched_meta = build_schedule(session, active_policy, current_tempo)
            if sched_meta is None:
                return PLAYBACK_QUIT
            actions = sched_meta.actions

            violations = validate_key_actions(actions, policy=active_policy)
            if violations:
                print("\n[Warning] Schedule Invariant Violations detected after tempo change:")
                for violation in violations:
                    print(f"  - [{violation.code}] {violation.message}")
                if not check_and_abort_violations(violations, is_dry_run):
                    return PLAYBACK_QUIT

    # Preflight check and window readiness
    if not _mini_preflight(is_dry_run, profile=current_profile, tempo=current_tempo, controls=controls):
        return PLAYBACK_QUIT

    # Check window/readiness only if we are NOT running dry-run mode
    if not is_dry_run:
        countdown_before_playback(countdown_seconds)
    else:
        print(f"[simulation] DRY-RUN enabled. Simulating playback of {song.name}...")

    user_cfg = load_config()
    verbose_hud_mode = user_cfg.verbose_hud
    telemetry_enabled = TELEMETRY_CSV_ENABLED or user_cfg.telemetry_enabled_by_default or PLAYBACK_DEBUG or force_dry_run

    backend = DryRunBackend() if is_dry_run else WinSendInputBackend()
    renderer = ProgressRenderer(
        controls,
        verbose=verbose_hud_mode,
        profile_name=current_profile,
        tempo_scale=current_tempo,
    )
    renderer.active_policy = active_policy

    # Clear preflight/countdown output so the live HUD starts on a clean terminal.
    # ProgressRenderer only erases its own previously-rendered lines; static print()
    # output from _mini_preflight would otherwise remain visible above the HUD.
    clear_terminal()

    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=backend,
        controls=controls,
        renderer=renderer,
        telemetry_enabled=telemetry_enabled,
        require_focus=not is_dry_run,
        profile_name=current_profile,
        tempo_scale=current_tempo,
        sleep_policy=active_sleep_policy,
        focus_restore_grace_us=active_policy.focus_restore_grace_us,
        fps=getattr(active_policy, "fps", None),
        min_hold_us=int(active_policy.min_hold_us),
        same_key_conflict_policy=active_policy.same_key_conflict_policy,
        use_dispatch_thread=USE_DISPATCH_THREAD,
        input_path_warn_us=user_cfg.input_path_warn_us,
        enable_timer_guard=ENABLE_TIMER_GUARD,
        enable_waitable_timer=ENABLE_WAITABLE_TIMER,
        enable_gc_pause=ENABLE_GC_PAUSE,
    )
    engine.telemetry.record_schedule_metadata(sched_meta)
    result = engine.play()
    clear_terminal()
    return result


def build_arg_parser() -> argparse.ArgumentParser:
    hk = HotkeyDefaults()
    parser = argparse.ArgumentParser(
        description="Play Sky song files from the terminal.",
    )

    # ── Song Selection ────────────────────────────────────────────────────────
    sel = parser.add_argument_group("Song selection")
    sel.add_argument(
        "--song",
        help="play a song by number, exact name, partial name, or file path",
    )
    sel.add_argument(
        "--list",
        action="store_true",
        help="list available songs and exit",
    )
    sel.add_argument(
        "--songs-dir",
        type=Path,
        default=SONG_DIR,
        help="folder containing .json/.skysheet/.txt song files",
    )
    sel.add_argument(
        "--countdown",
        type=int,
        default=3,
        help="seconds to wait before playback starts (default: 3)",
    )
    sel.add_argument(
        "--repeat",
        type=int,
        default=1,
    )
    # ── Playback Timing ───────────────────────────────────────────────────────
    timing = parser.add_argument_group("Playback timing")
    timing.add_argument(
        "--timing-profile",
        choices=list(CLI_PROFILE_NAMES),
        default="balanced",
        help=(
            "Timing profile: "
            "local-precise (low latency), "
            "audience-safe (online play / audience audibility), "
            "balanced (default)"
        ),
    )
    timing.add_argument(
        "--tempo-scale",
        type=float,
        default=1.0,
        help="Scale playback tempo: 1.2 = 20%% faster, 0.8 = 20%% slower (default: 1.0)",
    )
    timing.add_argument(
        "--hold-ms",
        type=float,
        help="Override key hold duration in ms (overrides profile)",
    )
    timing.add_argument(
        "--min-hold-ms",
        type=float,
        help="Override minimum key hold duration in ms (overrides profile)",
    )
    timing.add_argument(
        "--spin-threshold-us",
        type=int,
        help="Override CPU spin threshold in microseconds (precise=800, balanced=500, battery_safe=200/0) (overrides profile)",
    )
    timing.add_argument(
        "--focus-restore-grace-ms",
        type=float,
        help="Override focus restoration grace period in ms (precise=50, balanced=100, remote/safe=150-200) (overrides profile)",
    )
    timing.add_argument(
        "--scan-code-mode",
        choices=["physical", "mapped"],
        default="physical",
        help="physical = fixed QWERTY scan codes (default), mapped = OS keyboard layout",
    )
    timing.add_argument(
        "--same-key-conflict-policy",
        choices=["degraded", "strict"],
        help="degraded = warn and compress timing (default), strict = reject and abort playback",
    )
    timing.add_argument(
        "--fps",
        type=int,
        default=None,
        metavar="FPS",
        help=(
            "Game frame rate hint for frame-aware timing (e.g. 30, 60, 120). "
            "Scales hold timing via FrameTimingPolicy."
        ),
    )

    # ── Runtime Controls ──────────────────────────────────────────────────────
    ctrl = parser.add_argument_group("Runtime controls (hotkeys during playback)")
    ctrl.add_argument(
        "--pause-key",
        default=hk.pause,
        help="pause/resume hotkey, e.g. f8 or ctrl+p (default: f8)",
    )
    ctrl.add_argument(
        "--skip-key",
        default=hk.skip,
        help="skip current song hotkey (default: f9)",
    )
    ctrl.add_argument(
        "--quit-key",
        default=hk.quit,
        help="quit playback hotkey (default: f10; Esc not recommended — game may intercept it)",
    )
    ctrl.add_argument(
        "--refocus-key",
        default=hk.refocus,
        help="bring Sky window to foreground hotkey (default: f6)",
    )
    ctrl.add_argument(
        "--panic-key",
        default=hk.panic,
        help="emergency release all keys without stopping playback (default: ctrl+alt+backspace)",
    )
    ctrl.add_argument(
        "--disable-hotkeys",
        action="store_true",
        help="disable all runtime hotkeys; use Ctrl+C only",
    )
    ctrl.add_argument(
        "--allow-note-hotkeys",
        action="store_true",
        help="allow hotkeys that overlap with note keys (not recommended)",
    )
    ctrl.add_argument(
        "--no-dispatch-thread",
        action="store_true",
        help="run playback on the legacy single-thread dispatch path for debugging",
    )
    ctrl.add_argument(
        "--no-timer-guard",
        action="store_true",
        help="debug only: do not assert the 1ms timer-resolution guard in the dispatch thread",
    )
    ctrl.add_argument(
        "--no-waitable-timer",
        action="store_true",
        help="debug only: use the sleep/yield/spin sleeper instead of the high-resolution waitable timer",
    )
    ctrl.add_argument(
        "--no-gc-pause",
        action="store_true",
        help="debug only: do not collect and pause cyclic GC during playback",
    )

    # ── Safety & Diagnostics ──────────────────────────────────────────────────
    diag = parser.add_argument_group("Safety and diagnostics")
    diag.add_argument(
        "--doctor",
        action="store_true",
        help="run full readiness check (Sky window, timers, layout, key conflicts)",
    )
    diag.add_argument(
        "--doctor-timing",
        action="store_true",
        help="check high-precision multimedia timer subsystem only",
    )
    diag.add_argument(
        "--doctor-input",
        action="store_true",
        help="check keyboard layout mapping and physically held note keys only",
    )
    diag.add_argument(
        "--selftest-textual",
        action="store_true",
        help=argparse.SUPPRESS,
    )
    diag.add_argument(
        "--sky-process-names",
        default=sky_process_names_csv(),
        help="comma-separated Sky executable names to match (default: Sky.exe,...)",
    )
    diag.add_argument(
        "--allow-title-fallback",
        action="store_true",
        help="allow window title matching when process verification fails",
    )
    diag.add_argument(
        "--compare-profiles",
        action="store_true",
        help="print a side-by-side timing comparison table of all profiles and exit",
    )

    # ── Telemetry ─────────────────────────────────────────────────────────────
    telem = parser.add_argument_group("Telemetry")
    telem.add_argument(
        "--debug-csv",
        action="store_true",
        help="write per-event timing CSV + summary JSON to logs/ after each playback",
    )
    telem.add_argument(
        "--debug-playback",
        action="store_true",
        help="write verbose playback debug log to logs/",
    )
    telem.add_argument(
        "--dry-run",
        action="store_true",
        help="simulate playback in memory without sending any keystrokes (timing diagnosis)",
    )
    telem.add_argument(
        "--inspect-telemetry",
        help="read and summarize telemetry from a .summary.json file or logs/ directory and exit",
    )
    telem.add_argument(
        "--auto-calibrate",
        action="store_true",
        help=(
            "analyse the most recent telemetry log and print calibration recommendations "
            "(profile adjustments, tempo suggestions). Does NOT modify config.json automatically."
        ),
    )
    telem.add_argument(
        "--calibration-summary",
        type=Path,
        help=(
            "specific telemetry .summary.json, .csv, or logs directory to use for "
            "--auto-calibrate, --apply-calibration, and --save-calibration"
        ),
    )
    telem.add_argument(
        "--apply-calibration",
        action="store_true",
        help=(
            "apply calibration recommendations from the latest telemetry summary to the "
            "in-memory playback session (does not save config.json)."
        ),
    )
    telem.add_argument(
        "--save-calibration",
        action="store_true",
        help=(
            "apply calibration recommendations from the latest telemetry summary and "
            "persist profile, tempo, and FPS defaults to config.json."
        ),
    )

    # ── Display ───────────────────────────────────────────────────────────────
    disp = parser.add_argument_group("Display")
    disp.add_argument(
        "--theme",
        choices=["aurora", "minimalist", "slate", "cyberpunk", "classic"],
        default=None,
        help="song picker TUI theme (default: saved or aurora)",
    )
    disp.add_argument(
        "--no-clear",
        action="store_true",
        help="do not clear the terminal between songs",
    )
    disp.add_argument(
        "--verbose-hud",
        action="store_true",
        help="show detailed live timing/backend stats during playback (2-line HUD)",
    )


    return parser

def configure_from_args(args: argparse.Namespace, cfg: AppConfig | None = None) -> None:
    global PLAYBACK_DEBUG, DEBUG_LOG_PATH
    from sky_music.platform.win32 import inputs
    from sky_music.ui import picker as songs

    cfg = cfg or load_config()

    songs.SONG_DIR = args.songs_dir
    PLAYBACK_DEBUG = args.debug_playback
    inputs.PLAYBACK_DEBUG = args.debug_playback
    RUNTIME_STATE.telemetry_csv_enabled = args.debug_csv
    RUNTIME_STATE.dry_run = args.dry_run
    RUNTIME_STATE.tempo_scale = args.tempo_scale
    RUNTIME_STATE.scan_code_mode = args.scan_code_mode
    if RUNTIME_STATE.tempo_scale <= 0:
        raise ValueError("tempo_scale must be > 0")
    RUNTIME_STATE.verbose_hud = args.verbose_hud
    RUNTIME_STATE.use_dispatch_thread = not args.no_dispatch_thread
    RUNTIME_STATE.enable_timer_guard = not args.no_timer_guard
    RUNTIME_STATE.enable_waitable_timer = not args.no_waitable_timer
    RUNTIME_STATE.enable_gc_pause = not args.no_gc_pause

    if PLAYBACK_DEBUG:
        init_debug_log()

    session = PlaybackSessionContext.from_cli_args(args, cfg)
    spin_override = getattr(args, "spin_threshold_us", None)
    RUNTIME_STATE.apply_session(session, cfg, spin_threshold_us=spin_override)
    _sync_legacy_runtime_globals()

    if args.sky_process_names:
        inputs.EXPECTED_PROCESS_NAMES = {
            name.strip()
            for name in args.sky_process_names.split(",")
            if name.strip()
        }

    inputs.ALLOW_TITLE_FALLBACK = bool(args.allow_title_fallback)
    if args.theme is not None:
        songs.ACTIVE_THEME = args.theme
        songs.save_theme(args.theme)

def _check_textual_support() -> str | None:
    """Return a human-readable failure reason if Textual cannot run, or None if it can."""
    if not sys.stdout.isatty():
        return (
            "stdout không phải terminal tương tác (isatty = False). "
            "Hãy chạy Sky Player trực tiếp trong một cửa sổ terminal, "
            "không phải qua pipe hay redirect."
        )
    if sys.platform != "win32":
        return None  # non-Windows terminals generally support Textual
    # For frozen exes (double-click), WT_SESSION is absent even in Windows Terminal.
    # Check ENABLE_VIRTUAL_TERMINAL_PROCESSING via Win32 API instead.
    if getattr(sys, "frozen", False):
        try:
            import ctypes
            import ctypes.wintypes
            kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
            ENABLE_VIRTUAL_TERMINAL_PROCESSING = 0x0004
            STD_OUTPUT_HANDLE = ctypes.c_ulong(-11)
            handle = kernel32.GetStdHandle(STD_OUTPUT_HANDLE)
            mode = ctypes.wintypes.DWORD()
            if kernel32.GetConsoleMode(handle, ctypes.byref(mode)):
                if not (mode.value & ENABLE_VIRTUAL_TERMINAL_PROCESSING):
                    return (
                        "Console hiện tại không hỗ trợ ANSI / Virtual Terminal Processing. "
                        "Hãy mở ứng dụng từ Windows Terminal (wt.exe) hoặc "
                        "bật 'Let Windows decide' trong Settings > System > For developers > Terminal."
                    )
        except Exception:
            pass  # cannot query — assume capable
        return None
    # Dev/source mode: require WT_SESSION or TERM_PROGRAM
    if os.environ.get("WT_SESSION"):
        return None
    if os.environ.get("TERM_PROGRAM") == "vscode":
        return None
    return (
        "Terminal không được nhận diện là hỗ trợ Textual trên Windows. "
        "Chạy từ Windows Terminal hoặc VS Code terminal, "
        "hoặc dùng flag --ui textual để bỏ qua kiểm tra này."
    )

def _run_textual_selftest() -> int:
    """Headless frozen-exe smoke test for Textual picker packaging."""
    import asyncio

    try:
        from rapidfuzz import fuzz
        from sky_music.ui.textual_app import app as app_module
        from sky_music.ui.textual_app.app import SkyPickerApp
    except Exception as exc:
        print(f"Textual selftest import failed: {exc}", file=sys.stderr)
        return 1

    class SelftestMetadataCoordinator:
        def __init__(self, *_args: object, **_kwargs: object) -> None:
            self.closed = False

        @property
        def name(self) -> str:
            return "selftest-metadata"

        @property
        def phase(self) -> str:
            return "picker"

        def refresh(self, _paths: list[Path]) -> None:
            return

        def cancel(self) -> None:
            pass

        def close(self, *, wait: bool = False) -> None:
            self.closed = True

        def snapshot(self) -> object:
            from sky_music.infrastructure.background import WorkerSnapshot
            return WorkerSnapshot(
                name=self.name,
                phase=self.phase,
                closed=self.closed,
                state="closed" if self.closed else "open",
                pending_count=0,
                running_count=0,
            )

    async def run_picker_probe() -> None:
        original_get_song_choices = app_module.get_song_choices
        original_metadata = app_module.MetadataCoordinator
        app_module.get_song_choices = lambda force_refresh=False: [
            Path("songs/Diamonds.json"),
            Path("songs/Dandelions.json"),
        ]
        app_module.MetadataCoordinator = SelftestMetadataCoordinator
        try:
            app = SkyPickerApp(theme_name="aurora")
            async with app.run_test(size=(100, 30)) as pilot:
                await pilot.pause()
                table = app.query_one("#songs")
                if getattr(table, "row_count", 0) != 2:
                    raise RuntimeError("Textual picker table did not render selftest rows")
                if not app.screen.has_class("theme-aurora"):
                    raise RuntimeError("Textual picker did not apply the active theme class")
                await pilot.press("escape")
            if app.return_value is not None:
                raise RuntimeError("Textual picker selftest did not exit cleanly")
        finally:
            app_module.get_song_choices = original_get_song_choices
            app_module.MetadataCoordinator = original_metadata

    try:
        score = fuzz.WRatio("diamonds", "dimonds")
        if score <= 0:
            raise RuntimeError("rapidfuzz returned an invalid selftest score")
        asyncio.run(run_picker_probe())
    except Exception as exc:
        print(f"Textual selftest failed: {exc}", file=sys.stderr)
        return 1

    print("Textual selftest OK: rapidfuzz imported and SkyPickerApp mounted headlessly.")
    return 0

def build_playback_controls(args: argparse.Namespace) -> PlaybackControls:
    if args.disable_hotkeys:
        return PlaybackControls(
            pause=parse_hotkey(args.pause_key),
            skip=parse_hotkey(args.skip_key),
            quit=parse_hotkey(args.quit_key),
            refocus=parse_hotkey(args.refocus_key),
            panic=parse_hotkey(args.panic_key),
            enabled=False,
        )

    controls = PlaybackControls(
        pause=parse_hotkey(args.pause_key),
        skip=parse_hotkey(args.skip_key),
        quit=parse_hotkey(args.quit_key),
        refocus=parse_hotkey(args.refocus_key),
        panic=parse_hotkey(args.panic_key),
    )

    conflicting = [
        ("pause", controls.pause),
        ("skip", controls.skip),
        ("quit", controls.quit),
        ("refocus", controls.refocus),
        # panic always has modifiers, no need to check note conflicts
    ]
    unsafe = [f"{name}={hotkey.display}" for name, hotkey in conflicting if hotkey_conflicts_with_note_keys(hotkey)]
    if unsafe and not args.allow_note_hotkeys:
        raise ValueError(
            "Hotkey overlaps with note keys: "
            + ", ".join(unsafe)
            + ". Use Ctrl/Alt/Shift, a function key, or pass --allow-note-hotkeys if you accept the risk."
        )
    return controls

def _wait_key_and_exit(code: int = 1) -> None:
    """Print a 'press any key' prompt then exit — keeps the console window open
    when the exe is launched by double-click so the user can read the error."""
    print("\nNhấn phím bất kỳ để thoát...", file=sys.stderr, flush=True)
    try:
        if sys.platform == "win32":
            import msvcrt
            msvcrt.getch()
        else:
            input()
    except Exception:
        pass
    sys.exit(code)


def prompt_song_selection(
    profile: str = "balanced",
    tempo: float = 1.0,
    dry_run: bool = False,
    fps: int | None = None,
    scan_code_mode: str = "physical",
) -> "SongPickerResult | None":
    from sky_music.ui import picker as songs
    session = merge_session_with_overrides(
        PLAYBACK_SESSION or PlaybackSessionContext.balanced(
            tempo_scale=tempo,
            fps=fps,
            scan_code_mode=scan_code_mode,
        ),
        profile=profile,
        tempo=tempo,
        fps=fps,
    )
    unsupported_reason = _check_textual_support()
    if unsupported_reason is not None:
        print(
            "\n"
            "╔══════════════════════════════════════════════════════════════╗\n"
            "║          Sky Player — Yêu cầu hệ thống không đáp ứng        ║\n"
            "╠══════════════════════════════════════════════════════════════╣\n"
            f"║  {unsupported_reason[:62]:<62}║\n",
            file=sys.stderr,
            end="",
        )
        # Word-wrap the reason across multiple rows if it's long
        remaining = unsupported_reason[62:]
        while remaining:
            chunk, remaining = remaining[:62], remaining[62:]
            print(f"║  {chunk:<62}║", file=sys.stderr)
        print(
            "╚══════════════════════════════════════════════════════════════╝\n",
            file=sys.stderr,
        )
        _wait_key_and_exit(1)

    try:
        from sky_music.ui.textual_app import choose_song_interactively_textual
        return choose_song_interactively_textual(
            theme_name=songs.ACTIVE_THEME,
            initial_profile=session.profile_name,
            initial_tempo=session.tempo_scale,
            initial_fps=session.fps,
            initial_dry_run=dry_run,
            scan_code_mode=session.scan_code_mode,
        )
    except ImportError as exc:
        print(
            "\n"
            "╔══════════════════════════════════════════════════════════════╗\n"
            "║        Sky Player — Lỗi tải Textual UI                      ║\n"
            "╠══════════════════════════════════════════════════════════════╣\n"
            f"║  Module bị thiếu: {str(exc)[:44]:<44}          ║\n"
            "║  Đây là lỗi đóng gói. Hãy báo cáo lỗi này.                ║\n"
            "╚══════════════════════════════════════════════════════════════╝\n",
            file=sys.stderr,
        )
        _wait_key_and_exit(2)
    except Exception as exc:
        print(
            "\n"
            "╔══════════════════════════════════════════════════════════════╗\n"
            "║        Sky Player — Textual UI gặp lỗi nghiêm trọng         ║\n"
            "╠══════════════════════════════════════════════════════════════╣\n"
            f"║  {str(exc)[:62]:<62}║\n"
            "╚══════════════════════════════════════════════════════════════╝\n",
            file=sys.stderr,
        )
        _wait_key_and_exit(2)

def print_choices_local(song_choices: list[Path]) -> None:
    if not song_choices:
        print(f"No songs found in: {SONG_DIR.resolve()}")
        print(f"Supported extensions: {', '.join(sorted(SUPPORTED_EXTENSIONS))}")
        return
    print("Songs:")
    for index, path in enumerate(song_choices, start=1):
        print(f"  {index:>2}) {path.stem}")

def main() -> int:
    if sys.platform == 'win32':
        try:
            sys.stdout.reconfigure(encoding='utf-8')
            sys.stderr.reconfigure(encoding='utf-8')
        except Exception:
            pass

    user_cfg = load_config()
    parser = build_arg_parser()
    args = parser.parse_args()

    if getattr(args, "selftest_textual", False):
        return _run_textual_selftest()

    apply_config_defaults(args, user_cfg)
    configure_from_args(args, user_cfg)
    try:
        controls = build_playback_controls(args)
    except ValueError as exc:
        parser.error(str(exc))

    if args.inspect_telemetry is not None:
        from sky_music.orchestration.telemetry import inspect_telemetry_report
        inspect_telemetry_report(args.inspect_telemetry)
        return 0

    if getattr(args, "compare_profiles", False):
        _print_profile_comparison_table(user_cfg)
        return 0

    if getattr(args, "apply_calibration", False) or getattr(args, "save_calibration", False):
        if not _apply_calibration_from_telemetry(
            user_cfg,
            persist=bool(getattr(args, "save_calibration", False)),
            summary_path=getattr(args, "calibration_summary", None),
        ):
            return 1

    if getattr(args, "auto_calibrate", False):
        _run_auto_calibrate(getattr(args, "calibration_summary", None))
        return 0

    if args.doctor or args.doctor_timing or args.doctor_input:
        import sky_music.infrastructure.doctor as doctor
        if args.doctor:
            doctor.run_all_doctor_checks()
        elif args.doctor_timing:
            print("=" * 60)
            print("         SKY MUSIC PLAYER — TIMING CHECK")
            print("=" * 60)
            diag = doctor.check_timer_resolution()
            print(f"Status: {'OK' if diag['ok'] else 'FAILED'}\nDetails: {diag['msg']}")
            print("=" * 60)
        elif args.doctor_input:
            print("=" * 60)
            print("         SKY MUSIC PLAYER — INPUT CHECK")
            print("=" * 60)
            kb_diag = doctor.check_keyboard_layout()
            conflict_diag = doctor.check_physical_keys_held()
            print(f"Layout Mapping : {'OK' if kb_diag['ok'] else 'FAILED'} - {kb_diag['msg']}")
            print(f"Key Conflicts  : {'OK' if conflict_diag['ok'] else 'WARNING'} - {conflict_diag['msg']}")
            print("=" * 60)
        return 0

    song_choices = get_song_choices(force_refresh=True)

    if args.list:
        print_choices_local(song_choices)
        return 0

    if not song_choices and args.song is None:
        print_choices_local(song_choices)
        return 1

    try:
        enable_high_precision_timers()

        if args.song is not None:
            selected_song = resolve_song_selection(args.song, song_choices)
            if selected_song is None:
                return 2

            repeat_count = max(args.repeat, 1)
            for run_index in range(repeat_count):
                if repeat_count > 1:
                    print(f"Run {run_index + 1}/{repeat_count}: {selected_song.stem}")
                if not args.no_clear:
                    clear_terminal()
                result = play_selected_song(
                    selected_song,
                    args.countdown,
                    controls=controls,
                    overrides=PlaybackOverrides(dry_run=DRY_RUN_MODE),
                )
                if result == PLAYBACK_QUIT:
                    return 0
                if result == PLAYBACK_SKIPPED:
                    return 0
            return 0

        while True:
            # Resolve initial FPS prioritizing active CLI overrides, then persistent config defaults
            cli_fps_explicit = any(arg.startswith("--fps") for arg in sys.argv)
            resolved_fps = args.fps if cli_fps_explicit else (user_cfg.game_fps if user_cfg.game_fps > 0 else None)

            try:
                picker_result = prompt_song_selection(
                    profile=canonical_profile_name(user_cfg.default_timing_profile),
                    tempo=TEMPO_SCALE,
                    dry_run=DRY_RUN_MODE,
                    fps=resolved_fps,
                    scan_code_mode=CURRENT_SCAN_CODE_MODE,
                )
            except Exception as exc:
                print(f"\n[ERROR] Playback aborted due to background worker cleanup failure: {exc}")
                return 1
            if picker_result is None:
                return 0

            if not args.no_clear:
                clear_terminal()

            force_dry = (picker_result.action == "dry_run")
            result = play_selected_song(
                picker_result.song_path,
                args.countdown,
                controls=controls,
                overrides=PlaybackOverrides(
                    dry_run=force_dry,
                    profile=picker_result.profile_name,
                    tempo=picker_result.tempo_scale,
                    fps=picker_result.fps,
                )
            )
            if result == PLAYBACK_QUIT:
                return 0
            
            # P0 Fix: Update persistent loop state with picker decision
            # (Allows picker changes to persist across multiple songs)
            updated_session = merge_session_with_overrides(
                RUNTIME_STATE.session or PLAYBACK_SESSION or PlaybackSessionContext.balanced(
                    tempo_scale=RUNTIME_STATE.tempo_scale,
                    scan_code_mode=RUNTIME_STATE.scan_code_mode,
                ),
                profile=picker_result.profile_name,
                tempo=picker_result.tempo_scale,
                fps=picker_result.fps,
            )
            RUNTIME_STATE.apply_session(updated_session, user_cfg)
            RUNTIME_STATE.dry_run = (picker_result.action == "dry_run")
            _sync_legacy_runtime_globals()

            persist_playback_defaults(
                user_cfg,
                profile_name=updated_session.profile_name,
                tempo_scale=updated_session.tempo_scale,
                fps=picker_result.fps,
            )

            if result == PLAYBACK_SKIPPED:
                time.sleep(0.5)
            else:
                time.sleep(2)

            if not args.no_clear:
                clear_terminal()

    except KeyboardInterrupt:
        print("\nStopped by user.")
        return 130
    finally:
        disable_high_precision_timers()

if __name__ == '__main__':
    # Required for safe ProcessPoolExecutor startup on Windows and harmless
    # for normal `uv run python src/main.py` execution.
    try:
        import multiprocessing
        multiprocessing.freeze_support()
    except Exception:
        pass
    raise SystemExit(main())
