from __future__ import annotations

from typing import TYPE_CHECKING, Any

from rich.text import Text
from textual.message import Message
from textual.widgets import OptionList
from textual.widgets.option_list import Option

from sky_music.ui.picker_theme import pad_text, remove_accents
from sky_music.ui.textual_app.keymap import CommandSpec

if TYPE_CHECKING:
    from sky_music.ui.textual_app.app import SkyPickerApp


class CommandPaletteList(OptionList):
    """Grouped, filterable command palette built on :class:`OptionList`."""

    app: SkyPickerApp
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
        self.commands = commands
        self.filter_text = ""
        self.selectable_commands: list[CommandSpec] = self._filtered_commands()
        OptionList.__init__(self, *self._build_options(), **kwargs)
        self.show_vertical_scrollbar = True

    def _build_options(self) -> list[Option]:
        grouped: dict[str, list[CommandSpec]] = {}
        for cmd in self.selectable_commands:
            grouped.setdefault(cmd.group, []).append(cmd)

        options: list[Option] = []
        for group_name in self.GROUP_ORDER:
            if group_name not in grouped:
                continue
            header_prompt = Text(group_name.upper(), style="bold dim")
            options.append(Option(header_prompt, id=f"__header__:{group_name}", disabled=True))
            options.extend(
                Option(self._format_command_prompt(cmd), id=f"cmd:{cmd.id}")
                for cmd in grouped[group_name]
            )
        return options

    @staticmethod
    def _format_command_prompt(cmd: CommandSpec) -> Text:
        key_str = pad_text(cmd.key, 8)
        line = Text()
        line.append("  ")
        line.append(key_str, style="bold")
        line.append(cmd.label, style="bold")
        line.append(" · ", style="dim")
        line.append(cmd.description, style="dim")
        return line

    def set_filter(self, value: str) -> None:
        self.filter_text = value.strip()
        self.selectable_commands = self._filtered_commands()
        self.clear_options()
        new_opts = self._build_options()
        if new_opts:
            self.add_options(new_opts)
            first_idx = self._first_command_index()
            if first_idx is not None:
                self.highlighted = first_idx
        if self.selectable_commands:
            self.post_message(self.CommandHighlighted(self.selectable_commands[0]))

    def _first_command_index(self) -> int | None:
        for i, opt in enumerate(self._options):  # type: ignore[attr-defined]
            opt_id = getattr(opt, "id", None) or ""
            if opt_id.startswith("cmd:"):
                return i
        return None

    def _matches_filter(self, command: CommandSpec) -> bool:
        query = remove_accents(self.filter_text).casefold()
        if not query:
            return True
        haystack = " ".join(
            (command.id, command.key, command.label, command.description, command.group)
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

    def on_option_list_option_selected(self, event: OptionList.OptionSelected) -> None:
        cmd = self._command_for_option_index(getattr(event, "option_index", None))
        if cmd is not None:
            event.stop()
            self.post_message(self.CommandSelected(cmd))

    def on_option_list_option_highlighted(self, event: OptionList.OptionHighlighted) -> None:  # type: ignore[override]
        cmd = self._command_for_option_index(getattr(event, "option_index", None))
        if cmd is not None:
            event.stop()
            self.post_message(self.CommandHighlighted(cmd))

    def _command_for_option_index(self, idx: int | None) -> CommandSpec | None:
        if idx is None:
            return None
        try:
            opt = self._options[idx]  # type: ignore[attr-defined]
        except (IndexError, AttributeError):
            return None
        opt_id = getattr(opt, "id", None) or ""
        if not opt_id.startswith("cmd:"):
            return None
        cmd_id = opt_id[len("cmd:") :]
        for c in self.selectable_commands:
            if c.id == cmd_id:
                return c
        return None

    @property
    def highlighted_index(self) -> int:
        raw = self.highlighted
        if raw is None:
            return 0
        cmd = self._command_for_option_index(raw)
        if cmd is None:
            return 0
        try:
            return self.selectable_commands.index(cmd)
        except ValueError:
            return 0

    @highlighted_index.setter
    def highlighted_index(self, value: int) -> None:
        if not self.selectable_commands:
            return
        idx = max(0, min(value, len(self.selectable_commands) - 1))
        cmd = self.selectable_commands[idx]
        for i, opt in enumerate(self._options):  # type: ignore[attr-defined]
            if (getattr(opt, "id", None) or "") == f"cmd:{cmd.id}":
                self.highlighted = i
                return

    def move_highlight(self, delta: int) -> None:
        n = len(self.selectable_commands)
        if n == 0:
            return
        self.highlighted_index = (self.highlighted_index + delta) % n

    def select_highlighted(self) -> None:
        if self.selectable_commands:
            self.post_message(self.CommandSelected(self.selectable_commands[self.highlighted_index]))
