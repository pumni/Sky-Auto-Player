"""Post-session, non-realtime classification of native playback evidence."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass
from typing import Literal

DiagnosisCategory = Literal[
    "clean",
    "backend_rejection",
    "sendinput_latency",
    "scheduler_wake_latency",
    "bookkeeping_latency",
    "lead_saturation",
    "hold_visibility_risk",
    "focus_or_expiry_drop",
    "mixed",
]


@dataclass(frozen=True, slots=True)
class NativePlaybackDiagnosis:
    category: DiagnosisCategory
    evidence: tuple[str, ...]


def diagnose_native_playback(snapshot: Mapping[str, object]) -> NativePlaybackDiagnosis:
    """Classify sender-side evidence without claiming game receipt."""

    def positive(name: str) -> bool:
        value = snapshot.get(name, 0)
        return isinstance(value, (int, float)) and not isinstance(value, bool) and value > 0

    backend = any(
        positive(name)
        for name in (
            "keys_dropped",
            "dropped_backend",
            "sendinput_partial_events",
            "sendinput_zero_progress_failures",
            "chords_rejected",
            "authored_keys_rejected",
        )
    )
    sendinput = bool(snapshot.get("sendinput_path_degraded", False))
    bookkeeping = bool(snapshot.get("bookkeeping_degraded", False))
    scheduler = bool(snapshot.get("wait_path_degraded", False)) or any(
        positive(name)
        for name in ("late_5ms", "late_10ms", "wake_error_p99_us")
    )
    lead = positive("positive_residual_at_cap") or positive("lead_saturation_count_down")
    focus_or_expiry = positive("dropped_expired") or positive("focus_loss_diagnostics")

    evidence: list[str] = []
    if backend:
        evidence.append("native backend rejection counters are non-zero")
    if sendinput:
        evidence.append("SendInput path health degraded")
    if bookkeeping:
        evidence.append("native bookkeeping health degraded")
    if scheduler:
        evidence.append("scheduler wake/lateness evidence is elevated")
    if lead:
        evidence.append("adaptive lead reached its cap with positive residual")
    if focus_or_expiry:
        evidence.append("focus or expiry drop evidence is present")

    causes = sum((backend, sendinput, bookkeeping, scheduler, lead, focus_or_expiry))
    if causes == 0:
        return NativePlaybackDiagnosis("clean", ())
    if causes > 1:
        return NativePlaybackDiagnosis("mixed", tuple(evidence))
    if backend:
        category: DiagnosisCategory = "backend_rejection"
    elif sendinput:
        category = "sendinput_latency"
    elif bookkeeping:
        category = "bookkeeping_latency"
    elif scheduler:
        category = "scheduler_wake_latency"
    elif lead:
        category = "lead_saturation"
    else:
        category = "focus_or_expiry_drop"
    return NativePlaybackDiagnosis(category, tuple(evidence))
