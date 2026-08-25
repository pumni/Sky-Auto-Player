"""Orchestration package — engine re-exports are lazy (PEP 562).

``PlaybackEngine`` pulls the native dispatch subtree, which is only needed
at playback time. Eager re-exports forced every startup path (any
``from sky_music.orchestration.<module> import ...`` runs this ``__init__``)
to load the full engine before the picker UI appeared. Importing
``sky_music.orchestration.engine`` or ``...telemetry`` directly, or reading
these names off the package, keeps working unchanged.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from sky_music.orchestration.engine import (
        PLAYBACK_ERROR,
        PLAYBACK_FINISHED,
        PLAYBACK_QUIT,
        PLAYBACK_SKIPPED,
        PlaybackEngine,
    )
    from sky_music.orchestration.telemetry import TelemetryLogger

_ENGINE_EXPORTS = frozenset(
    {
        "PLAYBACK_ERROR",
        "PLAYBACK_FINISHED",
        "PLAYBACK_QUIT",
        "PLAYBACK_SKIPPED",
        "PlaybackEngine",
    }
)
_TELEMETRY_EXPORTS = frozenset({"TelemetryLogger"})
_ALL_EXPORTS = _ENGINE_EXPORTS | _TELEMETRY_EXPORTS


def __getattr__(name: str) -> Any:
    if name in _ENGINE_EXPORTS:
        from sky_music.orchestration import engine

        return getattr(engine, name)
    if name in _TELEMETRY_EXPORTS:
        from sky_music.orchestration import telemetry

        return getattr(telemetry, name)
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")


def __dir__() -> list[str]:
    return sorted(_ALL_EXPORTS)


__all__ = [
    "PLAYBACK_ERROR",
    "PLAYBACK_FINISHED",
    "PLAYBACK_QUIT",
    "PLAYBACK_SKIPPED",
    "PlaybackEngine",
    "TelemetryLogger",
]
