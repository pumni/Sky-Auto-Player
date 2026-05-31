import unicodedata
from typing import Any

THEME_PRESETS: dict[str, dict[str, Any]] = {
    "aurora": {
        "pointer": "❯",
        "song_icon": "♪",
        "empty_icon": "◇",
        "style": {
            "title": "fg:#a78bfa bold",
            "subtitle": "fg:#94a3b8",
            "divider": "fg:#334155",
            "input": "fg:#e2e8f0",
            "prompt": "fg:#38bdf8 bold",
            "results": "",
            "selected": "fg:#020617 bg:#38bdf8 bold",
            "unselected": "fg:#cbd5e1",
            "index": "fg:#64748b",
            "match": "fg:#fbbf24 bold",
            "muted": "fg:#64748b",
            "empty": "fg:#f97316 italic",
            "footer": "fg:#94a3b8",
            "key": "fg:#fbbf24 bold",
            "detail": "fg:#cbd5e1",
            "detail_label": "fg:#38bdf8 bold",
        },
    },
    "minimalist": {
        "pointer": "❯",
        "song_icon": "♪",
        "empty_icon": "·",
        "style": {
            "title": "fg:#e5e7eb bold",
            "subtitle": "fg:#9ca3af",
            "divider": "fg:#4b5563",
            "input": "fg:#e5e7eb",
            "prompt": "fg:#e5e7eb bold",
            "results": "",
            "selected": "fg:#00ffcc bold",
            "unselected": "fg:#cccccc",
            "index": "fg:#777777",
            "match": "fg:#ffffff bold",
            "muted": "fg:#777777",
            "empty": "fg:#999999 italic",
            "footer": "fg:#666666 italic",
            "key": "fg:#cccccc bold",
            "detail": "fg:#999999",
            "detail_label": "fg:#cccccc bold",
        },
    },
    "slate": {
        "pointer": "▌",
        "song_icon": "♫",
        "empty_icon": "□",
        "style": {
            "title": "fg:#cbd5e1 bold",
            "subtitle": "fg:#64748b",
            "divider": "fg:#475569",
            "input": "fg:#f8fafc",
            "prompt": "fg:#22d3ee bold",
            "results": "",
            "selected": "fg:#0f172a bg:#22d3ee bold",
            "unselected": "fg:#cbd5e1",
            "index": "fg:#64748b",
            "match": "fg:#67e8f9 bold",
            "muted": "fg:#64748b",
            "empty": "fg:#fca5a5 italic",
            "footer": "fg:#cbd5e1",
            "key": "fg:#67e8f9 bold",
            "detail": "fg:#cbd5e1",
            "detail_label": "fg:#67e8f9 bold",
        },
    },
    "cyberpunk": {
        "pointer": "➜",
        "song_icon": "✦",
        "empty_icon": "×",
        "style": {
            "title": "fg:#ffcc00 bold",
            "subtitle": "fg:#00ffcc",
            "divider": "fg:#7c3aed",
            "input": "fg:#00ffcc",
            "prompt": "fg:#ff00ff bold",
            "results": "",
            "selected": "fg:#0a0014 bg:#ffcc00 bold",
            "unselected": "fg:#b8b8ff",
            "index": "fg:#7c3aed",
            "match": "fg:#ff00ff bold",
            "muted": "fg:#777777",
            "empty": "fg:#ff00ff italic",
            "footer": "fg:#00ffcc",
            "key": "fg:#ffcc00 bold",
            "detail": "fg:#00ffcc",
            "detail_label": "fg:#ff00ff bold",
        },
    },
    "classic": {
        "pointer": ">",
        "song_icon": "-",
        "empty_icon": "!",
        "style": {
            "title": "fg:#ffffff bold",
            "subtitle": "fg:#ffffff",
            "divider": "fg:#ffffff",
            "input": "fg:#ffffff",
            "prompt": "fg:#ffffff bold",
            "results": "",
            "selected": "fg:#ffffff bold reverse",
            "unselected": "fg:#ffffff",
            "index": "fg:#ffffff",
            "match": "fg:#ffffff bold underline",
            "muted": "fg:#ffffff",
            "empty": "fg:#ffffff",
            "footer": "fg:#ffffff",
            "key": "fg:#ffffff bold",
            "detail": "fg:#ffffff",
            "detail_label": "fg:#ffffff bold",
        },
    },
}

def get_theme(theme_name: str | None = None, active_theme: str = "aurora") -> tuple[str, dict[str, Any]]:
    requested_theme = (theme_name or active_theme or "aurora").casefold()
    return requested_theme, THEME_PRESETS.get(requested_theme, THEME_PRESETS["aurora"])

def remove_accents(input_str: str) -> str:
    if not input_str:
        return ""
    nfkd_form = unicodedata.normalize('NFKD', input_str)
    res = "".join([c for c in nfkd_form if not unicodedata.combining(c)])
    return res.replace('đ', 'd').replace('Đ', 'D')

def normalized_index_map(text: str) -> tuple[str, list[int]]:
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
    if max_width <= 1:
        return "…"
    if len(text) <= max_width:
        return text
    return text[: max_width - 1] + "…"
