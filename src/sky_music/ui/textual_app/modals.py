from __future__ import annotations

import contextlib
from dataclasses import dataclass
from typing import Any, TypeVar

from textual import events
from textual.app import ComposeResult
from textual.containers import Vertical
from textual.screen import ModalScreen
from textual.widgets import Input, OptionList, ProgressBar, RichLog, Static

from sky_music.ui.picker_theme import THEME_PRESETS, get_theme_preset
from sky_music.ui.textual_app.components.command_palette import CommandPaletteList
from sky_music.ui.textual_app.components.footers import ModalHintBar
from sky_music.ui.textual_app.keymap import (
    COMMAND_MODAL_HINTS,
    INFO_MODAL_HINTS,
    CommandSpec,
    KeyHint,
)


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
        with contextlib.suppress(Exception):
            self.query_one("#modal-footer", ModalHintBar).set_theme(theme.key, theme.muted)
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
        # Cap the palette's max height based on the current terminal so a
        # shorter terminal reveals native scroll behaviour *inside the
        # OptionList* (giving us the highlight-to-top scroll UX) rather than
        # letting the palette overflow into the modal-content area (where
        # scroll_visible() does not bring the highlighted row into focus).
        # Window budget: viewport - filter (3+1) - footer (1+1) - modal
        # padding (2) - ambient row (1) = ~ viewport - 9 — but at least 4 to
        # keep a patch of the list visible even on the smallest terminals.
        viewport_height = self.app.size.height
        palette = self.query_one("#modal-options", CommandPaletteList)
        cap = max(4, viewport_height - 9)
        palette.styles.max_height = cap

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id != "command-filter":
            return
        event.stop()
        self.query_one("#modal-options", CommandPaletteList).set_filter(event.value)

    def on_command_palette_list_command_highlighted(self, event: CommandPaletteList.CommandHighlighted) -> None:
        event.stop()
        # The palette itself manages scroll-to-highlight (OptionList's
        # watch_highlighted hook calls scroll_to_highlight, which only has an
        # effect when max-height has been capped to a viewport-derived value
        # — that capping happens in ``on_modal_mounted`` so the user sees the
        # native OptionList scrollbar + the highlighted row is auto-kept-in
        # view on every arrow / PageDown navigation. Access ``event.command``
        # for clarity (and to keep the static analyser from flagging the
        # ``event`` arg as unused) but there is otherwise nothing to do.
        _ = event.command

    def on_command_palette_list_command_selected(self, event: CommandPaletteList.CommandSelected) -> None:
        event.stop()
        self.dismiss(event.command.id)

    def on_input_submitted(self, event: Input.Submitted) -> None:
        if event.input.id != "command-filter":
            return
        event.stop()
        self.query_one("#modal-options", CommandPaletteList).select_highlighted()

    def on_key(self, event: events.Key) -> None:
        # Escape always closes the modal, no matter which widget is focused.
        if event.key == "escape":
            event.stop()
            self.dismiss(None)
            return

        # When the focus is inside the CommandPaletteList, let OptionList own
        # the keyboard: it already handles up/down/enter plus PageUp/PageDown,
        # Home/End, and mouse-wheel — forwarding those keys here would silently
        # strip the modern scroll UX the list inherits from OptionList.
        if self.focused is not None and self.query_one("#command-filter", Input) is not self.focused:
            return

        # From the filter input we only need to bridge the keys that Input
        # itself does not consume and that OptionList does not see because its
        # focus is on the Input instead. Pagination keys in particular have
        # meaning in the palette, so forward them to the OptionList's own
        # built-in actions.
        palette = self.query_one("#modal-options", CommandPaletteList)
        if event.key == "up":
            event.stop()
            palette.move_highlight(-1)
        elif event.key == "down":
            event.stop()
            palette.move_highlight(1)
        elif event.key == "enter":
            event.stop()
            palette.select_highlighted()
        elif event.key == "pagedown":
            event.stop()
            palette.action_page_down()
        elif event.key == "pageup":
            event.stop()
            palette.action_page_up()
        elif event.key == "home":
            event.stop()
            palette.action_first()
        elif event.key == "end":
            event.stop()
            palette.action_last()

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

    def dismiss(self, result: Any = None) -> None:  # type: ignore[override]
        super().dismiss(result)


class UpdateModal(PickerModal[str | None]):
    """Modal to notify the user of an available update.

    Surfaces release notes (rendered from Markdown via ``RichLog``) and the
    publish timestamp so the user can decide skip vs download with context —
    previously the modal only said "v{X} is available" with no further info.
    """

    BINDINGS = [("escape", "remind", "Remind me later")]

    def __init__(
        self,
        latest_version: str,
        current_version: str,
        *,
        release_notes: str = "",
        published_at: str = "",
        theme_name: str = "aurora",
    ) -> None:
        PickerModal.__init__(
            self,
            f"Update Available: v{latest_version}",
            [
                KeyHint("enter", "Select"),
                KeyHint("escape", "Remind later"),
            ],
            theme_name=theme_name,
        )
        self.latest_version = latest_version
        self.current_version = current_version
        self.release_notes = (release_notes or "").strip()
        # Keep YYYY-MM-DD part of "2025-11-02T..Z" — displayed alongside version.
        self.published_at = (published_at or "").split("T", 1)[0]
        self.options = [
            PickerOption("github", "Download from GitHub"),
            PickerOption("download", "Download and auto-apply"),
            PickerOption("remind", "Remind me later"),
            PickerOption("skip", "Skip this version"),
        ]

    def compose_modal_content(self) -> ComposeResult:
        info_line = (
            f"You are running v{self.current_version}.\n"
            f"v{self.latest_version} is available."
        )
        if self.published_at:
            info_line += f"  Released {self.published_at}."
        yield Static(info_line, id="update-info")
        yield Static("", id="update-spacer")
        # The RichLog is populated in on_modal_mounted once the widget has a
        # DOM so Markdown rendering can use a real compute context.
        yield RichLog(id="update-notes", highlight=True, markup=True, wrap=True, auto_scroll=False)
        yield OptionList(*(o.label for o in self.options), id="modal-options")
        yield Static(
            "Auto-apply overwrites files in-place.\n"
            "If interrupted, re-download manually from GitHub.",
            id="update-caution",
        )

    def on_modal_mounted(self) -> None:
        from rich.markdown import Markdown
        notes = self.release_notes or "_(release notes unavailable)_"
        try:
            self.query_one("#update-notes", RichLog).write(Markdown(notes))
        except Exception:
            # Fallback: render the raw text — never let markdown failure break
            # the modal.
            self.query_one("#update-notes", RichLog).write(notes)
        options = self.query_one("#modal-options", OptionList)
        options.highlighted = 1
        self.set_focus(options)

    def on_key(self, event: events.Key) -> None:
        if event.key == "escape":
            event.stop()
            self.dismiss("remind")
        elif event.key == "enter":
            event.stop()
            self._select_current()

    def on_option_list_option_selected(self, event: OptionList.OptionSelected) -> None:
        event.stop()
        self._select_current()

    def _select_current(self) -> None:
        options = self.query_one("#modal-options", OptionList)
        idx = options.highlighted
        if idx is not None:
            self.dismiss(str(self.options[idx].value))
        else:
            self.dismiss("remind")

    def action_remind(self) -> None:
        self.dismiss("remind")


class UpdateProgressModal(PickerModal[None]):
    """Modal that shows download/apply progress for an auto-update.

    Replaces the previous notify-per-chunk spam — a single modal with one
    ``ProgressBar`` and one label ``Static`` updated via
    :meth:`update_progress` / :meth:`set_status`. There is NO cancel button on
    purpose: a half-staged download interrupted mid-stream would leave the
    staging dir in an unknown state. The user must wait for the worker to
    finish (success → ``sys.exit(0)`` from ``apply_staged_update``; failure →
    status label shows the error and Esc closes).
    """

    BINDINGS = [("escape", "close", "Close"), ("enter", "close", "Close")]

    _OBSERVABLE_DOWNLOAD: int = 1024 * 1024  # only update label at >=1 MiB deltas

    def __init__(
        self,
        latest_version: str,
        current_version: str,
        *,
        total: int | None = None,
        theme_name: str = "aurora",
    ) -> None:
        PickerModal.__init__(
            self,
            f"Updating to v{latest_version}",
            [KeyHint("esc", "Close when done")],
            theme_name=theme_name,
        )
        self.latest_version = latest_version
        self.current_version = current_version
        self._total = total
        self._last_reported = -1
        self._closed = False

    def compose_modal_content(self) -> ComposeResult:
        yield Static(
            f"Updating from v{self.current_version} to v{self.latest_version}…",
            id="update-progress-info",
        )
        yield ProgressBar(total=self._total, id="update-progress-bar")
        yield Static("", id="update-progress-status")

    def update_progress(self, downloaded: int, total: int | None) -> None:
        """Advance the progress bar. Called from the worker via call_from_thread.

        Throttles label updates to one per MiB to avoid swamping the
        Textual message queue on slow links — the bar still advances on every
        call.
        """
        bar = self.query_one("#update-progress-bar", ProgressBar)
        if total is not None and self._total is None:
            self._total = total
            bar.update(total=total)
        if total is not None and total > 0:
            bar.progress = downloaded
        else:
            # Unknown length — nudge forward without a known ceiling.
            bar.advance(1)
        # Throttle label update to >=1 MiB deltas.
        mb = downloaded // self._OBSERVABLE_DOWNLOAD
        if mb == self._last_reported:
            return
        self._last_reported = mb
        if total is not None and total > 0:
            pct = downloaded * 100 // total
            text = (
                f"Downloading: {pct}%  "
                f"({downloaded // 1024 // 1024} / {total // 1024 // 1024} MiB)"
            )
        else:
            text = f"Downloading: {downloaded // 1024 // 1024} MiB"
        self.query_one("#update-progress-status", Static).update(text)

    def set_status(self, text: str, *, severity: str = "information") -> None:
        """Replace the progress label with a final status line (done/failed)."""
        prefix = {
            "error": "[bold red]Error:[/] ",
            "warning": "[bold yellow]Warning:[/] ",
            "information": "",
        }.get(severity, "")
        self.query_one("#update-progress-status", Static).update(f"{prefix}{text}")

    def on_key(self, event: events.Key) -> None:
        if event.key in {"escape", "enter"} and not self._closed:
            event.stop()
            self._closed = True
            self.dismiss(None)

    def action_close(self) -> None:
        if not self._closed:
            self._closed = True
            self.dismiss(None)


class UpdateSettingsModal(PickerModal[None]):
    """Modal to toggle ``auto_check`` and ``auto_apply`` update settings.

    Each row is an ``OptionList`` entry labeled with its current state; Enter
    flips the setting via the persistence callbacks, refreshes the labels,
    and keeps focus. Esc closes. No new globals or services are introduced —
    the modal only calls ``persist_update_auto_check`` and
    ``persist_update_auto_apply`` already exported by ``sky_music.config``.
    """

    BINDINGS = [("escape", "close", "Close")]

    def __init__(
        self,
        *,
        auto_check: bool,
        auto_apply: bool,
        on_auto_check: Any,
        on_auto_apply: Any,
        theme_name: str = "aurora",
    ) -> None:
        PickerModal.__init__(
            self,
            "Update Settings",
            [
                KeyHint("enter", "Toggle"),
                KeyHint("esc", "Close"),
            ],
            theme_name=theme_name,
        )
        self._auto_check = bool(auto_check)
        self._auto_apply = bool(auto_apply)
        self._on_auto_check = on_auto_check
        self._on_auto_apply = on_auto_apply

    def _label_for(self, which: str) -> str:
        if which == "auto_check":
            state = self._auto_check
            text = "Auto-check updates on launch"
        else:
            state = self._auto_apply
            text = "Auto-apply without asking"
        mark = "[x]" if state else "[ ]"
        return f"{mark}  {text}"

    def compose_modal_content(self) -> ComposeResult:
        yield Static(
            "Auto-apply overwrites files in-place and restarts Sky Player.\n"
            "It is automatically deferred during playback.",
            id="update-settings-info",
        )
        yield OptionList(
            self._label_for("auto_check"),
            self._label_for("auto_apply"),
            id="modal-options",
        )

    def on_modal_mounted(self) -> None:
        options = self.query_one("#modal-options", OptionList)
        options.highlighted = 0
        self.set_focus(options)

    def on_key(self, event: events.Key) -> None:
        if event.key == "escape":
            event.stop()
            self.dismiss(None)
            return
        if event.key == "enter":
            event.stop()
            self._toggle_current()

    def on_option_list_option_selected(self, event: OptionList.OptionSelected) -> None:
        event.stop()
        self._toggle_current()

    def _toggle_current(self) -> None:
        options = self.query_one("#modal-options", OptionList)
        idx = options.highlighted
        if idx is None:
            return
        if idx == 0:
            self._auto_check = not self._auto_check
            self._on_auto_check(self._auto_check)
        elif idx == 1:
            self._auto_apply = not self._auto_apply
            self._on_auto_apply(self._auto_apply)
        # Re-render in place: clear and re-add the two rows, keeping highlight
        # at the same index so the user can immediately toggle again.
        options.clear_options()
        options.add_options([
            self._label_for("auto_check"),
            self._label_for("auto_apply"),
        ])
        options.highlighted = idx

    def action_close(self) -> None:
        self.dismiss(None)
