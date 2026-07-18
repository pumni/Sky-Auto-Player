from __future__ import annotations

import contextlib
from dataclasses import dataclass
from typing import Any, TypeVar

from textual import events
from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widgets import (
    Button,
    Checkbox,
    Input,
    OptionList,
    RichLog,
    Rule,
    Static,
)

from sky_music.ui.picker_theme import THEME_PRESETS, ThemePreset, get_theme_preset
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
    """Base modal shell with title, content area, and shortcut footer.

    ``on_key`` catches keys that bubble up to the modal screen after
    the focused widget and all intermediate widgets declined them.
    Keys that appear in the modal's own ``BINDINGS`` or in any bound
    widget's bindings are let through; everything else is stopped here
    to prevent leaking input to the screen underneath the modal.
    """

    def on_key(self, event: events.Key) -> None:
        # Let through keys that are handled by bindings anywhere in the
        # modal (the binding system checks them at the App level after
        # this event bubbles up).
        binding_chain = self._binding_chain
        for _node, bindings in binding_chain:
            if event.key in bindings.key_to_bindings:
                return  # let it bubble up to App._on_key → _check_bindings
        # Any key not bound anywhere in the modal is stopped here.
        event.stop()

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

        # Claim keyboard focus for the modal so key events do not leak
        # through to the screen underneath.  Set focus on the first
        # focusable child; subclasses may override this via
        # on_modal_mounted, but if they leave focus None the base
        # class falls back to the first Button / Input / Checkbox.
        self.on_modal_mounted()

        if self.focused is None:
            for widget in self.query("*"):
                if widget.focusable and widget.display:
                    self.set_focus(widget)
                    break

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





class CheckBoxSquare(Checkbox):
    """A ``Checkbox`` variant that renders as ``[✓]`` (green bold) /
    ``[✓]`` (dimmed) instead of the default ``▐X▌`` block, giving a
    traditional square-checkbox look.  The checkmark is always present;
    checked/unchecked is distinguished by colour and boldness (``.-on``
    CSS class) — consistent with Textual's own ``ToggleButton`` approach."""

    BUTTON_LEFT = "["
    BUTTON_INNER = "✓"
    BUTTON_RIGHT = "]"


class UpdateSettingsModal(PickerModal[str | None]):
    """Modal to view and toggle update settings using native Textual widgets.

    Layout (top to bottom):

      1. Header ``#update-settings-info``: cadence + last check summary.
      2. Two ``CheckBoxSquare`` rows for ``auto_check`` and the ``beta``
         update channel. Each renders ``[✓]`` (checked) / ``[  ]`` (unchecked)
         and fires the persistence callback on every toggle via
         ``Checkbox.Changed``.
      3. A ``Rule.horizontal()`` separator between the toggles and the
         action buttons.
      4. ``Button`` widgets for "Check for Update now" and, when a
         skip-version is recorded, "Clear skip-version vX.Y.Z".
      5. Footer-hint bar with keyboard map (``space/enter`` toggles focused
         checkbox, ``c`` triggers an immediate check, ``esc`` closes).

    All toggle changes are persisted immediately by the caller's callbacks;
    ``check_now`` and ``clear_skip`` are returned via :meth:`Screen.dismiss`
    and handled by the app's launch-check path — the modal itself never
    touches the network or config writes.
    """

    BINDINGS = [
        Binding("escape", "close", "Close", show=False),
    ]

    def __init__(
        self,
        *,
        auto_check: bool,
        on_auto_check: Any,
        skip_version: str = "",
        check_interval_s: int = 86400,
        last_check_ts: int = 0,
        theme_name: str = "aurora",
    ) -> None:
        PickerModal.__init__(
            self,
            "Update Settings",
            [
                KeyHint("space", "Toggle"),
                KeyHint("esc", "Close"),
            ],
            theme_name=theme_name,
        )
        self._auto_check = bool(auto_check)
        self._skip_version = (skip_version or "").strip()
        self._check_interval_s = int(check_interval_s) if isinstance(check_interval_s, int) else 86400
        self._last_check_ts = int(last_check_ts) if isinstance(last_check_ts, int) else 0
        self._on_auto_check = on_auto_check
        # Hot color references so renderers pick up theme-aware hex strings.
        self._theme: ThemePreset = get_theme_preset(theme_name)
        # Optional callback to clear the skip-version marker; set by the app.
        self._on_clear_skip: Any = None

    # ── Header text ─────────────────────────────────────────────────────────
    def _format_interval(self) -> str:
        secs = self._check_interval_s
        if secs <= 0:
            return "every launch"
        if secs >= 86400 and secs % 86400 == 0:
            days = secs // 86400
            return f"every {days}d" if days > 1 else "every day"
        if secs >= 3600 and secs % 3600 == 0:
            hours = secs // 3600
            return f"every {hours}h" if hours > 1 else "every hour"
        return f"every {secs}s"

    def _format_last_check(self) -> str:
        if self._last_check_ts <= 0:
            return "never"
        import time

        return time.strftime("%Y-%m-%d %H:%M", time.localtime(self._last_check_ts))

    def _info_text(self) -> str:
        accent = self._theme.accent
        muted = self._theme.muted
        cadence = f"[{accent}]{self._format_interval()}[/]" if self._auto_check else f"[{muted}]off[/]"
        last = self._format_last_check()
        lines = [
            f"[bold {accent}]Auto-check:[/]  {cadence}   [bold {accent}]Last check:[/]  {last}",
            "",
            "Toggle switches reflect and persist the live setting immediately.",
        ]
        if self._skip_version:
            lines.append("")
            lines.append(
                f"[bold {self._theme.warning}]Skip-version:[/] v{self._skip_version}.\n"
                "Use the button below to clear it so you get notified about this release again."
            )
        return "\n".join(lines)

    # ── Compose ─────────────────────────────────────────────────────────────
    def compose_modal_content(self) -> ComposeResult:
        muted = self._theme.muted
        yield Static(self._info_text(), id="update-settings-info", markup=True)
        yield Rule.horizontal(id="update-settings-divider")
        # Two toggle rows using CheckBoxSquare — renders [✓] / [  ] with
        # theme-aware success and muted colours.
        with Horizontal(id="row-auto-check"):
            yield CheckBoxSquare(value=self._auto_check, id="checkbox-auto-check", compact=True)
            yield Static(
                f"[bold]Auto-check for updates[/]\n"
                f"[{muted}]Check GitHub in the background at the cadence above.[/]",
                id="label-auto-check",
                markup=True,
            )

        # Action buttons — using Textual Button.Pressed → on_button_pressed.
        with Horizontal(id="row-actions"):
            yield Button(
                "Check for Update now",
                id="btn-check-now",
                variant="primary",
            )
            if self._skip_version:
                yield Button(
                    f"Clear skip-version v{self._skip_version}",
                    id="btn-clear-skip",
                    variant="warning",
                )
        yield Static(
            f"[{muted}]Tip: Space or Enter toggles the focused checkbox.[/]",
            id="update-settings-foot",
            markup=True,
        )

    def on_modal_mounted(self) -> None:
        pass

    # ── Checkbox handlers (Textual native) ─────────────────────────────────
    def on_checkbox_changed(self, event: Checkbox.Changed) -> None:
        # ``Checkbox.Changed`` bubbles up from the CheckBoxSquare widget to
        # this modal, so we get the live new value on ``event.value``
        # regardless of which toggle fired. The checkbox already updated its
        # own reactive value; our job is only to persist via the
        # caller-registered callback and refresh the cadence line in the info
        # header.
        event.stop()
        if event.checkbox.id == "checkbox-auto-check":
            self._auto_check = event.value
            self._on_auto_check(event.value)
        with contextlib.suppress(Exception):
            self.query_one("#update-settings-info", Static).update(self._info_text())

    # ── Button handlers (Textual native) ────────────────────────────────────
    def on_button_pressed(self, event: Button.Pressed) -> None:
        event.stop()
        if event.button.id == "btn-check-now":
            self.dismiss("check_now")
        elif event.button.id == "btn-clear-skip":
            # Persist + drop the local skip-version flag, then remove the
            # button from the DOM so it cannot be pressed twice.
            self._skip_version = ""
            if self._on_clear_skip is not None:
                self._on_clear_skip()
            event.button.remove()
            with contextlib.suppress(Exception):
                self.query_one("#update-settings-info", Static).update(self._info_text())

    # ── Modal-level key handling ─────────────────────────────────────────────

    def _clear_skip_persist(self) -> None:
        # Kept for callers/tests that exercise the action directly. The button
        # path goes through ``on_button_pressed`` instead.
        cb = self._on_clear_skip
        if cb is not None:
            cb()

    def action_close(self) -> None:
        self.dismiss(None)


class UpdateBannerModal(PickerModal[str | None]):
    """Modal to notify the user of an available update (Phase 5)."""

    BINDINGS = [("escape", "close", "Dismiss")]

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
            "Update available",
            [
                KeyHint("enter", "Select"),
                KeyHint("escape", "Dismiss"),
            ],
            theme_name=theme_name,
        )
        self.options = [
            PickerOption("github", "Open Releases"),
            PickerOption("skip", "Skip this version"),
            PickerOption("close", "Dismiss"),
        ]
        from sky_music.domain.update_checker import UpdateInfo
        from sky_music.orchestration.update_service import format_update_banner
        
        update_info = UpdateInfo(
            latest_version=latest_version,
            download_url="",
            release_notes=release_notes,
            html_url="",
            published_at=published_at,
        )
        self._banner_text = format_update_banner(update_info, current_version=current_version)

    def compose_modal_content(self) -> ComposeResult:
        # Use a fresh id for snapshot testing (Phase 5)
        yield Static(self._banner_text, id="update-banner-info")
        yield OptionList(*(o.label for o in self.options), id="update-banner-options")

    def on_modal_mounted(self) -> None:
        options = self.query_one("#update-banner-options", OptionList)
        options.highlighted = 0
        self.set_focus(options)

    def on_key(self, event: events.Key) -> None:
        if event.key == "escape":
            event.stop()
            self.dismiss("close")
        elif event.key == "enter":
            event.stop()
            self._select_current()

    def on_option_list_option_selected(self, event: OptionList.OptionSelected) -> None:
        event.stop()
        self._select_current()

    def _select_current(self) -> None:
        options = self.query_one("#update-banner-options", OptionList)
        idx = options.highlighted
        if idx is not None and 0 <= idx < len(self.options):
            self.dismiss(str(self.options[idx].value))
        else:
            self.dismiss(None)
