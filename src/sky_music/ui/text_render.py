"""Single source of truth for terminal cell-width math and box sizing.

Every picker module (``picker``, ``picker_theme``) and the
CLI panels in ``main`` must measure, truncate, and pad text through these
helpers. Keeping one implementation is what guarantees box borders line up:
divergent width math across modules is exactly what caused card borders to drift.
"""

from __future__ import annotations

import functools
import re
from typing import Literal

from rich.cells import cell_len as _pt_cwidth
from rich.cells import chop_cells


def hex_to_ansi(hex_color: str) -> str:
    """Convert a CSS hex color (#rrggbb or #rgb) to an ANSI 24-bit fg escape.

    Returns bright-cyan as a safe fallback for malformed input.
    """
    c = hex_color.lstrip("#")
    try:
        if len(c) == 3:
            c = "".join(ch * 2 for ch in c)
        r, g, b = int(c[0:2], 16), int(c[2:4], 16), int(c[4:6], 16)
        return f"\033[38;2;{r};{g};{b}m"
    except (ValueError, IndexError):
        return "\033[96m"

# Terminals narrower/wider than this are clamped so every panel renders at the
# same width regardless of which screen drew it (picker, preflight, comparison).
TERMINAL_WIDTH_MIN = 60
TERMINAL_WIDTH_MAX = 100


def clamp_terminal_width(columns: int) -> int:
    return max(TERMINAL_WIDTH_MIN, min(TERMINAL_WIDTH_MAX, columns))


def cell_width(text: str) -> int:
    """Terminal cell width of ``text`` (not ``len``): wide glyphs count as 2."""
    if not text:
        return 0
    return max(0, _pt_cwidth(text))


def truncate_cells(text: str, max_width: int) -> str:
    """Clip ``text`` to ``max_width`` cells, adding an ellipsis when it overflows."""
    if max_width <= 0:
        return ""
    if max_width == 1:
        return "…" if cell_width(text) > 1 else text
    if cell_width(text) <= max_width:
        return text

    chopped = chop_cells(text, max_width - 1)
    return chopped[0] + "…" if chopped else "…"


def truncate_ansi_cells(text: str, max_width: int) -> str:
    """Clip ANSI-coloured ``text`` to ``max_width`` visible cells.

    SGR escape sequences are copied through without counting toward width.
    If visible text is clipped, an ellipsis is appended and ANSI state is reset
    so colour does not leak into the right border or subsequent terminal text.
    """
    if max_width <= 0:
        return ""
    if visible_width(text) <= max_width:
        return text
    if max_width == 1:
        return "…"

    out: list[str] = []
    used = 0
    limit = max_width - 1
    pos = 0
    clipped = False
    for match in _ANSI_SGR_RE.finditer(text):
        plain = text[pos:match.start()]
        for char in plain:
            char_width = cell_width(char)
            if used + char_width > limit:
                clipped = True
                break
            out.append(char)
            used += char_width
        if clipped:
            break
        out.append(match.group(0))
        pos = match.end()

    if not clipped:
        for char in text[pos:]:
            char_width = cell_width(char)
            if used + char_width > limit:
                clipped = True
                break
            out.append(char)
            used += char_width

    return "".join(out) + "…" + _ANSI_RESET


def pad_cells(text: str, width: int, *, align: Literal["left", "right"] = "left") -> str:
    """Pad ``text`` to ``width`` cells so emoji/CJK/box chars don't shift columns."""
    padding = max(0, width - cell_width(text))
    if align == "right":
        return " " * padding + text
    return text + " " * padding


def fit_cells(text: str, width: int, *, align: Literal["left", "right"] = "left") -> str:
    """Truncate then pad so the result is exactly ``width`` cells wide."""
    return pad_cells(truncate_cells(text, width), width, align=align)


def fit_ansi_cells(text: str, width: int, *, align: Literal["left", "right"] = "left") -> str:
    """Truncate ANSI-coloured text then pad to exactly ``width`` visible cells."""
    clipped = truncate_ansi_cells(text, width)
    padding = max(0, width - visible_width(clipped))
    if align == "right":
        return " " * padding + clipped
    return clipped + " " * padding


_ANSI_SGR_RE = re.compile(r"\033\[[0-9;]*m")


def strip_ansi(text: str) -> str:
    """Remove ANSI SGR color escapes so visible width can be measured."""
    return _ANSI_SGR_RE.sub("", text)


def visible_width(text: str) -> int:
    """Cell width of ``text`` ignoring ANSI color escapes."""
    return cell_width(strip_ansi(text))


# Box-drawing glyphs shared by the prompt_toolkit cards and the print() panels.
BOX_TOP_LEFT = "╭"
BOX_TOP_RIGHT = "╮"
BOX_BOTTOM_LEFT = "╰"
BOX_BOTTOM_RIGHT = "╯"
BOX_HORIZONTAL = "─"
BOX_VERTICAL = "│"

_ANSI_RESET = "\033[0m"


def ansi_box(
    title: str,
    lines: list[str],
    *,
    width: int,
    border_color: str = "",
) -> list[str]:
    """Render a ``╭─ Title ──╮`` card around ANSI-colored ``lines`` as raw strings.

    For print()-based screens (preflight, diagnostics). Padding uses visible cell
    width so colored or wide-glyph content keeps the right border aligned — the
    same guarantee build_box gives prompt_toolkit cards.
    """
    width = max(8, width)
    inner_width = max(0, width - 4)

    def colored(text: str) -> str:
        return f"{border_color}{text}{_ANSI_RESET}" if border_color else text

    title_part = f"{BOX_HORIZONTAL} {title} "
    top_fill = max(0, width - 2 - cell_width(title_part))
    top = colored(f"{BOX_TOP_LEFT}{title_part}{BOX_HORIZONTAL * top_fill}{BOX_TOP_RIGHT}")
    bottom = colored(f"{BOX_BOTTOM_LEFT}{BOX_HORIZONTAL * (width - 2)}{BOX_BOTTOM_RIGHT}")

    out = [top]
    bar = colored(BOX_VERTICAL)
    for line in lines:
        fitted = fit_ansi_cells(line, inner_width)
        out.append(f"{bar} {fitted} {bar}")
    out.append(bottom)
    return out


def ansi_gradient_box(
    title: str,
    lines: list[str],
    *,
    width: int,
    gradient: tuple[str, ...],
    title_color: str = "",
    side_color: str | None = None,
) -> list[str]:
    """Render a ``╭─ Title ──╮`` card with a horizontal linear gradient border.

    Inside content stays with their individual colors. Uses textual.color.Color
    for character-by-character color blending along the top and bottom borders.
    """
    width = max(8, width)
    inner_width = max(0, width - 4)
    
    grad_ansi = build_horizontal_gradient_ansi(gradient, width)

    def g_ansi(i: int) -> str:
        if not grad_ansi:
            return ""
        return grad_ansi[i]

    # 1. Top rule
    top_parts = []
    top_parts.append(f"{g_ansi(0)}╭\033[0m")
    top_parts.append(f"{g_ansi(1)}─\033[0m")

    col = 2
    title_str = f" {title} " if title else ""
    title_cells = cell_width(title_str)
    if col + title_cells + 1 > width:
        title_str = truncate_cells(title_str, max(0, width - col - 2))
        title_cells = cell_width(title_str)

    if title_str:
        t_style = hex_to_ansi(title_color) if title_color else g_ansi(col)
        top_parts.append(f"{t_style}\033[1m{title_str}\033[0m")
        col += title_cells

    while col < width - 1:
        top_parts.append(f"{g_ansi(col)}─\033[0m")
        col += 1

    top_parts.append(f"{g_ansi(width - 1)}╮\033[0m")
    top_line = "".join(top_parts)

    # 2. Bottom rule
    bot_parts = [f"{g_ansi(0)}╰\033[0m"]
    bot_parts.extend(f"{g_ansi(col)}─\033[0m" for col in range(1, width - 1))
    bot_parts.append(f"{g_ansi(width - 1)}╯\033[0m")
    bot_line = "".join(bot_parts)

    # 3. Middle rule border colors
    left_border = f"{hex_to_ansi(side_color)}│\033[0m" if side_color else f"{g_ansi(0)}│\033[0m"
    right_border = f"{hex_to_ansi(side_color)}│\033[0m" if side_color else f"{g_ansi(width - 1)}│\033[0m"

    out = [top_line]
    for line in lines:
        fitted = fit_ansi_cells(line, inner_width)
        out.append(f"{left_border} {fitted} {right_border}")
    out.append(bot_line)
    return out


@functools.lru_cache(maxsize=64)
def build_horizontal_gradient_hex(stops: tuple[str, ...], width: int) -> tuple[str, ...]:
    from textual.color import Color
    if not stops:
        return ()
    c_stops = [Color.parse(c) for c in stops]
    if len(c_stops) == 1:
        return tuple(c_stops[0].hex for _ in range(width))
    out = []
    for i in range(width):
        pos = (i / max(width - 1, 1)) * (len(c_stops) - 1)
        k = int(pos)
        if k >= len(c_stops) - 1:
            out.append(c_stops[-1].hex)
        else:
            out.append(c_stops[k].blend(c_stops[k + 1], pos - k).hex)
    return tuple(out)

@functools.lru_cache(maxsize=64)
def build_horizontal_gradient_ansi(stops: tuple[str, ...], width: int) -> tuple[str, ...]:
    hex_tuple = build_horizontal_gradient_hex(stops, width)
    return tuple(hex_to_ansi(h) for h in hex_tuple)

