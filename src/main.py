import argparse
import os
import sys
import time
from pathlib import Path

from sky_music import __version__
from sky_music.cli.calibration_command import (
    apply_calibration_from_telemetry,
    run_auto_calibrate,
)
from sky_music.cli.console_playback import (
    _check_textual_support,
    _print_hold_comparison_table,
    _wait_key_and_exit,
    play_selected_song,
    print_choices_local,
)
from sky_music.cli.doctor_command import run_doctor_command
from sky_music.config import (
    VALID_FPS,
    AppConfig,
    HotkeyDefaults,
    apply_config_defaults,
    load_config,
    persist_playback_defaults,
    resolve_game_fps,
    sky_process_names_csv,
)
from sky_music.domain.hold_timing import HOLD_FRAME_OPTIONS
from sky_music.domain.session_context import (
    PlaybackSessionContext,
    merge_session_with_overrides,
)
from sky_music.infrastructure.hotkeys import (
    PlaybackControls,
    hotkey_conflicts_with_note_keys,
    parse_hotkey,
)
from sky_music.orchestration.native_admission import (
    NativeAdmissionError,
    require_rust_core,
)
from sky_music.orchestration.runtime_session import (
    RUNTIME_STATE,
    PlaybackOverrides,
)

# Imports from specialised modules
from sky_music.platform.win32 import window_target
from sky_music.ui.hud import PLAYBACK_QUIT, PLAYBACK_SKIPPED, clear_terminal
from sky_music.ui.picker import SongPickerResult
from sky_music.ui.picker_helpers import (
    SONG_DIR,
    get_song_choices,
    resolve_song_selection,
)

PLAYBACK_DEBUG = False
DEBUG_LOG_PATH = None
DEBUG_START_PERF = None
DEBUG_LOG_BUFFER = []


class _RecordExplicitAction(argparse.Action):
    """Record whether a value was supplied so config defaults cannot override it."""

    def __call__(
        self,
        parser: argparse.ArgumentParser,
        namespace: argparse.Namespace,
        values: object,
        option_string: str | None = None,
    ) -> None:
        del parser, option_string
        setattr(namespace, self.dest, values)
        setattr(namespace, f"_{self.dest}_explicit", True)


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
    # Auto-flush so a long debug session cannot retain an unbounded buffer.
    if len(DEBUG_LOG_BUFFER) >= 500:
        flush_debug_log()
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

# Bridge main.py's debug_log into the window-target diagnostics seam.
window_target._debug_log_callback = debug_log





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
        "--hold-frames",
        type=float,
        choices=HOLD_FRAME_OPTIONS,
        default=1.0,
        action=_RecordExplicitAction,
        help="Key hold length in game frames. Choices: 1.0, 1.25, 1.5. Default: 1.0. The duration is calculated from --fps.",
    )
    timing.add_argument(
        "--tempo-scale",
        type=float,
        default=1.0,
        help="Scale playback tempo: 1.2 = 20%% faster, 0.8 = 20%% slower (default: 1.0)",
    )
    timing.add_argument(
        "--scan-code-mode",
        choices=["physical", "mapped"],
        default="physical",
        help="physical = fixed QWERTY scan codes (default), mapped = OS keyboard layout",
    )
    timing.add_argument(
        "--same-key-conflict-policy",
        choices=["drop_chord", "degraded", "strict"],
        help="drop_chord = preserve chord fidelity (default), degraded = legacy partial chord, strict = reject and abort playback",
    )
    timing.add_argument(
        "--chord-stagger-us",
        type=int,
        help=(
            "Spread each chord's key-downs by this many microseconds per key so each note lands in "
            "its own game tick (mitigates remote-listener note drops on dense chords). "
            "0/unset = off (one SendInput per chord, local-optimal). Try 2000-3000 for online play."
        ),
    )
    timing.add_argument(
        "--chord-stagger-max-us",
        type=int,
        help=(
            "Cap on total intra-chord spread in microseconds (default 15000 = ~15ms, below the "
            "perceptual simultaneity threshold). Only used when --chord-stagger-us > 0."
        ),
    )
    timing.add_argument(
        "--fps",
        type=int,
        choices=VALID_FPS,
        default=None,
        metavar="FPS",
        help=(
            "FPS selected in Sky (e.g. 30, 60, 120). This value must match the FPS selected by the user inside Sky. "
            "Sky Auto Player does not auto-detect game FPS."
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

    # ── Safety & Diagnostics ──────────────────────────────────────────────────
    diag = parser.add_argument_group("Safety and diagnostics")
    diag.add_argument(
        "--version",
        action="version",
        version=f"sky-auto-player {__version__}",
        help="print version and exit",
    )
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
        "--doctor-calibrate",
        action="store_true",
        help="run input delivery latency calibration and save to .cache/input_latency.json",
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
        "--compare-holds",
        action="store_true",
        help="print the hold-frame comparison table for the selected FPS and exit",
    )
    diag.add_argument(
        "--check-update",
        action="store_true",
        help="check GitHub for a newer release, print the result and exit (no TUI)",
    )
    diag.add_argument(
        "--compare-versions",
        nargs=2,
        metavar=("CURRENT", "LATEST"),
        help="compare two version strings (PEP 440). Exit: 0=equal, 1=latest>current, 2=latest<current, 3=parse error",
    )
    diag.add_argument(
        "--no-update",
        action="store_true",
        help="suppress the launch-time automatic update check for this session "
        "(manual check via the 'u' key still works)",
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
            "analyse the most recent telemetry log and print hold-frame and tempo recommendations. "
            "Does NOT modify config.json automatically."
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
            "persist hold, tempo, and FPS defaults to config.json."
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
    from sky_music.platform.win32 import window_target
    from sky_music.ui import picker as songs
    from sky_music.ui import picker_helpers

    cfg = cfg or load_config()

    picker_helpers.SONG_DIR = args.songs_dir
    PLAYBACK_DEBUG = args.debug_playback
    window_target.PLAYBACK_DEBUG = args.debug_playback
    RUNTIME_STATE.telemetry_csv_enabled = args.debug_csv
    RUNTIME_STATE.dry_run = args.dry_run
    RUNTIME_STATE.tempo_scale = args.tempo_scale
    RUNTIME_STATE.scan_code_mode = args.scan_code_mode
    if RUNTIME_STATE.tempo_scale <= 0:
        raise ValueError("tempo_scale must be > 0")
    RUNTIME_STATE.verbose_hud = args.verbose_hud

    if PLAYBACK_DEBUG:
        init_debug_log()

    session = PlaybackSessionContext.from_cli_args(args, cfg)
    RUNTIME_STATE.apply_session(session, cfg)

    if args.sky_process_names:
        window_target.set_expected_process_names(args.sky_process_names.split(","))

    window_target.set_title_fallback(bool(args.allow_title_fallback))
    if args.theme is not None:
        songs.ACTIVE_THEME = args.theme
        songs.save_theme(args.theme)


def _run_textual_selftest() -> int:
    """Headless frozen-exe smoke test for Textual picker packaging."""
    import asyncio

    try:
        from rapidfuzz import fuzz

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

        def close(self, *, wait: bool = False) -> None:  # noqa: ARG002
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
        from sky_music.ui import picker_helpers as helpers_module
        original_get_song_choices = helpers_module.get_song_choices
        from sky_music.ui.textual_app.screens import picker as picker_module
        original_metadata = picker_module.MetadataCoordinator
        helpers_module.get_song_choices = lambda force_refresh=False: [  # noqa: ARG005
            Path("songs/Diamonds.json"),
            Path("songs/Dandelions.json"),
        ]
        picker_module.MetadataCoordinator = SelftestMetadataCoordinator # type: ignore[assignment]
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
            helpers_module.get_song_choices = original_get_song_choices
            picker_module.MetadataCoordinator = original_metadata

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


def _run_optimize_selftest() -> int:
    """Headless frozen-exe smoke test for the spec ``optimize=1`` contract.

    The PyInstaller spec at ``Sky-Auto-Player.spec:77-79`` strips docstrings
    and ``__debug__``-only blocks (``assert`` is NOT preserved at
    ``optimize>=1``). This selftest prints ``sys.flags.optimize`` and the
    ``__debug__`` flag, then exits non-zero when ``__debug__ is True`` so the
    frozen-build smoke step in ``src/build_app.py`` catches a regression
    before the release ships.

    Return values:
        0 — release build, ``__debug__`` is False (Python invoked with ``-O``
            or bytecode compiled with ``optimize>=1``). Contract holds.
        1 — dev / pytest build, ``__debug__`` is True. The freeze contract
            would not hold for a packaged binary in this state.

    The release-mode pass path is exercised by ``build_app.run_smoke_test``
    on the actual frozen binary; this unit test exercises only the
    fail-fast contract.
    """
    print(f"sys.flags.optimize: {sys.flags.optimize}")
    print(f"__debug__: {bool(__debug__)}")
    if __debug__:
        print(
            "selftest-optimize FAILED: __debug__ is True (assert statements "
            "would not be stripped in a frozen build at optimize=1).",
            file=sys.stderr,
        )
        return 1
    print("selftest-optimize OK: __debug__ is False (assert statements stripped as spec requires).")
    return 0


def _run_rust_selftest() -> int:
    """Verify production native admission with an empty native schedule."""
    frozen = bool(getattr(sys, "frozen", False))
    runtime_contract = False
    empty_session = False
    try:
        rust_build = require_rust_core()
        runtime_contract = True
        import sky_player_rs  # type: ignore[import-not-found]

        from sky_music.orchestration.native_admission import EXPECTED_NATIVE_ABI
        from sky_music.orchestration.native_models import RUST_DISPATCH_SCHEMA_VERSION

        session = sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
            [],
            [0x15],
            config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
                game_fps=60,
                min_hold_us=0,
                require_focus=False,
                telemetry=False,
                profile="production",
            ),
        )
        session.start()
        if session.join(timeout_ms=5_000) is not True:
            raise RuntimeError("native mock session did not terminate")
        snapshot = session.snapshot()
        if snapshot.get("status") != "finished":
            raise RuntimeError(f"unexpected native terminal snapshot: {snapshot!r}")
        empty_session = True
    except NativeAdmissionError as exc:
        print(f"runtime_contract={str(runtime_contract).lower()}")
        print(f"release_contract={'fail' if frozen else 'not_applicable'}")
        print("rust_selftest=fail")
        print(f"Rust selftest failed: {exc}", file=sys.stderr)
        return 1
    except Exception as exc:
        print(f"runtime_contract={str(runtime_contract).lower()}")
        print(f"release_contract={'pass' if frozen else 'not_applicable'}")
        print(f"empty_session={str(empty_session).lower()}")
        print("rust_selftest=fail")
        print(f"Rust selftest failed: {exc}", file=sys.stderr)
        return 1
    print("runtime_contract=true")
    if frozen:
        print("release_contract=true")
        print(f"application_commit={rust_build.app_build_commit}")
        print(f"native_commit={rust_build.native_build_commit}")
        print("sha_match=true")
    else:
        print("release_contract=not_applicable")
        print(f"native_commit={rust_build.native_build_commit}")
    print(
        "schema_match="
        f"{str(rust_build.schema_version == RUST_DISPATCH_SCHEMA_VERSION).lower()}"
    )
    print(f"abi_match={str(rust_build.native_abi == EXPECTED_NATIVE_ABI).lower()}")
    print(f"win32_backend={str(rust_build.win32_backend).lower()}")
    print(f"empty_session={str(empty_session).lower()}")
    print("rust_selftest=pass")
    print("Rust selftest OK: native module admitted and empty schedule terminated cleanly.")
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



def _run_check_update_command(cfg: AppConfig) -> int:
    """Headless ``--check-update`` — fetch latest release, print result, exit.

    Returns 0 if the check completed (whether or not a newer release was
    found) and 1 if the fetch itself failed (network error, rate limit,
    malformed payload). The non-zero exit on failure makes the flag usable
    in scripts that want to alert on a broken update channel.
    """
    from sky_music.orchestration.update_service import (
        check_for_update,
        record_successful_check,
    )

    print(f"Current version: v{__version__}")
    result = check_for_update(cfg, current_version=__version__)
    if result.error is not None:
        print(f"Update check failed: {result.error}")
        return 1
    record_successful_check(cfg)
    if result.update is None:
        print("Sky Auto Player is up to date.")
        return 0
    rel = result.update
    print(f"Update available: v{rel.latest_version}")
    if rel.published_at:
        print(f"  published: {rel.published_at[:10]}")
    if rel.html_url:
        print(f"  release:   {rel.html_url}")
    if rel.download_url:
        print(f"  download:  {rel.download_url}")
    if rel.sha256_url:
        print(f"  sha256:    {rel.sha256_url}")
    if rel.release_notes:
        print()
        print(rel.release_notes)
    return 0


def _run_compare_versions_command(current: str, latest: str) -> int:
    """Headless ``--compare-versions CURRENT LATEST`` — PEP 440 version compare.

    Exit codes:
      0  = equal
      1  = latest > current (newer)
      2  = latest < current (older)
      3  = parse error / invalid version
    """
    from sky_music.domain.update_checker import parse_version

    cv = parse_version(current)
    lv = parse_version(latest)
    if cv is None or lv is None:
        print(f"Error: invalid version string — current={current!r} latest={latest!r}", file=sys.stderr)
        return 3
    if lv > cv:
        return 1
    if lv < cv:
        return 2
    return 0



def prompt_song_selection(
    hold_frames: float = 1.0,
    tempo: float = 1.0,
    dry_run: bool = False,
    fps: int | None = None,
    scan_code_mode: str = "physical",
    background_mode: str | None = None,
) -> SongPickerResult | None:
    from sky_music.ui import picker as songs
    session = merge_session_with_overrides(
        RUNTIME_STATE.session or PlaybackSessionContext.default(
            tempo_scale=tempo,
            fps=fps,
            scan_code_mode=scan_code_mode,
        ),
        hold_frames=hold_frames,
        tempo=tempo,
        fps=fps,
    )
    unsupported_reason = _check_textual_support()
    if unsupported_reason is not None:
        print(
            "\n"
            "╔══════════════════════════════════════════════════════════════╗\n"
            "║        Sky Auto Player — System requirements not met        ║\n"
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
            initial_hold_frames=session.hold_frames,
            initial_tempo=session.tempo_scale,
            initial_fps=session.fps,
            initial_dry_run=dry_run,
            scan_code_mode=session.scan_code_mode,
        )
    except ImportError as exc:
        print(
            "\n"
            "╔══════════════════════════════════════════════════════════════╗\n"
            "║        Sky Auto Player — Textual UI failed to load          ║\n"
            "╠══════════════════════════════════════════════════════════════╣\n"
            f"║  Missing module: {str(exc)[:44]:<44}            ║\n"
            "║  This is a packaging error. Please report this bug.       ║\n"
            "╚══════════════════════════════════════════════════════════════╝\n",
            file=sys.stderr,
        )
        _wait_key_and_exit(2)
    except Exception as exc:
        print(
            "\n"
            "╔══════════════════════════════════════════════════════════════╗\n"
            "║    Sky Auto Player — Textual UI encountered a fatal error   ║\n"
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

    # Free-threaded-runtime fail-fast (review of main@7c548527 §3): refuses playback
    # before erecting the UI/backend if the interpreter is not a true GIL-disabled build.
    # Architecture invariant (AGENTS.md): the dispatch loop and the Textual UI thread must
    # not contend on the GIL, so a misconfigured runtime is a configuration error that
    # produces user-visible behaviour we can't reason about. Surfaced early with a banner
    # so the user gets an actionable message instead of degraded playback.
    from sky_music.infrastructure.realtime import (
        FreeThreadedRuntimeError,
        assert_free_threaded_runtime,
    )
    try:
        assert_free_threaded_runtime()
    except FreeThreadedRuntimeError as exc:
        print(
            "\n"
            "╔══════════════════════════════════════════════════════════════╗\n"
            "║   Sky Auto Player — free-threaded Python required           ║\n"
            "╠══════════════════════════════════════════════════════════════╣\n"
            f"║  {str(exc)[:60]:<60}  ║\n"
            "║  Install CPython 3.14t (free-threaded) and re-launch.       ║\n"
            "║  See docs/architecture.md for the invariant rationale.       ║\n"
            "╚══════════════════════════════════════════════════════════════╝\n",
            file=sys.stderr,
        )
        _wait_key_and_exit(2)

    if "--selftest-textual" in sys.argv:
        return _run_textual_selftest()

    if "--selftest-optimize" in sys.argv:
        return _run_optimize_selftest()

    if "--selftest-rust" in sys.argv:
        return _run_rust_selftest()

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

    if getattr(args, "no_update", False):
        RUNTIME_STATE.update_disabled = True

    if getattr(args, "check_update", False):
        return _run_check_update_command(user_cfg)

    if getattr(args, "compare_versions", None):
        current, latest = args.compare_versions
        return _run_compare_versions_command(current, latest)
    try:
        controls = build_playback_controls(args)
    except ValueError as exc:
        parser.error(str(exc))

    if args.inspect_telemetry is not None:
        from sky_music.orchestration.telemetry import inspect_telemetry_report
        inspect_telemetry_report(args.inspect_telemetry)
        return 0

    if getattr(args, "compare_holds", False):
        _print_hold_comparison_table(user_cfg, fps=args.fps)
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

    if getattr(args, "auto_calibrate", False):
        return run_auto_calibrate(getattr(args, "calibration_summary", None))

    if args.doctor or args.doctor_timing or args.doctor_input or args.doctor_calibrate:
        return run_doctor_command(
            full=bool(args.doctor),
            timing=bool(args.doctor_timing),
            input_check=bool(args.doctor_input),
            calibrate=bool(args.doctor_calibrate),
            song_path=args.song,
        )

    if args.list:
        song_choices = get_song_choices(force_refresh=True)
        print_choices_local(song_choices)
        return 0

    try:
        RUNTIME_STATE.rust_build_info = require_rust_core()
    except NativeAdmissionError as exc:
        print(
            "\nSky Auto Player cannot start playback because the Rust native core "
            f"failed admission: {exc}",
            file=sys.stderr,
        )
        return 2

    song_choices = get_song_choices(force_refresh=True)

    if not song_choices and args.song is None:
        print_choices_local(song_choices)
        _wait_key_and_exit(1)
        return 1

    try:
        # No process-wide timeBeginPeriod(1): the dispatch path uses the high-resolution
        # waitable timer (CREATE_WAITABLE_TIMER_HIGH_RESOLUTION), which does not need the global
        # 1 ms period — measured on Win11/py3.14t: high-res timer wake p99 ≈ 0.57 ms with the
        # period OFF vs 0.57 ms ON, and modern CPython's time.sleep is itself high-resolution.
        # Holding a global 1 ms period for the whole interactive session only raised the
        # system-wide timer-interrupt rate (laptop power) for no accuracy gain. The dispatch
        # thread still installs a scoped guard as a fallback ONLY when the high-res sleeper is
        # unavailable on older Windows; the native session remains fail-closed.
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
                        dry_run=RUNTIME_STATE.dry_run,
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
                RUNTIME_STATE.session or PlaybackSessionContext.default(
                    tempo_scale=RUNTIME_STATE.tempo_scale,
                    fps=resolved_fps,
                    scan_code_mode=RUNTIME_STATE.scan_code_mode,
                ),
                hold_frames=user_cfg.default_hold_frames,
                tempo=RUNTIME_STATE.tempo_scale,
                fps=resolved_fps,
            )

            try:
                return run_sky_app_unified(
                    theme_name=songs.ACTIVE_THEME,
                    background_mode=args.ui_background,
                    initial_hold_frames=session.hold_frames,
                    initial_tempo=session.tempo_scale,
                    initial_fps=session.fps,
                    initial_dry_run=RUNTIME_STATE.dry_run,
                    scan_code_mode=session.scan_code_mode,
                    controls=controls,
                    countdown_seconds=args.countdown,
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
                    hold_frames=user_cfg.default_hold_frames,
                    tempo=RUNTIME_STATE.tempo_scale,
                    dry_run=RUNTIME_STATE.dry_run,
                    fps=resolved_fps,
                    scan_code_mode=RUNTIME_STATE.scan_code_mode,
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
                    hold_frames=picker_result.hold_frames,
                    tempo=picker_result.tempo_scale,
                    fps=picker_result.fps,
                )
            )
            if result == PLAYBACK_QUIT:
                return 0
            
            # P0 Fix: Update persistent loop state with picker decision
            # (Allows picker changes to persist across multiple songs)
            updated_session = merge_session_with_overrides(
                RUNTIME_STATE.session or PlaybackSessionContext.default(
                    tempo_scale=RUNTIME_STATE.tempo_scale,
                    scan_code_mode=RUNTIME_STATE.scan_code_mode,
                ),
                hold_frames=picker_result.hold_frames,
                tempo=picker_result.tempo_scale,
                fps=picker_result.fps,
            )
            RUNTIME_STATE.apply_session(updated_session, user_cfg)
            RUNTIME_STATE.dry_run = (picker_result.action == "dry_run")

            persist_playback_defaults(
                user_cfg,
                hold_frames=updated_session.hold_frames,
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

def write_crash_log(exc: BaseException) -> None:
    import time
    import traceback
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
        print(f"\n[CRITICAL] Sky Auto Player crashed: {exc}", file=sys.stderr)
        write_crash_log(exc)
        if getattr(sys, "frozen", False):
            _wait_key_and_exit(1)
        raise
    finally:
        flush_debug_log()
