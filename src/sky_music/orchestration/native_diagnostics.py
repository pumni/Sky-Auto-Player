"""Post-session classification of native dispatch evidence.

This module runs outside the worker hot path. Its vocabulary is deliberately
limited to observations owned by Sky Auto Player; it never claims game receipt
or note onset.
"""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from enum import StrEnum


class DispatchDiagnostic(StrEnum):
    BACKEND_REJECTION = "backend_rejection"
    RECOVERED_RETRY_LATE = "recovered_retry_late"
    SCHEDULER_WAKE_DEGRADED = "scheduler_wake_degraded"
    SEND_LATENCY_DEGRADED = "send_latency_degraded"
    POST_SEND_DEGRADED = "post_send_degraded"
    LEAD_SATURATED = "lead_saturated"
    TIMELINE_REBASED = "timeline_rebased"
    CLEAN_NATIVE_DELIVERY = "clean_native_delivery"


@dataclass(frozen=True, slots=True)
class NativePlaybackDiagnosis:
    category: DispatchDiagnostic
    evidence: tuple[str, ...]


def _positive(value: object) -> bool:
    if isinstance(value, bool):
        return False
    if isinstance(value, (int, float)):
        return value > 0
    if isinstance(value, (list, tuple)):
        return any(_positive(item) for item in value)
    return False


def diagnose_native_playback(
    snapshot: Mapping[str, object],
) -> NativePlaybackDiagnosis:
    """Return the highest-priority native observation in ``snapshot``."""

    def positive(name: str) -> bool:
        return _positive(snapshot.get(name, 0))

    generation_counts = snapshot.get("generation_status_counts", {})
    dropped_backend = isinstance(generation_counts, Mapping) and _positive(
        generation_counts.get("dropped_backend", 0)
    )

    backend_names = (
        "keys_dropped",
        "dropped_backend",
        "sendinput_partial_events",
        "sendinput_zero_progress_failures",
        "chords_rejected",
        "authored_keys_rejected",
        "failed_release_count",
        "rollback_residue_keys",
    )
    backend = dropped_backend or any(positive(name) for name in backend_names)
    recovered_late = positive("recovered_zero_progress_but_late")
    wait = bool(snapshot.get("wait_path_degraded", False)) or positive(
        "wait_degraded_samples"
    )
    send = bool(snapshot.get("sendinput_path_degraded", False)) or positive(
        "sendinput_degraded_samples"
    )
    post_send = bool(snapshot.get("core_post_send_degraded", False)) or positive(
        "core_post_send_degraded_samples"
    )
    lead = positive("positive_residual_at_cap") or positive(
        "lead_saturation_count_down"
    ) or positive("lead_saturation_count_up")
    rebased = positive("timeline_rebase_count")

    if backend:
        return NativePlaybackDiagnosis(
            DispatchDiagnostic.BACKEND_REJECTION,
            ("native backend rejection counters are non-zero",),
        )
    if recovered_late:
        return NativePlaybackDiagnosis(
            DispatchDiagnostic.RECOVERED_RETRY_LATE,
            ("a zero-progress retry recovered after the authored deadline",),
        )
    if wait:
        return NativePlaybackDiagnosis(
            DispatchDiagnostic.SCHEDULER_WAKE_DEGRADED,
            ("scheduler wake latency evidence is elevated",),
        )
    if send:
        return NativePlaybackDiagnosis(
            DispatchDiagnostic.SEND_LATENCY_DEGRADED,
            ("SendInput latency evidence is elevated",),
        )
    if post_send:
        return NativePlaybackDiagnosis(
            DispatchDiagnostic.POST_SEND_DEGRADED,
            ("native post-send occupancy evidence is elevated",),
        )
    if lead:
        return NativePlaybackDiagnosis(
            DispatchDiagnostic.LEAD_SATURATED,
            ("adaptive lead reached its configured cap with positive residual",),
        )
    if rebased:
        return NativePlaybackDiagnosis(
            DispatchDiagnostic.TIMELINE_REBASED,
            ("the authored timeline was rebased for recovery",),
        )
    return NativePlaybackDiagnosis(
        DispatchDiagnostic.CLEAN_NATIVE_DELIVERY,
        ("Native dispatch completed without a reported SendInput rejection.",),
    )


# Compatibility alias for callers that imported the old name.
DiagnosisCategory = DispatchDiagnostic
