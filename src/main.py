import argparse
import os
import sys
import time
from pathlib import Path

# Import từ các mô-đun chuyên biệt
from sky_music.platform.win32 import inputs
from sky_music.config import (
    load_config,
    apply_config_defaults,
    HotkeyDefaults,
    AppConfig,
    persist_playback_defaults,
    resolve_game_fps,
    sky_process_names_csv,
    canonical_profile_name,
    CLI_PROFILE_NAMES,
)
from sky_music.domain.session_context import (
    PlaybackSessionContext,
    merge_session_with_overrides,
)
from sky_music.platform.win32.inputs import (
    enable_high_precision_timers,
    disable_high_precision_timers
)
from sky_music.ui.hud import (
    PLAYBACK_SKIPPED,
    PLAYBACK_QUIT,
    clear_terminal
)
from sky_music.ui.picker import SongPickerResult
from sky_music.infrastructure.hotkeys import (
    PlaybackControls,
    parse_hotkey,
    hotkey_conflicts_with_note_keys
)
from sky_music.ui.picker_helpers import (
    SONG_DIR,
    get_song_choices,
    resolve_song_selection,
)
from sky_music.cli.console_playback import (
    _wait_key_and_exit,
    print_choices_local,
    play_selected_song,
    _check_textual_support,
    _print_profile_comparison_table,
)
from sky_music.cli.calibration_command import (
    run_auto_calibrate,
    apply_calibration_from_telemetry,
)
from sky_music.cli.doctor_command import run_doctor_command
from sky_music.orchestration.runtime_session import (
    PlaybackOverrides,
    RUNTIME_STATE,
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
        "--dispatch-lead-us",
        type=int,
        default=0,
        help="Fixed lead time in microseconds to trigger input dispatch earlier (default: 0)",
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
        "--check-input-path",
        action="store_true",
        help="monitor input path duration and warn if degraded (OS-side/Filter Keys)",
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
        "--ui-background",
        choices=["transparent", "painted"],
        default=None,
        help="song picker background mode (default: saved or transparent)",
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
    from sky_music.ui import picker_helpers

    cfg = cfg or load_config()

    picker_helpers.SONG_DIR = args.songs_dir
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
    RUNTIME_STATE.check_input_path = args.check_input_path

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



def prompt_song_selection(
    profile: str = "balanced",
    tempo: float = 1.0,
    dry_run: bool = False,
    fps: int | None = None,
    scan_code_mode: str = "physical",
    background_mode: str | None = None,
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
            background_mode=background_mode,
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


def main() -> int:
    if getattr(sys, "frozen", False):
        # Ensure the working directory is the exe's folder so relative paths work
        os.chdir(Path(sys.executable).parent)

    if "--selftest-textual" in sys.argv:
        return _run_textual_selftest()

    if sys.platform == 'win32':
        try:
            sys.stdout.reconfigure(encoding='utf-8')  # type: ignore
            sys.stderr.reconfigure(encoding='utf-8')  # type: ignore
        except Exception:
            pass

    user_cfg = load_config()
    parser = build_arg_parser()
    args = parser.parse_args()

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
        res = apply_calibration_from_telemetry(
            user_cfg,
            RUNTIME_STATE,
            persist=bool(getattr(args, "save_calibration", False)),
            summary_path=getattr(args, "calibration_summary", None),
        )
        if res.exit_code != 0:
            return res.exit_code
        _sync_legacy_runtime_globals()

    if getattr(args, "auto_calibrate", False):
        return run_auto_calibrate(getattr(args, "calibration_summary", None))

    if args.doctor or args.doctor_timing or args.doctor_input:
        return run_doctor_command(
            full=bool(args.doctor),
            timing=bool(args.doctor_timing),
            input_check=bool(args.doctor_input),
        )

    song_choices = get_song_choices(force_refresh=True)

    if args.list:
        print_choices_local(song_choices)
        return 0

    if not song_choices and args.song is None:
        print_choices_local(song_choices)
        _wait_key_and_exit(1)
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
                        dispatch_lead_us=args.dispatch_lead_us,
                    ),
                )
                if result == PLAYBACK_QUIT:
                    return 0
                if result == PLAYBACK_SKIPPED:
                    return 0
            return 0

        if _check_textual_support() is None:
            from sky_music.ui import picker as songs
            from sky_music.ui.textual_app import run_sky_app_unified

            cli_fps_explicit = any(arg.startswith("--fps") for arg in sys.argv)
            resolved_fps = resolve_game_fps(args.fps if cli_fps_explicit else user_cfg.game_fps)

            session = merge_session_with_overrides(
                PLAYBACK_SESSION or PlaybackSessionContext.balanced(
                    tempo_scale=TEMPO_SCALE,
                    fps=resolved_fps,
                    scan_code_mode=CURRENT_SCAN_CODE_MODE,
                ),
                profile=canonical_profile_name(user_cfg.default_timing_profile),
                tempo=TEMPO_SCALE,
                fps=resolved_fps,
            )

            try:
                return run_sky_app_unified(
                    theme_name=songs.ACTIVE_THEME,
                    background_mode=args.ui_background,
                    initial_profile=session.profile_name,
                    initial_tempo=session.tempo_scale,
                    initial_fps=session.fps,
                    initial_dry_run=DRY_RUN_MODE,
                    scan_code_mode=session.scan_code_mode,
                    controls=controls,
                    countdown_seconds=args.countdown,
                    dispatch_lead_us=args.dispatch_lead_us,
                )
            except Exception as exc:
                print(f"\n[ERROR] Playback aborted due to background worker cleanup failure: {exc}")
                return 1

        while True:
            # Resolve initial FPS prioritizing active CLI overrides, then persistent config defaults
            cli_fps_explicit = any(arg.startswith("--fps") for arg in sys.argv)
            resolved_fps = resolve_game_fps(args.fps if cli_fps_explicit else user_cfg.game_fps)

            try:
                picker_result = prompt_song_selection(
                    profile=canonical_profile_name(user_cfg.default_timing_profile),
                    tempo=TEMPO_SCALE,
                    dry_run=DRY_RUN_MODE,
                    fps=resolved_fps,
                    scan_code_mode=CURRENT_SCAN_CODE_MODE,
                    background_mode=args.ui_background,
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
                    dispatch_lead_us=args.dispatch_lead_us,
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

def write_crash_log(exc: BaseException) -> None:
    import traceback
    import time
    log_dir = Path("logs")
    log_dir.mkdir(parents=True, exist_ok=True)
    path = log_dir / f"crash_{time.strftime('%Y%m%d_%H%M%S')}.log"
    path.write_text(
        "".join(traceback.format_exception(type(exc), exc, exc.__traceback__)),
        encoding="utf-8",
    )
    print(f"Crash log: {path.resolve()}", file=sys.stderr)

if __name__ == '__main__':
    # Required for safe ProcessPoolExecutor startup on Windows and harmless
    # for normal `uv run python src/main.py` execution.
    try:
        import multiprocessing
        multiprocessing.freeze_support()
    except Exception:
        # Safe to pass: multiprocessing or freeze_support might not be available or needed on all platforms/environments
        pass

    try:
        raise SystemExit(main())
    except SystemExit:
        raise
    except Exception as exc:
        print(f"\n[CRITICAL] Sky Player crashed: {exc}", file=sys.stderr)
        write_crash_log(exc)
        if getattr(sys, "frozen", False):
            _wait_key_and_exit(1)
        raise
    finally:
        flush_debug_log()
