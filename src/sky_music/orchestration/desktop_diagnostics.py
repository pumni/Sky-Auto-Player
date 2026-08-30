"""Bounded, renderer-independent diagnostics for the Desktop Core.

The service is an observer of the existing native progress snapshot.  It does
not calculate deadlines, alter the dispatch session, or feed any value back to
the playback worker.  The caller is expected to invoke ``publish_progress``
from its existing low-rate UI snapshot path.
"""

from __future__ import annotations

import math
import threading
import time
from collections.abc import Callable, Sequence

from sky_music.orchestration.desktop_models import DiagnosticsSnapshotDto
from sky_music.orchestration.native_models import BackendHealth, ProgressCounters

MAX_DIAGNOSTICS_HZ = 10
DIAGNOSTICS_INTERVAL_S = 1.0 / MAX_DIAGNOSTICS_HZ
MAX_DIAGNOSTICS_TEXT_BYTES = 4096


def _percentile(values: Sequence[int], fraction: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(int(value) for value in values)
    index = min(len(ordered) - 1, round(fraction * (len(ordered) - 1)))
    return ordered[index] / 1000.0


def _sigma_ms(values: Sequence[int]) -> float:
    if not values:
        return 0.0
    mean = sum(values) / len(values)
    return (
        math.sqrt(sum((value - mean) ** 2 for value in values) / len(values)) / 1000.0
    )


def _backend_status(backend: BackendHealth | None) -> str:
    if backend is None:
        return "unavailable"
    if backend.last_error or backend.failed_release_count > 0:
        return "error"
    if (
        backend.keys_dropped > 0
        or backend.chord_split_events > 0
        or backend.possibly_active_count > backend.active_count
    ):
        return "degraded"
    return "healthy"


class DesktopDiagnosticsService:
    """Enable/disable gate and latest-wins publisher for native diagnostics."""

    def __init__(
        self,
        *,
        publish_event: Callable[[str, dict[str, object]], None],
        clock: Callable[[], float] = time.monotonic,
    ) -> None:
        self._publish_event = publish_event
        self._clock = clock
        self._lock = threading.Lock()
        self._enabled = False
        self._epoch = 0
        self._seq = 0
        self._last_emit_at: float | None = None

    @property
    def enabled(self) -> bool:
        with self._lock:
            return self._enabled

    def set_enabled(self, enabled: bool) -> bool:
        if type(enabled) is not bool:
            raise TypeError("diagnostics enabled must be boolean")
        with self._lock:
            self._enabled = enabled
            self._epoch += 1
            if enabled:
                # Re-enabling creates a fresh sampling window while keeping
                # sequence IDs monotonic for consumers that retain history.
                self._last_emit_at = None
            else:
                self._last_emit_at = None
            return self._enabled

    def publish_progress(
        self,
        counters: ProgressCounters,
        backend: BackendHealth | None,
        *,
        session_id: str | None = None,
        now: float | None = None,
    ) -> bool:
        """Publish one bounded snapshot when diagnostics are enabled and due."""

        sample_time = self._clock() if now is None else float(now)
        recent = tuple(int(value) for value in counters.recent_latencies_us)
        with self._lock:
            if not self._enabled:
                return False
            if (
                self._last_emit_at is not None
                and sample_time - self._last_emit_at < DIAGNOSTICS_INTERVAL_S
            ):
                return False
            self._last_emit_at = sample_time
            self._seq += 1
            epoch = self._epoch
            snapshot = DiagnosticsSnapshotDto(
                seq=self._seq,
                max_lateness_us=max(0, int(counters.max_lateness_us)),
                p50_ms=_percentile(recent, 0.50),
                p95_ms=_percentile(recent, 0.95),
                sigma_onset_ms=_sigma_ms(recent),
                late_2ms=max(0, int(counters.late_2ms)),
                late_5ms=max(0, int(counters.late_5ms)),
                late_10ms=max(0, int(counters.late_10ms)),
                active_keys=max(0, int(backend.active_count)) if backend else 0,
                stuck_keys=max(0, int(backend.failed_release_count)) if backend else 0,
                keys_dropped=max(0, int(backend.keys_dropped)) if backend else 0,
                chord_split_events=(
                    max(0, int(backend.chord_split_events)) if backend else 0
                ),
                backend_status=_backend_status(backend),
                release_max_us=(
                    max(0, int(counters.release_max_us))
                    if int(counters.release_max_us) > 0
                    else None
                ),
                release_late_2ms=(
                    max(0, int(counters.release_late_2ms))
                    if int(counters.release_late_2ms) > 0
                    else None
                ),
                session_id=session_id,
            )
            # Keep the disable operation authoritative even if it races the
            # renderer callback. The event is sent while the same lock is held
            # so no post-disable sample can escape this gate.
            if not self._enabled or epoch != self._epoch:
                return False
            from dataclasses import asdict

            self._publish_event("diagnostics.snapshot", asdict(snapshot))
            return True


__all__ = [
    "DIAGNOSTICS_INTERVAL_S",
    "MAX_DIAGNOSTICS_HZ",
    "DesktopDiagnosticsService",
]
