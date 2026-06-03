from typing import Any, Literal
from dataclasses import dataclass
from sky_music.ui.picker_theme import get_match_span
from sky_music.ui.text_render import (
    cell_width as _cell_width,
    truncate_cells as _truncate_cells,
    pad_cells as _pad_cells,
    fit_cells as _fit_cells,
)


@dataclass(frozen=True, slots=True)
class ActionHint:
    key: str
    long: str
    short: str
    tiny: str


def format_actions(actions: list[ActionHint], width: int) -> list[tuple[str, str]]:
    def build_tokens(label_attr: Literal["long", "short", "tiny"]) -> list[tuple[str, str]]:
        tokens: list[tuple[str, str]] = []
        for act in actions:
            label = getattr(act, label_attr)
            tokens.append(("class:key", act.key))
            tokens.append(("class:footer", f" {label} │ "))
        if tokens:
            last_label = getattr(actions[-1], label_attr)
            tokens[-1] = ("class:footer", f" {last_label}")
        return tokens

    for label_attr in ("long", "short"):
        tokens = build_tokens(label_attr)  # type: ignore[arg-type]
        if sum(_cell_width(text) for _, text in tokens) <= width:
            return tokens

    tokens = build_tokens("tiny")
    total = 0
    fitted: list[tuple[str, str]] = []
    for style, text in tokens:
        remaining = width - total
        if remaining <= 0:
            break
        clipped = _truncate_cells(text, remaining)
        fitted.append((style, clipped))
        total += _cell_width(clipped)
    return fitted


def _format_duration(seconds: float) -> str:
    seconds = max(0, int(seconds))
    minutes, sec = divmod(seconds, 60)
    hours, minutes = divmod(minutes, 60)
    if hours:
        return f"{hours}:{minutes:02}:{sec:02}"
    return f"{minutes}:{sec:02}"


def build_box(title: str, content: list[Any], width: int = 76) -> list[tuple[str, str]]:
    top_left = "╭"
    top_right = "╮"
    bottom_left = "╰"
    bottom_right = "╯"
    horiz = "─"
    vert = "│"

    width = max(8, width)
    inner_width = max(0, width - 4)

    title_part = f"{horiz} {title} "
    top_fill = max(0, width - 2 - _cell_width(title_part))
    top_line = f"{top_left}{title_part}{horiz * top_fill}{top_right}\n"
    bottom_line = f"{bottom_left}{horiz * (width - 2)}{bottom_right}\n"

    tokens: list[tuple[str, str]] = [("class:divider", top_line)]
    for line in content:
        tokens.append(("class:divider", f"{vert} "))
        if isinstance(line, str):
            line_clean = _fit_cells(line, inner_width)
            tokens.append(("class:detail", line_clean))
        else:
            line_width = 0
            for style, text in line:
                remaining = inner_width - line_width
                if remaining <= 0:
                    break
                clipped = _truncate_cells(str(text), remaining)
                tokens.append((style, clipped))
                line_width += _cell_width(clipped)
            if line_width < inner_width:
                tokens.append(("class:detail", " " * (inner_width - line_width)))
        tokens.append(("class:divider", f" {vert}\n"))
    tokens.append(("class:divider", bottom_line))
    return tokens


def format_song_row(idx: int, metadata: Any, selected: bool, query: str, pointer: str, song_icon: str) -> list[tuple[str, str]]:
    dur_str = _format_duration(metadata.duration_seconds)
    risk_upper = metadata.risk.upper()[:5]

    risk_style = (
        "fg:#ef4444 bold"
        if risk_upper == "ERROR"
        else (
            "fg:#f97316 bold"
            if risk_upper == "HIGH"
            else ("fg:#fbbf24 bold" if risk_upper == "MED" or risk_upper == "MEDIUM" else "fg:#10b981")
        )
    )
    if selected:
        risk_style = "fg:#ffffff bold"

    tokens: list[tuple[str, str]] = []
    prefix = f"{pointer} {idx:<3} " if selected else f"  {idx:<3} "
    row_class = "class:selected" if selected else "class:unselected"

    tokens.append((row_class, prefix))

    title_width = 36
    song_name_trunc = _truncate_cells(metadata.name, title_width)
    song_name_padded = _pad_cells(song_name_trunc, title_width)

    if selected:
        tokens.append(("class:selected", song_name_padded))
    else:
        span = get_match_span(song_name_trunc, query)
        if span is None:
            tokens.append(("class:unselected", song_name_padded))
        else:
            start, end = span
            before = song_name_trunc[:start]
            match = song_name_trunc[start:end]
            after = song_name_trunc[end:]
            tokens.append(("class:unselected", before))
            tokens.append(("class:match", match))
            used = _cell_width(before) + _cell_width(match)
            tokens.append(("class:unselected", _pad_cells(after, title_width - used)))

    tokens.append((row_class, f"    {dur_str:>4}   {metadata.note_count:>5}   "))
    tokens.append((risk_style if not selected else row_class, _fit_cells(risk_upper, 5)))
    tokens.append((row_class, f"   {_fit_cells(metadata.recommended_profile.strip(), 11)}\n"))
    return tokens


def build_header_box(title: str, info_parts: list[str], width: int) -> list[tuple[str, str]]:
    """Header with a consistent border width (matches other picker boxes)."""
    width = max(8, width)
    inner_w = max(20, width - 4)
    info_str = format_info_str(info_parts, inner_w)
    # Match build_box: corner is followed by a horizontal rule then the title,
    # so all cards share the identical "╭─ Title ──╮" header style.
    title_label = f"─ {title.strip()} "
    top_fill = max(0, width - 2 - _cell_width(title_label))
    top_line = f"╭{title_label}{'─' * top_fill}╮\n"
    info_line = f"│ {_fit_cells(info_str, inner_w)} │\n"
    bottom_line = f"╰{'─' * (width - 2)}╯\n"
    return [
        ("class:title", top_line),
        ("class:subtitle", info_line),
        ("class:divider", bottom_line),
    ]


def format_info_str(parts: list[str], max_width: int) -> str:
    current_parts = list(parts)
    while current_parts:
        candidate = " │ ".join(current_parts)
        if _cell_width(candidate) <= max_width:
            return candidate
        current_parts.pop()
    return _truncate_cells(parts[0] if parts else "", max_width)
