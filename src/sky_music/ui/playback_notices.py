"""Shared playback notice state for all live HUD renderers."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Literal

NoticeSeverity = Literal["warning", "danger"]
NoticeSource = Literal["schedule", "runtime", "backend"]


@dataclass(frozen=True, slots=True)
class PlaybackNotice:
    code: str
    message: str
    severity: NoticeSeverity
    source: NoticeSource


@dataclass(frozen=True, slots=True)
class PlaybackHudState:
    """The renderer-neutral notice state for one playback poll."""

    persistent_notices: tuple[PlaybackNotice, ...] = ()
    runtime_notices: tuple[PlaybackNotice, ...] = ()
    backend_notices: tuple[PlaybackNotice, ...] = ()

    @property
    def notices(self) -> tuple[PlaybackNotice, ...]:
        return self.persistent_notices + self.runtime_notices + self.backend_notices


class PlaybackNoticeLedger:
    """Keeps schedule notices and first-seen backend failures alive for a session."""

    def __init__(self, schedule_warnings: tuple[str, ...] = ()) -> None:
        self._schedule_notices = tuple(
            PlaybackNotice(
                code=f"schedule-{index}",
                message=warning,
                severity="warning",
                source="schedule",
            )
            for index, warning in enumerate(schedule_warnings)
        )
        self._note_on_drop_seen = False
        self._note_on_drop_count = 0
        self._chord_split_count = 0

    def update(
        self,
        *,
        input_path_degraded: bool = False,
        keys_dropped: int = 0,
        chord_split_events: int = 0,
    ) -> PlaybackHudState:
        if keys_dropped > 0:
            self._note_on_drop_seen = True
            self._note_on_drop_count = max(self._note_on_drop_count, keys_dropped)
            self._chord_split_count = max(self._chord_split_count, chord_split_events)

        runtime = (
            PlaybackNotice(
                code="input-path-degraded",
                message="Input dispatch latency is elevated; playback timing may be unstable.",
                severity="warning",
                source="runtime",
            ),
        ) if input_path_degraded else ()

        backend = (
            PlaybackNotice(
                code="note-on-drops",
                message=(
                    f"Note-on drops: {self._note_on_drop_count} key(s) not injected "
                    f"({self._chord_split_count} chord split(s)) — incomplete chord, not late-retried."
                ),
                severity="danger",
                source="backend",
            ),
        ) if self._note_on_drop_seen else ()

        return PlaybackHudState(
            persistent_notices=self._schedule_notices,
            runtime_notices=runtime,
            backend_notices=backend,
        )
