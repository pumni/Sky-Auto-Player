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


def diagnose_native_playback(
    snapshot: Mapping[str, object],
) -> NativePlaybackDiagnosis:
    """Return the highest-priority native observation in ``snapshot``."""

    def positive(name: str) -> bool:
        value = snapshot.get(name, 0)
        return isinstance(value, (int, float)) and not isinstance(value, bool) and value > 0

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
    backend = any(positive(name) for name in backend_names)
    recovered_late = positive("recovered_zero_progress_but_late")
    wait = bool(snapshot.get("wait_path_degraded", False)) or positive(
        "wait_degraded_samples"
    )
    send = bool(snapshot.get("sendinput_path_degraded", False)) or positive(
        "sendinput_degraded_samples"
    )
    post_send = bool(snapshot.get("bookkeeping_degraded", False)) or positive(
        "post_send_degraded_samples"
    ) or positive("bookkeeping_degraded_samples")
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
