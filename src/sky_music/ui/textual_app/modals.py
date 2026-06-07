from __future__ import annotations

from dataclasses import dataclass
from typing import Any, TypeVar

from textual import events
from textual.app import ComposeResult
from textual.containers import Vertical
from textual.screen import ModalScreen
from textual.widgets import Input, OptionList, Static

from sky_music.ui.picker_theme import THEME_PRESETS, get_theme_preset
from sky_music.ui.textual_app.keymap import COMMAND_MODAL_HINTS, INFO_MODAL_HINTS, CommandSpec, KeyHint
from sky_music.ui.textual_app.widgets import CommandPaletteList, ModalHintBar


@dataclass(frozen=True, slots=True)
class PickerOption:
    value: object
    label: str


T = TypeVar("T")


class PickerModal(ModalScreen[T]):
    """Base modal shell with title, content area, and shortcut footer."""

    def __init__(self, title: str, hints: list[KeyHint], *, theme_name: str = "aurora") -> None:
        ModalScreen.__init__(self)
        self.title_text = title
        self.hints = hints
        self.theme_name = theme_name

    def compose(self) -> ComposeResult:
        with Vertical(id="modal"):
            with Vertical(id="modal-content"):
                yield from self.compose_modal_content()
            yield ModalHintBar(self.hints, id="modal-footer")

    def compose_modal_content(self) -> ComposeResult:
        """Override in subclasses to yield widgets for the modal body."""
        raise NotImplementedError()

    def on_mount(self) -> None:
        self._apply_theme_class()
        modal = self.query_one("#modal", Vertical)
        modal.border_title = self.title_text

        theme = get_theme_preset(self.theme_name)
        try:
            self.query_one("#modal-footer", ModalHintBar).set_theme(theme.key, theme.muted)
        except Exception:
            pass
        self.on_modal_mounted()

    def on_modal_mounted(self) -> None:
        """Override in subclasses to perform extra mount logic."""
        pass

    def _apply_theme_class(self) -> None:
        for name in THEME_PRESETS:
            self.remove_class(f"theme-{name}")
        self.add_class(f"theme-{self.theme_name}")


class OptionModal(PickerModal[object | None]):
    """Simple option modal used by Phase 2 picker controls."""

    BINDINGS = [("escape", "cancel", "Cancel")]

    def __init__(
        self,
        title: str,
        options: list[PickerOption],
        *,
        info_text: str = "",
        theme_name: str = "aurora",
    ) -> None:
        PickerModal.__init__(self, title, COMMAND_MODAL_HINTS, theme_name=theme_name)
        self.options = options
        self.info_text = info_text

    def compose_modal_content(self) -> ComposeResult:
        if self.info_text:
            yield Static(self.info_text, id="modal-info", markup=True)
        yield OptionList(*(option.label for option in self.options), id="modal-options")

    def on_modal_mounted(self) -> None:
        self.set_focus(self.query_one("#modal-options", OptionList))

    def on_key(self, event: events.Key) -> None:
        if event.key == "escape":
            event.stop()
            self.dismiss(None)
        elif event.key == "enter":
            event.stop()
            options = self.query_one("#modal-options", OptionList)
            index = options.highlighted
            if index is None:
                return
            self.dismiss(self.options[index].value)

    def on_option_list_option_selected(self, event: OptionList.OptionSelected) -> None:
        event.stop()
        index = event.option_index
        if index is None:
            return
        self.dismiss(self.options[index].value)

    def action_cancel(self) -> None:
        self.dismiss(None)


class CommandModal(PickerModal[str | None]):
    """Dedicated modal for the commands palette."""

    BINDINGS = [("escape", "cancel", "Cancel")]

    def __init__(self, title: str, commands: list[CommandSpec], *, theme_name: str = "aurora") -> None:
        PickerModal.__init__(self, title, COMMAND_MODAL_HINTS, theme_name=theme_name)
        self.commands = commands

    def compose_modal_content(self) -> ComposeResult:
        filter_input = Input(placeholder="Filter commands", id="command-filter")
        filter_input.border_title = "Filter"
        yield filter_input
        yield CommandPaletteList(self.commands, id="modal-options")

    def on_modal_mounted(self) -> None:
        self.set_focus(self.query_one("#command-filter", Input))

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id != "command-filter":
            return
        event.stop()
        self.query_one("#modal-options", CommandPaletteList).set_filter(event.value)

    def on_command_palette_list_command_highlighted(self, event: CommandPaletteList.CommandHighlighted) -> None:
        event.stop()

    def on_command_palette_list_command_selected(self, event: CommandPaletteList.CommandSelected) -> None:
        event.stop()
        self.dismiss(event.command.id)

    def on_input_submitted(self, event: Input.Submitted) -> None:
        if event.input.id != "command-filter":
            return
        event.stop()
        self.query_one("#modal-options", CommandPaletteList).select_highlighted()

    def on_key(self, event: events.Key) -> None:
        if event.key == "escape":
            event.stop()
            self.dismiss(None)
        elif event.key == "up":
            event.stop()
            self.query_one("#modal-options", CommandPaletteList).move_highlight(-1)
        elif event.key == "down":
            event.stop()
            self.query_one("#modal-options", CommandPaletteList).move_highlight(1)
        elif event.key == "enter":
            event.stop()
            self.query_one("#modal-options", CommandPaletteList).select_highlighted()

    def action_cancel(self) -> None:
        self.dismiss(None)


class InfoModal(PickerModal[None]):
    """Read-only modal for Phase 2 help and diagnostics."""

    BINDINGS = [("escape", "close", "Close"), ("enter", "close", "Close")]

    def __init__(self, title: str, content: Any = "", *, theme_name: str = "aurora") -> None:
        PickerModal.__init__(self, title, INFO_MODAL_HINTS, theme_name=theme_name)
        self.content = content

    def compose_modal_content(self) -> ComposeResult:
        yield Static(self.content, id="info", markup=isinstance(self.content, str))

    def on_modal_mounted(self) -> None:
        pass

    def action_close(self) -> None:
        self.dismiss(None)

    def on_key(self, event: events.Key) -> None:
        if event.key in {"escape", "enter"}:
            event.stop()
            self.dismiss(None)
