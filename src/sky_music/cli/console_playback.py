from __future__ import annotations

import contextlib
import itertools
import os
import sys
import time
from pathlib import Path
from typing import Any

from rich.console import Console
from rich.panel import Panel
from rich.style import Style
from rich.table import Table
from rich.text import Text

import sky_music.infrastructure.doctor as doctor
from sky_music.config import (
    AppConfig,
    load_config,
    resolve_game_fps,
)
from sky_music.domain.hold_timing import (
    HOLD_FRAME_OPTIONS,
    format_hold_frames,
    frame_duration_us,
    materialize_hold_us,
)
from sky_music.domain.session_context import (
    PlaybackSessionContext,
    merge_session_with_overrides,
)
from sky_music.infrastructure.hotkeys import PlaybackControls
from sky_music.orchestration.calibrated_policy import resolve_calibrated_policy
from sky_music.orchestration.runtime_session import (
    RUNTIME_STATE,
    PlaybackOverrides,
)
from sky_music.platform.win32 import window_target as _window_target
from sky_music.platform.win32.console import virtual_terminal_processing_enabled
from sky_music.ui.hud import (
    PLAYBACK_QUIT,
    ProgressRenderer,
    clear_terminal,
)
from sky_music.ui.picker_helpers import (
    SONG_DIR,
    SUPPORTED_EXTENSIONS,
)
from sky_music.ui.picker_theme import get_theme_preset

_console = Console(highlight=False)


def _wait_key_and_exit(code: int = 1) -> None:
    """Print a 'press any key' prompt then exit — keeps the console window open
    when the exe is launched by double-click so the user can read the error."""
    print("\nPress any key to exit...", file=sys.stderr, flush=True)
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


def _resolve_cli_theme_style() -> Style:
    cfg = load_config()
    theme_name = (cfg.theme or "aurora").casefold()
    preset = get_theme_preset(theme_name)
    return Style.parse(preset.accent)


def _handle_risk_analysis(report: Any, _song: Any, is_dry_run: bool, _controls: Any, _policy_override_fn: Any = None) -> tuple[bool, float | None, float | None]:
    """Display risk analysis, prompt user for action if severity is medium/high.

    Returns (should_continue, new_hold_frames_or_None, new_tempo_scale_or_None).
    """
    severity = report.severity.upper()
    recommended = report.suggested_hold_frames
    accent = _resolve_cli_theme_style()

    danger_style = Style.parse("#ef4444")
    warning_style = Style.parse("#f59e0b")
    muted_style = Style.parse("#94a3b8")

    severity_color = danger_style if severity == "HIGH" else warning_style

    content = Text()
    content.append("Schedule Risk Assessment\n", style=Style(bold=True, color=severity_color.color))
    content.append("\nSeverity: ", style=muted_style)
    content.append(severity, style=severity_color + Style(bold=True))
    content.append("\nRecommended hold: ", style=muted_style)
    content.append(f"{recommended:.2f} frames", style=accent + Style(bold=True))
    content.append("\n\nRecommendations:\n", style=muted_style)
    for rec in report.recommendations:
        content.append(f"  • {rec}\n")

    _console.print()
    _console.print(Panel(content, title="Risk Analysis", border_style=severity_color))
    _console.print()

    if is_dry_run:
        return True, None, None

    options_box = Text()
    options_box.append("What would you like to do?\n\n")
    options_box.append(f"[1] Use hold {recommended:.2f} frames\n")
    options_box.append("[2] Scale tempo down to 0.92x\n")
    options_box.append("[3] Dry-run first (simulate, no keystrokes)\n")
    options_box.append("[4] Proceed with current settings\n")
    options_box.append("[5] Cancel\n")

    _console.print(Panel(options_box, title="Decision", border_style=accent))

    try:
        choice = input("\n  Choice [1-5]: ").strip()
    except (EOFError, KeyboardInterrupt):
        return False, None, None

    if choice == "1":
        _console.print(f"\n  → Using hold {recommended:.2f} frames")
        return True, recommended, None
    if choice == "2":
        _console.print("\n  → Tempo scaled to 0.92x")
        return True, None, 0.92
    if choice == "3":
        _console.print("\n  → Running dry-run simulation first...")
        return True, None, None
    if choice == "5":
        return False, None, None
    _console.print("\n  → Proceeding with current settings.")
    return True, None, None


def _build_preflight_panel(
    title: str,
    content_parts: list[Text | str],
    border_style: Style,
) -> None:
    content = Text()
    for i, part in enumerate(content_parts):
        if i:
            content.append("\n")
        if isinstance(part, Text):
            content.append(part)
        else:
            content.append(part)
    _console.print()
    _console.print(Panel(content, title=title, border_style=border_style))


def _mini_preflight(is_dry_run: bool, hold_label: str = "hold 1.00f", tempo: float = 1.0, controls: Any = None) -> bool:
    """Preflight check before real playback — uniform premium TUI panel output."""
    if is_dry_run:
        return True

    checks: list[tuple[bool, str]] = []

    accent = _resolve_cli_theme_style()
    success = Style.parse("#22c55e")
    danger = Style.parse("#ef4444")
    warning = Style.parse("#f59e0b")

    win = doctor.check_sky_window()
    checks.append((win["ok"], "Sky window detected" if win["ok"] else f"Sky not found: {win['msg']}"))

    if not win["ok"]:
        while True:
            dry_str = "ON" if is_dry_run else "OFF"
            header_content = Text.assemble(
                "Readiness │ ", (hold_label, accent), " │ tempo ", (f"{tempo:.2f}x", accent), " │ dry ", (dry_str, accent),
            )
            error_content = Text.assemble(("✗ Sky not found: ", danger), win["msg"])
            status_content = Text("Waiting for Sky focus. Playback has not started yet.", style=warning)
            controls_content = Text.assemble(
                ("R", Style(bold=True)), " retry │ ",
                ("D", Style(bold=True)), " dry-run │ ",
                ("Enter", Style(bold=True)), " cancel",
            )

            _build_preflight_panel("SKY MUSIC HELPER", [header_content], accent)
            _build_preflight_panel("Checks", [error_content], accent)
            _build_preflight_panel("Status", [status_content, controls_content], warning)

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
                _console.print("  → Use --dry-run to simulate without Sky.")
                return False
            else:
                return False

    # Startup preflight is read-only: the user must focus Sky manually. An
    # explicit retry is the only path allowed to request refocus.
    focus_ok = _window_target.is_sky_active()
    if not focus_ok:
        while True:
            dry_str = "ON" if is_dry_run else "OFF"
            header_content = Text.assemble(
                "Readiness │ ", (hold_label, accent), " │ tempo ", (f"{tempo:.2f}x", accent), " │ dry ", (dry_str, accent),
            )

            check_lines = []
            for ok, msg in checks:
                icon = "✓" if ok else "✗"
                check_style = success if ok else danger
                check_lines.append(Text.assemble((f"{icon} {msg}", check_style)))

            check_lines.append(Text("✗ Focus failed", style=danger))
            status_content = Text("Waiting for Sky focus. Playback has not started yet.", style=warning)
            controls_content = Text.assemble(
                ("R", Style(bold=True)), " retry │ ",
                ("D", Style(bold=True)), " dry-run │ ",
                ("Enter", Style(bold=True)), " cancel",
            )

            _build_preflight_panel("SKY MUSIC HELPER", [header_content], accent)
            _build_preflight_panel("Checks", check_lines, accent)
            _build_preflight_panel("Status", [status_content, controls_content], warning)

            try:
                choice = input("  Choice: ").strip().casefold()
            except (EOFError, KeyboardInterrupt):
                return False
            if choice == "r":
                _window_target.focus_window()
                time.sleep(0.25)
                if _window_target.is_sky_active():
                    break
            elif choice == "d":
                _console.print("  → Use --dry-run to simulate without Sky.")
                return False
            else:
                return False

    checks.append((True, "Focus confirmed"))

    timer = doctor.check_timer_resolution()
    checks.append((timer["ok"], "Timer active" if timer["ok"] else timer["msg"]))

    keys = doctor.check_physical_keys_held()
    checks.append((keys["ok"], "No note keys held" if keys["ok"] else f"Held: {', '.join(keys.get('held_keys', []))}"))

    dry_str = "ON" if is_dry_run else "OFF"
    header_content = Text.assemble(
        "Readiness │ ", (hold_label, accent), " │ tempo ", (f"{tempo:.2f}x", accent), " │ dry ", (dry_str, accent),
    )

    check_lines = []
    for ok, msg in checks:
        icon = "✓" if ok else "✗"
        check_style = success if ok else danger
        check_lines.append(Text.assemble((f"{icon} {msg}", check_style)))

    status_content = Text("Readiness checks passed. Starting playback...", style=success)
    if controls is not None and controls.enabled:
        ctrls_text = Text.assemble(
            (controls.panic.display, Style(bold=True)), " panic │ ",
            (controls.pause.display, Style(bold=True)), " pause/resume │ ",
            (controls.skip.display, Style(bold=True)), " skip │ ",
            (controls.quit.display, Style(bold=True)), " quit │ ",
            (controls.refocus.display, Style(bold=True)), " refocus",
        )
        status_lines: list[Text | str] = [status_content, ctrls_text]
    else:
        status_lines = [status_content]

    _build_preflight_panel("SKY MUSIC HELPER", [header_content], accent)
    _build_preflight_panel("Checks", check_lines, accent)
    _build_preflight_panel("Status", status_lines, success)
    return True


def print_choices_local(song_choices: list[Path]) -> None:
    if not song_choices:
        _console.print(
            Panel(
                Text.assemble(
                    f"No songs found in: {SONG_DIR.resolve()}\n",
                    f"Supported extensions: {', '.join(sorted(SUPPORTED_EXTENSIONS))}",
                ),
                title="Songs",
                border_style=Style.parse("#f59e0b"),
            )
        )
        return

    table = Table(title="Songs", title_style="bold", border_style=Style.parse("#94a3b8"))
    table.add_column("#", style="dim", justify="right", width=4)
    table.add_column("Title", style=Style(bold=True))
    for index, path in enumerate(song_choices, start=1):
        table.add_row(str(index), path.stem)
    _console.print(table)


def print_schedule_summary(actions: tuple[Any, ...], sched_meta: Any, *, fps: int) -> None:
    max_chord = max((len(a.scan_codes) for a in actions if a.kind == "down"), default=0)
    down_timestamps = sorted({a.at_us for a in actions if a.kind == "down"})
    min_timestamp_gap_us = min((b - a for a, b in itertools.pairwise(down_timestamps)), default=0)

    min_ts_gap_str = f"{min_timestamp_gap_us / 1000:.1f} ms" if min_timestamp_gap_us > 0 else "N/A"
    min_sk_gap_us = sched_meta.shortest_same_key_interval_us
    min_sk_gap_str = f"{min_sk_gap_us / 1000:.1f} ms" if min_sk_gap_us is not None else "N/A"

    accent = _resolve_cli_theme_style()
    muted = Style.parse("#94a3b8")
    warning = Style.parse("#f59e0b")

    text = Text()
    text.append(f"Notes                       : {sched_meta.note_count}\n", style=muted)
    text.append(f"Timeline groups             : {len(actions)}\n", style=muted)
    text.append(f"Max chord                   : {max_chord}\n", style=muted)
    text.append(f"Min timestamp gap           : {min_ts_gap_str}\n", style=muted)
    text.append(f"Min same-key gap            : {min_sk_gap_str}\n", style=muted)
    text.append(f"Infeasible same-key repeats : {sched_meta.impossible_same_key_repeats}\n", style=muted)
    if sched_meta.impossible_same_key_repeats > 0:
        text.append(
            f"  ({sched_meta.impossible_same_key_repeats} same-key repeats faster than one frame @{fps}fps - the game may merge them)\n",
            style=warning,
        )
    text.append(f"Risky same-key repeats      : {sched_meta.risky_same_key_repeats}\n", style=muted)
    text.append(f"Duplicate same-key slots    : {sched_meta.duplicate_note_count}\n", style=muted)

    warnings_list = getattr(sched_meta, "warnings", None) or ()
    if warnings_list:
        text.append("\n", style=muted)
        for warn in warnings_list:
            text.append(f"  [!] {warn}\n", style=warning)

    _console.print()
    _console.print(Panel(text, title="Schedule Summary", border_style=accent))
    _console.print()


def _check_textual_support() -> str | None:
    """Return a human-readable failure reason if Textual cannot run, or None if it can."""
    if not sys.stdout.isatty():
        return (
            "stdout is not an interactive terminal (isatty = False). "
            "Run Sky Auto Player directly in a terminal window, "
            "not through a pipe or redirect."
        )
    if sys.platform != "win32":
        return None  # non-Windows terminals generally support Textual
    # For frozen exes (double-click), WT_SESSION is absent even in Windows Terminal.
    # Check ENABLE_VIRTUAL_TERMINAL_PROCESSING via Win32 API instead.
    if getattr(sys, "frozen", False):
        if virtual_terminal_processing_enabled() is False:
            return (
                "The current console does not support ANSI / Virtual Terminal Processing. "
                "Launch the app from Windows Terminal (wt.exe) or "
                "enable 'Let Windows decide' under Settings > System > For developers > Terminal."
            )
        return None
    # Dev/source mode: require WT_SESSION or TERM_PROGRAM
    if os.environ.get("WT_SESSION"):
        return None
    if os.environ.get("TERM_PROGRAM") == "vscode":
        return None
    return (
        "Terminal is not recognised as supporting Textual on Windows. "
        "Run from Windows Terminal. "
    )


def play_selected_song(
    selected_song: Path,
    countdown_seconds: int,
    controls: PlaybackControls | None = None,
    overrides: PlaybackOverrides | None = None,
) -> str:
    from sky_music.domain.scheduler import ScheduleBuildError, build_key_actions
    from sky_music.domain.song_repository import get_shared_song_repository
    from sky_music.orchestration.engine import PlaybackEngine
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
    force_hold = overrides.hold_frames if overrides else None
    force_tempo = overrides.tempo if overrides else None
    force_fps = overrides.fps if overrides else None

    user_cfg = load_config()
    base_session = RUNTIME_STATE.session or PlaybackSessionContext.default(
        tempo_scale=RUNTIME_STATE.tempo_scale,
        fps=resolve_game_fps(user_cfg.game_fps),
        scan_code_mode=RUNTIME_STATE.scan_code_mode,
    )
    session = merge_session_with_overrides(
        base_session,
        hold_frames=force_hold,
        tempo=force_tempo,
        fps=force_fps,
    )

    is_dry_run = RUNTIME_STATE.dry_run or force_dry_run
    current_hold_label = session.display_hold_label()
    current_tempo = session.tempo_scale

    active_policy = resolve_calibrated_policy(session, user_cfg)

    # build_key_actions builds DefaultNoteResolver(instrument_profile) when resolver is None; that
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
            print("  Please reduce tempo or choose a shorter hold for more same-key repeat room.")
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
            if exc.recommended_hold_frames is not None and session.hold_frames != exc.recommended_hold_frames:
                print(f"  Use a shorter hold: --hold-frames {exc.recommended_hold_frames:.2f}")
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
    report = analyze_schedule(
        sched_meta,
        raw_notes=song.notes,
        current_hold_frames=session.hold_frames,
        current_tempo_scale=session.tempo_scale,
    )

    # If the picker already decided the hold/tempo, skip the prompt.
    if report.severity != "low" and force_hold is None and force_tempo is None:
        should_prompt = True
        if (report.severity == "medium" and not user_cfg.safety.prompt_on_medium_risk) or (report.severity == "high" and not user_cfg.safety.prompt_on_high_risk):
            should_prompt = False

        if should_prompt:
            should_continue, new_hold, new_tempo = _handle_risk_analysis(
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
            new_hold, new_tempo = None, None
        if new_hold is not None and new_hold != session.hold_frames:
            session = session.with_hold_frames(new_hold)
            active_policy = resolve_calibrated_policy(session, user_cfg)
            current_hold_label = session.display_hold_label()

            sched_meta = build_schedule(session, active_policy, current_tempo)
            if sched_meta is None:
                return PLAYBACK_QUIT
            actions = sched_meta.actions

            violations = validate_key_actions(actions, policy=active_policy)
            if violations:
                print("\n[Warning] Schedule Invariant Violations detected after hold change:")
                for violation in violations:
                    print(f"  - [{violation.code}] {violation.message}")
                if not check_and_abort_violations(violations, is_dry_run):
                    return PLAYBACK_QUIT
                    
        if new_tempo is not None:
            session = session.with_tempo(new_tempo)
            current_tempo = session.tempo_scale
            active_policy = resolve_calibrated_policy(session, user_cfg)
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
    print_schedule_summary(actions, sched_meta, fps=int(active_policy.fps))

    # Phase C: FPS assumption advisory — non-blocking notice when short notes exist under
    # high configured FPS. Printed once at play-start so the user has the information
    # before the countdown; never shown at 60 fps or below (I9 — never clamp fps).
    _fps_for_advisory = getattr(active_policy, "fps", None)
    _short_notes = getattr(sched_meta, "sub_60fps_frame_notes", 0)
    if _fps_for_advisory is not None and _fps_for_advisory > 60 and _short_notes > 0:
        from sky_music.ui.timing_guidance import fps_play_advisory
        _advisory = fps_play_advisory(fps=_fps_for_advisory, short_note_count=_short_notes)
        if _advisory:
            print(f"\n[Advisory] {_advisory}")

    # Preflight check and window readiness
    if not _mini_preflight(is_dry_run, hold_label=current_hold_label, tempo=current_tempo, controls=controls):
        return PLAYBACK_QUIT

    if not is_dry_run and controls is not None:
        try:
            controls.start()
        except Exception as exc:
            with contextlib.suppress(Exception):
                controls.close()
            _console.print(f"[red]Playback aborted: global hotkeys could not be registered: {exc}[/red]")
            return PLAYBACK_QUIT

    # The native session owns the absolute epoch. The UI countdown is now
    # display-only pre-roll, so the engine is launched immediately below.
    if is_dry_run:
        print(f"[simulation] DRY-RUN enabled. Simulating playback of {song.name}...")

    user_cfg = load_config()
    verbose_hud_mode = user_cfg.verbose_hud
    telemetry_enabled = RUNTIME_STATE.telemetry_csv_enabled or user_cfg.telemetry_enabled_by_default or _window_target.PLAYBACK_DEBUG or force_dry_run

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
            hold_label=current_hold_label,
            hold_frames=session.hold_frames,
            tempo_scale=current_tempo,
            accent_hex=_theme_tokens.accent,
            theme_name=_active_theme_name,
            schedule_warnings=sched_meta.warnings,
        )
        renderer.active_policy = active_policy

    # Clear preflight/countdown output so the live HUD starts on a clean terminal.
    # ProgressRenderer only erases its own previously-rendered lines; static print()
    # output from _mini_preflight would otherwise remain visible above the HUD.
    clear_terminal()

    engine = PlaybackEngine(
        song=song,
        actions=actions,
        dry_run=is_dry_run,
        controls=controls,
        renderer=renderer,
        telemetry_enabled=telemetry_enabled,
        require_focus=not is_dry_run,
        hold_label=current_hold_label,
        hold_frames=session.hold_frames,
        game_fps=int(active_policy.fps),
        tempo_scale=current_tempo,
        focus_restore_grace_us=int(active_policy.focus_restore_grace_us),
        min_hold_us=int(active_policy.min_hold_us),
        min_hold_margin_us=int(active_policy.min_hold_margin_us),
        min_hold_margin_source=active_policy.min_hold_margin_source,
        pre_roll_us=max(0, int(countdown_seconds)) * 1_000_000,
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
            warnings=sched_meta.warnings,
        )
    else:
        result = engine.play()

    clear_terminal()
    return result


def _print_hold_comparison_table(
    cfg: AppConfig | None = None,
    *,
    fps: int | None = None,
) -> None:
    """Print the explicit hold-frame choices for the selected FPS."""
    cfg = cfg or load_config()
    fps = resolve_game_fps(fps if fps is not None else cfg.game_fps)
    active_session = PlaybackSessionContext.default(
        hold_frames=cfg.default_hold_frames,
        fps=fps,
    )
    active_policy = resolve_calibrated_policy(active_session, cfg)
    margin = int(active_policy.min_hold_margin_us)
    margin_label = f"{margin} µs ({active_policy.min_hold_margin_source})"
    intents = {1.0: "Sharpest; default; maximum same-key scheduling room", 1.25: "Moderate visibility cushion", 1.5: "Longest visibility; useful when registration reliability matters more than short repeat room"}

    accent = _resolve_cli_theme_style()

    table = Table(
        title=f"Hold Comparison ({fps} FPS)",
        title_style=Style(bold=True),
        border_style=accent,
        header_style=Style(bold=True, color=accent.color),
        show_lines=True,
    )
    for header in ("Hold", "Frames", "FPS", "Frame duration", "Base hold", "Device margin", "Effective hold", "Intent"):
        table.add_column(header, justify="left" if header in ("Hold", "Intent") else "right")
    frame_us = frame_duration_us(fps)
    for ratio in HOLD_FRAME_OPTIONS:
        base = round(ratio * frame_us)
        effective = materialize_hold_us(ratio, fps, margin)
        table.add_row(format_hold_frames(ratio), f"{ratio:.2f}", str(fps), f"{frame_us} µs", f"{base} µs", margin_label, f"{effective} µs", intents[ratio])

    _console.print()
    _console.print(table)
    _console.print()
    _console.print(Text(f"Device margin: {margin_label}", style=Style(dim=True)))
    _console.print(
        Text("Select hold frames directly; FPS must match the value configured inside Sky.", style=Style(dim=True))
    )
