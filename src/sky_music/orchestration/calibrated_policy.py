"""Single-entry-point for calibration-aware policy resolution.

Every production path that resolves a :class:`~sky_music.domain.scheduler_types.FrameTimingPolicy`
with device-calibrated margins must go through :func:`resolve_calibrated_policy` rather than
calling ``session.resolve_effective_policy(cfg)`` directly.  This guarantees that:

1. The loader is called exactly once per resolution.
2. The ``min_hold_margin_source`` in the returned policy reflects the actual
   cache state, including unhealthy fallback states.
3. Console, Textual, and picker-metadata paths all behave identically.

Layer contract (AGENTS.md Architecture Invariants):
* ``infrastructure/`` may be imported here (this module lives in
  ``orchestration/``).
* ``domain/`` and ``orchestration/`` must not import ``ctypes``, wall-clock,
  or Windows-specific modules.
* The loader ``infrastructure.calibration_loader`` is the only place that
  does filesystem I/O for the calibration cache; this helper only calls it
  and forwards the primitives into the pure domain factory.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from sky_music.infrastructure.calibration_loader import (
    load_calibration_resolution,
)

if TYPE_CHECKING:
    from sky_music.config import AppConfig
    from sky_music.domain.scheduler_types import FrameTimingPolicy
    from sky_music.domain.session_context import PlaybackSessionContext


def resolve_calibrated_policy(
    session: PlaybackSessionContext,
    cfg: AppConfig,
) -> FrameTimingPolicy:
    """Resolve the effective :class:`~sky_music.domain.scheduler_types.FrameTimingPolicy`
    for *session* using the device-calibrated margin from the cache.

    This is the **production entry point** for any code that needs a
    playback policy.  It:

    1. Calls :func:`~sky_music.infrastructure.calibration_loader.load_calibration_resolution`
       to read ``.cache/input_latency.json`` and classify its health.
    2. Forwards ``hold_margin_us`` and ``hold_margin_source`` into
       :meth:`~sky_music.domain.session_context.PlaybackSessionContext.resolve_effective_policy`
       so the returned policy carries the correct ``min_hold_margin_source``.

    Parameters
    ----------
    session:
        The current playback session (hold selection, tempo, FPS, and controls).
    cfg:
        The loaded :class:`~sky_music.config.AppConfig`.

    Returns
    -------
    FrameTimingPolicy
        A fully-resolved timing policy whose hold margin reflects the device
        cache or the explicit fallback source for its health state.
    """
    resolution = load_calibration_resolution()
    return session.resolve_effective_policy(
        cfg,
        hold_margin_us=resolution.resolved_margin_us,
        hold_margin_source=resolution.margin_source,
    )


__all__ = ["resolve_calibrated_policy"]
