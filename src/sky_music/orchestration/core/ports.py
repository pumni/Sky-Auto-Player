"""Protocol seam for the dispatch core.

The dispatch core (``sky_music.orchestration.core``) replaces today's
duck-typed side channels with typed Protocols so the future Rust
worker consumes a well-defined surface and nothing else.

Boundary rule (enforced by ``tests/test_core_boundary.py``): nothing
under ``core/`` imports from ``sky_music.platform.*``, ``sky_music.ui.*``,
or ``sky_music.infrastructure.focus``. Platform, UI, and policy live
behind the ports declared here or behind the concrete adapter classes
that implement them (``DirectFocusSignal``, ``WinSendInputBackend``, …).

Why re-exports: ``Clock`` / ``Sleeper`` / ``SleepPolicy`` /
``WaitStrategy`` / ``InputBackend`` already live in
``infrastructure/*``, which is *not* a forbidden dependency. Moving
the definitions would churn every implementation site; the seam is
exposed via ``core/ports.*`` for callers while the contracts stay
where they are.
"""

from __future__ import annotations

from enum import StrEnum
from typing import Protocol

from sky_music.domain.scheduler_types import ActionKind

# Re-exports of protocols from infrastructure.* — the seam surface. The ``as Name as
# Name`` form is the explicit re-export idiom: callers can write
# ``from sky_music.orchestration.core.ports import InputBackend`` instead of reaching
# into infrastructure.backend, and the linters see the names as "accessed" — exactly
# the same pattern as ``orchestration.playback_supervisor`` for its own re-exports.
from sky_music.infrastructure.backend import (
    BackendHealth as BackendHealth,
)
from sky_music.infrastructure.backend import (
    InputBackend as InputBackend,
)
from sky_music.infrastructure.backend import (
    InputSendResult as InputSendResult,
)
from sky_music.infrastructure.backend import (
    ReleaseAllOutcome as ReleaseAllOutcome,
)
from sky_music.infrastructure.timing import (
    Clock as Clock,
)
from sky_music.infrastructure.timing import (
    PerfCounterClock as PerfCounterClock,
)
from sky_music.infrastructure.timing import (
    RealSleeper as RealSleeper,
)
from sky_music.infrastructure.timing import (
    Sleeper as Sleeper,
)
from sky_music.infrastructure.timing import (
    SleepPolicy as SleepPolicy,
)
from sky_music.infrastructure.wait_strategy import (
    HybridWaitStrategy as HybridWaitStrategy,
)
from sky_music.infrastructure.wait_strategy import (
    WaitStrategy as WaitStrategy,
)

# Playback outcome strings — the loop's ``run()`` return contract.
PLAYBACK_FINISHED = "finished"
PLAYBACK_QUIT = "quit"
PLAYBACK_SKIPPED = "skipped"
PLAYBACK_SHUTDOWN_TIMEOUT = "shutdown_timeout"


class PlaybackCommand(StrEnum):
    """Typed commands crossing the core boundary.

    ``StrEnum`` preserves every existing ``== "pause"`` comparison during
    migration. Producers at the edge (hotkeys, Textual command bridge,
    ``QueueCommandSource`` fill sites) construct the enum; consumers in
    ``core/`` receive a ``PlaybackCommand``. Legacy string producers are
    still accepted via :meth:`coerce` so the migration is non-breaking;
    value mismatches map to ``None`` so callers can defensively ignore
    them at the edge without crashing the dispatch thread.
    """

    PAUSE = "pause"
    SKIP = "skip"
    QUIT = "quit"
    PANIC = "panic"
    REFOCUS = "refocus"
    RESUME = "resume"

    @staticmethod
    def coerce(value: object) -> PlaybackCommand | None:
        """Best-effort parse from a string (or enum) command."""
        if isinstance(value, PlaybackCommand):
            return value
        if isinstance(value, str):
            try:
                return PlaybackCommand(value)
            except ValueError:
                return None
        return None


class CommandSource(Protocol):
    """Legacy string-poll command source — kept for the migration window.

    See ``PlaybackCommand`` for the typed replacement. Implementations
    may return either a ``PlaybackCommand`` member or a raw string; the
    core coerces via :meth:`PlaybackCommand.coerce`. Returning ``None``
    means "no command this poll".
    """

    def poll(self) -> str | None: ...


class FocusSignal(Protocol):
    """A bool-valued "is the target app focused" signal sampled by the core.

    The supervisor owns the real sampling cadence (periodic thread); the
    core consumes a ``set_active``-shared flag of this shape when in
    threaded mode, and a direct callable wrapper (``DirectFocusSignal``)
    when in direct mode.
    """

    def is_active(self) -> bool: ...


class FocusController(Protocol):
    """Focus *policy* port — the seam replacement for ``infrastructure.focus.FocusGuard``.

    The concrete win32 guard (``Win32SkyFocusGuard`` / ``NoopFocusGuard``) lives in
    ``infrastructure.focus`` which the core MUST NOT import (boundary rule). The core
    types the health monitor's guard against this Protocol instead; the concrete guard
    satisfies it structurally. ``is_active`` answers "is the target focused"; ``focus``
    attempts to bring it foreground (used by the ``refocus`` command) and returns whether
    the attempt believes it succeeded.
    """

    def is_active(self) -> bool: ...

    def focus(self) -> bool: ...


class ProgressSink(Protocol):
    """Display-side update port — receives elapsed/total + status + counters.

    The implementation lives at the edge (Textual renderer, snapshot
    puller for tests, …); the core only writes through this contract.
    """

    def publish(
        self,
        *,
        elapsed_us: int,
        total_us: int,
        status: str,
        lateness_us: int | None = None,
        health: BackendHealth | None = None,
        input_path_degraded: bool = False,
        force: bool = False,
    ) -> None: ...

    def finish(self, message: str) -> None: ...

    def update_counters(self, lateness_us: int, kind: str = "down") -> None: ...


class LeadEstimator(Protocol):
    """Per-kind EMA of SendInput durations used to derive the dispatch lead.

    Owned by the engine so the warm cache survives across plays; consumed by the loop
    only via this protocol (§7.4 — the loop holds no engine back-reference). The engine's
    actual ``SendLatencyEstimator`` implementation lives next to the engine and is not
    pulled into the core. ``kind`` is the ``ActionKind`` (down/up); ``n_keys`` buckets the
    EMA by polyphony.
    """

    def get_lead_us(self, kind: ActionKind, n_keys: int = 1) -> int: ...

    def update(self, kind: ActionKind, duration_us: int, n_keys: int = 1) -> None: ...

    def update_completion_error(self, kind: ActionKind, error_us: int) -> None: ...
