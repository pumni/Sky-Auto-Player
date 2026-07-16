"""Playback state machine — the single-interval pause model + cross-thread display snapshot.

Post-Phase-1 pause accounting uses ONE contiguous interval owner: ``pause_reasons`` (a set)
plus one anchor (``pause_interval_started_us``). Enter-pause from an empty set captures
``now_us`` into the anchor; the second concurrent reason does not move the anchor; only
the last exiting reason accumulates the interval into ``pause_time_us`` exactly once.

The cross-thread display snapshot (``elapsed_snapshot_us``) is the Phase 4 §7.4 fix for
the race that exists in this otherwise-cleaner machine: the supervisor thread reads
``state`` for progress display while the dispatch thread mutates pause fields. Single
tuple reference assignment is atomic even under free-threaded Python, so readers always
observe either the pre-transition or post-transition snapshot — never a torn read.

Boundary rule (enforced by ``tests/test_core_boundary.py``): nothing in
``sky_music.orchestration.core`` may import from ``sky_music.platform.*``,
``sky_music.ui.*``, or ``sky_music.infrastructure.focus``. ``Clock`` is taken from
``sky_music.infrastructure.timing`` (allowed — public protocol).
"""

from __future__ import annotations

from dataclasses import dataclass, field

from sky_music.infrastructure.timing import Clock


@dataclass(slots=True)
class PlaybackState:
    """Owns playback pause accounting + cross-thread-safe display snapshot.

    Pause accounting uses a single contiguous-interval owner so interleaved
    focus-lost and manual-pause never double-count overlap into ``pause_time_us``
    (finding A2). Ownership model: dispatch-thread single-writer for the live
    pause fields and the ``_display_snapshot`` tuple; the supervisor may read
    :meth:`elapsed_snapshot_us` and :meth:`is_paused` for display only.

    Telemetry attribution: when a contiguous pause interval closes, its full
    duration is recorded under the *first* reason that opened it
    (``pause_open_reason``). Overlap is not split across reasons.
    """

    start_perf: int
    pause_time_us: int = 0
    # Nonempty ⇒ paused. Subset of {"manual", "focus"}.
    pause_reasons: set[str] = field(default_factory=set)
    # Wall anchor of the CURRENT contiguous paused interval (set when first reason enters).
    pause_interval_started_us: int | None = None
    # First reason that opened the current interval (telemetry attribution).
    pause_open_reason: str | None = None
    epoch_us: int = 0
    # Atomic display snapshot — single tuple assignment by the dispatch-thread writer;
    # cross-thread readers (supervisor) get either pre- or post-transition state via
    # ``elapsed_snapshot_us``. ``init=False`` so the constructor doesn't need a clock;
    # populated by ``__post_init__``.
    _display_snapshot: tuple[int, int | None, bool] = field(
        init=False,
        default=(0, None, False),
        repr=False,
        compare=False,
    )

    def __post_init__(self) -> None:
        self.epoch_us = self.start_perf + self.pause_time_us
        self._refresh_snapshot()

    def _refresh_snapshot(self) -> None:
        """Publish a new (epoch_us, pause_anchor, paused) tuple for cross-thread readers.

        Single-writer discipline: only the dispatch thread mutates pause state, so
        only this method writes. The tuple assignment is one reference store which
        is atomic even under free-threading — readers always observe a consistent
        pre- or post-transition snapshot rather than torn fields.
        """
        self._display_snapshot = (
            self.epoch_us,
            self.pause_interval_started_us,
            self.is_paused(),
        )

    def is_paused(self) -> bool:
        return bool(self.pause_reasons)

    def has_pause_reason(self, reason: str) -> bool:
        return reason in self.pause_reasons

    def enter_pause(self, reason: str, now_us: int) -> bool:
        """Add a pause reason. Returns True if this opened a new contiguous interval."""
        if reason in self.pause_reasons:
            return False
        was_empty = not self.pause_reasons
        self.pause_reasons.add(reason)
        if was_empty:
            self.pause_interval_started_us = now_us
            self.pause_open_reason = reason
        self._refresh_snapshot()
        return was_empty

    def exit_pause(self, reason: str, now_us: int) -> tuple[int, str] | None:
        """Remove a pause reason.

        When the reason set becomes empty, accumulate ``now - pause_interval_started_us``
        into ``pause_time_us`` exactly once and return ``(duration_us, attribution_reason)``
        where attribution is the first reason that opened the interval. While other
        reasons remain, returns None (interval still open; anchor unchanged).
        """
        if reason not in self.pause_reasons:
            return None
        self.pause_reasons.discard(reason)
        if self.pause_reasons:
            # Interval still open at a different (later) edge — anchor in the snapshot
            # is unchanged. Publish the new ``is_paused`` value regardless so a
            # concurrent reader sees the same bool it would compute from pause_reasons.
            self._refresh_snapshot()
            return None
        assert self.pause_interval_started_us is not None
        duration_us = now_us - self.pause_interval_started_us
        attribution = self.pause_open_reason or reason
        self.pause_interval_started_us = None
        self.pause_open_reason = None
        self.update_pause_time(duration_us)
        return duration_us, attribution

    def update_pause_time(self, duration_us: int) -> None:
        self.pause_time_us += duration_us
        self.epoch_us = self.start_perf + self.pause_time_us
        self._refresh_snapshot()

    def rebase_epoch(self, now_us: int) -> int:
        """Move the playback anchor to now and return the old-to-new delta."""
        old_start_perf = self.start_perf
        self.start_perf = now_us
        self.epoch_us = self.start_perf + self.pause_time_us
        self._refresh_snapshot()
        return now_us - old_start_perf

    def get_elapsed_us(self, clock: Clock, now_us: int | None = None) -> int:
        """Compute elapsed playback time in microseconds, accounting for pauses.

        Dispatch-thread call site (writer thread). Reads the live pause fields directly:
        they are SINGLE-WRITER so this read sees the writer's own updates immediately.

        While paused, elapsed is frozen at the interval start (never decreases with
        wall time). While playing, ``now - epoch_us``.
        """
        if now_us is None:
            now_us = clock.now_us()
        if self.pause_interval_started_us is not None:
            return max(0, self.pause_interval_started_us - self.epoch_us)
        return max(0, now_us - self.epoch_us)

    def elapsed_snapshot_us(self, clock: Clock) -> tuple[int, bool]:
        """Approximate (elapsed_us, paused) read for cross-thread display consumers.

        Implements Phase 4 §7.4: a single atomically-assigned tuple
        ``(epoch_us, pause_anchor, paused)`` is overwritten on every pause transition
        by the dispatch-thread writer (single-writer discipline). This method reads
        the tuple once and derives elapsed from it, returning both the elapsed
        microseconds and the paused boolean that the supervisor wants for
        conditional progress publishing.

        APPROXIMATE: a cross-thread reader may see elapsed slightly stale by one
        pause transition's interval anchor — display only, never use for
        timing-critical dispatch decisions. The exact ``get_elapsed_us`` method
        remains the writer-thread source of truth.
        """
        epoch_us, pause_anchor, paused = self._display_snapshot
        if paused:
            assert pause_anchor is not None
            return max(0, pause_anchor - epoch_us), True
        return max(0, clock.now_us() - epoch_us), False


__all__ = ["PlaybackState"]
