from __future__ import annotations

from typing import Any

from rich.table import Table

from sky_music.ui.textual_app.components.command_palette import CommandPaletteList

# Re-export from new component locations
from sky_music.ui.textual_app.components.footers import (
    AppFooter,
    CustomFooter,
    ModalHintBar,
)

__all__ = ["AppFooter", "CommandPaletteList", "CustomFooter", "GridRenderable", "ModalHintBar"]

class GridRenderable(Table):
    """A helper class to wrap a Table with a .plain property for test compatibility."""

    def __init__(self, *args: Any, plain_text: str = "", **kwargs: Any) -> None:
        super().__init__(*args, **kwargs)
        self._plain_text = plain_text

    @property
    def plain(self) -> str:
        return self._plain_text
