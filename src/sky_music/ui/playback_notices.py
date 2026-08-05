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
        # Backend failures are the strongest evidence and must lead the HUD;
        # transient path warnings follow, then schedule guidance.
        return self.backend_notices + self.runtime_notices + self.persistent_notices


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
        self._rejected_chords = 0
        self._rejected_keys = 0
        self._partial_packet_seen = False

    def update(
        self,
        *,
        input_path_degraded: bool = False,
        sendinput_path_degraded: bool = False,
        bookkeeping_degraded: bool = False,
        wait_path_degraded: bool = False,
        keys_dropped: int = 0,
        chord_split_events: int = 0,
        chords_rejected: int = 0,
        authored_keys_rejected: int = 0,
        sendinput_partial_events: int = 0,
        sendinput_zero_progress_failures: int = 0,
    ) -> PlaybackHudState:
        if any(
            value > 0
            for value in (
                keys_dropped,
                chords_rejected,
                authored_keys_rejected,
                sendinput_partial_events,
                sendinput_zero_progress_failures,
            )
        ):
            self._note_on_drop_seen = True
            self._note_on_drop_count = max(self._note_on_drop_count, keys_dropped)
            self._chord_split_count = max(self._chord_split_count, chord_split_events)
            self._rejected_chords = max(self._rejected_chords, chords_rejected)
            self._rejected_keys = max(
                self._rejected_keys, authored_keys_rejected, keys_dropped
            )
            self._partial_packet_seen = self._partial_packet_seen or (
                sendinput_partial_events > 0
            )

        rejected_chords = self._rejected_chords
        rejected_keys = self._rejected_keys

        runtime: list[PlaybackNotice] = []
        if wait_path_degraded:
            runtime.append(
                PlaybackNotice(
                    code="scheduler-wake-slow",
                    message="Scheduler wake latency is elevated; playback deadlines may be missed.",
                    severity="warning",
                    source="runtime",
                )
            )
        if sendinput_path_degraded or (input_path_degraded and not bookkeeping_degraded):
            runtime.append(
                PlaybackNotice(
                    code="sendinput-slow",
                    message="Windows input injection is responding slowly; note timing may be delayed.",
                    severity="warning",
                    source="runtime",
                )
            )
        if bookkeeping_degraded:
            runtime.append(
                PlaybackNotice(
                    code="native-bookkeeping-slow",
                    message="Native post-send processing is elevated; timing diagnostics are active.",
                    severity="warning",
                    source="runtime",
                )
            )

        backend_list: list[PlaybackNotice] = []
        if self._note_on_drop_seen:
            backend_list.append(
                PlaybackNotice(
                    code="native-input-rejection",
                    message=(
                        f"Native input rejection detected: {rejected_chords} chord(s), "
                        f"{rejected_keys} authored key(s)."
                    ),
                    severity="danger",
                    source="backend",
                )
            )
            if self._partial_packet_seen:
                backend_list.append(
                    PlaybackNotice(
                        code="partial-input-packet",
                        message="A Windows input packet was partially accepted; chord integrity was lost.",
                        severity="danger",
                        source="backend",
                    )
                )
        backend = tuple(backend_list)

        # Kept only as a source-level migration marker; the old aggregate
        # wording "Input dispatch latency is elevated; playback timing may be unstable."
        # must not be rendered now that each health path has its own signal.

        return PlaybackHudState(
            persistent_notices=self._schedule_notices,
            runtime_notices=tuple(runtime),
            backend_notices=backend,
        )
