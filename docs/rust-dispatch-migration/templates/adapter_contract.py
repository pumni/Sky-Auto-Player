"""Contract sketch only; integrate with current engine/protocols."""
from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from typing import Literal


@dataclass(frozen=True, slots=True)
class ActionDTO:
    source_action_index: int
    kind: Literal["down", "up"]
    at_us: int
    scan_codes: tuple[int, ...]
    reason: str


class RustDispatchRuntime:
    """The only Python layer allowed to import sky_player_rs."""

    def __init__(self, native_session: object) -> None:
        self._session = native_session

    @classmethod
    def prepare(cls, actions: Sequence[ActionDTO], config: dict[str, object]):
        import sky_player_rs
        return cls(sky_player_rs.DispatchSession.prepare(actions, config))

    def send_command(self, command: str) -> bool:
        return bool(self._session.send_command(command))

    def update_focus(self, active: bool, hwnd: int | None) -> None:
        self._session.update_focus(active, hwnd)

    def snapshot(self):
        return self._session.snapshot()

    def join(self, timeout_ms: int | None = None):
        return self._session.join(timeout_ms)
