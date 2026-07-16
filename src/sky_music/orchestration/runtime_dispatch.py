"""Compatibility shim — the coordinator moved into ``core/coordinator.py`` (Phase 4 §7.1).

``RuntimeDispatchCoordinator`` and its data types are the dispatch core's schedule
engine; Phase 4 relocated them under ``sky_music.orchestration.core`` so the future
Rust worker replaces a self-contained seam. This module re-exports the moved names so
existing importers (``engine``, tests) keep working unchanged. New code should import
from ``sky_music.orchestration.core.coordinator`` directly.
"""

from __future__ import annotations

from sky_music.orchestration.core.coordinator import (
    ActiveKeyGeneration as ActiveKeyGeneration,
)
from sky_music.orchestration.core.coordinator import (
    GenerationStatus as GenerationStatus,
)
from sky_music.orchestration.core.coordinator import (
    PendingRelease as PendingRelease,
)
from sky_music.orchestration.core.coordinator import (
    RuntimeActionBatch as RuntimeActionBatch,
)
from sky_music.orchestration.core.coordinator import (
    RuntimeDispatchCoordinator as RuntimeDispatchCoordinator,
)
from sky_music.orchestration.core.coordinator import (
    RuntimeKeyIntent as RuntimeKeyIntent,
)
from sky_music.orchestration.core.coordinator import (
    RuntimeSchedule as RuntimeSchedule,
)
from sky_music.orchestration.core.coordinator import (
    compile_runtime_intents as compile_runtime_intents,
)

__all__ = [
    "ActiveKeyGeneration",
    "GenerationStatus",
    "PendingRelease",
    "RuntimeActionBatch",
    "RuntimeDispatchCoordinator",
    "RuntimeKeyIntent",
    "RuntimeSchedule",
    "compile_runtime_intents",
]
