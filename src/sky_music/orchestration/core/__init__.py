"""Core dispatch package — the platform-free real-time seam (Phase 4 §7).

This package is the well-defined boundary the future Rust worker replaces:
*(compiled schedule in, backend + clock + waiter in, command/focus/progress
ports in, result + telemetry stream out)* and nothing else. The boundary is
enforced structurally by ``tests/test_core_boundary.py``: no module here imports
``sky_music.platform.*``, ``sky_music.ui.*``, or ``sky_music.infrastructure.focus``.

Layout:
- ``loop`` — ``DispatchLoop`` + ``DispatchHealthMonitor`` (the wait→drain→execute loop).
- ``coordinator`` — ``RuntimeDispatchCoordinator`` (schedule → batches, generation tracking).
- ``state`` — ``PlaybackState`` (single-interval pause machine + cross-thread display snapshot).
- ``ports`` — the typed Protocols (``InputBackend``, ``Clock``, ``WaitStrategy``,
  ``CommandSource``, ``FocusSignal``, ``FocusController``, ``ProgressSink``,
  ``LeadEstimator``) and ``PlaybackCommand``.
"""

from sky_music.orchestration.core.coordinator import RuntimeDispatchCoordinator
from sky_music.orchestration.core.loop import DispatchHealthMonitor, DispatchLoop
from sky_music.orchestration.core.ports import PlaybackCommand
from sky_music.orchestration.core.state import PlaybackState

__all__: list[str] = [
    "DispatchHealthMonitor",
    "DispatchLoop",
    "PlaybackCommand",
    "PlaybackState",
    "RuntimeDispatchCoordinator",
]
