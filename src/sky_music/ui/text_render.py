"""Single source of truth for terminal cell-width math and box sizing.

Every picker module (``picker``, ``picker_theme``) and the
CLI panels in ``main`` must measure, truncate, and pad text through these
helpers. Keeping one implementation is what guarantees box borders line up:
divergent width math across modules is exactly what caused card borders to drift.
"""

from __future__ import annotations

import re
import unicodedata
from typing import Literal

from rich.cells import cell_len as _pt_cwidth

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
    if _pt_cwidth is not None:
        return max(0, _pt_cwidth(text))
    width = 0
    for char in text:
        if unicodedata.combining(char):
            continue
        width += 2 if unicodedata.east_asian_width(char) in {"F", "W"} else 1
    return width


def truncate_cells(text: str, max_width: int) -> str:
    """Clip ``text`` to ``max_width`` cells, adding an ellipsis when it overflows."""
    if max_width <= 0:
        return ""
    if max_width == 1:
        return "…" if cell_width(text) > 1 else text
    if cell_width(text) <= max_width:
        return text

    out: list[str] = []
    used = 0
    limit = max_width - 1
    for char in text:
        char_width = cell_width(char)
        if used + char_width > limit:
            break
        out.append(char)
        used += char_width
    return "".join(out) + "…"


def pad_cells(text: str, width: int, *, align: Literal["left", "right"] = "left") -> str:
    """Pad ``text`` to ``width`` cells so emoji/CJK/box chars don't shift columns."""
    padding = max(0, width - cell_width(text))
    if align == "right":
        return " " * padding + text
    return text + " " * padding


def fit_cells(text: str, width: int, *, align: Literal["left", "right"] = "left") -> str:
    """Truncate then pad so the result is exactly ``width`` cells wide."""
    return pad_cells(truncate_cells(text, width), width, align=align)


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
        pad = max(0, inner_width - visible_width(line))
        out.append(f"{bar} {line}{' ' * pad} {bar}")
    out.append(bottom)
    return out
