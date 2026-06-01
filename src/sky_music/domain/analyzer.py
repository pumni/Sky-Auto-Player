from dataclasses import dataclass
from typing import Any, Literal
from sky_music.domain.scheduler_types import ScheduleMetadata

@dataclass(frozen=True, slots=True)
class DenseCluster:
    start_us: int
    end_us: int
    note_count: int

@dataclass(frozen=True, slots=True)
class ScheduleRiskReport:
    severity: Literal["low", "medium", "high"]
    impossible_repeats: int
    impossible_same_key_repeats: int
    compressed_holds: int
    max_polyphony: int
    min_any_note_gap_us: int | None
    min_same_key_gap_us: int | None
    dense_clusters: tuple[DenseCluster, ...]
    recommendations: tuple[str, ...]
    average_notes_per_second: float = 0.0
    peak_notes_per_second_1s: float = 0.0
    suggested_profile: str = "balanced"
    suggested_tempo_scale: float = 1.0
    reason: str = ""
    max_chord_size: int = 0
    chords_count: int = 0
    timing_stress_rate: float = 0.0

def _find_dense_clusters(down_events: list[Any]) -> list[DenseCluster]:
    """Sliding Window O(n) scan to identify dense clusters (> 6 notes within any 100ms)"""
    dense_clusters_list = []
    left = 0
    n = len(down_events)
    for right in range(n):
        while down_events[right].at_us - down_events[left].at_us > 100_000:
            left += 1
            
        note_count = right - left + 1
        if note_count > 6:
            start_us = down_events[left].at_us
            end_us = down_events[right].at_us
            
            # Coalesce/merge overlapping dense clusters to avoid duplicate warnings
            if dense_clusters_list and start_us <= dense_clusters_list[-1].end_us + 50_000:
                prev = dense_clusters_list[-1]
                dense_clusters_list[-1] = DenseCluster(
                    start_us=prev.start_us,
                    end_us=end_us,
                    note_count=max(prev.note_count, note_count)
                )
            else:
                dense_clusters_list.append(DenseCluster(
                    start_us=start_us,
                    end_us=end_us,
                    note_count=note_count
                ))
    return dense_clusters_list

def _compute_min_gaps(
    res: ScheduleMetadata,
    raw_notes: tuple[Any, ...] | None,
    down_events: list[Any] | None = None,
) -> tuple[int | None, int | None]:
    """Compute minimum gap between ANY two notes and SAME physical key.

    ``analyze_schedule`` already creates a sorted down-event list, so accept it
    here to avoid sorting the same schedule twice for every UI metadata render.
    """
    if down_events is None:
        down_events = sorted(
            [action for action in res.actions if action.kind == "down"],
            key=lambda a: a.at_us,
        )
    n = len(down_events)
    
    # Post-scheduler fallback
    min_any_note_gap_us = None
    if n > 1:
        min_any_note_gap_us = min(down_events[i].at_us - down_events[i-1].at_us for i in range(1, n))
        
    min_same_key_gap_us = None
    key_last_down = {}
    same_key_gaps = []
    
    for action in down_events:
        for sc in action.scan_codes:
            if sc in key_last_down:
                same_key_gaps.append(action.at_us - key_last_down[sc])
            key_last_down[sc] = action.at_us
                
    if same_key_gaps:
        min_same_key_gap_us = min(same_key_gaps)

    # Compute metrics from raw, un-deduplicated notes if provided
    raw_min_any_note_gap_us = min_any_note_gap_us
    raw_min_same_key_gap_us = min_same_key_gap_us
    
    if raw_notes:
        sorted_notes = sorted(raw_notes, key=lambda note: note.time_ms)
        onsets = sorted({note.time_ms for note in sorted_notes})
        if len(onsets) > 1:
            raw_min_any_note_gap_us = min(onsets[i] - onsets[i-1] for i in range(1, len(onsets))) * 1000
            
        key_last_ms = {}
        same_key_gaps_ms = []
        for note in sorted_notes:
            k = note.key
            if k in key_last_ms:
                same_key_gaps_ms.append(note.time_ms - key_last_ms[k])
            key_last_ms[k] = note.time_ms
        if same_key_gaps_ms:
            raw_min_same_key_gap_us = min(same_key_gaps_ms) * 1000
            
    return raw_min_any_note_gap_us, raw_min_same_key_gap_us

def _evaluate_risk_severity(res: ScheduleMetadata, dense_clusters_list: list[DenseCluster]) -> tuple[Literal["low", "medium", "high"], list[str], list[str]]:
    severity: Literal["low", "medium", "high"] = "low"
    recommendations = []
    reasons_list = []
    
    if res.impossible_same_key_repeats > 0:
        severity = "high"
        reasons_list.append("same-key repeats")
        recommendations.append(
            f"{res.impossible_same_key_repeats} repeated notes are too close for the current timing profile."
        )
        recommendations.append(
            "Some holds were shortened, which may cause dropped notes."
        )
        
    if res.risky_same_key_repeats > 0:
        if severity == "low":
            severity = "medium"
        reasons_list.append("risky repeats")
        recommendations.append(
            f"Detected {res.risky_same_key_repeats} risky same-key repeat(s) close to min hold."
        )
        
    if res.shortest_same_key_interval_us is not None:
        recommendations.append(
            f"Shortest same-key repeat interval: {res.shortest_same_key_interval_us / 1000.0:.1f}ms."
        )
        
    dense_count = len(dense_clusters_list)
    if dense_count > 5:
        if severity != "high":
            severity = "high" if dense_count > 15 else "medium"
        reasons_list.append("dense clusters")
        recommendations.append(
            f"Detected {dense_count} distinct dense cluster(s) (more than 6 notes in 100ms)."
        )
        
    if res.compressed_holds > 5:
        if severity == "low":
            severity = "medium"
        reasons_list.append("compressed holds")
        recommendations.append(
            f"{res.compressed_holds} note holds were compressed due to dense scheduling."
        )
        
    if res.max_polyphony > 8:
        if severity == "low":
            severity = "medium"
        reasons_list.append("high polyphony")
        recommendations.append(
            f"High polyphony detected (max {res.max_polyphony} simultaneous keys)."
        )
        
    if not recommendations:
        recommendations.append("No timing conflicts detected. Balanced playback is recommended.")
    
    return severity, recommendations, reasons_list

def analyze_schedule(res: ScheduleMetadata, raw_notes: tuple[Any, ...] | None = None) -> ScheduleRiskReport:
    """Analyze a ScheduleMetadata and optional raw notes to detect timing conflicts, density risks, and suggest overrides."""
    down_events = sorted([action for action in res.actions if action.kind == "down"], key=lambda a: a.at_us)
    
    dense_clusters_list = _find_dense_clusters(down_events)
    raw_min_any_note_gap_us, raw_min_same_key_gap_us = _compute_min_gaps(res, raw_notes, down_events)

    # Compute note density metrics
    duration_sec = res.source_duration_us / 1_000_000
    n = len(down_events)
    average_notes_per_second = n / duration_sec if duration_sec > 0 else 0.0
    
    peak_notes_per_second_1s = 0
    left_1s = 0
    for right_1s in range(n):
        while down_events[right_1s].at_us - down_events[left_1s].at_us > 1_000_000:
            left_1s += 1
        peak_notes_per_second_1s = max(peak_notes_per_second_1s, right_1s - left_1s + 1)
        
    severity, recommendations, reasons_list = _evaluate_risk_severity(res, dense_clusters_list)
    reason = f"{' and '.join(reasons_list)} detected" if reasons_list else "No timing conflicts detected."
    
    # 7. Unified Recommendation Engine decisions (P1.3)
    has_repeats = res.impossible_same_key_repeats > 0 or res.risky_same_key_repeats > 0
    high_poly = res.max_polyphony >= 5
    
    if severity == "high":
        suggested_profile = "dense-safe" if has_repeats else "audience-safe"
        suggested_tempo_scale = 0.92
    elif severity == "medium":
        suggested_profile = "audience-safe" if high_poly else "balanced"
        suggested_tempo_scale = 0.95
    else:
        suggested_profile = "balanced"
        suggested_tempo_scale = 1.00
        
    max_chord_size = max(len(a.scan_codes) for a in down_events) if down_events else 0
    chords_count = sum(1 for a in down_events if len(a.scan_codes) > 1)
    note_count = len(raw_notes) if raw_notes else n
    timing_stress_rate = (res.impossible_same_key_repeats / note_count * 100) if note_count > 0 else 0.0

    return ScheduleRiskReport(
        severity=severity,
        impossible_repeats=res.impossible_same_key_repeats,
        impossible_same_key_repeats=res.impossible_same_key_repeats,
        compressed_holds=res.compressed_holds,
        max_polyphony=res.max_polyphony,
        min_any_note_gap_us=raw_min_any_note_gap_us,
        min_same_key_gap_us=raw_min_same_key_gap_us,
        dense_clusters=tuple(dense_clusters_list),
        recommendations=tuple(recommendations),
        average_notes_per_second=average_notes_per_second,
        peak_notes_per_second_1s=float(peak_notes_per_second_1s),
        suggested_profile=suggested_profile,
        suggested_tempo_scale=suggested_tempo_scale,
        reason=reason,
        max_chord_size=max_chord_size,
        chords_count=chords_count,
        timing_stress_rate=timing_stress_rate
    )
