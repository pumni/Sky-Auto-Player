from __future__ import annotations

from typing import Any

from rich.markup import escape
from rich.table import Table
from rich.text import Text
from textual import events
from textual.message import Message
from textual.widgets import Static

from sky_music.ui.picker_theme import get_theme_preset, pad_text, remove_accents
from sky_music.ui.text_render import cell_width
from sky_music.ui.textual_app.keymap import PICKER_HINTS, CommandSpec, KeyHint


class GridRenderable(Table):
    """A helper class to wrap a Table with a .plain property for test compatibility."""

    def __init__(self, *args: Any, plain_text: str = "", **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self._plain_text = plain_text

    @property
    def plain(self) -> str:
        return self._plain_text


class AppFooter(Static):
    """Clean app action bar/footer for the main picker screen."""

    def __init__(self, hints: list[KeyHint], *, key_color: str = "#facc15", muted_color: str = "#6b7a93", **kwargs: Any) -> None:
        Static.__init__(self, "", markup=True, **kwargs)
        self.hints = hints
        self.key_color = key_color
        self.muted_color = muted_color
        self._click_spans: list[tuple[int, int, str]] = []

    def set_theme(self, key_color: str, muted_color: str) -> AppFooter:
        self.key_color = key_color
        self.muted_color = muted_color
        self.refresh()
        return self

    def _render_hint(self, hint: KeyHint) -> Text:
        text = Text()
        text.append(" ")
        text.append(hint.key, style=f"bold {self.key_color}")
        text.append(" ")
        text.append(" ")
        text.append(hint.label, style=self.muted_color)
        return text

    def render(self) -> Text:
        right_text = Text()
        self._click_spans = []
        for index, hint in enumerate(self.hints):
            if index:
                right_text.append("  ·  ", style=self.muted_color)
            start = cell_width(right_text.plain)
            right_text.append_text(self._render_hint(hint))
            end = cell_width(right_text.plain)
            if hint.action is not None:
                self._click_spans.append((start, end, hint.action))
        return right_text

    def on_click(self, event: events.Click) -> None:
        for start, end, action in self._click_spans:
            if start <= event.x < end:
                event.stop()
                handler_name = f"action_{action.rsplit('.', 1)[-1]}"
                handler = getattr(self.app, handler_name, None)
                if callable(handler):
                    handler()
                return


class ModalHintBar(Static):
    """Muted instruction footer/hint bar for modals."""

    def __init__(self, hints: list[KeyHint], *, key_color: str = "#6b7a93", muted_color: str = "#6b7a93", **kwargs: Any) -> None:
        Static.__init__(self, "", markup=True, **kwargs)
        self.hints = hints
        self.key_color = key_color
        self.muted_color = muted_color
        self._update_markup()

    def set_theme(self, key_color: str, muted_color: str) -> ModalHintBar:
        self.key_color = key_color
        self.muted_color = muted_color
        self._update_markup()
        return self

    def _update_markup(self) -> None:
        markup_str = "  ·  ".join(self._render_hint(h) for h in self.hints)
        self.update(markup_str)

    def _render_hint(self, hint: KeyHint) -> str:
        key_display = hint.key.lower()
        label_display = hint.label.lower()
        return f"[bold {self.key_color}] {escape(key_display)} [/][{self.muted_color}] {escape(label_display)}[/]"


class CustomFooter(AppFooter):
    """A custom, clean footer that displays a concise hint of the most important keys."""

    def __init__(self, **kwargs: Any) -> None:
        AppFooter.__init__(self, PICKER_HINTS, **kwargs)


class CommandPaletteList(Static):
    """Custom widget for grouped command selection."""

    can_focus = True
    GROUP_ORDER = ["View", "Playback", "Interface", "Library", "System"]

    class CommandHighlighted(Message):
        def __init__(self, command: CommandSpec) -> None:
            super().__init__()
            self.command = command

    class CommandSelected(Message):
        def __init__(self, command: CommandSpec) -> None:
            super().__init__()
            self.command = command

    def __init__(self, commands: list[CommandSpec], **kwargs: Any) -> None:
        Static.__init__(self, "", **kwargs)
        self.commands = commands
        self.filter_text = ""
        self.selectable_commands: list[CommandSpec] = self._filtered_commands()
        self.highlighted_index = 0
        self._row_commands: list[CommandSpec | None] = []

    def set_filter(self, value: str) -> None:
        self.filter_text = value.strip()
        self.selectable_commands = self._filtered_commands()
        self.highlighted_index = 0
        self.refresh()
        if self.selectable_commands:
            self.post_message(self.CommandHighlighted(self.selectable_commands[self.highlighted_index]))

    def move_highlight(self, delta: int) -> None:
        if not self.selectable_commands:
            return
        self.highlighted_index = (self.highlighted_index + delta) % len(self.selectable_commands)
        self.refresh()
        self.post_message(self.CommandHighlighted(self.selectable_commands[self.highlighted_index]))

    def select_highlighted(self) -> None:
        if self.selectable_commands:
            self.post_message(self.CommandSelected(self.selectable_commands[self.highlighted_index]))

    def _matches_filter(self, command: CommandSpec) -> bool:
        query = remove_accents(self.filter_text).casefold()
        if not query:
            return True
        haystack = " ".join(
            (
                command.id,
                command.key,
                command.label,
                command.description,
                command.group,
            )
        )
        return query in remove_accents(haystack).casefold()

    def _filtered_commands(self) -> list[CommandSpec]:
        matched = [cmd for cmd in self.commands if self._matches_filter(cmd)]
        return sorted(
            matched,
            key=lambda cmd: (
                self.GROUP_ORDER.index(cmd.group) if cmd.group in self.GROUP_ORDER else len(self.GROUP_ORDER),
                self.commands.index(cmd),
            ),
        )

    def _render_rows(self) -> list[CommandSpec | None]:
        grouped: dict[str, list[CommandSpec]] = {}
        rows: list[CommandSpec | None] = []
        for cmd in self.selectable_commands:
            grouped.setdefault(cmd.group, []).append(cmd)

        first = True
        for group_name in self.GROUP_ORDER:
            if group_name not in grouped:
                continue
            if not first:
                rows.append(None)
            first = False
            rows.append(None)
            rows.extend(grouped[group_name])
        return rows

    def on_mount(self) -> None:
        self._row_commands = self._render_rows()
        if self.selectable_commands:
            self.post_message(self.CommandHighlighted(self.selectable_commands[self.highlighted_index]))

    def on_key(self, event: events.Key) -> None:
        if event.key == "up":
            event.stop()
            self.move_highlight(-1)
        elif event.key == "down":
            event.stop()
            self.move_highlight(1)
        elif event.key == "enter":
            event.stop()
            self.select_highlighted()

    def on_mouse_move(self, event: events.MouseMove) -> None:
        rows = self._row_commands or self._render_rows()
        if 0 <= event.y < len(rows):
            command = rows[event.y]
            if command is None:
                return
            try:
                index = self.selectable_commands.index(command)
            except ValueError:
                return
            if index != self.highlighted_index:
                self.highlighted_index = index
                self.refresh()
                self.post_message(self.CommandHighlighted(command))

    def on_click(self, event: events.Click) -> None:
        rows = self._row_commands or self._render_rows()
        if 0 <= event.y < len(rows):
            command = rows[event.y]
            if command is None:
                return
            self.highlighted_index = self.selectable_commands.index(command)
            self.refresh()
            self.post_message(self.CommandHighlighted(command))
            self.post_message(self.CommandSelected(command))

    def render(self) -> Text:
        try:
            t = get_theme_preset(self.app.active_theme)  # type: ignore[attr-defined]
        except Exception:
            from sky_music.ui.picker_theme import THEME_PRESETS
            t = THEME_PRESETS["aurora"]

        grouped: dict[str, list[CommandSpec]] = {}
        for cmd in self.selectable_commands:
            grouped.setdefault(cmd.group, []).append(cmd)

        txt = Text()
        self._row_commands = []
        if not self.selectable_commands:
            empty = f'No commands match "{self.filter_text}"' if self.filter_text else "No commands available"
            txt.append(empty, style=t.muted)
            return txt

        highlighted_cmd = self.selectable_commands[self.highlighted_index]

        first = True
        for group_name in self.GROUP_ORDER:
            if group_name not in grouped:
                continue
            if not first:
                txt.append("\n\n")
                self._row_commands.append(None)
            first = False

            txt.append(group_name, style=f"bold {t.key}")
            self._row_commands.append(None)

            for cmd in grouped[group_name]:
                txt.append("\n")
                is_selected = cmd.id == highlighted_cmd.id
                self._row_commands.append(cmd)

                key_str = pad_text(cmd.key, 8)
                label_str = pad_text(cmd.label, 20)

                if is_selected:
                    line = Text()
                    line.append("▌ ", style=f"bold {t.accent}")
                    line.append(key_str, style=f"bold {t.accent}")
                    line.append(label_str, style=f"bold {t.foreground}")
                    line.append(cmd.description, style=t.detail)
                    txt.append(line)
                else:
                    line = Text()
                    line.append("  ")
                    line.append(key_str, style=f"{t.accent_dim}")
                    line.append(label_str, style=f"{t.foreground}")
                    line.append(cmd.description, style=t.muted)
                    txt.append(line)
        return txt
