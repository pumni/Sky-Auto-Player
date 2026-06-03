import time
from concurrent.futures import Future, ProcessPoolExecutor, ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal
import shutil

from sky_music.ui.picker_theme import (
    THEME_PRESETS,
    get_theme,
    remove_accents,
    normalized_index_map,
    get_match_span,
    append_highlighted_song_name,
    truncate_text,
)
from sky_music.ui.picker_helpers import (
    SONG_DIR,
    SUPPORTED_EXTENSIONS,
    load_saved_theme,
    save_theme,
    load_song_choices,
    get_song_choices,
    resolve_song_selection,
    countdown_before_playback,
    ensure_sky_ready,
)
from sky_music.ui.picker_layout import (
    ActionHint,
    format_actions,
    _format_duration,
    build_box,
    build_header_box,
    format_song_row,
    format_info_str,
)
from sky_music.ui.picker_metadata import (
    SongUiMetadata,
    get_song_ui_metadata,
    get_cached_song_ui_metadata,
    peek_cached_song_ui_metadata,
    hydrate_persistent_metadata_for_paths,
    warm_persistent_metadata_cache,
    compute_song_ui_metadata_payloads,
    session_to_worker_payload,
    store_computed_song_ui_metadata_payloads,
    clear_metadata_cache,
    _get_song_recommendation,
)
from sky_music.domain.session_context import PlaybackSessionContext

ACTIVE_THEME: str = load_saved_theme()

try:
    from prompt_toolkit.application import Application
    from prompt_toolkit.key_binding import KeyBindings
    from prompt_toolkit.layout.containers import HSplit, Window, ConditionalContainer
    from prompt_toolkit.layout.controls import FormattedTextControl
    from prompt_toolkit.layout.layout import Layout
    from prompt_toolkit.styles import Style
    from prompt_toolkit.widgets import TextArea
    from prompt_toolkit.filters import Condition
    from prompt_toolkit.utils import get_cwidth as _get_cwidth
    HAS_PROMPT_TOOLKIT = True
except ImportError:
    _get_cwidth = None
    HAS_PROMPT_TOOLKIT = False

@dataclass
class PickerState:
    """Encapsulates the mutable state for the song picker UI."""
    song_choices: list[Path]
    selected_index: int = 0
    filtered_songs: list[Path] = None  # type: ignore
    
    current_view: Literal["picker", "preview", "profile_select", "tempo_select", "fps_select", "theme_select", "calibration", "commands", "help"] = "picker"
    previous_view: Literal["picker", "preview"] = "picker"
    
    selected_command_index: int = 0
    
    current_profile: str = "balanced"
    current_tempo: float = 1.0
    current_fps: int | None = None
    dry_run_mode: bool = False
    
    # Cached UI metrics
    term_width: int = 80
    term_height: int = 24
    results_height: int = 13
    detail_height: int = 5
    scroll_offset: int = 0
    last_query: str = ""
    last_view: str = "picker"
    metadata_prefetch_pending: bool = False
    metadata_hydration_pending: bool = False
    metadata_refresh_pending: bool = False
    metadata_generation: int = 0
    metadata_prefetch_future: Future[None] | None = None
    metadata_hydration_future: Future[int] | None = None
    active_session: Any = None  # PlaybackSessionContext
    user_cfg: Any = None # AppConfig
    song_search_keys: list[str] = None # type: ignore
    
    risk_hint: str = ""
    temp_profile: str = "balanced"
    temp_tempo: float = 1.0
    temp_fps: int | None = None
    temp_theme: str = ""

    def __post_init__(self):
        if self.filtered_songs is None:
            self.filtered_songs = list(self.song_choices)
        if self.song_search_keys is None:
            self.song_search_keys = [remove_accents(p.stem).casefold() for p in self.song_choices]

@dataclass(frozen=True, slots=True)
class SongPickerResult:
    """Carries the user's confirmed decision from the song picker."""
    song_path: Path
    action: Literal["play", "dry_run"]
    profile_name: str
    tempo_scale: float
    fps: int | None = None
    verbose_hud: bool | None = None
    telemetry_enabled: bool | None = None

PROFILES_INFO = [
    ("local-precise", "Local Precise: sharp local play, less safe for remote listeners"),
    ("balanced", "Balanced: default setting for local or online play"),
    ("audience-safe", "Audience Safe: helps online players hear notes clearly"),
    ("dense-safe", "Dense Safe: safer for fast repeats and dense songs"),
]

def get_profiles_info(fps: int | None) -> list[tuple[str, str]]:
    return list(PROFILES_INFO)

TEMPO_OPTIONS = [
    (0.90, "safer for listeners"),
    (0.95, "recommended for medium/high risk songs"),
    (1.00, "original speed"),
    (1.05, "faster"),
    (1.10, "high risk"),
]

FPS_OPTIONS = [
    (None, "Auto (No forced sync)"),
    (30, "30 FPS (Mobile/Emulator)"),
    (60, "60 FPS (Standard)"),
    (90, "90 FPS (High Performance)"),
    (120, "120 FPS (High Refresh)"),
    (144, "144 FPS (High Refresh)"),
    (165, "165 FPS (High Refresh)"),
    (240, "240 FPS (Ultra Refresh)"),
]


RESULTS_HEADER_HEIGHT = 2
UNKNOWN_FIELD = "—"


def _cell_width(text: str) -> int:
    if not text:
        return 0
    if _get_cwidth is not None:
        return max(0, _get_cwidth(text))
    return len(text)


def _truncate_cells(text: str, max_width: int) -> str:
    if max_width <= 0:
        return ""
    if max_width == 1:
        return "…"
    if _cell_width(text) <= max_width:
        return text

    out: list[str] = []
    used = 0
    limit = max_width - 1
    for char in text:
        char_width = _cell_width(char)
        if used + char_width > limit:
            break
        out.append(char)
        used += char_width
    return "".join(out) + "…"


def _pad_cells(text: str, width: int, *, align: Literal["left", "right"] = "left") -> str:
    padding = max(0, width - _cell_width(text))
    if align == "right":
        return " " * padding + text
    return text + " " * padding


def _title_column_width(terminal_width: int) -> int:
    # Fixed fields consume roughly 45 cells: pointer/index + time/notes/risk/suggested.
    return max(20, min(36, terminal_width - 45))


def _format_results_header(title_width: int) -> list[tuple[str, str]]:
    title = _pad_cells("Song Title", title_width)
    divider = "─" * title_width
    return [
        ("class:divider", f"  #   {title}    Time   Notes   Risk    Suggested\n"),
        ("class:divider", f"  ──  {divider}    ────   ─────   ─────   ───────────\n"),
    ]


def _highlighted_title_fragments(
    song_name: str,
    normalized_query: str,
    title_width: int,
    *,
    selected: bool,
) -> list[tuple[str, str]]:
    base_style = "class:selected" if selected else "class:unselected"
    truncated = _truncate_cells(song_name, title_width)
    padded_truncated = _pad_cells(truncated, title_width)
    if selected or not normalized_query:
        return [(base_style, padded_truncated)]

    span = get_match_span(truncated, normalized_query)
    if span is None:
        return [(base_style, padded_truncated)]

    start, end = span
    return [
        (base_style, truncated[:start]),
        ("class:match", truncated[start:end]),
        (base_style, _pad_cells(truncated[end:], title_width - _cell_width(truncated[:end]))),
    ]


def _format_song_row_fast(
    song_index: int,
    song_path: Path,
    metadata: SongUiMetadata | None,
    selected: bool,
    search_text: str,
    pointer: str,
    terminal_width: int,
) -> list[tuple[str, str]]:
    """Render one picker row without forcing expensive metadata analysis."""
    title_width = _title_column_width(terminal_width)
    row_style = "class:selected" if selected else "class:unselected"
    index_style = row_style if selected else "class:index"
    normalized_query = remove_accents(search_text).casefold().strip()

    title = metadata.name if metadata is not None else song_path.stem
    duration = _format_duration(metadata.duration_seconds) if metadata is not None else UNKNOWN_FIELD
    notes = str(metadata.note_count) if metadata is not None else UNKNOWN_FIELD
    risk = metadata.risk.upper() if metadata is not None else UNKNOWN_FIELD
    suggested = metadata.recommended_profile if metadata is not None else "loading"

    prefix = f"{pointer if selected else ' '} {_pad_cells(str(song_index), 2, align='right')}  "
    fragments: list[tuple[str, str]] = [
        (row_style, _pad_cells(prefix, 6)),
    ]
    fragments.extend(_highlighted_title_fragments(title, normalized_query, title_width, selected=selected))
    fragments.extend([
        (row_style, "    "),
        (row_style, _pad_cells(duration, 4, align="right")),
        (row_style, "   "),
        (row_style, _pad_cells(notes, 5, align="right")),
        (row_style, "   "),
        (row_style, _pad_cells(risk, 5)),
        (row_style, "   "),
        (row_style, _truncate_cells(suggested, 11)),
        (row_style, "\n"),
    ])
    return fragments


def _sync_scroll_offset(state: PickerState, max_visible: int) -> None:
    if not state.filtered_songs:
        state.scroll_offset = 0
        state.selected_index = 0
        return

    max_start = max(0, len(state.filtered_songs) - max_visible)
    state.selected_index = max(0, min(state.selected_index, len(state.filtered_songs) - 1))
    if state.selected_index < state.scroll_offset:
        state.scroll_offset = state.selected_index
    elif state.selected_index >= state.scroll_offset + max_visible:
        state.scroll_offset = state.selected_index - max_visible + 1
    state.scroll_offset = max(0, min(state.scroll_offset, max_start))

def safe_exit(app: Any, result: SongPickerResult | None) -> None:
    future = getattr(app, "future", None)
    if future is not None and future.done():
        return
    try:
        app.exit(result=result)
    except Exception:
        pass

def choose_song_interactively(
    theme_name: str | None = None,
    initial_profile: str = "balanced",
    initial_tempo: float = 1.0,
    initial_fps: int | None = None,
    initial_dry_run: bool = False,
    scan_code_mode: str = "physical",
) -> SongPickerResult | None:
    if not HAS_PROMPT_TOOLKIT:
        return None

    song_choices = get_song_choices(force_refresh=True)
    if not song_choices:
        return None

    from sky_music.config import (
        load_config,
        save_config,
        canonical_profile_name,
        persist_calibration_defaults,
        persist_default_fps,
        persist_default_profile,
        persist_default_tempo,
    )
    from sky_music.orchestration.calibration import (
        calibrate_profile,
        calibration_input_from_summary,
        load_latest_telemetry_summary,
    )

    user_cfg = load_config()
    state = PickerState(song_choices=song_choices, user_cfg=user_cfg)
    state.current_profile = canonical_profile_name(initial_profile)
    state.current_tempo = initial_tempo
    state.current_fps = initial_fps
    state.dry_run_mode = initial_dry_run
    state.temp_profile = state.current_profile
    state.temp_tempo = state.current_tempo
    state.temp_fps = state.current_fps

    verbose_hud_mode = user_cfg.verbose_hud
    telemetry_mode = user_cfg.telemetry_enabled_by_default

    metadata_process_executor: ProcessPoolExecutor | None = None
    metadata_thread_executor: ThreadPoolExecutor | None = None
    metadata_uses_process_pool = True
    try:
        # Keep exactly one CPU worker. The goal is not throughput; it is to move
        # parse/schedule/analyze work away from prompt_toolkit's UI thread and
        # away from the main process GIL on Windows.
        metadata_process_executor = ProcessPoolExecutor(max_workers=1)
        metadata_executor = metadata_process_executor
    except Exception:
        metadata_uses_process_pool = False
        metadata_thread_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-picker-meta")
        metadata_executor = metadata_thread_executor

    cache_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-picker-cache")
    app_ref: Any | None = None

    commands = [
        ("preview", "Song Details", "View detailed timing analysis"),
        ("profile", "Timing Profile", "Change instrument response timing"),
        ("tempo", "Adjust Tempo", "Speed up or slow down playback"),
        ("fps", "FPS Sync", "Synchronize with game frame rate"),
        ("calibration", "Calibration", "Apply recommendations from logs"),
        ("dry_run", "Toggle Dry-run", "Simulate without sending keys"),
        ("hud", "Toggle HUD", "Show/hide on-screen info"),
        ("telemetry", "Toggle Telemetry", "Enable/disable CSV logging"),
        ("reload", "Reload Songs", "Refresh songs/ directory"),
        ("theme", "Change Theme", "Switch UI color scheme"),
        ("help", "Help Guide", "View all keyboard shortcuts"),
    ]

    def picker_session() -> PlaybackSessionContext:
        return PlaybackSessionContext(
            profile_name=state.current_profile,
            tempo_scale=state.current_tempo,
            fps=state.current_fps,
            scan_code_mode=scan_code_mode,
        )

    def build_picker_result() -> SongPickerResult:
        return SongPickerResult(
            state.filtered_songs[state.selected_index],
            "dry_run" if state.dry_run_mode else "play",
            state.current_profile,
            state.current_tempo,
            state.current_fps,
            verbose_hud=verbose_hud_mode,
            telemetry_enabled=telemetry_mode,
        )

    active_theme_name, theme = get_theme(theme_name or ACTIVE_THEME)
    current_theme_name = active_theme_name
    style_dict = theme["style"]
    style = Style.from_dict(style_dict)
    pointer = theme["pointer"]
    song_icon = theme["song_icon"]
    empty_icon = theme["empty_icon"]
    theme_names = list(THEME_PRESETS.keys())

    song_indices = {path: idx for idx, path in enumerate(song_choices, start=1)}

    search_field = TextArea(
        prompt=[("class:prompt", "Search: ")],
        multiline=False,
        style="class:input",
    )
    
    is_picker_view = Condition(lambda: state.current_view == "picker")
    search_container = ConditionalContainer(search_field, filter=is_picker_view)

    header_control = FormattedTextControl(text="")
    results_control = FormattedTextControl(text="")
    detail_control = FormattedTextControl(text="")
    footer_control = FormattedTextControl(text="")

    def get_layout_heights() -> tuple[int, int]:
        term_height = state.term_height
        if state.current_view == "picker":
            overhead = 9
            available = max(0, term_height - overhead)
            if available >= 18:
                return 13, 5
            elif available >= 10:
                return available - 5, 4
            else:
                return max(3, available), 0
        elif state.current_view == "preview":
            overhead = 8
            available = max(0, term_height - overhead)
            has_warnings = False
            if state.filtered_songs:
                metadata = peek_cached_song_ui_metadata(state.filtered_songs[state.selected_index], state.active_session, state.user_cfg)
                if metadata is not None and metadata.risk != "low":
                    has_warnings = True
            if not has_warnings:
                return min(11, available), 0
            if available >= 16:
                return 11, 5
            elif available >= 13:
                return 10, 3
            else:
                return max(3, available), 0
        elif state.current_view in {"profile_select", "tempo_select", "fps_select", "theme_select", "calibration", "commands", "help"}:
            overhead = 8
            available = max(0, term_height - overhead)
            if state.current_view == "profile_select": return min(len(get_profiles_info(state.current_fps)) + 2, available), 0
            if state.current_view == "tempo_select": return min(len(TEMPO_OPTIONS) + 4, available), 0
            if state.current_view == "fps_select": return min(len(FPS_OPTIONS) + 4, available), 0
            if state.current_view == "theme_select": return min(len(theme_names) + 4, available), 0
            if state.current_view == "calibration": return min(10, available), 0
            if state.current_view == "commands": return min(len(commands) + 2, available), 0
            if state.current_view == "help": return min(17, available), 0
        return 13, 5

    def get_results_height() -> int: return state.results_height
    def get_detail_height() -> int: return state.detail_height

    header_window = Window(content=header_control, height=3)
    results_window = Window(content=results_control, height=get_results_height, style="class:results")
    detail_window = Window(content=detail_control, height=get_detail_height, style="class:detail")
    footer_window = Window(content=footer_control, height=7, style="class:footer")

    layout = Layout(
        HSplit([
            header_window,
            search_container,
            results_window,
            detail_window,
            footer_window,
        ])
    )

    kb = KeyBindings()

    def build_header_text() -> list[tuple[str, str]]:
        terminal_width = state.term_width

        mode_label = {
            "picker": "Picker", "preview": "Preview", "profile_select": "Profile Selection",
            "tempo_select": "Tempo Adjustment", "fps_select": "FPS Selection",
            "calibration": "Calibration", "commands": "Commands", "help": "Help Guide"
        }.get(state.current_view, "Picker")

        dry_str = "ON" if state.dry_run_mode else "OFF"
        hud_str = "ON" if verbose_hud_mode else "OFF"
        fps_str = str(state.current_fps) if state.current_fps else "Auto"
        telem_str = "ON" if telemetry_mode else "OFF"

        parts = [
            mode_label, f"profile: {state.current_profile}", f"tempo: {state.current_tempo:.2f}x",
            f"fps: {fps_str}", f"dry: {dry_str}", f"hud: {hud_str}", f"telem: {telem_str}",
            f"theme: {current_theme_name}", f"songs: {len(state.song_choices)}",
        ]
        return build_header_box("SKY MUSIC PLAYER", parts, terminal_width)

    def build_results_text() -> list[tuple[str, str]]:
        terminal_width = state.term_width
        if state.current_view == "picker":
            title_width = _title_column_width(terminal_width)
            lines = _format_results_header(title_width)
            
            if not state.filtered_songs:
                lines.append(("class:empty", f"  {empty_icon} No songs found\n"))
                return lines

            max_visible = max(1, state.results_height - RESULTS_HEADER_HEIGHT)
            _sync_scroll_offset(state, max_visible)
            start_idx = state.scroll_offset
            end_idx = min(len(state.filtered_songs), start_idx + max_visible)
                
            for idx in range(start_idx, end_idx):
                path = state.filtered_songs[idx]
                orig_idx = song_indices.get(path, idx + 1)
                is_selected = idx == state.selected_index
                metadata = peek_cached_song_ui_metadata(path, state.active_session, state.user_cfg)
                lines.extend(
                    _format_song_row_fast(
                        orig_idx,
                        path,
                        metadata,
                        is_selected,
                        search_field.text.strip(),
                        pointer,
                        terminal_width,
                    )
                )
            return lines
            
        elif state.current_view == "preview":
            if not state.filtered_songs:
                return []
            selected_path = state.filtered_songs[state.selected_index]
            metadata = peek_cached_song_ui_metadata(selected_path, state.active_session, state.user_cfg)
            if metadata is None:
                return build_box(
                    "Song Detail",
                    [
                        selected_path.stem,
                        "Metadata is loading…",
                        "Esc to return to picker.",
                    ],
                    width=terminal_width,
                )

            preview_content = [
                f"{metadata.name}",
                f"Time {_format_duration(metadata.duration_seconds)} │ Notes {metadata.note_count} │ Polyphony {metadata.max_polyphony}",
                f"Risk {metadata.risk.upper()} │ Stress: {metadata.timing_stress_rate:.1f}% ({metadata.impossible_repeats} conflicts)",
                f"Min repeat gap: {metadata.min_same_key_gap_ms:.0f}ms │ Peak density: {metadata.peak_notes_per_second_1s:.1f} n/s",
            ]

            timing_content = [
                f"Current:   {state.current_profile} @ {state.current_tempo:.2f}x",
                f"Suggested: {metadata.recommended_profile} @ {metadata.recommended_tempo_scale:.2f}x",
                f"FPS Sync:  {state.current_fps or 'Auto'}"
            ]
            return build_box("Song Detail", preview_content, width=terminal_width) + build_box("Timing Settings", timing_content, width=terminal_width)
            
        elif state.current_view == "profile_select":
            content = []
            for name, desc in get_profiles_info(state.current_fps):
                bullet = "●" if name == state.current_profile else "○"
                row = f"{bullet} {name:<15}   {desc}"
                content.append([("class:selected" if name == state.temp_profile else "class:unselected", f" {'➜' if name == state.temp_profile else ' '} {row}")])
            return build_box("Select Timing Profile", content, width=terminal_width)
            
        elif state.current_view == "tempo_select":
            content = [f"Adjust: {state.temp_tempo:.2f}x", ""]
            for val, desc in TEMPO_OPTIONS:
                bullet = "●" if abs(val - state.current_tempo) < 0.005 else "○"
                row = f"{bullet} {val:.2f}x   {desc}"
                content.append([("class:selected" if abs(val - state.temp_tempo) < 0.005 else "class:unselected", f" {'➜' if abs(val - state.temp_tempo) < 0.005 else ' '} {row}")])
            return build_box("Adjust Tempo", content, width=terminal_width)

        elif state.current_view == "fps_select":
            content = [f"Target: {state.temp_fps if state.temp_fps else 'Auto'}", ""]
            fps_vals = [f[0] for f in FPS_OPTIONS]
            if state.temp_fps in fps_vals:
                hover_idx = fps_vals.index(state.temp_fps)
            else:
                temp_val = state.temp_fps if state.temp_fps is not None else 60
                hover_idx = min(range(len(fps_vals)), key=lambda i: abs((fps_vals[i] if fps_vals[i] is not None else 60) - temp_val))
            
            for i, (val, desc) in enumerate(FPS_OPTIONS):
                bullet = "●" if val == state.current_fps else "○"
                val_str = str(val) if val else "Auto"
                row = f"{bullet} {val_str:<4}   {desc}"
                is_hover = (i == hover_idx)
                content.append([("class:selected" if is_hover else "class:unselected", f" {'➜' if is_hover else ' '} {row}")])
            return build_box("FPS Sync Selection", content, width=terminal_width)

        elif state.current_view == "theme_select":
            content = [f"Current: {current_theme_name}", ""]
            for name in theme_names:
                bullet = "●" if name == current_theme_name else "○"
                row = f"{bullet} {name}"
                content.append([("class:selected" if name == state.temp_theme else "class:unselected", f" {'➜' if name == state.temp_theme else ' '} {row}")])
            return build_box("Select Theme", content, width=terminal_width)

        elif state.current_view == "calibration":
            summary = load_latest_telemetry_summary()
            if summary is None:
                return build_box(
                    "Telemetry Calibration",
                    ["No telemetry summary found in logs/.", "Run playback with --debug-csv first."],
                    width=terminal_width,
                )
            inp = calibration_input_from_summary(summary)
            rec = calibrate_profile(inp)
            content = [
                f"Latest: {summary.get('song', 'Unknown')} @ {inp.fps} FPS",
                f"Profile: {inp.profile_name} -> {rec.profile_name}",
                f"Tempo:   {inp.tempo_scale:.2f}x -> {rec.tempo_scale:.2f}x",
                f"Hold:    {rec.hold_us / 1000:.1f}ms",
                f"Severity {rec.severity.upper()}",
                rec.reason,
            ]
            return build_box("Telemetry Calibration", content, width=terminal_width)

        elif state.current_view == "commands":
            content = []
            for idx, (cmd_id, label, desc) in enumerate(commands):
                is_sel = idx == state.selected_command_index
                row = f"{label:<15}   {desc}"
                content.append([("class:selected" if is_sel else "class:unselected", f" {'➜' if is_sel else ' '} {row}")])
            return build_box("Command Palette", content, width=terminal_width)

        elif state.current_view == "help":
            help_lines = [
                ("/", "Open Command Palette"),
                ("Enter", "Play selected song or Confirm selection"),
                ("Up/Down", "Navigate lists"),
                ("Esc", "Back to Picker or Quit"),
                ("F1 or ?", "Show this Help Guide"),
                ("F2", "Toggle verbose HUD in game"),
                ("F3", "Toggle telemetry logging"),
                ("Ctrl+R", "Reload songs from disk"),
                ("Ctrl+T", "Cycle through themes"),
                ("Ctrl+C", "Force Quit"),
            ]
            content = [[("class:key", f"  {k:<12}"), ("class:detail", d)] for k, d in help_lines]
            return build_box("Keyboard Shortcuts", content, width=terminal_width)
        return []

    def build_detail_text() -> list[tuple[str, str]]:
        terminal_width = state.term_width
        d_height = state.detail_height
        if d_height == 0 or not state.filtered_songs:
            return []

        selected_path = state.filtered_songs[state.selected_index]
        metadata = peek_cached_song_ui_metadata(selected_path, state.active_session, state.user_cfg)
        if metadata is None:
            return build_box(
                "Selected",
                [
                    selected_path.stem,
                    "Metadata is loading…",
                ],
                width=terminal_width,
            )

        content = [metadata.name]
        if d_height >= 6:
            content.append(
                f"Time {_format_duration(metadata.duration_seconds)} │ "
                f"Notes {metadata.note_count} │ Risk {metadata.risk.upper()}"
            )
            content.append(f"Poly: {metadata.max_polyphony} │ Gap: {metadata.min_same_key_gap_ms:.0f}ms")
            content.append(
                f"Density: {metadata.average_notes_per_second:.1f}/s "
                f"(peak {metadata.peak_notes_per_second_1s:.1f}/s)"
            )
        elif d_height >= 4:
            content.append(
                f"Time {_format_duration(metadata.duration_seconds)} │ Notes {metadata.note_count} │ "
                f"Risk {metadata.risk.upper()}"
            )
            content.append(f"Poly: {metadata.max_polyphony} │ Density: {metadata.average_notes_per_second:.1f}/s")
        else:
            content.append(
                f"Time {_format_duration(metadata.duration_seconds)} │ Notes {metadata.note_count} │ "
                f"Risk {metadata.risk.upper()}"
            )
        return build_box("Selected", content, width=terminal_width)

    def build_footer_text() -> list[tuple[str, str]]:
        terminal_width = state.term_width
        if state.current_view in {"picker", "preview"}:
            if not state.filtered_songs:
                return []

            meta = peek_cached_song_ui_metadata(state.filtered_songs[state.selected_index], state.active_session, state.user_cfg)
            if meta is None:
                line1 = [("class:detail", "Metadata loading…")]
            else:
                risk_style = (
                    "fg:#ef4444 bold"
                    if meta.risk == "error"
                    else ("fg:#f97316 bold" if meta.risk == "high" else ("fg:#fbbf24 bold" if meta.risk == "medium" else "fg:#10b981"))
                )
                line1 = [(risk_style, f"{meta.risk.upper()} risk: "), ("class:detail", f"Suggested {meta.recommended_profile} @ {meta.recommended_tempo_scale:.2f}x")]
            
            actions = [
                ActionHint("Enter", "play", "play", "play"),
                ActionHint("/", "commands", "cmd", "/"),
                ActionHint("F2", "HUD", "hud", "h2"),
                ActionHint("F3", "telemetry", "telem", "h3"),
                ActionHint("^R", "reload", "rl", "rl"),
                ActionHint("^T", "theme", "theme", "th"),
                ActionHint("Esc", "quit", "quit", "q"),
            ]
            row_w = terminal_width - 4
            action_rows = [
                format_actions(actions[0:4], row_w),
                format_actions(actions[4:], row_w),
            ]
            return build_box("Actions", [line1, *action_rows], width=terminal_width)
        return build_box("Navigation", [[("class:footer", "Arrow keys to choose │ Enter to apply │ Esc to back")]], width=terminal_width)

    def _invalidate_metadata_work(*, cancel_pending: bool = True) -> None:
        """Mark already scheduled metadata work stale.

        Running analysis cannot be force-stopped safely, but the batch worker checks
        this generation between songs so old search/selection batches do not keep
        filling the executor queue.
        """
        state.metadata_generation += 1
        state.metadata_prefetch_pending = False
        state.metadata_hydration_pending = False
        if cancel_pending and state.metadata_prefetch_future is not None and not state.metadata_prefetch_future.done():
            state.metadata_prefetch_future.cancel()
        if cancel_pending and state.metadata_hydration_future is not None and not state.metadata_hydration_future.done():
            state.metadata_hydration_future.cancel()

    def _visible_picker_paths() -> list[Path]:
        if state.current_view != "picker" or not state.filtered_songs:
            return []
        max_visible = max(1, state.results_height - RESULTS_HEADER_HEIGHT)
        _sync_scroll_offset(state, max_visible)
        start_idx = state.scroll_offset
        end_idx = min(len(state.filtered_songs), start_idx + max_visible)
        return list(state.filtered_songs[start_idx:end_idx])

    def _metadata_paths_for_current_view() -> list[Path]:
        if not state.filtered_songs or not (0 <= state.selected_index < len(state.filtered_songs)):
            return []

        selected_path = state.filtered_songs[state.selected_index]
        if state.current_view == "preview":
            return [selected_path]
        if state.current_view != "picker":
            return []

        visible_paths = _visible_picker_paths()
        # Selected first makes the detail/action boxes update before lower-priority rows.
        return [selected_path, *[path for path in visible_paths if path != selected_path]]

    def _schedule_coalesced_metadata_refresh(app: Any, generation: int, delay: float = 0.035) -> None:
        if state.metadata_refresh_pending or getattr(app, "is_done", False):
            return
        state.metadata_refresh_pending = True

        def refresh() -> None:
            state.metadata_refresh_pending = False
            if getattr(app, "is_done", False) or generation != state.metadata_generation:
                return
            update_ui(prefetch_metadata=False)
            app.invalidate()

        loop = getattr(app, "loop", None)
        if loop is None:
            refresh()
            return
        try:
            loop.call_later(delay, refresh)
        except RuntimeError:
            state.metadata_refresh_pending = False

    def _metadata_worker_payload(paths: list[Path], session: PlaybackSessionContext) -> tuple[list[str], dict[str, Any]]:
        return [str(path) for path in paths], session_to_worker_payload(session)

    def request_visible_metadata_prefetch() -> None:
        app = app_ref
        if app is None or state.current_view not in {"picker", "preview"} or not state.filtered_songs:
            return

        session = state.active_session or picker_session()
        paths = [
            path
            for path in _metadata_paths_for_current_view()
            if peek_cached_song_ui_metadata(path, session, state.user_cfg) is None
        ]
        if not paths:
            return

        current_future = state.metadata_prefetch_future
        if current_future is not None and not current_future.done():
            # Keep at most one batch queued/running. It will exit early if stale.
            return

        generation = state.metadata_generation
        worker_paths, worker_session = _metadata_worker_payload(paths, session)
        future = metadata_executor.submit(
            compute_song_ui_metadata_payloads,
            worker_paths,
            worker_session,
            state.user_cfg,
        )
        state.metadata_prefetch_future = future

        def on_done(
            done_future: Future[list[dict[str, Any]]],
            done_generation: int = generation,
            done_session: PlaybackSessionContext = session,
            done_app: Any = app,
        ) -> None:
            if state.metadata_prefetch_future is done_future:
                state.metadata_prefetch_future = None

            if done_future.cancelled():
                return

            stored_count = 0
            try:
                payloads = done_future.result()
                stored_count = store_computed_song_ui_metadata_payloads(payloads, done_session, state.user_cfg)
            except Exception:
                # If a process worker breaks, keep the picker usable by falling back
                # to the thread backend for later batches. The UI still remains
                # non-blocking; only the metadata backend changes.
                nonlocal metadata_uses_process_pool, metadata_process_executor, metadata_thread_executor, metadata_executor
                if metadata_uses_process_pool:
                    metadata_uses_process_pool = False
                    try:
                        if metadata_process_executor is not None:
                            metadata_process_executor.shutdown(wait=False, cancel_futures=True)
                    except Exception:
                        pass
                    metadata_process_executor = None
                    if metadata_thread_executor is None:
                        metadata_thread_executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-picker-meta")
                    metadata_executor = metadata_thread_executor

            if getattr(done_app, "is_done", False):
                return

            loop = getattr(done_app, "loop", None)
            if loop is None:
                if done_generation == state.metadata_generation and stored_count > 0:
                    _schedule_coalesced_metadata_refresh(done_app, done_generation)
                return

            def finish_on_ui_thread() -> None:
                if getattr(done_app, "is_done", False):
                    return
                if done_generation == state.metadata_generation:
                    if stored_count > 0:
                        _schedule_coalesced_metadata_refresh(done_app, done_generation)
                else:
                    # A newer search/selection/view exists. Kick one fresh batch instead
                    # of repainting stale data.
                    schedule_visible_metadata_prefetch(done_app, delay=0.01)

            try:
                loop.call_soon_threadsafe(finish_on_ui_thread)
            except RuntimeError:
                return

        future.add_done_callback(on_done)

    def schedule_visible_metadata_prefetch(app: Any | None = None, delay: float = 0.08) -> None:
        """Start expensive metadata work after the UI has had a chance to repaint."""
        target_app = app or app_ref
        if target_app is None or state.current_view not in {"picker", "preview"} or state.metadata_prefetch_pending:
            return

        state.metadata_prefetch_pending = True
        generation = state.metadata_generation

        def run_prefetch() -> None:
            state.metadata_prefetch_pending = False
            if (
                getattr(target_app, "is_done", False)
                or generation != state.metadata_generation
                or state.current_view not in {"picker", "preview"}
            ):
                return
            request_visible_metadata_prefetch()

        loop = getattr(target_app, "loop", None)
        if loop is None:
            run_prefetch()
            return

        try:
            loop.call_later(delay, run_prefetch)
        except RuntimeError:
            state.metadata_prefetch_pending = False

    def request_visible_persistent_hydration(app: Any | None = None) -> None:
        """Hydrate disk-cached metadata for only the visible/selected rows first.

        This is intentionally separate from expensive metadata analysis. The
        cache worker does tiny SQLite reads, while the metadata worker can spend
        time parsing/scheduling/analyzing cache misses.
        """
        target_app = app or app_ref
        if target_app is None or state.current_view not in {"picker", "preview"} or not state.filtered_songs:
            return

        current_future = state.metadata_hydration_future
        if current_future is not None and not current_future.done():
            return

        session = state.active_session or picker_session()
        paths = _metadata_paths_for_current_view()
        if not paths:
            return

        generation = state.metadata_generation
        future = cache_executor.submit(hydrate_persistent_metadata_for_paths, paths, session, state.user_cfg)
        state.metadata_hydration_future = future

        def on_done(done_future: Future[int], done_generation: int = generation, done_app: Any = target_app) -> None:
            if state.metadata_hydration_future is done_future:
                state.metadata_hydration_future = None

            if done_future.cancelled() or getattr(done_app, "is_done", False):
                return
            try:
                loaded_count = done_future.result()
            except Exception:
                loaded_count = 0

            loop = getattr(done_app, "loop", None)

            def finish_on_ui_thread() -> None:
                if getattr(done_app, "is_done", False) or done_generation != state.metadata_generation:
                    return
                if loaded_count > 0:
                    _schedule_coalesced_metadata_refresh(done_app, done_generation, delay=0.0)
                # After cheap disk hydration, compute only the true misses.
                schedule_visible_metadata_prefetch(done_app, delay=0.02)

            if loop is None:
                finish_on_ui_thread()
                return
            try:
                loop.call_soon_threadsafe(finish_on_ui_thread)
            except RuntimeError:
                return

        future.add_done_callback(on_done)

    def schedule_visible_persistent_hydration(app: Any | None = None, delay: float = 0.03) -> None:
        target_app = app or app_ref
        if target_app is None or state.current_view not in {"picker", "preview"} or state.metadata_hydration_pending:
            return

        state.metadata_hydration_pending = True
        generation = state.metadata_generation

        def run_hydration() -> None:
            state.metadata_hydration_pending = False
            if (
                getattr(target_app, "is_done", False)
                or generation != state.metadata_generation
                or state.current_view not in {"picker", "preview"}
            ):
                return
            request_visible_persistent_hydration(target_app)

        loop = getattr(target_app, "loop", None)
        if loop is None:
            run_hydration()
            return

        try:
            loop.call_later(delay, run_hydration)
        except RuntimeError:
            state.metadata_hydration_pending = False

    def start_persistent_metadata_warmup(app: Any) -> None:
        """Warm disk-backed metadata cache without blocking the first paint.

        This lets repeated picker sessions reuse previous analysis results. The
        UI can render immediately; once the persistent cache is loaded, we
        repaint once and then only compute metadata that is truly missing.
        """
        if getattr(app, "is_done", False):
            return

        future = cache_executor.submit(warm_persistent_metadata_cache)

        def on_done(done_future: Future[int]) -> None:
            try:
                done_future.result()
            except Exception:
                return
            if getattr(app, "is_done", False):
                return

            loop = getattr(app, "loop", None)
            if loop is None:
                update_ui(prefetch_metadata=False)
                schedule_visible_metadata_prefetch(app, delay=0.01)
                return

            def refresh_after_warmup() -> None:
                if getattr(app, "is_done", False):
                    return
                update_ui(prefetch_metadata=False)
                app.invalidate()
                schedule_visible_metadata_prefetch(app, delay=0.01)

            try:
                loop.call_soon_threadsafe(refresh_after_warmup)
            except RuntimeError:
                return

        future.add_done_callback(on_done)

    def update_ui(force_filter: bool = False, *, prefetch_metadata: bool = True, prefetch_delay: float = 0.08):
        start = time.perf_counter()

        # 1. Update terminal metrics. Compare against the clamped width, otherwise
        # terminals wider than the clamp make term_changed=True on every keypress.
        size = shutil.get_terminal_size((80, 24))
        next_width = max(60, min(100, size.columns))
        next_height = size.lines
        term_changed = state.term_width != next_width or state.term_height != next_height
        if term_changed:
            state.term_width = next_width
            state.term_height = next_height
        
        # 2. Update session context
        new_session = PlaybackSessionContext(
            profile_name=state.current_profile,
            tempo_scale=state.current_tempo,
            fps=state.current_fps,
            scan_code_mode=scan_code_mode,
        )
        session_changed = state.active_session != new_session
        if session_changed:
            state.active_session = new_session

        view_changed = state.last_view != state.current_view

        # 3. Filter only when the query/session actually changed.
        query = remove_accents(search_field.text).casefold().strip()
        query_changed = query != state.last_query
        should_filter = force_filter or query_changed or (state.current_view == "picker" and session_changed)
        metadata_context_changed = session_changed or view_changed or query_changed or force_filter
        if metadata_context_changed:
            _invalidate_metadata_work()

        if should_filter:
            previous_selected = (
                state.filtered_songs[state.selected_index]
                if state.filtered_songs and 0 <= state.selected_index < len(state.filtered_songs)
                else None
            )
            state.filtered_songs = [
                path
                for idx, path in enumerate(state.song_choices)
                if query in state.song_search_keys[idx]
            ]
            if not state.filtered_songs:
                state.selected_index = 0
                state.scroll_offset = 0
            elif query_changed:
                state.selected_index = 0
                state.scroll_offset = 0
            elif previous_selected in state.filtered_songs:
                state.selected_index = state.filtered_songs.index(previous_selected)
            else:
                state.selected_index = max(0, min(state.selected_index, len(state.filtered_songs) - 1))
            state.last_query = query

        # 4. Update layout heights when the view/terminal/session changes.
        if term_changed or session_changed or view_changed or force_filter:
            state.results_height, state.detail_height = get_layout_heights()

        # 5. Render to controls
        header_control.text = build_header_text()
        results_control.text = build_results_text()
        detail_control.text = build_detail_text()
        footer_control.text = build_footer_text()
        state.last_view = state.current_view

        if state.current_view in {"picker", "preview"} and prefetch_metadata:
            schedule_visible_persistent_hydration(delay=prefetch_delay)
        
        elapsed = time.perf_counter() - start
        if elapsed > 0.05:
            with open("ui_profile.log", "a", encoding="utf-8") as f:
                f.write(f"update_ui {state.current_view} took {elapsed:.4f}s\n")

    def _on_search_changed(_: Any) -> None:
        # Render the filtered list immediately, but delay metadata warmup so
        # typing remains responsive.
        update_ui(force_filter=True, prefetch_metadata=False)
        schedule_visible_persistent_hydration(delay=0.08)

    search_field.buffer.on_text_changed += _on_search_changed

    @kb.add("up")
    def _(event):
        if state.current_view == "picker" and state.filtered_songs:
            state.selected_index = (state.selected_index - 1) % len(state.filtered_songs)
            _invalidate_metadata_work()
        elif state.current_view == "commands":
            state.selected_command_index = (state.selected_command_index - 1) % len(commands)
        elif state.current_view == "profile_select":
            profiles = [p[0] for p in get_profiles_info(state.current_fps)]
            idx = profiles.index(state.temp_profile) if state.temp_profile in profiles else 0
            state.temp_profile = profiles[(idx - 1) % len(profiles)]
        elif state.current_view == "tempo_select":
            presets = [t[0] for t in TEMPO_OPTIONS]
            idx = min(range(len(presets)), key=lambda i: abs(presets[i] - state.temp_tempo))
            state.temp_tempo = presets[(idx - 1) % len(presets)]
        elif state.current_view == "fps_select":
            fps = [f[0] for f in FPS_OPTIONS]
            if state.temp_fps in fps:
                idx = fps.index(state.temp_fps)
            else:
                temp_val = state.temp_fps if state.temp_fps is not None else 60
                idx = min(range(len(fps)), key=lambda i: abs((fps[i] if fps[i] is not None else 60) - temp_val))
            state.temp_fps = fps[(idx - 1) % len(fps)]
        elif state.current_view == "theme_select":
            idx = theme_names.index(state.temp_theme) if state.temp_theme in theme_names else 0
            state.temp_theme = theme_names[(idx - 1) % len(theme_names)]
        update_ui()

    @kb.add("down")
    def _(event):
        if state.current_view == "picker" and state.filtered_songs:
            state.selected_index = (state.selected_index + 1) % len(state.filtered_songs)
            _invalidate_metadata_work()
        elif state.current_view == "commands":
            state.selected_command_index = (state.selected_command_index + 1) % len(commands)
        elif state.current_view == "profile_select":
            profiles = [p[0] for p in get_profiles_info(state.current_fps)]
            idx = profiles.index(state.temp_profile) if state.temp_profile in profiles else 0
            state.temp_profile = profiles[(idx + 1) % len(profiles)]
        elif state.current_view == "tempo_select":
            presets = [t[0] for t in TEMPO_OPTIONS]
            idx = min(range(len(presets)), key=lambda i: abs(presets[i] - state.temp_tempo))
            state.temp_tempo = presets[(idx + 1) % len(presets)]
        elif state.current_view == "fps_select":
            fps = [f[0] for f in FPS_OPTIONS]
            if state.temp_fps in fps:
                idx = fps.index(state.temp_fps)
            else:
                temp_val = state.temp_fps if state.temp_fps is not None else 60
                idx = min(range(len(fps)), key=lambda i: abs((fps[i] if fps[i] is not None else 60) - temp_val))
            state.temp_fps = fps[(idx + 1) % len(fps)]
        elif state.current_view == "theme_select":
            idx = theme_names.index(state.temp_theme) if state.temp_theme in theme_names else 0
            state.temp_theme = theme_names[(idx + 1) % len(theme_names)]
        update_ui()

    @kb.add("/")
    def _(event):
        if state.current_view in {"picker", "preview"} and not search_field.text.strip():
            state.previous_view, state.current_view = state.current_view, "commands"
            state.selected_command_index = 0
            update_ui()
        else:
            event.app.current_buffer.insert_text("/")

    @kb.add("c-r")
    def _(event):
        if state.current_view == "picker":
            clear_metadata_cache()
            _invalidate_metadata_work()
            state.song_choices = get_song_choices(force_refresh=True)
            state.song_search_keys = [remove_accents(p.stem).casefold() for p in state.song_choices]
            song_indices.clear()
            song_indices.update({path: idx for idx, path in enumerate(state.song_choices, start=1)})
            state.filtered_songs = list(state.song_choices)
            state.selected_index = 0
            state.scroll_offset = 0
            update_ui(force_filter=True)

    @kb.add("f2")
    def _(event):
        nonlocal verbose_hud_mode
        if state.current_view == "picker":
            verbose_hud_mode = not verbose_hud_mode
            user_cfg.verbose_hud = verbose_hud_mode
            save_config(user_cfg)
            update_ui()

    @kb.add("f3")
    def _(event):
        nonlocal telemetry_mode
        if state.current_view == "picker":
            telemetry_mode = not telemetry_mode
            user_cfg.telemetry_enabled_by_default = telemetry_mode
            save_config(user_cfg)
            update_ui()

    @kb.add("c-t")
    def _(event):
        global ACTIVE_THEME
        nonlocal active_theme_name, current_theme_name, style_dict, style, pointer, song_icon, empty_icon
        if not theme_names:
            return
        try:
            current_index = theme_names.index(current_theme_name)
        except ValueError:
            current_index = -1
        next_theme = theme_names[(current_index + 1) % len(theme_names)]
        ACTIVE_THEME = next_theme
        save_theme(next_theme)
        active_theme_name, next_theme_data = get_theme(next_theme)
        current_theme_name = active_theme_name
        style_dict = next_theme_data["style"]
        style = Style.from_dict(style_dict)
        pointer = next_theme_data["pointer"]
        song_icon = next_theme_data["song_icon"]
        empty_icon = next_theme_data["empty_icon"]
        try:
            event.app.style = style
        except Exception:
            pass
        update_ui()

    @kb.add("c-c")
    def _(event):
        safe_exit(event.app, None)

    @kb.add("f1")
    def _(event):
        state.previous_view, state.current_view = state.current_view, "help" if state.current_view != "help" else state.previous_view
        update_ui()

    @kb.add("?")
    def _(event):
        if state.current_view in {"picker", "preview"} and not search_field.text.strip():
            state.previous_view, state.current_view = state.current_view, "help"
            update_ui()
        else:
            event.app.current_buffer.insert_text("?")

    @kb.add("enter")
    def _(event):
        global ACTIVE_THEME
        nonlocal active_theme_name, current_theme_name, style_dict, style, pointer, song_icon, empty_icon
        if state.current_view in {"picker", "preview"}:
            if state.filtered_songs:
                safe_exit(event.app, build_picker_result())
        elif state.current_view == "commands":
            cmd_id = commands[state.selected_command_index][0]
            if cmd_id == "preview":
                state.current_view = "preview"
            elif cmd_id == "profile":
                state.previous_view, state.current_view, state.temp_profile = "picker", "profile_select", state.current_profile
            elif cmd_id == "tempo":
                state.previous_view, state.current_view, state.temp_tempo = "picker", "tempo_select", state.current_tempo
            elif cmd_id == "fps":
                state.previous_view, state.current_view, state.temp_fps = "picker", "fps_select", state.current_fps
            elif cmd_id == "calibration":
                state.previous_view, state.current_view = "picker", "calibration"
            elif cmd_id == "dry_run":
                state.dry_run_mode = not state.dry_run_mode
                state.current_view = "picker"
            elif cmd_id == "hud":
                nonlocal verbose_hud_mode
                verbose_hud_mode = not verbose_hud_mode
                user_cfg.verbose_hud = verbose_hud_mode
                save_config(user_cfg)
                state.current_view = "picker"
            elif cmd_id == "telemetry":
                nonlocal telemetry_mode
                telemetry_mode = not telemetry_mode
                user_cfg.telemetry_enabled_by_default = telemetry_mode
                save_config(user_cfg)
                state.current_view = "picker"
            elif cmd_id == "reload":
                clear_metadata_cache()
                state.song_choices = get_song_choices(force_refresh=True)
                state.song_search_keys = [remove_accents(p.stem).casefold() for p in state.song_choices]
                song_indices.clear()
                song_indices.update({path: idx for idx, path in enumerate(state.song_choices, start=1)})
                state.filtered_songs = list(state.song_choices)
                state.selected_index = 0
                state.scroll_offset = 0
                state.current_view = "picker"
            elif cmd_id == "theme":
                state.previous_view, state.current_view, state.temp_theme = "picker", "theme_select", current_theme_name
            elif cmd_id == "help":
                state.previous_view, state.current_view = "picker", "help"
            update_ui()
        elif state.current_view == "profile_select":
            state.current_profile, state.current_view = state.temp_profile, "picker"
            try:
                persist_default_profile(load_config(), state.current_profile)
            except Exception:
                pass
        elif state.current_view == "tempo_select":
            state.current_tempo, state.current_view = state.temp_tempo, "picker"
            try:
                persist_default_tempo(load_config(), state.current_tempo)
            except Exception:
                pass
        elif state.current_view == "fps_select":
            state.current_fps, state.current_view = state.temp_fps, "picker"
            try:
                persist_default_fps(load_config(), state.current_fps)
            except Exception:
                pass
        elif state.current_view == "theme_select":
            next_theme = state.temp_theme
            ACTIVE_THEME = next_theme
            save_theme(next_theme)
            active_theme_name, next_theme_data = get_theme(next_theme)
            current_theme_name = active_theme_name
            style_dict = next_theme_data["style"]
            style = Style.from_dict(style_dict)
            pointer = next_theme_data["pointer"]
            song_icon = next_theme_data["song_icon"]
            empty_icon = next_theme_data["empty_icon"]
            try: event.app.style = style
            except Exception: pass
            state.current_view = "picker"
        elif state.current_view == "calibration":
            summary = load_latest_telemetry_summary()
            if summary is not None:
                try:
                    inp = calibration_input_from_summary(summary)
                    rec = calibrate_profile(inp)
                    persist_calibration_defaults(
                        load_config(),
                        profile_name=rec.profile_name,
                        tempo_scale=rec.tempo_scale,
                        fps=inp.fps,
                    )
                    state.current_profile = canonical_profile_name(rec.profile_name)
                    state.current_tempo = rec.tempo_scale
                    state.current_fps = inp.fps if inp.fps > 0 else None
                except Exception:
                    pass
            state.current_view = "picker"
        update_ui()

    @kb.add("escape")
    def _(event):
        if state.current_view == "picker":
            safe_exit(event.app, None)
            return
        state.current_view = "picker"
        # Do not start metadata work synchronously on the back-transition path.
        # First repaint the picker, then warm the cache shortly after.
        update_ui(prefetch_metadata=False)
        event.app.invalidate()
        schedule_visible_persistent_hydration(event.app, delay=0.08)

    app = Application(layout=layout, key_bindings=kb, style=style, full_screen=False)
    app_ref = app
    update_ui(prefetch_metadata=False)

    def _pre_run() -> None:
        # First hydrate the rows the user can actually see, then warm the rest
        # of the persistent cache in a separate lightweight cache worker.
        request_visible_persistent_hydration(app)
        start_persistent_metadata_warmup(app)

    try:
        return app.run(pre_run=_pre_run)
    finally:
        try:
            metadata_executor.shutdown(wait=False, cancel_futures=True)
        except Exception:
            pass
        cache_executor.shutdown(wait=False, cancel_futures=True)
