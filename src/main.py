import argparse
import sys
import time
from pathlib import Path

# Import từ các mô-đun chuyên biệt
from sky_music.platform.win32 import inputs
from sky_music.config import load_config, apply_config_defaults
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
from sky_music.ui.picker import (
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
TELEMETRY_CSV_ENABLED = False
DRY_RUN_MODE = False
TEMPO_SCALE = 1.0
TIMING_PROFILE_NAME = "balanced"
VERBOSE_HUD = False

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

def _recommended_profile(severity: str, has_same_key_repeats: bool, high_polyphony: bool) -> str:
    """Suggest a timing profile name based on risk characteristics."""
    if severity == "high":
        if has_same_key_repeats:
            return "dense-safe"
        return "remote-safe"
    if severity == "medium":
        if high_polyphony:
            return "remote-safe"
        return "balanced"
    return "balanced"


def _handle_risk_analysis(report, song, is_dry_run: bool, controls, policy_override_fn=None) -> tuple[bool, str | None, float | None]:
    """Display risk analysis, prompt user for action if severity is medium/high.

    Returns (should_continue, new_profile_name_or_None, new_tempo_scale_or_None).
    """

    severity = report.severity.upper()
    recommended = _recommended_profile(
        report.severity,
        has_same_key_repeats=report.impossible_same_key_repeats > 0,
        high_polyphony=report.max_polyphony >= 5,
    )

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
    
    # Standardize width dynamic determination with songs.py and ui.py
    import shutil
    terminal_width = shutil.get_terminal_size((80, 24)).columns
    width = max(60, min(80, terminal_width))

    def pad_line(content: str, w: int) -> str:
        import re
        clean = re.sub(r'\033\[[0-9;]*m', '', content)
        cur_len = len(clean)
        if cur_len < w:
            return content + " " * (w - cur_len)
        return content

    def print_ansi_box(title: str, lines: list[str], border_color: str = ANSI_CYAN) -> None:
        top_left = "╭"
        top_right = "╮"
        bottom_left = "╰"
        bottom_right = "╯"
        horiz = "─"
        vert = "│"
        
        title_part = f"{horiz} {title} "
        top_line = f"{border_color}{top_left}{title_part}{horiz * (width - len(title_part) - 2)}{top_right}{ANSI_RESET}"
        bottom_line = f"{border_color}{bottom_left}{horiz * (width - 2)}{bottom_right}{ANSI_RESET}"
        
        print(top_line)
        for line in lines:
            padded = pad_line(line, width - 4)
            print(f"{border_color}{vert}{ANSI_RESET} {padded} {border_color}{vert}{ANSI_RESET}")
        print(bottom_line)

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
    checks.append((keys["ok"], "No note keys held" if keys["ok"] else f"Keys held: {', '.join(keys.get('held_keys', []))}"))

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
            f"{ANSI_BOLD}{controls.pause.display}{ANSI_RESET} pause/resume │ "
            f"{ANSI_BOLD}{controls.skip.display}{ANSI_RESET} skip │ "
            f"{ANSI_BOLD}{controls.quit.display}{ANSI_RESET} quit │ "
            f"{ANSI_BOLD}{controls.refocus.display}{ANSI_RESET} refocus │ "
            f"{ANSI_BOLD}{controls.panic.display}{ANSI_RESET} panic"
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


def _print_post_run_report(engine, profile_name: str, tempo_scale: float) -> None:
    """Print a readable post-run timing/backend summary from the engine's telemetry."""
    summary = engine.telemetry.get_summary()
    if not summary:
        return

    lat = summary.get("lateness_us", {})
    snd = summary.get("send_duration_us", {})
    hld = summary.get("note_hold_duration_us", {})
    bk = summary.get("backend", {})
    total = summary.get("total_events", 0)

    print()
    print("  ┌─ Post-run Report " + "─" * 41)
    print(f"  │  Profile: {profile_name}  |  Tempo: {tempo_scale:.2f}x  |  Events: {total}")
    print(f"  │  Lateness     p50={lat.get('p50_us',0):.0f}µs  p95={lat.get('p95_us',0):.0f}µs  p99={lat.get('p99_us',0):.0f}µs  max={lat.get('max_us',0):.0f}µs")
    print(f"  │  Late >2ms={lat.get('over_2ms',0)}  >5ms={lat.get('over_5ms',0)}  >10ms={lat.get('over_10ms',0)}")
    print(f"  │  Send dur    p95={snd.get('p95_us',0):.0f}µs  |  Hold avg={hld.get('avg_us',0)/1000:.1f}ms")
    failures = bk.get("panic_release_failures", 0)
    stuck = bk.get("failed_release_keys_final", [])
    if failures or stuck:
        print(f"  │  Backend: ⚠ {failures} panic failure(s), stuck keys: {stuck}")
    else:
        print( "  │  Backend: healthy (✓ no stuck keys)")

    # Advanced Calibration Recommendations based on Telemetry Feedback Loop
    p99 = lat.get("p99_us", 0)
    p95_send = snd.get("p95_us", 0)
    over_10ms = lat.get("over_10ms", 0)
    
    if failures or stuck:
        print( "  │  ⚠ Recommendation: Stuck keys detected — run --doctor before next playback.")
    else:
        # Check high keyboard injection latency (UIPI / OS throttling)
        if p95_send > 1500:
            print(f"  │  ⚠ Warning: High input injection delay (p95_send={p95_send/1000:.1f}ms).")
            print( "  │  Recommend: Run Sky Player as Administrator or close background overlays.")
            
        # Timing recommendations based on latency percentiles
        if over_10ms > 5 or p99 > 15000:
            print(f"  │  Recommendation: Severe latency jitter detected (p99={p99/1000:.1f}ms).")
            print( "  │  Try: --timing-profile low_fps_30 or remote_safe, check CPU load.")
        elif over_10ms > 0 or p99 > 8000:
            print(f"  │  Recommendation: Moderate latency (p99={p99/1000:.1f}ms).")
            print( "  │  Try: --timing-profile balanced_60fps, or reduce --tempo-scale slightly.")
        elif p99 < 3000:
            print(f"  │  Timing quality: excellent ✓ (p99={p99/1000:.1f}ms)")
            print( "  │  Try: --timing-profile balanced_120fps or local_precise for higher timing accuracy.")
        else:
            print( "  │  Timing quality: good ✓")
            
    print(  "  └" + "─" * 57)
    print()


from dataclasses import dataclass

@dataclass
class PlaybackOverrides:
    dry_run: bool = False
    profile: str | None = None
    tempo: float | None = None
    fps: int | None = None

def play_selected_song(
    selected_song: Path,
    countdown_seconds: int,
    controls: PlaybackControls | None = None,
    overrides: PlaybackOverrides | None = None,
) -> str:
    from sky_music.domain.parser import parse_song_file
    from sky_music.domain.scheduler import build_key_actions
    from sky_music.infrastructure.backend import WinSendInputBackend, DryRunBackend
    from sky_music.orchestration.engine import PlaybackEngine
    from sky_music.ui.hud import ProgressRenderer

    try:
        song = parse_song_file(selected_song)
    except Exception as exc:
        print(f"Failed to parse song: {exc}")
        return PLAYBACK_QUIT

    # Extract overrides
    force_dry_run = overrides.dry_run if overrides else False
    force_profile = overrides.profile if overrides else None
    force_tempo   = overrides.tempo   if overrides else None
    force_fps     = overrides.fps     if overrides else None

    is_dry_run = DRY_RUN_MODE or force_dry_run
    # Picker decision overrides global config (advisory-only flow)
    current_profile = force_profile if force_profile is not None else TIMING_PROFILE_NAME
    current_tempo   = force_tempo   if force_tempo   is not None else TEMPO_SCALE

    # Build scheduled actions using specified TimingPolicy and selected mode
    from sky_music.domain.scheduler_types import TimingPolicy
    from sky_music.infrastructure.timing import SleepPolicy
    active_policy = TIMING_POLICY
    active_sleep_policy = SLEEP_POLICY
    
    if force_profile is not None:
        active_policy = TimingPolicy.from_profile_name(force_profile)
        # Look up spin_threshold_us for the forced profile
        profile_key = force_profile.lower().replace("-", "_")
        user_cfg = load_config()
        if profile_key in user_cfg.timing_profiles:
            spin_us = user_cfg.timing_profiles[profile_key].get("spin_threshold_us", 500)
        else:
            from sky_music.config import DEFAULT_TIMING_PROFILES
            spin_us = DEFAULT_TIMING_PROFILES.get(profile_key, {}).get("spin_threshold_us", 500)
        active_sleep_policy = SleepPolicy(
            spin_threshold_us=spin_us,
            poll_s=0.025
        )

    if force_fps is not None and force_fps > 0:
        from sky_music.domain.scheduler_types import FrameTimingPolicy
        active_policy = FrameTimingPolicy.from_timing_policy(
            active_policy,
            fps=force_fps,
            same_key_conflict_policy=active_policy.same_key_conflict_policy
        )

    import sys
    resolver = None
    if sys.platform == "win32":
        from sky_music.platform.win32.keycodes import Win32NoteResolver
        from sky_music.layouts import SKY_15_KEY_PROFILE
        resolver = Win32NoteResolver(SKY_15_KEY_PROFILE)

    def check_and_abort_violations(violations_tuple, is_dry_run_flag) -> bool:
        if not violations_tuple:
            return True
        fatal_violations = [v for v in violations_tuple if v.code in ("negative_timestamp", "duplicate_down", "stuck_keys")]
        if fatal_violations and not is_dry_run_flag:
            print("\n[FATAL] Real Playback aborted due to severe schedule invariant violations:")
            for violation in fatal_violations:
                print(f"  - [{violation.code}] {violation.message}")
            print("  Please try choosing a safer Timing Profile or decrease --tempo-scale.")
            return False
        return True

    sched_meta = build_key_actions(
        song, policy=active_policy, scan_code_mode=CURRENT_SCAN_CODE_MODE,
        resolver=resolver, tempo_scale=current_tempo
    )
    actions = sched_meta.actions

    # Run Schedule Invariant Validator
    from sky_music.domain.validation import validate_key_actions
    violations = validate_key_actions(actions)
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
        user_cfg = load_config()
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
        if new_profile is not None and new_profile != current_profile:
            # Rebuild schedule with the switched profile
            new_policy = TimingPolicy.from_profile_name(new_profile)
            active_policy = new_policy
            
            # Switch SleepPolicy corresponding to the dynamically chosen profile
            profile_key = new_profile.lower().replace("-", "_")
            user_cfg = load_config()
            if profile_key in user_cfg.timing_profiles:
                new_spin = user_cfg.timing_profiles[profile_key].get("spin_threshold_us", 500)
            else:
                from sky_music.config import DEFAULT_TIMING_PROFILES
                new_spin = DEFAULT_TIMING_PROFILES.get(profile_key, {}).get("spin_threshold_us", 500)
            active_sleep_policy = SleepPolicy(
                spin_threshold_us=new_spin,
                poll_s=0.025
            )
            
            sched_meta = build_key_actions(
                song, policy=new_policy, scan_code_mode=CURRENT_SCAN_CODE_MODE,
                resolver=resolver, tempo_scale=current_tempo
            )
            actions = sched_meta.actions
            current_profile = new_profile
            
            violations = validate_key_actions(actions)
            if violations:
                print("\n[Warning] Schedule Invariant Violations detected after profile change:")
                for violation in violations:
                    print(f"  - [{violation.code}] {violation.message}")
                if not check_and_abort_violations(violations, is_dry_run):
                    return PLAYBACK_QUIT
                    
        if new_tempo is not None:
            sched_meta = build_key_actions(
                song, policy=active_policy, scan_code_mode=CURRENT_SCAN_CODE_MODE,
                resolver=resolver, tempo_scale=new_tempo
            )
            actions = sched_meta.actions
            current_tempo = new_tempo
            
            violations = validate_key_actions(actions)
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

    backend = DryRunBackend() if is_dry_run else WinSendInputBackend()
    renderer = ProgressRenderer(
        controls,
        verbose=VERBOSE_HUD,
        profile_name=current_profile,
        tempo_scale=current_tempo,
    )
    
    engine = PlaybackEngine(
        song=song,
        actions=actions,
        backend=backend,
        controls=controls,
        renderer=renderer,
        telemetry_enabled=TELEMETRY_CSV_ENABLED or PLAYBACK_DEBUG or force_dry_run,
        require_focus=not is_dry_run,
        profile_name=current_profile,
        tempo_scale=current_tempo,
        fps=force_fps,
        sleep_policy=active_sleep_policy,
        focus_restore_grace_us=active_policy.focus_restore_grace_us
    )
    result = engine.play()
    # P3: post-run summary
    _print_post_run_report(engine, current_profile, current_tempo)
    return result


def build_arg_parser():
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
        help="folder containing .json/.skysheet files",
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
        choices=[
            "local-precise", "balanced", "remote-safe", "dense-safe"
        ],
        default="balanced",
        help=(
            "Timing profile: "
            "local-precise (low latency), "
            "remote-safe (listener quality), "
            "dense-safe (many chords/repeats), "
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
        "--fps",
        type=int,
        default=60,
        help="Target FPS for frame-aware timing scaling (default: 60)",
    )
    timing.add_argument(
        "--hold-ms",
        type=int,
        help="Override key hold duration in ms (overrides profile)",
    )
    timing.add_argument(
        "--min-hold-ms",
        type=int,
        help="Override minimum key hold duration in ms (overrides profile)",
    )
    timing.add_argument(
        "--min-scheduled-hold-ms",
        type=int,
        help="Override minimum scheduled hold duration in ms (overrides profile)",
    )
    timing.add_argument(
        "--release-gap-ms",
        type=int,
        help="Override release gap in ms (overrides profile)",
    )
    timing.add_argument(
        "--repeat-release-gap-ms",
        type=int,
        help="Override same-key repeat gap in ms (overrides profile)",
    )
    timing.add_argument(
        "--input-lead-ms",
        type=int,
        help="Override input lead duration in ms (overrides profile)",
    )
    timing.add_argument(
        "--chord-merge-window-ms",
        type=int,
        help="Override chord merge tolerance window in ms (overrides profile)",
    )
    timing.add_argument(
        "--spin-threshold-us",
        type=int,
        help="Override CPU spin threshold in microseconds (precise=800, balanced=500, battery_safe=200/0) (overrides profile)",
    )
    timing.add_argument(
        "--focus-restore-grace-ms",
        type=int,
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

    # ── Runtime Controls ──────────────────────────────────────────────────────
    ctrl = parser.add_argument_group("Runtime controls (hotkeys during playback)")
    ctrl.add_argument(
        "--pause-key",
        default="f8",
        help="pause/resume hotkey, e.g. f8 or ctrl+p (default: f8)",
    )
    ctrl.add_argument(
        "--skip-key",
        default="f9",
        help="skip current song hotkey (default: f9)",
    )
    ctrl.add_argument(
        "--quit-key",
        default="f10",
        help="quit playback hotkey (default: f10; Esc not recommended — game may intercept it)",
    )
    ctrl.add_argument(
        "--refocus-key",
        default="f6",
        help="bring Sky window to foreground hotkey (default: f6)",
    )
    ctrl.add_argument(
        "--panic-key",
        default="ctrl+alt+backspace",
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
        "--sky-process-names",
        default="Sky.exe,Sky Children of the Light.exe",
        help="comma-separated Sky executable names to match (default: Sky.exe,...)",
    )
    diag.add_argument(
        "--allow-title-fallback",
        action="store_true",
        help="allow window title matching when process verification fails",
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

def configure_from_args(args: argparse.Namespace) -> None:
    global PLAYBACK_DEBUG, DEBUG_LOG_PATH, CURRENT_SCAN_CODE_MODE, TIMING_POLICY, SLEEP_POLICY, TELEMETRY_CSV_ENABLED, DRY_RUN_MODE, TEMPO_SCALE, TIMING_PROFILE_NAME, VERBOSE_HUD
    from sky_music.platform.win32 import inputs
    from sky_music.ui import picker as songs
    from sky_music.domain.scheduler_types import TimingPolicy
    from sky_music.config import load_config
    from sky_music.infrastructure.timing import SleepPolicy

    CURRENT_SCAN_CODE_MODE = args.scan_code_mode
    songs.SONG_DIR = args.songs_dir
    PLAYBACK_DEBUG = args.debug_playback
    inputs.PLAYBACK_DEBUG = args.debug_playback
    TELEMETRY_CSV_ENABLED = args.debug_csv
    DRY_RUN_MODE = args.dry_run
    TEMPO_SCALE = args.tempo_scale
    if TEMPO_SCALE <= 0:
        raise ValueError("tempo_scale must be > 0")
    TIMING_PROFILE_NAME = args.timing_profile
    VERBOSE_HUD = args.verbose_hud

    if PLAYBACK_DEBUG:
        init_debug_log()

    # Determine base TimingPolicy from persistent profile in config or fallbacks
    profile = args.timing_profile.lower().replace("-", "_")
    user_cfg = load_config()
    
    if profile in user_cfg.timing_profiles:
        p_dict = user_cfg.timing_profiles[profile]
    else:
        fallback_profile = user_cfg.default_timing_profile.lower().replace("-", "_")
        p_dict = user_cfg.timing_profiles.get(fallback_profile, user_cfg.timing_profiles["balanced"])
        
    base_spin_threshold_us = p_dict.get("spin_threshold_us", 500)
    policy = TimingPolicy(
        hold_us=p_dict.get("hold_us", 24_000),
        min_hold_us=p_dict.get("min_hold_us", 12_000),
        release_gap_us=p_dict.get("release_gap_us", 3_000),
        repeat_release_gap_us=p_dict.get("repeat_release_gap_us", 2_000),
        min_scheduled_hold_us=p_dict.get("min_scheduled_hold_us", 500),
        input_lead_us=p_dict.get("input_lead_us", 0),
        chord_merge_window_us=p_dict.get("chord_merge_window_us", 0),
        focus_restore_grace_us=p_dict.get("focus_restore_grace_us", 100_000),
        same_key_conflict_policy=p_dict.get("same_key_conflict_policy", "degraded")
    )

    # Perform overrides from arguments
    hold_us = args.hold_ms * 1000 if args.hold_ms is not None else policy.hold_us
    min_hold_us = args.min_hold_ms * 1000 if args.min_hold_ms is not None else policy.min_hold_us
    release_gap_us = args.release_gap_ms * 1000 if args.release_gap_ms is not None else policy.release_gap_us
    repeat_release_gap_us = args.repeat_release_gap_ms * 1000 if args.repeat_release_gap_ms is not None else policy.repeat_release_gap_us
    input_lead_us = getattr(args, "input_lead_ms", None)
    if input_lead_us is not None:
        input_lead_us = input_lead_us * 1000
    else:
        input_lead_us = policy.input_lead_us
        
    chord_merge_window_us = getattr(args, "chord_merge_window_ms", None)
    if chord_merge_window_us is not None:
        chord_merge_window_us = chord_merge_window_us * 1000
    else:
        chord_merge_window_us = policy.chord_merge_window_us

    spin_threshold_us = getattr(args, "spin_threshold_us", None)
    if spin_threshold_us is None:
        spin_threshold_us = base_spin_threshold_us

    focus_restore_grace_us = getattr(args, "focus_restore_grace_ms", None)
    if focus_restore_grace_us is not None:
        focus_restore_grace_us = focus_restore_grace_us * 1000
    else:
        focus_restore_grace_us = policy.focus_restore_grace_us
    
    min_scheduled_hold_us = getattr(args, "min_scheduled_hold_ms", None)
    if min_scheduled_hold_us is not None:
        min_scheduled_hold_us = min_scheduled_hold_us * 1000
    else:
        min_scheduled_hold_us = policy.min_scheduled_hold_us

    same_key_conflict_policy = args.same_key_conflict_policy if args.same_key_conflict_policy is not None else policy.same_key_conflict_policy

    TIMING_POLICY = TimingPolicy(
        hold_us=hold_us,
        min_hold_us=min_hold_us,
        release_gap_us=release_gap_us,
        repeat_release_gap_us=repeat_release_gap_us,
        min_scheduled_hold_us=min_scheduled_hold_us,
        input_lead_us=input_lead_us,
        chord_merge_window_us=chord_merge_window_us,
        focus_restore_grace_us=focus_restore_grace_us,
        same_key_conflict_policy=same_key_conflict_policy
    )

    # Upgrade to FrameTimingPolicy when --fps is provided
    fps_hint = getattr(args, "fps", None)
    if fps_hint is not None and fps_hint > 0:
        from sky_music.domain.scheduler_types import FrameTimingPolicy
        TIMING_POLICY = FrameTimingPolicy.from_timing_policy(
            TIMING_POLICY,
            fps=fps_hint,
            same_key_conflict_policy=same_key_conflict_policy,
        )
        # Reflect fps-aware hold_us back so post-run report shows correct profile
        TIMING_PROFILE_NAME = f"{args.timing_profile}@{fps_hint}fps"

    SLEEP_POLICY = SleepPolicy(
        spin_threshold_us=spin_threshold_us,
        poll_s=0.025
    )

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

def prompt_song_selection(
    profile: str = "balanced",
    tempo: float = 1.0,
    dry_run: bool = False,
    fps: int | None = None,
) -> "SongPickerResult | None":
    from sky_music.ui import picker as songs
    if songs.HAS_PROMPT_TOOLKIT:
        return songs.choose_song_interactively(
            initial_profile=profile,
            initial_tempo=tempo,
            initial_dry_run=dry_run,
            initial_fps=fps,
        )

    # Fallback CLI mode (no prompt_toolkit)
    song_choices = get_song_choices(force_refresh=True)
    print_choices_local(song_choices)

    while True:
        print("Commands: number/name = play, r = refresh, q = quit")
        selection = input("Select song: ").strip()

        if selection.casefold() in {"q", "quit", "exit", "0"}:
            return None
        if selection.casefold() in {"r", "refresh"}:
            clear_terminal()
            song_choices = get_song_choices(force_refresh=True)
            print_choices_local(song_choices)
            continue

        selected_song = resolve_song_selection(selection, song_choices)
        if selected_song is not None:
            return songs.SongPickerResult(
                song_path=selected_song,
                action="dry_run" if dry_run else "play",
                profile_name=profile,
                tempo_scale=tempo,
                fps=fps,
            )
        print("")

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

    parser = build_arg_parser()
    args = parser.parse_args()

    # P4: Load user config and apply saved defaults (CLI flags override these)
    user_cfg = load_config()
    apply_config_defaults(args, user_cfg)

    configure_from_args(args)
    try:
        controls = build_playback_controls(args)
    except ValueError as exc:
        parser.error(str(exc))

    if args.inspect_telemetry is not None:
        from sky_music.orchestration.telemetry import inspect_telemetry_report
        inspect_telemetry_report(args.inspect_telemetry)
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
                    overrides=PlaybackOverrides(
                        dry_run=DRY_RUN_MODE,
                        profile=TIMING_PROFILE_NAME,
                        tempo=TEMPO_SCALE,
                        fps=getattr(args, "fps", None)
                    )
                )
                if result == PLAYBACK_QUIT:
                    return 0
                if result == PLAYBACK_SKIPPED:
                    return 0
            return 0

        while True:
            picker_result = prompt_song_selection(
                profile=TIMING_PROFILE_NAME,
                tempo=TEMPO_SCALE,
                dry_run=DRY_RUN_MODE,
                fps=getattr(args, "fps", None)
            )
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
    raise SystemExit(main())
