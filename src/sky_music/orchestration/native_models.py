"""Small application-facing models mapped from the Rust dispatch session."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from enum import StrEnum


class NativeDispatchError(RuntimeError):
    """A controlled native worker failure after cleanup and report capture."""

    def __init__(
        self,
        message: str,
        *,
        snapshot: Mapping[str, object] | None = None,
        telemetry: Mapping[str, object] | None = None,
    ) -> None:
        super().__init__(message)
        self.snapshot = snapshot
        self.telemetry = telemetry


class NativeSessionStatus(StrEnum):
    """Lifecycle values produced by the Rust session boundary."""

    READY = "ready"
    PREROLL = "preroll"
    PLAYING = "playing"
    PAUSED = "paused"
    FINISHED = "finished"
    QUIT = "quit"
    SKIPPED = "skipped"
    ERROR = "error"
    PANICKED = "panicked"
    POISONED = "poisoned"


LIVE_NATIVE_STATUSES = frozenset(
    {NativeSessionStatus.PREROLL, NativeSessionStatus.PLAYING, NativeSessionStatus.PAUSED}
)
TERMINAL_NATIVE_STATUSES = frozenset(
    {
        NativeSessionStatus.FINISHED,
        NativeSessionStatus.QUIT,
        NativeSessionStatus.SKIPPED,
        NativeSessionStatus.ERROR,
        NativeSessionStatus.PANICKED,
        NativeSessionStatus.POISONED,
    }
)


def parse_native_session_status(raw: str) -> NativeSessionStatus:
    """Parse the Rust lifecycle domain, never the UI presentation domain."""

    try:
        return NativeSessionStatus(raw)
    except ValueError as exc:
        raise NativeDispatchError(f"unknown native session status: {raw}") from exc


class PlaybackOutcome(StrEnum):
    FINISHED = "finished"
    QUIT = "quit"
    SKIPPED = "skipped"
    SHUTDOWN_TIMEOUT = "shutdown_timeout"
    ERROR = "error"


class PlaybackStatus(StrEnum):
    PLAYING = "playing"
    PAUSED = "paused"
    FOCUS_LOST = "focus_lost"
    WAITING_FOR_FOCUS = "waiting_for_focus"
    REFOCUS = "refocus"
    PANIC = "panic"
    DONE = "done"


PLAYBACK_FINISHED = PlaybackOutcome.FINISHED
PLAYBACK_QUIT = PlaybackOutcome.QUIT
PLAYBACK_SKIPPED = PlaybackOutcome.SKIPPED
PLAYBACK_SHUTDOWN_TIMEOUT = PlaybackOutcome.SHUTDOWN_TIMEOUT
PLAYBACK_ERROR = PlaybackOutcome.ERROR
STATUS_LABELS: Mapping[PlaybackStatus, str] = {
    PlaybackStatus.PLAYING: "Playing",
    PlaybackStatus.PAUSED: "Paused",
    PlaybackStatus.FOCUS_LOST: "Focus Lost",
    PlaybackStatus.WAITING_FOR_FOCUS: "Waiting for Focus",
    PlaybackStatus.REFOCUS: "Refocusing",
    PlaybackStatus.PANIC: "Panic Release",
    PlaybackStatus.DONE: "Done",
}
RUST_DISPATCH_SCHEMA_VERSION = 4


@dataclass(frozen=True, slots=True)
class BackendHealth:
    """Live health counters exposed to the HUD.

    The name is retained for the renderer contract; this is no longer a Python
    input-backend model. Rust is the sole owner of the underlying state.
    """

    active_count: int
    possibly_active_count: int
    failed_release_count: int
    last_error: str | None
    min_same_key_up_gap_us: int | None = None
    impossible_same_key_repeats: int = 0
    send_while_unfocused: int = 0
    keys_dropped: int = 0
    chord_split_events: int = 0
    sendinput_partial_events: int = 0
    sendinput_zero_progress_failures: int = 0
    chords_rejected: int = 0
    authored_conflict_events: int = 0
    authored_chords_rejected: int = 0
    authored_keys_rejected: int = 0
    keys_inserted_before_failure: int = 0
    keys_rolled_back: int = 0
    rollback_residue_keys: int = 0

    @classmethod
    def from_native(cls, native: object) -> BackendHealth:
        """Map one complete native health object or final snapshot."""

        def required(name: str) -> object:
            try:
                if isinstance(native, Mapping):
                    return native[name]
                return getattr(native, name)
            except (AttributeError, KeyError) as exc:
                raise ValueError(f"native snapshot is missing required field: {name}") from exc

        def required_int(name: str) -> int:
            value = required(name)
            if isinstance(value, bool) or not isinstance(value, int):
                raise ValueError(f"native snapshot field {name} must be an integer")
            return value

        last_error = required("last_error")
        if last_error is not None and not isinstance(last_error, str):
            raise ValueError("native snapshot field last_error must be a string or null")
        return cls(
            active_count=required_int("active_count"),
            possibly_active_count=required_int("possibly_active_count"),
            failed_release_count=required_int("failed_release_count"),
            last_error=last_error,
            keys_dropped=required_int("keys_dropped"),
            chord_split_events=required_int("chord_split_events"),
            sendinput_partial_events=required_int("sendinput_partial_events"),
            sendinput_zero_progress_failures=required_int(
                "sendinput_zero_progress_failures"
            ),
            chords_rejected=required_int("chords_rejected"),
            authored_conflict_events=required_int("authored_conflict_events"),
            authored_chords_rejected=required_int("authored_chords_rejected"),
            authored_keys_rejected=required_int("authored_keys_rejected"),
            keys_inserted_before_failure=required_int("keys_inserted_before_failure"),
            keys_rolled_back=required_int("keys_rolled_back"),
            rollback_residue_keys=required_int("rollback_residue_keys"),
        )


@dataclass(frozen=True, slots=True)
class ReleaseAllOutcome:
    """Final native cleanup report used by telemetry and the UI."""

    attempted: tuple[int, ...]
    released_successfully: bool
    stuck_keys: tuple[int, ...]
    verification_inconclusive: bool


@dataclass(frozen=True, slots=True)
class ProgressCounters:
    max_lateness_us: int
    late_2ms: int
    late_5ms: int
    late_10ms: int
    release_max_us: int
    release_late_2ms: int
    recent_latencies_us: tuple[int, ...]
