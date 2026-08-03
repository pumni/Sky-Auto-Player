"""Small application-facing models mapped from the Rust dispatch session."""

from __future__ import annotations

from dataclasses import dataclass

PLAYBACK_FINISHED = "finished"
PLAYBACK_QUIT = "quit"
PLAYBACK_SKIPPED = "skipped"
PLAYBACK_SHUTDOWN_TIMEOUT = "shutdown_timeout"
PLAYBACK_ERROR = "error"
RUST_DISPATCH_SCHEMA_VERSION = 2


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
