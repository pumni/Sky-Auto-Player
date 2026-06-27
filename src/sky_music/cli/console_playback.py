from __future__ import annotations

import itertools
import os
import shutil
import sys
import time
from pathlib import Path
from typing import Any

import sky_music.infrastructure.doctor as doctor
from sky_music.config import (
    AppConfig,
    canonical_profile_name,
    load_config,
    merged_timing_profiles,
    persist_default_profile,
    resolve_game_fps,
)
from sky_music.domain.session_context import (
    PlaybackSessionContext,
    merge_session_with_overrides,
)
from sky_music.infrastructure.hotkeys import PlaybackControls
from sky_music.orchestration.runtime_session import (
    RUNTIME_STATE,
    PlaybackOverrides,
)
from sky_music.platform.win32 import inputs as _inputs
from sky_music.ui.hud import (
    PLAYBACK_QUIT,
    ProgressRenderer,
    clear_terminal,
)
from sky_music.ui.picker_helpers import (
    SONG_DIR,
    SUPPORTED_EXTENSIONS,
    countdown_before_playback,
)
from sky_music.ui.text_render import ansi_box, clamp_terminal_width


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
        # Safe to pass: input/msvcrt may fail if standard streams are closed or detached (e.g. running in non-TTY environments)
        pass
    sys.exit(code)


def _handle_risk_analysis(report: Any, song: Any, is_dry_run: bool, controls: Any, policy_override_fn: Any = None) -> tuple[bool, str | None, float | None]:
    """Display risk analysis, prompt user for action if severity is medium/high.

    Returns (should_continue, new_profile_name_or_None, new_tempo_scale_or_None).
    """
    severity = report.severity.upper()
    recommended = report.suggested_profile

    print()
    print(f"  ┌─ Schedule Risk: {severity} " + "─" * max(0, 38 - len(severity)))
    for rec in report.recommendations:
        print(f"  │  * {rec}")
    print(f"  │  Recommended profile: {recommended}")
    print(f"  └{'─' * 44}")
    print()

    if is_dry_run:
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
        try:
            user_cfg = load_config()
            persist_default_profile(user_cfg, recommended)
        except Exception:
            # Safe to pass: failing to save user profile selection to config.json is non-fatal for playback
            pass
        return True, recommended, None
    elif choice == "2":
        print( "  → Tempo scaled to 0.92x")
        return True, None, 0.92
    elif choice == "3":
        print( "  → Running dry-run simulation first...")
        return True, None, None
    elif choice == "5":
        return False, None, None
    else:
        print( "  → Proceeding with current settings.")
        return True, None, None


def _mini_preflight(is_dry_run: bool, profile: str = "balanced", tempo: float = 1.0, controls: Any = None) -> bool:
    """Preflight check before real playback — uniform premium TUI panel output."""
    if is_dry_run:
        return True

    checks: list[tuple[bool, str]] = []

    ANSI_RESET = "\033[0m"
    ANSI_BOLD = "\033[1m"
    ANSI_CYAN = "\033[36m"
    ANSI_GREEN = "\033[32m"
    ANSI_RED = "\033[31m"
    ANSI_YELLOW = "\033[33m"
    
    terminal_width = shutil.get_terminal_size((80, 24)).columns
    width = clamp_terminal_width(terminal_width)

    def print_ansi_box(title: str, lines: list[str], border_color: str = ANSI_CYAN) -> None:
        for rendered in ansi_box(title, lines, width=width, border_color=border_color):
            print(rendered)

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

    _inputs.focusWindow()
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
            print_ansi_box("Checks", [*check_lines, col1], border_color=ANSI_CYAN)
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

    timer = doctor.check_timer_resolution()
    checks.append((timer["ok"], "Timer active" if timer["ok"] else timer["msg"]))

    keys = doctor.check_physical_keys_held()
    checks.append((keys["ok"], "No note keys held" if keys["ok"] else f"Held: {', '.join(keys.get('held_keys', []))}"))

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


def print_choices_local(song_choices: list[Path]) -> None:
    if not song_choices:
        print(f"No songs found in: {SONG_DIR.resolve()}")
        print(f"Supported extensions: {', '.join(sorted(SUPPORTED_EXTENSIONS))}")
        return
    print("Songs:")
    for index, path in enumerate(song_choices, start=1):
        print(f"  {index:>2}) {path.stem}")


def print_schedule_summary(actions: tuple[Any, ...], sched_meta: Any) -> None:
    max_chord = max((len(a.scan_codes) for a in actions if a.kind == "down"), default=0)
    down_timestamps = sorted({a.at_us for a in actions if a.kind == "down"})
    min_timestamp_gap_us = min((b - a for a, b in itertools.pairwise(down_timestamps)), default=0)
    
    min_ts_gap_str = f"{min_timestamp_gap_us / 1000:.1f} ms" if min_timestamp_gap_us > 0 else "N/A"
    min_sk_gap_us = sched_meta.shortest_same_key_interval_us
    min_sk_gap_str = f"{min_sk_gap_us / 1000:.1f} ms" if min_sk_gap_us is not None else "N/A"
    
    print()
    print("  \033[1m\033[36mSchedule Summary:\033[0m")
    print(f"    Notes                       : {sched_meta.note_count}")
    print(f"    Timeline groups             : {len(actions)}")
    print(f"    Max chord                   : {max_chord}")
    print(f"    Min timestamp gap           : {min_ts_gap_str}")
    print(f"    Min same-key gap            : {min_sk_gap_str}")
    print(f"    Infeasible same-key repeats : {sched_meta.impossible_same_key_repeats}")
    print(f"    Duplicate same-key slots    : {sched_meta.duplicate_note_count}")
    print()


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
            if kernel32.GetConsoleMode(handle, ctypes.byref(mode)) and not (mode.value & ENABLE_VIRTUAL_TERMINAL_PROCESSING):
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
        "Chạy từ Windows Terminal. "
    )


def play_selected_song(
    selected_song: Path,
    countdown_seconds: int,
    controls: PlaybackControls | None = None,
    overrides: PlaybackOverrides | None = None,
) -> str:
    from sky_music.domain.scheduler import ScheduleBuildError, build_key_actions
    from sky_music.domain.song_repository import get_shared_song_repository
    from sky_music.infrastructure.backend import DryRunBackend, WinSendInputBackend
    from sky_music.orchestration.engine import _LEAD_CACHE_PATH, PlaybackEngine
    from sky_music.ui.textual_app import TEXTUAL_THEME_TOKENS
    from sky_music.ui.textual_app.playback_app import (
        SnapshotRenderer,
        run_playback_textual,
    )

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
    dispatch_lead_us = overrides.dispatch_lead_us if overrides else 0
    onset_bias_us = overrides.onset_bias_us if (overrides and overrides.onset_bias_us is not None) else RUNTIME_STATE.onset_bias_us
    if force_profile is not None:
        force_profile = canonical_profile_name(force_profile)

    user_cfg = load_config()
    base_session = RUNTIME_STATE.session or PlaybackSessionContext.balanced(
        tempo_scale=RUNTIME_STATE.tempo_scale,
        fps=resolve_game_fps(user_cfg.game_fps),
        scan_code_mode=RUNTIME_STATE.scan_code_mode,
    )
    session = merge_session_with_overrides(
        base_session,
        profile=force_profile,
        tempo=force_tempo,
        fps=force_fps,
    )

    is_dry_run = RUNTIME_STATE.dry_run or force_dry_run
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
        if (report.severity == "medium" and not user_cfg.safety.prompt_on_medium_risk) or (report.severity == "high" and not user_cfg.safety.prompt_on_high_risk):
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

    # Print Schedule Summary
    print_schedule_summary(actions, sched_meta)

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
    telemetry_enabled = RUNTIME_STATE.telemetry_csv_enabled or user_cfg.telemetry_enabled_by_default or _inputs.PLAYBACK_DEBUG or force_dry_run

    backend = DryRunBackend() if is_dry_run else WinSendInputBackend()
    # Resolve the accent colour for the active theme so the HUD borders match
    # the picker's colour scheme rather than always rendering in bright-cyan.
    _active_theme_name = (user_cfg.theme or "aurora").casefold()
    _theme_tokens = TEXTUAL_THEME_TOKENS.get(_active_theme_name, TEXTUAL_THEME_TOKENS["aurora"])

    use_textual_ui = (_check_textual_support() is None)

    if use_textual_ui:
        renderer = SnapshotRenderer()
    else:
        renderer = ProgressRenderer(
            controls,
            verbose=verbose_hud_mode,
            profile_name=current_profile,
            tempo_scale=current_tempo,
            accent_hex=_theme_tokens.accent,
            theme_name=_active_theme_name,
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
        use_dispatch_thread=RUNTIME_STATE.use_dispatch_thread,
        input_path_warn_us=user_cfg.input_path_warn_us if RUNTIME_STATE.check_input_path else 0,
        enable_timer_guard=RUNTIME_STATE.enable_timer_guard,
        enable_waitable_timer=RUNTIME_STATE.enable_waitable_timer,
        enable_gc_pause=RUNTIME_STATE.enable_gc_pause,
        enable_switch_interval_tuning=RUNTIME_STATE.enable_switch_interval_tuning,
        enable_adaptive_lead=RUNTIME_STATE.enable_adaptive_lead,
        enable_adaptive_spin=RUNTIME_STATE.enable_adaptive_spin,
        enable_event_wait=RUNTIME_STATE.enable_event_wait,
        enable_epoch_rebase=RUNTIME_STATE.enable_epoch_rebase,
        rt_priority_mode=RUNTIME_STATE.rt_priority_mode,
        dispatch_lead_us=dispatch_lead_us,
        onset_bias_us=onset_bias_us,
        lead_cache_path=_LEAD_CACHE_PATH,
    )
    engine.telemetry.record_schedule_metadata(sched_meta)

    if use_textual_ui:
        if not isinstance(renderer, SnapshotRenderer):
            raise TypeError(
                f"expected SnapshotRenderer for Textual UI, got {type(renderer).__name__}"
            )
        result = run_playback_textual(
            engine,
            renderer,
            theme_name=_active_theme_name,
            song_name=song.name,
            total_us=sched_meta.playback_duration_us,
        )
    else:
        result = engine.play()

    clear_terminal()
    return result


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
            value = data.get(fallback_unframed_key)
        if value is None:
            value = 0
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
