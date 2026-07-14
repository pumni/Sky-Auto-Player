from __future__ import annotations

from typing import Any

from rich.text import Text
from textual.color import Color
from textual.widgets import Static

from sky_music.ui.text_render import cell_width, truncate_cells


class StatusBar(Static):
    """Compact picker status line."""


class DetailPanel(Static):
    """Selected song detail panel."""


class GradientHeader(Static):
    """Header drawn with a hand-rolled linear-gradient frame."""

    def __init__(self, title: str, tagline: str, version: str = "", **kwargs: Any) -> None:
        super().__init__("", **kwargs)
        self._title = title
        self._tagline = tagline
        self._version = version
        self._status = ""
        self._lead = 2
        # Colors are intentionally empty until set_theme() is called by
        # PickerScreen._apply_theme_class(). render() guards against this.
        self._stops: list[str] = []
        self._title_color = ""
        self._tagline_color = ""
        self._status_color = ""
        self._version_highlight = False
        self._version_highlight_color: str = ""

    def set_theme(
        self,
        gradient: tuple[str, ...],
        title_color: str,
        tagline_color: str,
        status_color: str,
        lead: int = 2,
    ) -> None:
        self._stops = list(gradient) or [title_color]
        self._title_color = title_color
        self._tagline_color = tagline_color
        self._status_color = status_color
        self._lead = lead
        self.refresh()

    def set_tagline(self, tagline: str) -> None:
        self._tagline = tagline
        self.refresh()

    def set_status(self, status: str) -> None:
        self._status = status
        self.refresh()

    def set_version(self, version: str, *, highlight: bool = False, highlight_color: str = "") -> None:
        self._version = version
        self._version_highlight = highlight
        if highlight and highlight_color:
            self._version_highlight_color = highlight_color
        self.refresh()

    def on_resize(self) -> None:
        self.refresh()

    def render(self) -> Text:
        # Guard: theme not yet applied — return empty until set_theme() is called.
        if not self._stops or not self._title_color:
            return Text("")
        width = self.size.width or 60
        if width < 12:
            return Text("")
        stops = [Color.parse(c) for c in self._stops]

        def g(i: int) -> str:
            if len(stops) == 1:
                return stops[0].hex
            pos = (i / max(width - 1, 1)) * (len(stops) - 1)
            k = int(pos)
            if k >= len(stops) - 1:
                return stops[-1].hex
            return stops[k].blend(stops[k + 1], pos - k).hex

        top = Text()
        top.append("╭", style=g(0))
        for col in range(1, 1 + self._lead):
            top.append("─", style=g(col))

        title_str = f" {self._title} "
        title_cells = cell_width(title_str)

        version_str = f" {self._version} " if self._version else ""
        version_cells = cell_width(version_str)

        if 1 + self._lead + title_cells + version_cells + 1 > width:
            version_str = ""
            version_cells = 0
            if 1 + self._lead + title_cells + 1 > width:
                title_str = truncate_cells(title_str, max(0, width - self._lead - 3))
                title_cells = cell_width(title_str)

        top.append(title_str, style=f"bold {self._title_color}")

        start_fill = 1 + self._lead + title_cells
        end_fill = width - 1 - version_cells

        for cell_idx in range(start_fill, end_fill):
            top.append("─", style=g(cell_idx))

        if version_str:
            if self._version_highlight and self._version_highlight_color and "\u2191" in version_str:
                before, arrow, after = version_str.partition("\u2191")
                if before:
                    top.append(before, style=f"{self._tagline_color}")
                top.append(arrow, style=f"bold {self._version_highlight_color}")
                if after:
                    top.append(after, style=f"{self._tagline_color}")
            else:
                top.append(version_str, style=f"{self._tagline_color}")

        top.append("╮", style=g(width - 1))

        content_w = width - 4
        left = self._tagline
        right = self._status

        left_w = cell_width(left)
        right_w = cell_width(right)

        if left_w > content_w - 2:
            left = truncate_cells(left, max(0, content_w - 2))
            left_w = cell_width(left)

        if left_w + right_w + 1 > content_w:
            right = truncate_cells(right, max(0, content_w - left_w - 1))
            right_w = cell_width(right)

        pad = max(1, content_w - left_w - right_w)
        mid = Text()
        mid.append("│", style=g(0))
        mid.append(" ")
        mid.append(left, style=f"italic {self._tagline_color}")
        mid.append(" " * pad)
        mid.append(right, style=f"bold {self._status_color}")
        mid.append(" ")
        mid.append("│", style=g(width - 1))

        bot = Text()
        bot.append("╰", style=g(0))
        for cell_idx in range(1, width - 1):
            bot.append("─", style=g(cell_idx))
        bot.append("╯", style=g(width - 1))

        return Text("\n").join([top, mid, bot])
