from __future__ import annotations

from typing import Any

from rich.markup import escape
from textual import events
from textual.app import ComposeResult
from textual.containers import Horizontal
from textual.widgets import Static

from sky_music.ui.textual_app.keymap import PICKER_HINTS, KeyHint


class FooterAction(Static):
    """A clickable action button in the footer."""

    def __init__(self, hint: KeyHint, key_color: str, muted_color: str, **kwargs: Any) -> None:
        super().__init__("", markup=True, **kwargs)
        self.hint = hint
        self.key_color = key_color
        self.muted_color = muted_color
        self._update_label()

    def _update_label(self) -> None:
        key = escape(self.hint.key)
        lbl = escape(self.hint.label)
        label = f" [bold {self.key_color}]{key}[/] [{self.muted_color}]{lbl}[/] "
        self.update(label)

    def on_click(self, event: events.Click) -> None:
        event.stop()
        if self.hint.action:
            from sky_music.ui.textual_app.messages import PickerActionRequested
            action = self.hint.action.rsplit('.', 1)[-1]
            self.post_message(PickerActionRequested(action))

    def update_theme(self, key_color: str, muted_color: str) -> None:
        self.key_color = key_color
        self.muted_color = muted_color
        self._update_label()


class AppFooter(Horizontal):
    """Clean app action bar/footer for the main picker screen."""

    DEFAULT_CSS = """
    AppFooter {
        dock: bottom;
        height: 1;
        layout: horizontal;
    }
    FooterAction {
        width: auto;
    }
    .footer-separator {
        width: auto;
    }
    """

    def __init__(self, hints: list[KeyHint], *, key_color: str = "#facc15", muted_color: str = "#6b7a93", **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.hints = hints
        self.key_color = key_color
        self.muted_color = muted_color

    def compose(self) -> ComposeResult:
        for index, hint in enumerate(self.hints):
            if index > 0:
                sep = Static("  ·  ", classes="footer-separator", markup=True)
                sep.styles.color = self.muted_color
                yield sep
            yield FooterAction(hint, self.key_color, self.muted_color)

    def set_theme(self, key_color: str, muted_color: str) -> AppFooter:
        self.key_color = key_color
        self.muted_color = muted_color
        for action in self.query(FooterAction):
            action.update_theme(key_color, muted_color)
        for sep in self.query(".footer-separator"):
            sep.styles.color = muted_color
        return self


class CustomFooter(AppFooter):
    """A custom, clean footer that displays a concise hint of the most important keys."""

    def __init__(self, **kwargs: Any) -> None:
        AppFooter.__init__(self, PICKER_HINTS, **kwargs)


class ModalHintBar(Horizontal):
    """Muted instruction footer/hint bar for modals."""

    DEFAULT_CSS = """
    ModalHintBar {
        height: 1;
        layout: horizontal;
        align: center middle;
    }
    ModalHintBar Static {
        width: auto;
    }
    """

    def __init__(self, hints: list[KeyHint], *, key_color: str = "#6b7a93", muted_color: str = "#6b7a93", **kwargs: Any) -> None:
        super().__init__(**kwargs)
        self.hints = hints
        self.key_color = key_color
        self.muted_color = muted_color

    def compose(self) -> ComposeResult:
        for index, hint in enumerate(self.hints):
            if index > 0:
                sep = Static("  ·  ", classes="modal-footer-separator", markup=True)
                sep.styles.color = self.muted_color
                yield sep
            
            key_display = hint.key.lower()
            label_display = hint.label.lower()
            label = f"[bold {self.key_color}] {escape(key_display)} [/][{self.muted_color}] {escape(label_display)}[/]"
            yield Static(label, classes="modal-footer-hint", markup=True)

    def set_theme(self, key_color: str, muted_color: str) -> ModalHintBar:
        self.key_color = key_color
        self.muted_color = muted_color
        hint_statics = list(self.query(".modal-footer-hint").results(Static))
        if len(hint_statics) != len(self.hints):
            return self
        for hint, st in zip(self.hints, hint_statics, strict=True):
            key_display = hint.key.lower()
            label_display = hint.label.lower()
            label = f"[bold {self.key_color}] {escape(key_display)} [/][{self.muted_color}] {escape(label_display)}[/]"
            st.update(label)
        for sep in self.query(".modal-footer-separator"):
            sep.styles.color = muted_color
        return self
