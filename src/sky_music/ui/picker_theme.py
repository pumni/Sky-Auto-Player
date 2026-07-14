import functools
import unicodedata
from dataclasses import dataclass
from typing import Any

from sky_music.ui.text_render import cell_width, pad_cells, truncate_cells

# Backwards-compatible aliases — terminal cell-width math lives in text_render now.
display_width = cell_width


def pad_text(text: str, width: int, align: str = "left") -> str:
    """Pad by terminal cell width so emoji/box chars don't shift columns."""
    return pad_cells(text, width, align="right" if align == "right" else "left")


@dataclass(frozen=True, slots=True)
class ThemePreset:
    name: str
    pointer: str
    song_icon: str
    empty_icon: str

    background: str
    foreground: str
    muted: str
    border: str
    accent: str
    accent_dim: str
    divider: str
    key: str
    use_gradient_border: bool
    header_lead: int
    gradient: tuple[str, ...]
    cursor_background: str
    cursor_foreground: str
    detail: str
    modal_background: str
    modal_title: str
    match: str

    success: str = "#4ade80"
    warning: str = "#fbbf24"
    danger: str = "#f87171"


THEME_PRESETS: dict[str, ThemePreset] = {
    "aurora": ThemePreset(
        name="aurora",
        pointer="❯",
        song_icon="♪",
        empty_icon="◇",
        background="#080e1c",
        foreground="#d8e2f0",
        muted="#6b7a93",
        border="#38506f",
        accent="#4fd1ff",
        accent_dim="#1d7fa3",
        divider="#23324a",
        key="#facc15",
        use_gradient_border=True,
        header_lead=4,
        gradient=("#4fd1ff", "#7c5cf2", "#e040a8"),
        cursor_background="#075985",
        cursor_foreground="#e0f2fe",
        detail="#93a3bb",
        modal_background="#111a2e",
        modal_title="#67e8f9",
        match="#facc15",
        success="#4ade80",
        warning="#fbbf24",
        danger="#f87171",
    ),
    "minimalist": ThemePreset(
        name="minimalist",
        pointer="❯",
        song_icon="♪",
        empty_icon="·",
        background="#080808",
        foreground="#e5e7eb",
        muted="#6b6b6b",
        border="#3a3a3a",
        accent="#00ffcc",
        accent_dim="#009977",
        divider="#2a2a2a",
        key="#00ffcc",
        use_gradient_border=False,
        header_lead=2,
        gradient=("#3a3a3a", "#3a3a3a"),
        cursor_background="#00ffcc",
        cursor_foreground="#111827",
        detail="#9aa0a6",
        modal_background="#101010",
        modal_title="#e5e7eb",
        match="#ffffff",
        success="#2dd4bf",  # a softer teal
        warning="#fbbf24",
        danger="#f87171",
    ),
    "slate": ThemePreset(
        name="slate",
        pointer="▌",
        song_icon="♫",
        empty_icon="□",
        background="#0f172a",
        foreground="#cbd5e1",
        muted="#5f7088",
        border="#36486a",
        accent="#22d3ee",
        accent_dim="#0891b2",
        divider="#233148",
        key="#67e8f9",
        use_gradient_border=False,
        header_lead=3,
        gradient=("#36486a", "#36486a"),
        cursor_background="#123a52",
        cursor_foreground="#d6f6fb",
        detail="#97a6ba",
        modal_background="#16233b",
        modal_title="#67e8f9",
        match="#67e8f9",
        success="#4ade80",
        warning="#fbbf24",
        danger="#f87171",
    ),
    "cyberpunk": ThemePreset(
        name="cyberpunk",
        pointer="➜",
        song_icon="✦",
        empty_icon="×",
        background="#0a0014",
        foreground="#c8c8ff",
        muted="#7a6398",
        border="#4a2d70",
        accent="#e879f9",
        accent_dim="#a21caf",
        divider="#2e1844",
        key="#facc15",
        use_gradient_border=True,
        header_lead=5,
        gradient=("#ff4fd6", "#a85cf2", "#5f7fff"),
        cursor_background="#facc15",
        cursor_foreground="#111827",
        detail="#67e8f9",
        modal_background="#15011f",
        modal_title="#facc15",
        match="#f472b6",
        success="#00ffcc",
        warning="#facc15",
        danger="#ff3366",
    ),
    "classic": ThemePreset(
        name="classic",
        pointer=">",
        song_icon="-",
        empty_icon="!",
        background="#000000",
        foreground="#ffffff",
        muted="#9a9a9a",
        border="#6b7280",
        accent="#ffffff",
        accent_dim="#9a9a9a",
        divider="#555555",
        key="#ffffff",
        use_gradient_border=False,
        header_lead=2,
        gradient=("#6b7280", "#6b7280"),
        cursor_background="#ffffff",
        cursor_foreground="#000000",
        detail="#e5e7eb",
        modal_background="#000000",
        modal_title="#ffffff",
        match="#ffffff",
        success="#ffffff",
        warning="#ffffff",
        danger="#ffffff",
    ),
}


def get_theme_preset(theme_name: str | None = None, active_theme: str = "aurora") -> ThemePreset:
    requested_theme = (theme_name or active_theme or "aurora").casefold()
    return THEME_PRESETS.get(requested_theme, THEME_PRESETS["aurora"])


def get_theme(theme_name: str | None = None, active_theme: str = "aurora") -> tuple[str, dict[str, Any]]:
    requested_theme = (theme_name or active_theme or "aurora").casefold()
    preset = get_theme_preset(requested_theme)
    legacy_dict: dict[str, Any] = {
        "pointer": preset.pointer,
        "song_icon": preset.song_icon,
        "empty_icon": preset.empty_icon,
        "style": {
            "title": f"fg:{preset.modal_title} bold",
            "subtitle": f"fg:{preset.muted}",
            "divider": f"fg:{preset.divider}",
            "input": f"fg:{preset.foreground}",
            "prompt": f"fg:{preset.accent} bold",
            "results": "",
            "selected": f"fg:{preset.cursor_foreground} bg:{preset.cursor_background} bold",
            "unselected": f"fg:{preset.foreground}",
            "index": f"fg:{preset.muted}",
            "match": f"fg:{preset.match} bold",
            "muted": f"fg:{preset.muted}",
            "empty": f"fg:{preset.muted} italic" if requested_theme == "slate" else f"fg:{preset.warning} italic",
            "footer": f"fg:{preset.muted}",
            "key": f"fg:{preset.key} bold",
            "detail": f"fg:{preset.detail}",
            "detail_label": f"fg:{preset.accent_dim} bold",
        }
    }
    if requested_theme == "classic":
        legacy_dict["style"]["selected"] = "fg:#ffffff bold reverse"
    return requested_theme, legacy_dict


def remove_accents(input_str: str) -> str:
    if not input_str:
        return ""
    nfkd_form = unicodedata.normalize('NFKD', input_str)
    res = "".join([c for c in nfkd_form if not unicodedata.combining(c)])
    return res.replace('đ', 'd').replace('Đ', 'D')


@functools.lru_cache(maxsize=2048)
def normalized_index_map(text: str) -> tuple[str, list[int]]:
    """Return (normalized_text, index_map) for accent/case-insensitive matching.

    Cached with :func:`functools.lru_cache` (LRU, max 2048 entries) so hot
    query paths avoid repeated Unicode normalisation. The cache is thread-safe
    and evicts least-recently-used entries automatically.
    """
    normalized_chars = []
    index_map = []
    for original_index, char in enumerate(text):
        normalized = remove_accents(char).casefold()
        for normalized_char in normalized:
            normalized_chars.append(normalized_char)
            index_map.append(original_index)

    return "".join(normalized_chars), index_map


def get_match_span(text: str, normalized_query: str) -> tuple[int, int] | None:
    if not normalized_query:
        return None
    normalized_text, index_map = normalized_index_map(text)
    match_start = normalized_text.find(normalized_query)
    if match_start == -1 or not index_map:
        return None
    match_end = match_start + len(normalized_query) - 1
    return index_map[match_start], index_map[match_end] + 1


def append_highlighted_song_name(lines: list[tuple[str, str]], song_name: str, normalized_query: str, selected: bool = False) -> None:
    if selected:
        lines.append(("class:selected", song_name))
        return

    span = get_match_span(song_name, normalized_query)
    if span is None:
        lines.append(("class:unselected", song_name))
        return

    start, end = span
    lines.append(("class:unselected", song_name[:start]))
    lines.append(("class:match", song_name[start:end]))
    lines.append(("class:unselected", song_name[end:]))


def truncate_text(text: str, max_width: int) -> str:
    return truncate_cells(text, max_width)
