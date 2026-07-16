"""Compatibility shim — the dispatch loop moved into ``core/loop.py`` (Phase 4 §7.1/§7.7).

``DispatchLoop`` and its helpers are the real-time dispatch core; Phase 4 relocated them
under ``sky_music.orchestration.core`` so the future Rust worker replaces a self-contained,
platform-free seam. This module re-exports the moved names so existing importers (``engine``,
tests) keep working unchanged. New code should import from
``sky_music.orchestration.core.loop`` (or the ``core`` package) directly.
"""

from __future__ import annotations

from sky_music.orchestration.core.loop import (
    DispatchHealthMonitor as DispatchHealthMonitor,
)
from sky_music.orchestration.core.loop import (
    DispatchLoop as DispatchLoop,
)
from sky_music.orchestration.core.loop import (
    ExecutionResult as ExecutionResult,
)
from sky_music.orchestration.core.loop import (
    LeadEstimator as LeadEstimator,
)
from sky_music.orchestration.core.loop import (
    OutcomeResolver as OutcomeResolver,
)
from sky_music.orchestration.core.loop import (
    RuntimeSameKeyConflictError as RuntimeSameKeyConflictError,
)
from sky_music.orchestration.core.state import (
    PlaybackState as PlaybackState,
)

__all__ = [
    "DispatchHealthMonitor",
    "DispatchLoop",
    "ExecutionResult",
    "LeadEstimator",
    "OutcomeResolver",
    "PlaybackState",
    "RuntimeSameKeyConflictError",
]
