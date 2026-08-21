"""Bounded Windows acceptance benchmark for the native dispatch worker.

The default benchmark uses the Rust worker with an explicit, polyphony-aware
mock latency model. It never emits input to a game or another process. This
isolates estimator behaviour while still exercising non-zero completion
latency. ``--backend sendinput --allow-real-input`` is an explicit fixed-host
run against the real SendInput seam; it must not be used while an unintended
foreground application can receive the test keys.

Run the same command on the baseline and follow-up revisions, then compare the
JSON files::

    uv run --env-file .env python scripts/bench_native_acceptance.py \
        --repeats 2 --actions 128 --output native-followup.json

The primary qualification metric is the signed pre-call residual
(`pre_call_qpc - physical_target_qpc`) from strict diagnostic telemetry. It is
a host-side sender boundary, not a Windows insertion, game receipt, render, or
audio timestamp.
"""

from __future__ import annotations

import argparse
import collections
import json
import math
import os
import platform
import subprocess
import sys
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any, cast

from sky_music.layouts import SKY_15_SCAN_CODES
from sky_music.orchestration.telemetry import (
    TelemetryRecord,
    materialize_native_trace,
)

REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
MIN_BENCHMARK_BUDGET_SECONDS = 1.0
MAX_BENCHMARK_BUDGET_SECONDS = 600.0
BENCHMARK_SCHEMA_VERSION = 8
TIMELINE_SEMANTICS_VERSION = 2
KNOWN_TIMELINE_SEMANTICS = {
    "109f1c33d5410e92bbb9669632ebed7037852a16": 1,
    "9ef9e5785c746ff45678a8a4cc5b9b6d66ede466": 2,
}
SAME_SEMANTICS_REFERENCE_SHA = "9ef9e5785c746ff45678a8a4cc5b9b6d66ede466"
TRANSPORT_REFERENCE_SHA = "109f1c33d5410e92bbb9669632ebed7037852a16"
SAME_SEMANTICS = "SAME_SEMANTICS"
TRANSPORT_REFERENCE = "TRANSPORT_REFERENCE"
ABSOLUTE_WAKE_P99_LIMIT_US = 300
ABSOLUTE_PRE_CALL_P99_LIMIT_US = 250
ABSOLUTE_PRE_CALL_P999_LIMIT_US = 750
ABSOLUTE_PRE_CALL_LATE_ABORT_US = 2_000
COMMAND_TIMING_DOMAIN = "native_qpc_v1"
LATENCY_SEGMENT_DOMAIN = "native_trace_v1"
SEND_COLD_THRESHOLD_US = 20_000
HOT_CYCLE_US = 10_000
COLD_CYCLE_US = 60_000
DEFAULT_DOWN_LATE_GRACE_US = 500
DEFAULT_TRANSPORT_MARGIN_US = 300
MIN_QUALIFICATION_PHYSICAL_BOUNDARIES = 10_000

PRODUCTION_CORRECTNESS_COUNTERS = (
    "production_completion_hold_below_frame_count",
    "production_release_gap_below_policy_count",
    "production_same_call_same_key_retrigger_count",
    "production_anchor_overwrite_count",
    "production_unmatched_up_count",
    "production_anomaly_ring_overwrite_count",
    "production_forensics_anomaly_count",
)

MIXED_POLY_TIMING_FIELDS = (
    "wake_error_us",
    "pre_send_software_latency_us",
    "pre_call_lateness_us",
    "sendinput_call_duration_us",
    "core_post_send_duration_us",
    "sender_completion_error_us",
    "wake_error",
    "pre_send",
    "sendinput",
    "core_post_send",
    "observer",
    "sender_completion_error",
    "startup_latency_us",
    "spin_cpu_time_us",
    "worker_cpu_time_us",
    "process_cpu_time_us",
    "playback_wall_time_us",
    "spin_duty_cycle_ppm",
    "worker_cpu_ratio_ppm",
    "process_cpu_ratio_ppm",
    "spin_cpu_ratio_ppm",
    "peak_rss_bytes",
    "lead_by_polyphony",
    "sendinput_warn_threshold_us",
    "core_post_send_warn_threshold_us",
    "wait_warn_threshold_us",
    "sendinput_degraded_samples",
    "core_post_send_degraded_samples",
    "wait_degraded_samples",
    "positive_residual_at_cap",
)


def _native_build_flavor() -> str:
    import sky_player_rs

    return (
        "test_support"
        if callable(getattr(sky_player_rs, "TestDispatchSession", None))
        else "production"
    )


def _normalize_historical_native_trace(
    output: dict[str, Any], *, native_build_commit: str
) -> dict[str, Any]:
    """Project the known schema-8 baseline field into the canonical metric.

    The same-semantics reference predates the ``core_post_send`` name but has
    the same authored timeline semantics. Keep this compatibility projection
    scoped to that exact SHA and schema; current candidate telemetry remains
    strict and the raw envelope is retained separately for diagnostics.
    """

    if (
        native_build_commit != SAME_SEMANTICS_REFERENCE_SHA
        or output.get("schema_version") != 8
    ):
        return output
    records = output.get("records")
    if not isinstance(records, list):
        return output
    normalized_records: list[dict[str, Any]] = []
    changed = False
    for record in records:
        if not isinstance(record, dict):
            return output
        normalized = dict(record)
        if (
            "core_post_send_duration_us" not in normalized
            and "bookkeeping_duration_us" in normalized
        ):
            normalized["core_post_send_duration_us"] = normalized[
                "bookkeeping_duration_us"
            ]
            changed = True
        normalized_records.append(normalized)
    if not changed:
        return output
    return {**output, "records": normalized_records}


def _cycle_us(*, game_fps: int, gap_profile: str) -> int:
    if not 15 <= game_fps <= 240:
        raise ValueError("game_fps must be in 15..=240")
    if gap_profile == "cold":
        return COLD_CYCLE_US
    if gap_profile != "hot":
        raise ValueError("gap_profile must be hot or cold")
    frame_period_us = (1_000_000 + game_fps - 1) // game_fps
    hold_us = frame_period_us + DEFAULT_DOWN_LATE_GRACE_US + DEFAULT_TRANSPORT_MARGIN_US
    return max(HOT_CYCLE_US, hold_us + frame_period_us)


def _materialized_hold_us(*, game_fps: int, gap_profile: str) -> int:
    if not 15 <= game_fps <= 240:
        raise ValueError("game_fps must be in 15..=240")
    if gap_profile == "hot":
        frame_period_us = (1_000_000 + game_fps - 1) // game_fps
        return frame_period_us + DEFAULT_DOWN_LATE_GRACE_US + DEFAULT_TRANSPORT_MARGIN_US
    if gap_profile == "cold":
        return COLD_CYCLE_US // 2
    raise ValueError("gap_profile must be hot or cold")


def _actions(
    count: int,
    polyphony: int,
    *,
    gap_profile: str = "hot",
    game_fps: int = 60,
    start_delay_us: int = 0,
    scenario: str = "paired",
    warmup_cycles: int = 0,
) -> list[tuple[int, str, int, list[int], str]]:
    if not 1 <= polyphony <= len(SKY_15_SCAN_CODES):
        raise ValueError(f"polyphony must be in 1..{len(SKY_15_SCAN_CODES)}")
    if gap_profile not in {"hot", "cold"}:
        raise ValueError("gap_profile must be hot or cold")
    if scenario not in {"paired", "mixed", "coalesced"}:
        raise ValueError("scenario must be paired, mixed, or coalesced")
    if scenario in {"mixed", "coalesced"} and polyphony < 2:
        raise ValueError(
            "mixed/coalesced scenarios require polyphony >= 2; "
            "polyphony=1 would silently change packet size"
        )
    if start_delay_us < 0:
        raise ValueError("start_delay_us must be non-negative")
    if warmup_cycles < 0:
        raise ValueError("warmup_cycles must be non-negative")
    cycle_us = _cycle_us(game_fps=game_fps, gap_profile=gap_profile)
    hold_us = _materialized_hold_us(game_fps=game_fps, gap_profile=gap_profile)
    actions: list[tuple[int, str, int, list[int], str]] = []
    if scenario in {"mixed", "coalesced"}:
        # The release-gap validator rejects same-key Up+Down at one target.
        # Positive mixed/coalesced coverage therefore uses two disjoint masks:
        # the first mask is released while the second mask is pressed.
        key_count = polyphony
        boundary_scan_codes = [
            int(SKY_15_SCAN_CODES[(group * key_count + offset) % len(SKY_15_SCAN_CODES)])
            for group in range(count)
            for offset in range(key_count)
        ]
    else:
        boundary_scan_codes = []
    if scenario in {"mixed", "coalesced"}:
        # Each group has one clean Down, one Up+Down mixed boundary, and one
        # final Up.  A full cooldown separates groups so repeated groups do
        # not manufacture an ownership conflict unrelated to the profile.
        group_span_us = max(cycle_us * 5, 100_000)
        for group in range(count):
            base_index = group * 4
            at_us = start_delay_us + group * group_span_us
            group_codes = boundary_scan_codes[
                group * polyphony : (group + 1) * polyphony
            ]
            split = max(1, len(group_codes) // 2)
            up_scan_codes = group_codes[:split]
            down_scan_codes = group_codes[split:]
            if not down_scan_codes:
                down_scan_codes = [up_scan_codes.pop()]
            actions.extend(
                (
                    (base_index, "down", at_us, up_scan_codes, f"{scenario}-down"),
                    (
                        base_index + 1,
                        "up",
                        at_us + cycle_us,
                        up_scan_codes,
                        f"{scenario}-up",
                    ),
                    (
                        base_index + 2,
                        "down",
                        at_us + cycle_us,
                        down_scan_codes,
                        f"{scenario}-retrigger-down",
                    ),
                    (
                        base_index + 3,
                        "up",
                        at_us + cycle_us * 2,
                        down_scan_codes,
                        f"{scenario}-release",
                    ),
                )
            )
    else:
        for index in range(count):
            scan_codes = [
                int(
                    SKY_15_SCAN_CODES[
                        (index * polyphony + offset) % len(SKY_15_SCAN_CODES)
                    ]
                )
                for offset in range(polyphony)
            ]
            at_us = start_delay_us + index * cycle_us
            actions.extend(
                (
                    (index * 2, "down", at_us, scan_codes, "bench-down"),
                    (index * 2 + 1, "up", at_us + hold_us, scan_codes, "bench-up"),
                )
            )
    return actions


def _actions_per_polyphony(*, actions: int, scenario: str) -> int:
    return actions * (4 if scenario in {"mixed", "coalesced"} else 2)


def _percentile(values: list[int], fraction: float) -> int:
    if not values:
        return 0
    ordered = sorted(values)
    index = max(0, math.ceil(len(ordered) * fraction) - 1)
    return ordered[min(index, len(ordered) - 1)]


def _stats(values: list[int]) -> dict[str, int]:
    return {
        "n": len(values),
        "p50": _percentile(values, 0.50),
        "p95": _percentile(values, 0.95),
        "p99": _percentile(values, 0.99),
        "p999": _percentile(values, 0.999),
        "max": max(values, default=0),
    }


def _required_stats(values: list[int], name: str) -> dict[str, int]:
    if not values:
        raise RuntimeError(f"required metric {name} has no measurements")
    return _stats(values)


def _ratio_ppm(numerator_us: int, denominator_us: int) -> int:
    if denominator_us <= 0:
        raise RuntimeError("playback wall time must be positive")
    return numerator_us * 1_000_000 // denominator_us


def _benchmark_config(
    *,
    args: argparse.Namespace,
    polyphonies: list[int],
    mock_base_latency_us: int,
    mock_per_key_latency_us: int,
) -> dict[str, Any]:
    require_focus = getattr(args, "require_focus", None)
    if require_focus is None:
        require_focus = args.backend == "sendinput"
    if args.backend == "sendinput":
        # DispatchSession owns these production settings; the legacy CLI
        # knobs are not passed through to the real backend.
        effective_priority = "auto"
        effective_adaptive_spin = False
        effective_lead_mode = "fixed"
        effective_fixed_lead_us = 0
        native_profile = "strict_timing_diagnostic"
    else:
        effective_priority = args.rt_priority_mode
        effective_adaptive_spin = not args.no_adaptive_spin
        effective_lead_mode = args.lead_mode
        effective_fixed_lead_us = args.fixed_lead_us
        native_profile = "mock_test"
    return {
        "backend": args.backend,
        "game_fps": args.game_fps,
        "rt_priority_mode": effective_priority,
        "adaptive_spin": effective_adaptive_spin,
        "waitable_timer": True,
        "event_wait": True,
        "mock_base_latency_us": mock_base_latency_us,
        "mock_per_key_latency_us": mock_per_key_latency_us,
        "actions": args.actions,
        "polyphony": polyphonies,
        "start_delay_us": getattr(args, "start_delay_us", 0),
        "scenario": getattr(args, "scenario", "paired"),
        "lead_mode": effective_lead_mode,
        "fixed_lead_us": effective_fixed_lead_us,
        "gap_profile": args.gap_profile,
        "warmup_cycles": args.warmup_cycles,
        "native_profile": native_profile,
        "native_build_flavor": (
            "production" if args.backend == "sendinput" else "test_support"
        ),
        "require_focus": require_focus,
        "materialized_min_hold_us": _materialized_hold_us(
            game_fps=args.game_fps,
            gap_profile=args.gap_profile,
        ),
    }


class TelemetryIntegrityError(RuntimeError):
    """Raised when a benchmark trace cannot prove one record per authored action."""

    def __init__(self, diagnostics: dict[str, Any]) -> None:
        self.diagnostics = diagnostics
        super().__init__(
            "native telemetry integrity failure: "
            + json.dumps(diagnostics, sort_keys=True)
        )


@dataclass(frozen=True, slots=True)
class BenchmarkRunResult:
    """Outcome of one requested repetition; failed runs are never statistics."""

    run_index: int
    polyphony: int
    result: dict[str, Any] | None
    failure: dict[str, Any] | None


class BenchmarkRunFailure(RuntimeError):
    """A repetition failed with the raw session evidence attached."""

    def __init__(
        self,
        *,
        original: BaseException,
        snapshot: dict[str, Any] | None,
        telemetry: dict[str, Any] | None,
        diagnostics: dict[str, Any] | None,
    ) -> None:
        self.original = original
        self.snapshot = snapshot
        self.telemetry = telemetry
        self.diagnostics = diagnostics
        super().__init__(str(original))


def _run_validity_summary(
    requested_runs: int, results: list[BenchmarkRunResult]
) -> dict[str, Any]:
    """Return the explicit validity contract for a repetition set."""

    successful_runs = sum(
        1 for result in results if result.result is not None and result.failure is None
    )
    failed_runs = sum(1 for result in results if result.failure is not None)
    return {
        "requested_runs": requested_runs,
        "successful_runs": successful_runs,
        "failed_runs": failed_runs,
        "run_validity": "complete"
        if requested_runs == successful_runs and failed_runs == 0
        else "invalid",
    }


def _expected_record_layout(
    actions: list[tuple[int, str, int, list[int], str]],
    scenario: str,
) -> list[tuple[int, str, set[int]]]:
    if scenario not in {"paired", "mixed", "coalesced"}:
        raise ValueError("scenario must be paired, mixed, or coalesced")
    combined_boundaries = scenario in {"mixed", "coalesced"}
    if combined_boundaries and (len(actions) < 2 or len(actions) % 2 != 0):
        raise ValueError("mixed/coalesced scenarios require Down/Up action pairs")
    layout: list[tuple[int, str, set[int]]] = []
    for position, action in enumerate(actions):
        index, kind, scheduled_us, *_ = action
        if (
            combined_boundaries
            and kind == "up"
            and position + 1 < len(actions)
            and actions[position + 1][1] == "down"
            and actions[position + 1][2] == scheduled_us
        ):
            continue
        record_kind = str(kind)
        consumed = {int(index)}
        if (
            combined_boundaries
            and kind == "down"
            and position > 0
            and actions[position - 1][1] == "up"
            and actions[position - 1][2] == scheduled_us
        ):
            record_kind = "mixed"
            consumed.add(int(actions[position - 1][0]))
        layout.append((int(index), record_kind, consumed))
    return layout


def _warmup_record_count(
    actions: list[tuple[int, str, int, list[int], str]],
    scenario: str,
    warmup_cycles: int,
) -> int:
    if warmup_cycles < 0:
        raise ValueError("warmup_cycles must be non-negative")
    warmup_action_count = _actions_per_polyphony(
        actions=warmup_cycles,
        scenario=scenario,
    )
    return sum(
        1
        for _, _, consumed in _expected_record_layout(actions, scenario)
        if consumed and max(consumed) < warmup_action_count
    )


def _expected_hold_pair_samples(
    actions: list[tuple[int, str, int, list[int], str]],
    scenario: str,
) -> int:
    """Simulate canonical physical ownership and count closed generations.

    The input is the authored action set, not observer output.  A generation
    is closed by an Up before a later Down can open a replacement, matching
    the packet builder's all-Up-before-all-Down ordering.
    """

    if scenario not in {"paired", "mixed", "coalesced"}:
        raise ValueError("scenario must be paired, mixed, or coalesced")
    grouped: dict[int, list[tuple[str, list[int]]]] = {}
    for _index, kind, scheduled_us, scan_codes, _label in actions:
        slots = {int(scan_code) for scan_code in scan_codes}
        if len(slots) != len(scan_codes):
            raise ValueError("authored action contains duplicate physical keys")
        if kind not in {"up", "down"}:
            raise ValueError(f"unsupported authored action kind: {kind!r}")
        grouped.setdefault(int(scheduled_us), []).append(
            (str(kind), [int(scan_code) for scan_code in scan_codes])
        )

    active: set[int] = set()
    samples = 0
    for _scheduled_us, group in sorted(grouped.items()):
        # PreparedPhysicalPacket emits every Up before every Down in one
        # timestamp group.  Keep the simulator independent of authored list
        # order so it predicts the native generation ownership, not caller
        # ordering.
        for kind, scan_codes in group:
            if kind == "up":
                slots = set(scan_codes)
                samples += len(active & slots)
                active.difference_update(slots)
        for kind, scan_codes in group:
            if kind == "down":
                active.update(scan_codes)
    if active:
        raise ValueError("authored action set leaves physical generations open")
    return samples


def _validate_telemetry_integrity(
    *,
    actions: list[tuple[int, str, int, list[int], str]],
    telemetry: dict[str, Any],
    records: list[TelemetryRecord],
    polyphony: int,
    scenario: str = "paired",
    snapshot: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Prove that the compact native trace is a complete authored action set.

    A matching record count is insufficient: a duplicate event index can hide a
    missing action.  This validation deliberately checks the exact authored
    index and kind mapping before a repetition is eligible for statistics.
    """

    authored_indices = [int(action[0]) for action in actions]
    layout = _expected_record_layout(actions, scenario)
    expected_indices = [index for index, _, _ in layout]
    expected_kinds = {index: kind for index, kind, _ in layout}
    expected_authored_indices = set(authored_indices)
    expected_counts = collections.Counter(expected_indices)
    expected_duplicate_indices = sorted(
        index for index, count in expected_counts.items() if count > 1
    )
    actual_indices = [int(record.event_index) for record in records]
    counts = collections.Counter(actual_indices)
    duplicate_indices = sorted(index for index, count in counts.items() if count > 1)
    expected_set = set(expected_indices)
    actual_set = set(actual_indices)
    missing_indices = sorted(expected_set - actual_set)
    unexpected_indices = sorted(actual_set - expected_set)
    kind_mismatches = [
        {
            "event_index": record.event_index,
            "expected": expected_kinds[record.event_index],
            "actual": record.kind,
        }
        for record in records
        if record.event_index in expected_kinds
        and record.kind != expected_kinds[record.event_index]
    ]
    consumed_authored_indices: set[int] = set()
    combined_boundaries = scenario in {"mixed", "coalesced"}
    for record in records:
        if combined_boundaries and record.kind == "mixed":
            consumed_authored_indices.update((record.event_index - 1, record.event_index))
        else:
            consumed_authored_indices.add(record.event_index)
    unconsumed_authored_indices = sorted(
        expected_authored_indices - consumed_authored_indices
    )
    diagnostics: dict[str, Any] = {
        "polyphony": polyphony,
        "snapshot_status": None if snapshot is None else snapshot.get("status"),
        "snapshot_outcome": None if snapshot is None else snapshot.get("outcome"),
        "terminal_error": None if snapshot is None else snapshot.get("terminal_error"),
        "generation_count": None if snapshot is None else snapshot.get("generation_count"),
        "generation_status_counts": (
            None
            if snapshot is None
            else snapshot.get("generation_status_counts", {})
        ),
        "authored_action_count": len(actions),
        "trace_expected_count": len(actions),
        "expected_record_count": len(expected_indices),
        "trace_actual_count": len(records),
        "last_trace_source_index": actual_indices[-1] if actual_indices else None,
        "expected_count": len(actions),
        "records_count": len(records),
        "attempted": telemetry.get("attempted"),
        "accepted": telemetry.get("accepted"),
        "dropped": telemetry.get("dropped"),
        "truncated": telemetry.get("truncated"),
        "expected_indices": expected_indices,
        "expected_authored_indices": authored_indices,
        "unconsumed_authored_indices": unconsumed_authored_indices,
        "expected_duplicate_indices": expected_duplicate_indices,
        "actual_indices": actual_indices,
        "missing_indices": missing_indices,
        "duplicate_indices": duplicate_indices,
        "unexpected_indices": unexpected_indices,
        "kind_mismatches": kind_mismatches,
    }
    valid = (
        diagnostics["attempted"] == len(expected_indices)
        and diagnostics["accepted"] == len(expected_indices)
        and diagnostics["dropped"] == 0
        and diagnostics["truncated"] is False
        and len(records) == len(expected_indices)
        and not expected_duplicate_indices
        and not missing_indices
        and not duplicate_indices
        and not unexpected_indices
        and not kind_mismatches
        and not unconsumed_authored_indices
    )
    if not valid:
        raise TelemetryIntegrityError(diagnostics)
    return diagnostics


def _correctness_counters(
    snapshot: dict[str, Any],
    diagnostics: dict[str, Any],
    *,
    expected_hold_pair_samples: int | None = None,
) -> dict[str, int]:
    statuses = snapshot.get("generation_status_counts", {})
    release_pending = (
        int(statuses.get("release_pending", 0))
        if isinstance(statuses, dict)
        else 0
    )
    release_outcome = snapshot.get("release_outcome")
    if not isinstance(release_outcome, dict):
        release_outcome = {}
    stuck_keys = release_outcome.get("stuck_keys", [])
    unexpected_held = (
        len(stuck_keys)
        if isinstance(stuck_keys, (list, tuple))
        else int(release_outcome.get("stuck_mask", 0) != 0)
    )
    trace_errors = sum(
        len(diagnostics.get(name, []))
        for name in (
            "missing_indices",
            "unconsumed_authored_indices",
            "duplicate_indices",
            "unexpected_indices",
            "kind_mismatches",
        )
    )
    chord_integrity_lost = int(
        snapshot.get("chord_integrity_lost", snapshot.get("chord_split_events", 0))
    )
    partial = int(snapshot.get("sendinput_partial_events", 0))
    zero_progress = int(snapshot.get("sendinput_zero_progress_failures", 0))
    actual_hold_pair_samples = int(snapshot.get("hold_pair_samples", 0))
    hold_pair_sample_mismatch = int(
        expected_hold_pair_samples is None
        or actual_hold_pair_samples != expected_hold_pair_samples
    )
    counters = {
        "chord_integrity_lost": chord_integrity_lost,
        "unexpected_held": unexpected_held,
        "pending_unresolved": release_pending,
        "cleanup_uncertainty": int(
            bool(release_outcome.get("verification_inconclusive", False))
        ),
        "telemetry_integrity_failures": 0,
        "sender_integrity_failures": partial + zero_progress,
        "unexpected_transport_failures": partial + zero_progress,
        "authored_trace_missing_duplicate_mismatch": trace_errors,
        "missed_down_boundaries": int(snapshot.get("missed_down_boundaries", 0)),
        "pre_call_hold_shrink_over_grace_count": int(
            snapshot.get("pre_call_hold_shrink_over_grace_count", 0)
        ),
        "hold_unmatched_up_count": int(snapshot.get("hold_unmatched_up_count", 0)),
        "hold_anchor_overwrite_count": int(
            snapshot.get("hold_anchor_overwrite_count", 0)
        ),
        "hold_pair_sample_mismatch": hold_pair_sample_mismatch,
    }
    counters.update(
        {
            name: int(snapshot.get(name, 0))
            for name in PRODUCTION_CORRECTNESS_COUNTERS
        }
    )
    return counters


def _aggregate_correctness(runs: list[dict[str, Any]]) -> dict[str, int]:
    names = (
        "chord_integrity_lost",
        "unexpected_held",
        "pending_unresolved",
        "cleanup_uncertainty",
        "telemetry_integrity_failures",
        "sender_integrity_failures",
        "unexpected_transport_failures",
        "authored_trace_missing_duplicate_mismatch",
        "missed_down_boundaries",
        "pre_call_hold_shrink_over_grace_count",
        "hold_unmatched_up_count",
        "hold_anchor_overwrite_count",
        "hold_pair_sample_mismatch",
        *PRODUCTION_CORRECTNESS_COUNTERS,
    )
    return {
        name: sum(int(run["correctness"].get(name, 0)) for run in runs)
        for name in names
    }


def _acceptance_failure_reasons(report: dict[str, Any]) -> list[str]:
    """Classify a report without hiding a timing or delivery failure in JSON."""

    reasons: list[str] = []
    if report.get("run_validity") != "complete":
        reasons.append("incomplete_run_set")
    if report.get("failed_dispatch_suites", 0) or report.get("failed_command_samples", 0):
        reasons.append("failed_run")
    if report.get("deadline_missed_before_send_count", 0):
        reasons.append("deadline_missed_before_send")
    if report.get("non_dispatch_count", 0):
        reasons.append("non_dispatch")
    if report.get("observer_dropped_records", 0):
        reasons.append("observer_dropped_records")

    expected_hold_pair_samples = report.get("expected_hold_pair_samples")
    actual_hold_pair_samples = report.get("hold_pair_samples")
    if (
        not isinstance(expected_hold_pair_samples, int)
        or not isinstance(actual_hold_pair_samples, int)
        or actual_hold_pair_samples != expected_hold_pair_samples
    ):
        reasons.append("hold_pair_sample_completeness")

    correctness = report.get("correctness")
    if isinstance(correctness, dict):
        for name, value in correctness.items():
            if isinstance(value, int) and not isinstance(value, bool) and value != 0:
                reasons.append(f"correctness:{name}")

    for name in (
        "failed_release_count",
        "chord_split_events",
        "chords_rejected",
        "authored_keys_rejected",
        "sendinput_partial_events",
        "sendinput_zero_progress_failures",
    ):
        value = report.get(name)
        if isinstance(value, int) and not isinstance(value, bool) and value != 0:
            reasons.append(name)

    config = report.get("benchmark_config")
    fixed_hot_60 = (
        isinstance(config, dict)
        and config.get("game_fps") == 60
        and config.get("gap_profile") == "hot"
        and config.get("lead_mode") == "fixed"
    )
    wake = report.get("wake_error_us")
    if fixed_hot_60 and isinstance(wake, dict):
        absolute = wake.get("absolute")
        if isinstance(absolute, dict) and absolute.get("p99", 0) > ABSOLUTE_WAKE_P99_LIMIT_US:
            reasons.append("wake_p99_slo")

    pre_call = report.get("pre_call_lateness_us")
    if isinstance(pre_call, dict):
        if pre_call.get("early_count", 0) != 0:
            reasons.append("early_physical_send")
        if pre_call.get("late_over_2ms_count", 0) != 0:
            reasons.append("pre_call_over_2ms")
        late = pre_call.get("late")
        if isinstance(late, dict):
            if late.get("p99", 0) > ABSOLUTE_PRE_CALL_P99_LIMIT_US:
                reasons.append("pre_call_p99_slo")
            if late.get("p999", 0) > ABSOLUTE_PRE_CALL_P999_LIMIT_US:
                reasons.append("pre_call_p999_slo")
    return reasons


def _failed_run_artifact_path(output: Path | None, run_index: int) -> Path:
    """Choose a unique diagnostic path without overwriting an earlier failure."""

    stem = (output or (REPOSITORY_ROOT / "native-acceptance.json")).resolve()
    candidate = stem.with_name(f"{stem.stem}-failed-run-{run_index}.json")
    attempt = 1
    while candidate.exists():
        attempt += 1
        candidate = stem.with_name(
            f"{stem.stem}-failed-run-{run_index}-attempt-{attempt}.json"
        )
    return candidate


def _write_failed_run_artifact(
    path: Path,
    *,
    git_info: dict[str, Any],
    native_info: dict[str, Any],
    host_info: dict[str, Any],
    run_index: int,
    polyphony: int,
    actions: list[tuple[int, str, int, list[int], str]],
    snapshot: dict[str, Any] | None,
    telemetry: dict[str, Any] | None,
    diagnostics: dict[str, Any] | None,
    exception: BaseException,
) -> Path:
    """Persist every available failed-run diagnostic and never replace a file."""

    if path.exists():
        raise FileExistsError(f"refusing to overwrite failed-run artifact {path}")
    payload = {
        "git_provenance": git_info,
        "native_provenance": native_info,
        "host_fingerprint": host_info,
        "run_index": run_index,
        "polyphony": polyphony,
        "actions": actions,
        "session_snapshot": snapshot,
        "raw_telemetry": telemetry,
        "validation_diagnostics": diagnostics,
        "exception": f"{type(exception).__name__}: {exception}",
        "command_line": list(sys.argv),
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, default=str) + "\n", encoding="utf-8")
    return path


def _git_provenance() -> dict[str, Any]:
    try:
        sha_result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=REPOSITORY_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
        status_result = subprocess.run(
            ["git", "status", "--porcelain"],
            cwd=REPOSITORY_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise RuntimeError(f"could not read Git provenance: {exc}") from exc
    sha = sha_result.stdout.strip()
    if not sha:
        raise RuntimeError("Git HEAD is empty")
    dirty = bool(status_result.stdout.strip())
    if dirty:
        raise RuntimeError("acceptance evidence requires a clean worktree")
    return {"git_sha": sha, "dirty_worktree": False}


def _native_provenance(expected_commit: str) -> dict[str, Any]:
    import sky_player_rs

    info = dict(sky_player_rs.build_info())  # type: ignore[attr-defined]
    required = (
        "native_build_commit",
        "rustc_version",
        "schema_version",
        "native_abi",
        "qpc_frequency_hz",
    )
    for name in required:
        value = info.get(name)
        if value in (None, "", "unknown"):
            raise RuntimeError(f"native build provenance is missing {name}")
    if info["native_build_commit"] != expected_commit:
        raise RuntimeError(
            "native build commit does not match the current checkout: "
            f"native={info['native_build_commit']} expected={expected_commit}"
        )
    return info


def _host_fingerprint(native_info: dict[str, Any]) -> dict[str, Any]:
    return {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "processor": platform.processor(),
        "windows_build": platform.version(),
        "qpc_frequency_hz": int(native_info["qpc_frequency_hz"]),
    }


def _completion_error_report_pairs(rows: list[tuple[str, int]]) -> dict[str, Any]:
    """Return signed, absolute, early and late error distributions.

    Signed aggregate percentiles are retained for report compatibility, but
    regression gates must inspect all three non-negative views.  The per-kind
    split prevents a Down-only or Up-only regression from being diluted by the
    other half of the timeline.
    """

    def values_for(rows: list[tuple[str, int]]) -> list[int]:
        values: list[int] = []
        for _, value in rows:
            if not isinstance(value, int) or isinstance(value, bool):
                raise RuntimeError(
                    "sender telemetry is missing exact sender_completion_error_us"
                )
            values.append(value)
        return values

    if not rows:
        raise RuntimeError("required sender telemetry has no records")

    def report_for(rows: list[tuple[str, int]], name: str) -> dict[str, Any]:
        signed = values_for(rows)
        return {
            "signed": _required_stats(signed, f"{name}.signed"),
            "absolute": _required_stats([abs(value) for value in signed], f"{name}.absolute"),
            "late": _stats([value for value in signed if value > 0]),
            "early": _stats([-value for value in signed if value < 0]),
        }

    by_kind = {
        kind: report_for([row for row in rows if row[0] == kind], kind)
        for kind in ("down", "up")
    }
    result = report_for(rows, "all")
    result["by_kind"] = by_kind
    return result


def _pre_call_error_report_pairs(rows: list[tuple[str, int]]) -> dict[str, Any]:
    report = _completion_error_report_pairs(rows)
    signed = [value for _, value in rows]
    report["early_count"] = sum(value < 0 for value in signed)
    report["late_over_2ms_count"] = sum(
        value > ABSOLUTE_PRE_CALL_LATE_ABORT_US for value in signed
    )
    return report


def _nonnegative_metric_report(rows: list[tuple[str, int]], name: str) -> dict[str, Any]:
    values: list[int] = []
    for _kind, value in rows:
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            raise RuntimeError(f"required nonnegative telemetry is invalid: {name}")
        values.append(value)
    if not values:
        raise RuntimeError(f"required metric {name} has no measurements")
    return {
        **_required_stats(values, name),
        "by_kind": {
            kind: _required_stats(
                [value for row_kind, value in rows if row_kind == kind],
                f"{name}.{kind}",
            )
            for kind in ("down", "up")
        },
    }


def _required_int(value: Any, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise RuntimeError(f"required telemetry field is missing or invalid: {name}")
    return value


def _trace_metric_rows(records: list[TelemetryRecord]) -> dict[str, list[tuple[str, int]]]:
    rows: dict[str, list[tuple[str, int]]] = collections.defaultdict(list)
    for record in records:
        kind = record.kind
        wake_us = _required_int(record.wake_us, "wake_us")
        sender_started_us = _required_int(record.sender_started_us, "sender_started_us")
        sender_completed_us = _required_int(record.sender_completed_us, "sender_completed_us")
        if sender_started_us < wake_us:
            raise RuntimeError("invalid timestamp ordering: sender_started_us < wake_us")
        if sender_completed_us < sender_started_us:
            raise RuntimeError(
                "invalid timestamp ordering: sender_completed_us < sender_started_us"
            )
        rows["wake_error_us"].append((kind, _required_int(record.wake_error_us, "wake_error_us")))
        rows["pre_call_lateness_us"].append(
            (
                kind,
                _required_int(
                    record.dispatch_start_error_us,
                    "dispatch_start_error_us",
                ),
            )
        )
        rows["pre_send_software_latency_us"].append(
            (kind, sender_started_us - wake_us)
        )
        rows["sendinput_call_duration_us"].append(
            (kind, _required_int(record.sendinput_call_duration_us, "sendinput_call_duration_us"))
        )
        rows["core_post_send_duration_us"].append(
            (kind, _required_int(record.core_post_send_duration_us, "core_post_send_duration_us"))
        )
        rows["sender_completion_error_us"].append(
            (
                kind,
                _required_int(
                    record.sender_completion_error_us,
                    "sender_completion_error_us",
                ),
            )
        )
        _required_int(record.native_polyphony, "native_polyphony")
    return dict(rows)


def _native_packet_size_counts(
    records: list[TelemetryRecord],
) -> dict[str, dict[str, int]]:
    """Count actual native packet sizes without conflating suite labels."""

    counts: dict[str, dict[str, int]] = {"down": {}, "up": {}}
    for record in records:
        size = _required_int(record.native_polyphony, "native_polyphony")
        if size <= 0:
            raise RuntimeError("native_polyphony must be positive")
        kind_counts = counts.setdefault(record.kind, {})
        key = str(size)
        kind_counts[key] = kind_counts.get(key, 0) + 1
    return counts


def _aggregate_metric(runs: list[dict[str, Any]], name: str) -> dict[str, Any]:
    rows = [row for run in runs for row in run["_metric_rows"][name]]
    if name in {"wake_error_us", "sender_completion_error_us", "pre_call_lateness_us"}:
        if name == "sender_completion_error_us":
            return _completion_error_report_pairs(rows)
        if name == "pre_call_lateness_us":
            return _pre_call_error_report_pairs(rows)
        signed = [value for _, value in rows]
        return {
            "signed": _required_stats(signed, name),
            "absolute": _required_stats([abs(value) for value in signed], name),
            "late": _stats([value for value in signed if value > 0]),
            "early": _stats([-value for value in signed if value < 0]),
            "by_kind": {
                kind: {
                    "signed": _required_stats(
                        [value for row_kind, value in rows if row_kind == kind], name
                    ),
                    "absolute": _required_stats(
                        [abs(value) for row_kind, value in rows if row_kind == kind], name
                    ),
                    "late": _stats(
                        [value for row_kind, value in rows if row_kind == kind and value > 0]
                    ),
                    "early": _stats(
                        [-value for row_kind, value in rows if row_kind == kind and value < 0]
                    ),
                }
                for kind in ("down", "up")
            },
        }
    return _nonnegative_metric_report(rows, name)


def _aggregate_scalar_sum(runs: list[dict[str, Any]], name: str) -> int:
    return sum(int(run.get(name, 0)) for run in runs)


def _aggregate_native_packet_size_counts(
    runs: list[dict[str, Any]],
) -> dict[str, dict[str, int]]:
    counts: dict[str, dict[str, int]] = {"down": {}, "up": {}}
    for run in runs:
        for kind, kind_counts in run["native_packet_size_counts"].items():
            target = counts.setdefault(kind, {})
            for size, count in kind_counts.items():
                target[size] = target.get(size, 0) + int(count)
    return counts


def _aggregate_warmup_records(runs: list[dict[str, Any]]) -> int:
    return sum(int(run["warmup_records"]) for run in runs)


def _aggregate_scalar_min_nonzero(runs: list[dict[str, Any]], name: str) -> int:
    values = [int(run.get(name, 0)) for run in runs if int(run.get(name, 0)) > 0]
    return min(values, default=0)


def _aggregate_scalar_max(runs: list[dict[str, Any]], name: str) -> int:
    return max((int(run.get(name, 0)) for run in runs), default=0)


def _completion_error_report(records: list[TelemetryRecord]) -> dict[str, Any]:
    return _completion_error_report_pairs(
        [
            (record.kind, record.sender_completion_error_us)
            for record in records
            if isinstance(record.sender_completion_error_us, int)
            and not isinstance(record.sender_completion_error_us, bool)
        ]
    )


def _peak_working_set_bytes() -> int | None:
    """Read this benchmark process's peak working set without Python Win32 ctypes."""
    if os.name != "nt":
        return None
    process_id = str(os.getpid())
    command = f"(Get-Process -Id {process_id}).PeakWorkingSet64"
    for executable in ("powershell", "pwsh"):
        try:
            result = subprocess.run(
                [executable, "-NoProfile", "-NonInteractive", "-Command", command],
                capture_output=True,
                text=True,
                check=True,
                timeout=5,
            )
        except (OSError, subprocess.SubprocessError):
            continue
        try:
            return int(result.stdout.strip().splitlines()[-1])
        except (IndexError, ValueError):
            return None
    return None


def _new_session(
    actions: list[tuple[int, str, int, list[int], str]],
    *,
    backend: str,
    mock_base_latency_us: int,
    mock_per_key_latency_us: int,
    adaptive_spin: bool,
    rt_priority_mode: str,
    lead_mode: str = "fixed",
    fixed_lead_us: int = 0,
    game_fps: int = 60,
    gap_profile: str = "hot",
    require_focus: bool = False,
    fault_mode: str = "none",
) -> Any:
    import sky_player_rs

    materialized_min_hold_us = _materialized_hold_us(
        game_fps=game_fps,
        gap_profile=gap_profile,
    )

    if backend == "mock":
        test_session = getattr(sky_player_rs, "TestDispatchSession", None)
        if not callable(test_session):
            raise RuntimeError(
                "mock acceptance requires a test-support native wheel; "
                "production wheels expose only DispatchSession"
            )
        return test_session(
            actions,
            list(SKY_15_SCAN_CODES),
            min_hold_us=materialized_min_hold_us,
            game_fps=game_fps,
            mock_latency_base_us=mock_base_latency_us,
            mock_latency_per_key_us=mock_per_key_latency_us,
            telemetry_capacity=min(4_096, max(64, len(actions) + 64)),
            rt_priority_mode=rt_priority_mode,
            enable_waitable_timer=True,
            enable_event_wait=True,
            enable_adaptive_spin=adaptive_spin,
            dispatch_lead_us=fixed_lead_us,
            enable_dispatch_cost_lead=lead_mode == "adaptive",
            fault_mode=fault_mode,
        )
    if backend != "sendinput":
        raise ValueError(f"unsupported native benchmark backend: {backend}")
    if callable(getattr(sky_player_rs, "TestDispatchSession", None)):
        raise RuntimeError(
            "SendInput qualification requires a production native wheel; "
            "test-support is not a valid physical timing path"
        )
    target_hwnd = _real_input_target_hwnd()
    return sky_player_rs.DispatchSession(  # type: ignore[attr-defined]
        actions,
        config=sky_player_rs.SessionConfig(  # type: ignore[attr-defined]
            game_fps=game_fps,
            min_hold_us=materialized_min_hold_us,
            require_focus=require_focus,
            target_hwnd=target_hwnd,
            telemetry=True,
            profile="strict_timing_diagnostic",
        ),
    )


def _same_key_zero_gap_actions() -> list[tuple[int, str, int, list[int], str]]:
    """Return the deliberately invalid negative-admission fixture."""

    return [
        (0, "down", 0, [int(SKY_15_SCAN_CODES[0])], "zero-gap-down"),
        (1, "up", 20_000, [int(SKY_15_SCAN_CODES[0])], "zero-gap-up"),
        (2, "down", 20_000, [int(SKY_15_SCAN_CODES[0])], "zero-gap-retrigger"),
        (3, "up", 40_000, [int(SKY_15_SCAN_CODES[0])], "zero-gap-release"),
    ]


def _command_interrupt_actions() -> list[tuple[int, str, int, list[int], str]]:
    interrupt_key = [int(SKY_15_SCAN_CODES[0])]
    return [
        (0, "down", 100_000, interrupt_key, "interrupt-down"),
        (1, "up", 10_000_000, interrupt_key, "interrupt-cleanup"),
    ]


def _command_interrupt_polyphony(
    actions: list[tuple[int, str, int, list[int], str]],
) -> int:
    down_actions = [action for action in actions if action[1] == "down"]
    if len(down_actions) != 1 or not down_actions[0][3]:
        raise RuntimeError("command-interrupt fixture must contain one non-empty Down")
    return len(down_actions[0][3])


def _assert_same_key_zero_gap_rejected(
    *,
    backend: str,
    mock_base_latency_us: int,
    mock_per_key_latency_us: int,
    adaptive_spin: bool,
    rt_priority_mode: str,
    game_fps: int,
) -> None:
    """Prove admission rejects the old mixed positive-case construction.

    Construction must fail before a session can be armed, so this negative
    case performs zero musical SendInput calls by construction.
    """

    try:
        _new_session(
            _same_key_zero_gap_actions(),
            backend=backend,
            mock_base_latency_us=mock_base_latency_us,
            mock_per_key_latency_us=mock_per_key_latency_us,
            adaptive_spin=adaptive_spin,
            rt_priority_mode=rt_priority_mode,
            game_fps=game_fps,
            gap_profile="hot",
        )
    except Exception as exc:
        message = str(exc).lower()
        if "release gap" not in message or "same-key" not in message:
            raise RuntimeError(
                "same-key zero-gap negative case failed for an unexpected reason: "
                f"{type(exc).__name__}: {exc}"
            ) from exc
        return
    raise RuntimeError(
        "native admission accepted a same-key zero-gap schedule; "
        "the negative case would be unsafe to run"
    )


def _real_input_target_hwnd(*, require_focus: bool = True) -> int:
    del require_focus
    raw = os.environ.get("SKY_NATIVE_TARGET_HWND")
    if raw is None or not raw.strip():
        raise RuntimeError(
            "real SendInput qualification requires SKY_NATIVE_TARGET_HWND from the isolated test sink"
        )
    try:
        hwnd = int(raw, 0)
    except ValueError as exc:
        raise RuntimeError("SKY_NATIVE_TARGET_HWND must be a positive integer") from exc
    if hwnd <= 0 or hwnd > (1 << 63) - 1:
        raise RuntimeError("SKY_NATIVE_TARGET_HWND must be a positive integer in the platform handle range")
    return hwnd


def _arm_acceptance_session(session: Any, *, backend: str) -> None:
    """Use the public production arm API; ``start`` is test-support-only."""

    if backend == "sendinput":
        session.arm(0)
    else:
        session.start()


def _run_dispatch(
    actions: list[tuple[int, str, int, list[int], str]],
    polyphony: int,
    *,
    backend: str,
    mock_base_latency_us: int,
    mock_per_key_latency_us: int,
    adaptive_spin: bool,
    rt_priority_mode: str,
    lead_mode: str = "fixed",
    fixed_lead_us: int = 0,
    game_fps: int = 60,
    gap_profile: str = "hot",
    require_focus: bool = False,
    scenario: str = "paired",
    warmup_cycles: int = 0,
    timeout_ms: int = 60_000,
    fault_mode: str = "none",
    native_build_commit: str | None = None,
) -> dict[str, Any]:
    expected_hold_pair_samples = _expected_hold_pair_samples(actions, scenario)
    session = _new_session(
        actions,
        backend=backend,
        mock_base_latency_us=mock_base_latency_us,
        mock_per_key_latency_us=mock_per_key_latency_us,
        adaptive_spin=adaptive_spin,
        rt_priority_mode=rt_priority_mode,
        lead_mode=lead_mode,
        fixed_lead_us=fixed_lead_us,
        game_fps=game_fps,
        gap_profile=gap_profile,
        require_focus=require_focus,
        fault_mode=fault_mode,
    )
    snapshot: dict[str, Any] | None = None
    telemetry: dict[str, Any] | None = None
    diagnostics: dict[str, Any] | None = None
    try:
        started_ns = time.perf_counter_ns()
        _arm_acceptance_session(session, backend=backend)
        startup_deadline = time.perf_counter() + min(timeout_ms / 1_000, 60.0)
        while not bool(dict(session.snapshot()).get("startup_ready")):
            # The native worker has a bounded supervisor lease.  Acceptance
            # drives the session directly rather than through the normal
            # Python supervisor, so it must publish the same heartbeat while
            # waiting for a long schedule to finish.  Without this, a valid
            # ~5-second benchmark is terminalized at the three-second lease
            # boundary and looks like missing telemetry (303/528 records).
            session.heartbeat()
            if time.perf_counter() >= startup_deadline:
                raise RuntimeError("native worker did not publish startup-ready boundary")
            time.sleep(0.001)

        # Polling and heartbeat publication are supervisor work only; the
        # Rust worker remains the sole owner of deadlines, waits, dispatch and
        # key state.  Do not block in join() before the heartbeat has kept the
        # worker lease alive for the whole authored schedule.
        completion_deadline = time.perf_counter() + timeout_ms / 1_000
        while not bool(dict(session.snapshot()).get("is_finished")):
            session.heartbeat()
            if time.perf_counter() >= completion_deadline:
                break
            time.sleep(0.005)

        if not session.join(timeout_ms=timeout_ms):
            session.panic_release()
            session.quit()
            session.join(timeout_ms=5_000)
            raise RuntimeError("native acceptance session exceeded its hard budget")
        wall_us = (time.perf_counter_ns() - started_ns) // 1_000
        snapshot = dict(session.snapshot())
        telemetry = json.loads(session.take_telemetry_json())
        if not isinstance(telemetry, dict):
            raise RuntimeError("native telemetry envelope must be an object")
        records = materialize_native_trace(
            _normalize_historical_native_trace(
                telemetry,
                native_build_commit=native_build_commit or "",
            )
        )
        diagnostics = _validate_telemetry_integrity(
            actions=actions,
            telemetry=telemetry,
            records=records,
            polyphony=polyphony,
            scenario=scenario,
            snapshot=snapshot,
        )
        all_metric_rows = _trace_metric_rows(records)
        warmup_record_count = _warmup_record_count(
            actions,
            scenario,
            warmup_cycles,
        )
        if warmup_record_count < 0 or warmup_record_count >= len(records):
            raise RuntimeError("warmup cycles must leave measurement records")
        measurement_records = records[warmup_record_count:]
        metric_rows = _trace_metric_rows(measurement_records)
        sender_errors = [value for _, value in metric_rows["sender_completion_error_us"]]
        lead_by_polyphony = {
            str(record.native_polyphony): int(record.applied_lead_us)
            for record in records
            if record.kind == "down" and record.native_polyphony is not None
        }
        completion_error_rows = [
            (
                record.kind,
                _required_int(record.sender_completion_error_us, "sender_completion_error_us"),
            )
            for record in measurement_records
        ]
        peak_rss = _peak_working_set_bytes()
        result: dict[str, Any] = {
            "polyphony": polyphony,
            "wall_us": wall_us,
            "_sender_error_values": sender_errors,
            "_completion_error_rows": completion_error_rows,
            "_metric_rows": metric_rows,
            "_all_metric_rows": all_metric_rows,
            "warmup_records": warmup_record_count,
            "measurement_records": len(measurement_records),
            "_snapshot": snapshot,
            "_telemetry": telemetry,
            "_telemetry_integrity": diagnostics,
            "native_packet_size_counts": _native_packet_size_counts(measurement_records),
            "correctness": _correctness_counters(
                snapshot,
                diagnostics,
                expected_hold_pair_samples=expected_hold_pair_samples,
            ),
            "sender_completion_error_us": _required_stats(sender_errors, "sender_completion_error_us"),
            "pre_call_lateness_us": _pre_call_error_report_pairs(
                metric_rows["pre_call_lateness_us"]
            ),
            "completion_error_us": _completion_error_report_pairs(completion_error_rows),
            "spin_cpu_time_us": int(snapshot.get("spin_time_us", 0)),
            "worker_cpu_time_us": int(snapshot.get("worker_cpu_time_us", 0)),
            "process_cpu_time_us": int(snapshot.get("process_cpu_time_us", 0)),
            "playback_wall_time_us": int(snapshot.get("playback_wall_time_us", 0)),
            "spin_duty_cycle_ppm": int(snapshot.get("spin_duty_cycle_ppm", 0)),
            "peak_rss_bytes": peak_rss,
            "keys_dropped": int(snapshot.get("keys_dropped", 0)),
            "failed_release_count": int(snapshot.get("failed_release_count", 0)),
            "chord_split_events": int(snapshot.get("chord_split_events", 0)),
            "chords_rejected": int(snapshot.get("chords_rejected", 0)),
            "authored_keys_rejected": int(snapshot.get("authored_keys_rejected", 0)),
            "sendinput_partial_events": int(snapshot.get("sendinput_partial_events", 0)),
            "sendinput_zero_progress_failures": int(
                snapshot.get("sendinput_zero_progress_failures", 0)
            ),
            "sendinput_path_degraded": bool(snapshot.get("sendinput_path_degraded", False)),
            "core_post_send_degraded": bool(snapshot.get("core_post_send_degraded", False)),
            "observer_duration_max_us": int(snapshot.get("observer_duration_max_us", 0)),
            "wait_path_degraded": bool(snapshot.get("wait_path_degraded", False)),
            "sendinput_warn_threshold_us": int(snapshot.get("sendinput_warn_threshold_us", 0)),
            "core_post_send_warn_threshold_us": int(
                snapshot.get("core_post_send_warn_threshold_us", 0)
            ),
            "wait_warn_threshold_us": int(snapshot.get("wait_warn_threshold_us", 0)),
            "sendinput_degraded_samples": int(
                snapshot.get("sendinput_degraded_samples", 0)
            ),
            "core_post_send_degraded_samples": int(
                snapshot.get("core_post_send_degraded_samples", 0)
            ),
            "wait_degraded_samples": int(snapshot.get("wait_degraded_samples", 0)),
            "lead_saturation_count_down": list(
                snapshot.get("lead_saturation_count_down", [])
            ),
            "lead_saturation_count_up": list(snapshot.get("lead_saturation_count_up", [])),
            "positive_residual_at_cap": int(snapshot.get("positive_residual_at_cap", 0)),
            "lead_by_polyphony": lead_by_polyphony,
            "generation_status_counts": dict(snapshot.get("generation_status_counts", {})),
            "missed_down_boundaries": int(snapshot.get("missed_down_boundaries", 0)),
            "missed_backlog_boundaries": int(
                snapshot.get("missed_backlog_boundaries", 0)
            ),
            "missed_hard_late_boundaries": int(
                snapshot.get("missed_hard_late_boundaries", 0)
            ),
            "hold_pair_samples": int(snapshot.get("hold_pair_samples", 0)),
            "expected_hold_pair_samples": expected_hold_pair_samples,
            "min_pre_call_hold_us": int(snapshot.get("min_pre_call_hold_us", 0)),
            "min_completion_hold_us": int(
                snapshot.get("min_completion_hold_us", 0)
            ),
            "max_pre_call_hold_shrink_us": int(
                snapshot.get("max_pre_call_hold_shrink_us", 0)
            ),
            "max_completion_hold_shrink_us": int(
                snapshot.get("max_completion_hold_shrink_us", 0)
            ),
            "pre_call_hold_shrink_over_grace_count": int(
                snapshot.get("pre_call_hold_shrink_over_grace_count", 0)
            ),
            "hold_unmatched_up_count": int(
                snapshot.get("hold_unmatched_up_count", 0)
            ),
            "hold_anchor_overwrite_count": int(
                snapshot.get("hold_anchor_overwrite_count", 0)
            ),
            "same_call_retrigger_boundaries": int(
                snapshot.get("same_call_retrigger_boundaries", 0)
            ),
            "same_call_retrigger_keys": int(
                snapshot.get("same_call_retrigger_keys", 0)
            ),
            "production_completion_hold_below_frame_count": int(
                snapshot.get("production_completion_hold_below_frame_count", 0)
            ),
            "production_release_gap_below_policy_count": int(
                snapshot.get("production_release_gap_below_policy_count", 0)
            ),
            "production_same_call_same_key_retrigger_count": int(
                snapshot.get("production_same_call_same_key_retrigger_count", 0)
            ),
            "production_anchor_overwrite_count": int(
                snapshot.get("production_anchor_overwrite_count", 0)
            ),
            "production_unmatched_up_count": int(
                snapshot.get("production_unmatched_up_count", 0)
            ),
            "production_anomaly_ring_overwrite_count": int(
                snapshot.get("production_anomaly_ring_overwrite_count", 0)
            ),
            "production_forensics_anomaly_count": int(
                snapshot.get("production_forensics_anomaly_count", 0)
            ),
            "observer_dropped_records": int(snapshot.get("observer_dropped_samples", 0)),
            "outcome": snapshot.get("outcome"),
            "startup_latency_us": _required_int(
                snapshot.get("startup_latency_us"), "startup_latency_us"
            ),
        }
        result["worker_cpu_ratio_ppm"] = _ratio_ppm(
            result["worker_cpu_time_us"], result["playback_wall_time_us"]
        )
        result["process_cpu_ratio_ppm"] = _ratio_ppm(
            result["process_cpu_time_us"], result["playback_wall_time_us"]
        )
        result["spin_cpu_ratio_ppm"] = _ratio_ppm(
            result["spin_cpu_time_us"], result["playback_wall_time_us"]
        )
        return result
    except Exception as exc:
        if snapshot is None:
            try:
                snapshot = dict(session.snapshot())
            except Exception:
                snapshot = None
        if telemetry is None:
            try:
                telemetry = json.loads(session.take_telemetry_json())
            except Exception:
                telemetry = None
        if diagnostics is None and isinstance(exc, TelemetryIntegrityError):
            diagnostics = exc.diagnostics
        raise BenchmarkRunFailure(
            original=exc,
            snapshot=snapshot,
            telemetry=telemetry,
            diagnostics=diagnostics,
        ) from exc


def _measure_command_interrupt(
    *,
    backend: str,
    mock_base_latency_us: int,
    mock_per_key_latency_us: int,
    adaptive_spin: bool,
    rt_priority_mode: str,
    lead_mode: str = "fixed",
    fixed_lead_us: int = 0,
    game_fps: int = 60,
    require_focus: bool = False,
) -> dict[str, int]:
    # Put the first Down in a controlled future slot, then measure the pause
    # command after that first musical commit.  Pre-roll Pause now cancels the
    # start attempt by contract, so this probe must exercise the mid-play
    # command path rather than the preroll cancellation path.
    actions = _command_interrupt_actions()
    session = _new_session(
        actions,
        backend=backend,
        mock_base_latency_us=mock_base_latency_us,
        mock_per_key_latency_us=mock_per_key_latency_us,
        adaptive_spin=adaptive_spin,
        rt_priority_mode=rt_priority_mode,
        lead_mode=lead_mode,
        fixed_lead_us=fixed_lead_us,
        game_fps=game_fps,
        require_focus=require_focus,
    )
    # Test-only epoch choice made before worker arm; the frozen authored
    # timestamps remain unchanged and production callers do not use this path.
    session.arm(500_000)
    deadline = time.perf_counter() + 2.0
    while not bool(dict(session.snapshot()).get("startup_ready")):
        if time.perf_counter() >= deadline:
            raise RuntimeError("native worker did not publish startup-ready boundary")
        time.sleep(0.001)

    commit_deadline = time.perf_counter() + 2.0
    while True:
        progress = dict(session.snapshot())
        if progress.get("recent_latencies_us") or int(progress.get("active_count", 0)) > 0:
            break
        if bool(progress.get("is_finished")):
            session.join(timeout_ms=5_000)
            raise RuntimeError(
                "native worker terminated before first musical commit: "
                f"{progress.get('terminal_error')}"
            )
        if time.perf_counter() >= commit_deadline:
            session.quit()
            session.join(timeout_ms=5_000)
            raise RuntimeError("native worker did not publish first musical commit")
        time.sleep(0.001)

    pause_with_timing_token = getattr(session, "pause_with_timing_token", None)
    pause_timing_result = getattr(session, "pause_timing_result", None)
    if not callable(pause_with_timing_token) or not callable(pause_timing_result):
        session.quit()
        session.join(timeout_ms=5_000)
        raise RuntimeError(
            "native command timing requires a test-support wheel with QPC pause instrumentation"
        )
    request_pause = cast(Callable[[], int], pause_with_timing_token)
    get_pause_result = cast(Callable[[int], Any], pause_timing_result)
    generation = int(request_pause())
    while True:
        native_result = get_pause_result(generation)
        if native_result is not None:
            result: dict[str, Any] = dict(native_result)
            break
        if time.perf_counter() >= deadline + 2.0:
            session.quit()
            session.join(timeout_ms=5_000)
            raise RuntimeError("native pause command was not observed")
        time.sleep(0.001)
    session.quit()
    if not session.join(timeout_ms=5_000):
        raise RuntimeError("native command-interrupt session did not terminate")
    required = (
        "generation",
        "requested_ticks",
        "observed_ticks",
        "acknowledged_ticks",
        "observation_latency_us",
        "completion_latency_us",
        "cleanup_cost_us",
    )
    if result.get("generation") != generation or any(
        key not in result for key in required
    ):
        raise RuntimeError("native pause timing result is incomplete")
    if not (
        int(result["requested_ticks"])
        <= int(result["observed_ticks"])
        <= int(result["acknowledged_ticks"])
    ):
        raise RuntimeError("native pause timing QPC ordering is invalid")
    if any(int(result[key]) < 0 for key in required[4:]):
        raise RuntimeError("native pause timing latency must be non-negative")
    return {key: int(result[key]) for key in required}


def _measure_preroll_pause_cancellation(
    *,
    backend: str,
    mock_base_latency_us: int,
    mock_per_key_latency_us: int,
    adaptive_spin: bool,
    rt_priority_mode: str,
) -> dict[str, Any]:
    """Verify the locked pre-roll Pause cancellation contract."""
    key = [int(SKY_15_SCAN_CODES[0])]
    session = _new_session(
        [(0, "down", 10_000_000, key, "preroll-cancel")],
        backend=backend,
        mock_base_latency_us=mock_base_latency_us,
        mock_per_key_latency_us=mock_per_key_latency_us,
        adaptive_spin=adaptive_spin,
        rt_priority_mode=rt_priority_mode,
    )
    session.start()
    deadline = time.perf_counter() + 2.0
    while not bool(dict(session.snapshot()).get("startup_ready")):
        if time.perf_counter() >= deadline:
            session.quit()
            session.join(timeout_ms=5_000)
            raise RuntimeError("native worker did not publish startup-ready boundary")
        time.sleep(0.001)
    session.pause()
    while True:
        snapshot = dict(session.snapshot())
        if bool(snapshot.get("is_finished")):
            break
        if time.perf_counter() >= deadline + 2.0:
            session.quit()
            session.join(timeout_ms=5_000)
            raise RuntimeError("native preroll pause did not cancel the start attempt")
        time.sleep(0.001)
    if not session.join(timeout_ms=5_000):
        raise RuntimeError("native preroll cancellation session did not terminate")
    return snapshot


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--actions", type=int, default=128)
    parser.add_argument(
        "--repeats",
        type=int,
        default=None,
        help="deprecated alias for --dispatch-repeats",
    )
    parser.add_argument("--dispatch-repeats", type=int, default=None)
    parser.add_argument("--command-samples", type=int, default=100)
    parser.add_argument(
        "--skip-command-samples",
        action="store_true",
        help="skip the fresh-session command phase",
    )
    parser.add_argument("--warmup-cycles", type=int, default=8)
    parser.add_argument(
        "--start-delay-us",
        type=int,
        default=0,
        help="delay the first authored action after arm (default: 0)",
    )
    parser.add_argument(
        "--continue-after-failure",
        action="store_true",
        help="run remaining repetitions for diagnostics, but still exit non-zero",
    )
    parser.add_argument(
        "--budget-seconds",
        type=float,
        default=120.0,
        help="hard whole-command budget in seconds (1..600; default: 120)",
    )
    parser.add_argument("--label", default="native")
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--polyphony",
        default="1,2,3,5,8,15",
        help=(
            "comma-separated chord sizes to exercise (default: 1,2,3,5,8,15); "
            "mixed/coalesced require every value >= 2"
        ),
    )
    parser.add_argument(
        "--scenario",
        choices=("paired", "mixed", "coalesced"),
        default="paired",
        help=(
            "authored action profile; mixed/coalesced use disjoint adjacent Up/Down "
            "masks and are correctness-only per requested suite"
        ),
    )
    parser.add_argument(
        "--backend",
        choices=("mock", "sendinput"),
        default="mock",
        help="mock = injected polyphony-aware latency (default), sendinput = real fixed-host seam",
    )
    parser.add_argument(
        "--allow-real-input",
        action="store_true",
        help="required with --backend sendinput; keys may reach the foreground window",
    )
    parser.add_argument(
        "--require-focus",
        action=argparse.BooleanOptionalAction,
        default=None,
        help="require the configured target window to remain focused (sendinput default: true)",
    )
    parser.add_argument(
        "--mock-base-latency-us",
        type=int,
        default=None,
        help="mock backend base latency in microseconds (default: 80; mock only)",
    )
    parser.add_argument(
        "--mock-per-key-latency-us",
        type=int,
        default=None,
        help="mock backend per-key latency in microseconds (default: 40; mock only)",
    )
    parser.add_argument(
        "--no-adaptive-spin",
        action="store_true",
        help="disable adaptive wait probing; adaptive lead remains enabled",
    )
    parser.add_argument(
        "--rt-priority-mode",
        choices=("auto", "mmcss", "time_critical", "highest", "off"),
        default="off",
        help="real-time priority policy (default: off)",
    )
    parser.add_argument(
        "--lead-mode",
        choices=("fixed", "adaptive"),
        default="fixed",
        help="fixed raw regression mode or production adaptive-lead mode",
    )
    parser.add_argument("--fixed-lead-us", type=int, default=0)
    parser.add_argument("--gap-profile", choices=("hot", "cold"), default="hot")
    parser.add_argument("--game-fps", type=int, default=60)
    parser.add_argument("--expected-native-build-commit")
    parser.add_argument(
        "--baseline",
        type=Path,
        help="optional JSON report with sender-side regression thresholds",
    )
    return parser.parse_args()


def _parse_polyphony(raw: str) -> list[int]:
    try:
        values = [int(part.strip()) for part in raw.split(",") if part.strip()]
    except ValueError as exc:
        raise SystemExit("--polyphony must contain comma-separated integers") from exc
    if not values or len(set(values)) != len(values):
        raise SystemExit("--polyphony must contain at least one unique value")
    if any(value < 1 or value > len(SKY_15_SCAN_CODES) for value in values):
        raise SystemExit(f"--polyphony values must be in 1..{len(SKY_15_SCAN_CODES)}")
    return values


def _resolve_mock_latency_values(
    *,
    backend: str,
    mock_base_latency_us: int | None,
    mock_per_key_latency_us: int | None,
) -> tuple[int, int]:
    if backend == "sendinput" and (
        mock_base_latency_us is not None or mock_per_key_latency_us is not None
    ):
        raise SystemExit("mock latency values are only valid with --backend mock")

    base_latency_us = (
        80 if mock_base_latency_us is None else mock_base_latency_us
    )
    per_key_latency_us = (
        40 if mock_per_key_latency_us is None else mock_per_key_latency_us
    )
    if base_latency_us < 0 or per_key_latency_us < 0:
        raise SystemExit("mock latency values must be non-negative")
    if backend == "sendinput":
        return 0, 0
    return base_latency_us, per_key_latency_us


def _resolve_lead_config(args: argparse.Namespace) -> tuple[str, int]:
    if args.fixed_lead_us < 0 or args.fixed_lead_us > 10_000:
        raise SystemExit("--fixed-lead-us must be between 0 and 10000")
    if args.lead_mode == "adaptive" and args.fixed_lead_us != 0:
        raise SystemExit("--fixed-lead-us must be 0 in adaptive lead mode")
    return args.lead_mode, args.fixed_lead_us


def _assert_correctness(run: dict[str, Any]) -> None:
    if run["outcome"] != "finished":
        raise RuntimeError(f"native acceptance outcome was {run['outcome']!r}")
    if run["keys_dropped"] or run["failed_release_count"] or run["chord_split_events"]:
        raise RuntimeError(
            "native acceptance correctness failure: "
            f"polyphony={run['polyphony']} "
            f"keys_dropped={run['keys_dropped']} "
            f"failed_release_count={run['failed_release_count']} "
            f"chord_split_events={run['chord_split_events']}"
        )
    correctness = run.get("correctness")
    if isinstance(correctness, dict) and any(correctness.values()):
        raise RuntimeError(
            "native acceptance correctness counters are non-zero: "
            + json.dumps(correctness, sort_keys=True)
        )
    expected_hold_pair_samples = run.get("expected_hold_pair_samples")
    actual_hold_pair_samples = run.get("hold_pair_samples")
    if (
        not isinstance(expected_hold_pair_samples, int)
        or not isinstance(actual_hold_pair_samples, int)
        or actual_hold_pair_samples != expected_hold_pair_samples
    ):
        raise RuntimeError(
            "native acceptance hold-pair completeness failure: "
            f"actual={actual_hold_pair_samples!r} "
            f"expected={expected_hold_pair_samples!r}"
        )
    statuses = run["generation_status_counts"]
    nonterminal = sum(
        int(statuses.get(name, 0)) for name in ("scheduled", "active", "release_pending")
    )
    if nonterminal:
        raise RuntimeError(
            f"native acceptance left {nonterminal} nonterminal generations "
            f"for polyphony={run['polyphony']}"
        )


def _report_sha(report: dict[str, Any]) -> str:
    for field in ("candidate_sha", "native_build_commit", "git_sha"):
        value = report.get(field)
        if isinstance(value, str) and value:
            return value
    raise SystemExit("benchmark report is missing an exact candidate SHA")


def _require_native_build_flavor(backend: str) -> str:
    flavor = _native_build_flavor()
    required = "production" if backend == "sendinput" else "test_support"
    if flavor != required:
        raise SystemExit(
            f"{backend} benchmark requires the {required} native wheel; "
            f"loaded flavor is {flavor}"
        )
    return flavor


def _timeline_semantics_version(report: dict[str, Any]) -> int:
    sha = _report_sha(report)
    value = report.get("timeline_semantics_version")
    if value is None:
        value = KNOWN_TIMELINE_SEMANTICS.get(sha)
        if value is None:
            raise SystemExit(
                "benchmark report is missing timeline semantics for an unknown SHA"
            )
    if value not in (1, 2):
        raise SystemExit("benchmark timeline semantics version is invalid")
    return int(value)


def _assert_comparison_contract(
    report: dict[str, Any], baseline: dict[str, Any]
) -> None:
    role = report.get("comparison_role")
    if role not in {SAME_SEMANTICS, TRANSPORT_REFERENCE}:
        raise SystemExit("benchmark comparison_role is invalid")
    candidate_sha = _report_sha(report)
    baseline_sha = _report_sha(baseline)
    reference_sha = report.get("reference_sha")
    if reference_sha != baseline_sha:
        raise SystemExit("benchmark reference_sha does not match the baseline SHA")
    candidate_semantics = _timeline_semantics_version(report)
    baseline_semantics = _timeline_semantics_version(baseline)
    if role == SAME_SEMANTICS:
        if baseline_sha != SAME_SEMANTICS_REFERENCE_SHA:
            raise SystemExit("SAME_SEMANTICS requires the canonical 9ef9e578 reference")
        if candidate_sha == baseline_sha:
            raise SystemExit("candidate and SAME_SEMANTICS reference must differ")
        if candidate_semantics != baseline_semantics or candidate_semantics != 2:
            raise SystemExit("SAME_SEMANTICS requires timeline semantics version 2")
    else:
        if baseline_sha != TRANSPORT_REFERENCE_SHA:
            raise SystemExit("TRANSPORT_REFERENCE requires the canonical 109f1c33 reference")
        if candidate_semantics != 2 or baseline_semantics != 1:
            raise SystemExit("TRANSPORT_REFERENCE requires candidate v2 and reference v1")


def _assert_report_correctness(report: dict[str, Any]) -> None:
    correctness = report.get("correctness")
    if not isinstance(correctness, dict):
        raise SystemExit("benchmark correctness counters are missing")
    required_zero = (
        "chord_integrity_lost",
        "unexpected_held",
        "pending_unresolved",
        "cleanup_uncertainty",
        "telemetry_integrity_failures",
        "sender_integrity_failures",
        "unexpected_transport_failures",
        "authored_trace_missing_duplicate_mismatch",
        "missed_down_boundaries",
        "pre_call_hold_shrink_over_grace_count",
        "hold_unmatched_up_count",
        "hold_anchor_overwrite_count",
        *PRODUCTION_CORRECTNESS_COUNTERS,
    )
    nonzero = {
        name: correctness.get(name)
        for name in required_zero
        if correctness.get(name) != 0
    }
    if nonzero:
        raise SystemExit(
            "native benchmark correctness failure before percentile comparison: "
            + json.dumps(nonzero, sort_keys=True)
        )
    expected_hold_pair_samples = report.get("expected_hold_pair_samples")
    actual_hold_pair_samples = report.get("hold_pair_samples")
    if (
        not isinstance(expected_hold_pair_samples, int)
        or not isinstance(actual_hold_pair_samples, int)
        or actual_hold_pair_samples != expected_hold_pair_samples
    ):
        raise SystemExit(
            "native benchmark hold-pair completeness failure: "
            f"actual={actual_hold_pair_samples!r} "
            f"expected={expected_hold_pair_samples!r}"
        )


def _assert_absolute_wake_slo(report: dict[str, Any]) -> None:
    config = report.get("benchmark_config")
    if not isinstance(config, dict):
        raise SystemExit("benchmark_config is required for the wake SLO")
    if (
        config.get("game_fps") == 60
        and config.get("gap_profile") == "hot"
        and config.get("lead_mode") == "fixed"
    ):
        observed = _metric_at(report, ("wake_error_us", "absolute", "p99"))
        if observed > ABSOLUTE_WAKE_P99_LIMIT_US:
            raise SystemExit(
                "fixed-hot 60 FPS absolute wake SLO failed: "
                f"p99={observed:g}us limit={ABSOLUTE_WAKE_P99_LIMIT_US}us"
            )


def _assert_absolute_pre_call_slo(report: dict[str, Any]) -> None:
    metric = report.get("pre_call_lateness_us")
    if not isinstance(metric, dict):
        raise SystemExit("pre_call_lateness_us is required for the timing SLO")
    early_count = metric.get("early_count")
    if early_count != 0:
        raise SystemExit(f"early physical send detected: count={early_count}")
    late_over_2ms_count = metric.get("late_over_2ms_count")
    if late_over_2ms_count != 0:
        raise SystemExit(
            "pre-call lateness safety SLO failed: "
            f">2ms={late_over_2ms_count}"
        )
    p99 = _metric_at(metric, ("late", "p99"))
    p999 = _metric_at(metric, ("late", "p999"))
    if p99 > ABSOLUTE_PRE_CALL_P99_LIMIT_US:
        raise SystemExit(
            "absolute pre-call p99 SLO failed: "
            f"p99={p99:g}us limit={ABSOLUTE_PRE_CALL_P99_LIMIT_US}us"
        )
    if p999 > ABSOLUTE_PRE_CALL_P999_LIMIT_US:
        raise SystemExit(
            "absolute pre-call p99.9 SLO failed: "
            f"p99.9={p999:g}us limit={ABSOLUTE_PRE_CALL_P999_LIMIT_US}us"
        )


def _qualification_boundary_gate(*, backend: str, measured_boundaries: int) -> bool:
    """Return whether a report has enough physical samples for qualification."""

    return backend != "sendinput" or measured_boundaries >= MIN_QUALIFICATION_PHYSICAL_BOUNDARIES


def _assert_minimum_qualification_boundaries(
    *, backend: str, measured_boundaries: int
) -> None:
    if not _qualification_boundary_gate(
        backend=backend, measured_boundaries=measured_boundaries
    ):
        raise SystemExit(
            "SendInput qualification requires at least "
            f"{MIN_QUALIFICATION_PHYSICAL_BOUNDARIES} physical boundaries; "
            f"measured={measured_boundaries}"
        )


def _comparison_metadata(
    candidate_sha: str, baseline_path: Path | None
) -> dict[str, Any]:
    reference_sha = SAME_SEMANTICS_REFERENCE_SHA
    comparison_role = SAME_SEMANTICS
    if baseline_path is not None:
        try:
            baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise SystemExit(f"cannot read benchmark baseline provenance: {exc}") from exc
        if not isinstance(baseline, dict):
            raise SystemExit("benchmark baseline provenance must be an object")
        reference_sha = _report_sha(baseline)
        if reference_sha == TRANSPORT_REFERENCE_SHA:
            comparison_role = TRANSPORT_REFERENCE
    return {
        "candidate_sha": candidate_sha,
        "reference_sha": reference_sha,
        "comparison_role": comparison_role,
        "timeline_semantics_version": TIMELINE_SEMANTICS_VERSION,
    }
def _resolve_repeat_counts(args: argparse.Namespace) -> tuple[int, int]:
    if args.repeats is not None and args.dispatch_repeats is not None:
        raise SystemExit(
            "--repeats and --dispatch-repeats are ambiguous; use one"
        )
    dispatch_repeats = (
        args.dispatch_repeats
        if args.dispatch_repeats is not None
        else args.repeats
        if args.repeats is not None
        else 2
    )
    if dispatch_repeats <= 0:
        raise SystemExit("--dispatch-repeats must be positive")
    if args.command_samples < 0:
        raise SystemExit("--command-samples must be non-negative")
    if getattr(args, "skip_command_samples", False):
        return dispatch_repeats, 0
    return dispatch_repeats, args.command_samples


def _assert_baseline_compatible(
    report: dict[str, Any],
    baseline: dict[str, Any],
) -> None:
    if baseline.get("benchmark_schema_version") != BENCHMARK_SCHEMA_VERSION:
        raise SystemExit(
            "legacy baseline is incompatible; regenerate with benchmark schema version 8"
        )
    if baseline.get("command_timing_domain") != COMMAND_TIMING_DOMAIN:
        raise SystemExit(
            "baseline command timing domain is incompatible; regenerate with native_qpc_v1"
        )
    if baseline.get("latency_segment_domain") != LATENCY_SEGMENT_DOMAIN:
        raise SystemExit(
            "baseline latency segment domain is incompatible; regenerate with native_trace_v1"
        )
    if report.get("benchmark_schema_version") != BENCHMARK_SCHEMA_VERSION:
        raise SystemExit("candidate benchmark schema version is invalid")
    if report.get("command_timing_domain") != COMMAND_TIMING_DOMAIN:
        raise SystemExit("candidate command timing domain is invalid")
    if report.get("latency_segment_domain") != LATENCY_SEGMENT_DOMAIN:
        raise SystemExit("candidate latency segment domain is invalid")
    _assert_comparison_contract(report, baseline)
    if report.get("candidate_sha") != _report_sha(report):
        raise SystemExit("candidate_sha must identify the candidate artifact")
    required_config = (
        "backend",
        "rt_priority_mode",
        "adaptive_spin",
        "waitable_timer",
        "event_wait",
        "mock_base_latency_us",
        "mock_per_key_latency_us",
        "actions",
        "polyphony",
        "lead_mode",
        "fixed_lead_us",
        "gap_profile",
        "warmup_cycles",
        "start_delay_us",
        "scenario",
        "native_profile",
        "native_build_flavor",
        "require_focus",
        "materialized_min_hold_us",
    )
    baseline_config = baseline.get("benchmark_config")
    report_config = report.get("benchmark_config")
    if not isinstance(baseline_config, dict) or not isinstance(report_config, dict):
        raise SystemExit(
            "baseline is incompatible; benchmark_config fingerprint is required"
        )
    for key in required_config:
        if key not in baseline_config or key not in report_config:
            raise SystemExit(
                f"baseline is incompatible; benchmark_config.{key} is required"
            )
    if baseline_config != report_config:
        raise SystemExit(
            "benchmark config fingerprint mismatch; refusing to compare metrics"
        )
    if baseline.get("statistics_eligible") is not True:
        raise SystemExit("baseline is incompatible; it is not statistics-eligible")
    if baseline.get("excluded_runs") != 0:
        raise SystemExit("baseline is incompatible; excluded runs are not allowed")
    if baseline.get("statistics_eligible") is not True or report.get("statistics_eligible") is not True:
        raise SystemExit("baseline and candidate must be statistics-eligible")


def allowed_value(
    baseline: float,
    *,
    relative_fraction: float,
    absolute_floor: float,
) -> float:
    return baseline + max(math.ceil(baseline * relative_fraction), absolute_floor)


def _metric_at(payload: dict[str, Any], path: tuple[str, ...]) -> float:
    value: Any = payload
    for part in path:
        if not isinstance(value, dict) or part not in value:
            raise SystemExit(f"benchmark metric is missing: {'.'.join(path)}")
        value = value[part]
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        raise SystemExit(f"benchmark metric is invalid: {'.'.join(path)}")
    return float(value)


def _assert_metric_threshold(
    report: dict[str, Any],
    baseline: dict[str, Any],
    path: tuple[str, ...],
    *,
    relative_fraction: float,
    absolute_floor: float,
) -> None:
    observed = _metric_at(report, path)
    expected = _metric_at(baseline, path)
    allowed = allowed_value(
        expected,
        relative_fraction=relative_fraction,
        absolute_floor=absolute_floor,
    )
    if observed > allowed:
        raise SystemExit(
            "native benchmark regression in "
            f"{'.'.join(path)}: observed={observed:g}, baseline={expected:g}, allowed={allowed:g}"
        )


def _assert_baseline(report: dict[str, Any], baseline_path: Path) -> None:
    try:
        baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise SystemExit(f"cannot read benchmark baseline {baseline_path}: {exc}") from exc

    _assert_baseline_compatible(report, baseline)
    _assert_report_correctness(report)
    _assert_absolute_wake_slo(report)
    if report.get("comparison_role") != TRANSPORT_REFERENCE:
        _assert_absolute_pre_call_slo(report)
    comparison_role = report["comparison_role"]
    correctness_only_scenario = report.get("benchmark_config", {}).get("scenario") in {
        "mixed",
        "coalesced",
    }
    signed_metrics = (
        ("wake_error_us", "absolute", "p99", 0.05, 5),
        ("wake_error_us", "absolute", "p999", 0.10, 10),
        ("sender_completion_error_us", "absolute", "p99", 0.05, 5),
        ("sender_completion_error_us", "absolute", "p999", 0.10, 10),
        ("sender_completion_error_us", "absolute", "max", 0.10, 20),
    )
    nonnegative_metrics = (
        ("pre_send_software_latency_us", "p99", 0.05, 5),
        ("pre_send_software_latency_us", "p999", 0.10, 10),
        ("sendinput_call_duration_us", "p99", 0.05, 5),
        ("sendinput_call_duration_us", "p999", 0.10, 10),
        ("core_post_send_duration_us", "p99", 0.05, 5),
        ("core_post_send_duration_us", "p999", 0.10, 10),
    )
    if comparison_role == TRANSPORT_REFERENCE:
        signed_metrics = ()
        nonnegative_metrics = (
            ("sendinput_call_duration_us", "p99", 0.05, 5),
            ("sendinput_call_duration_us", "p999", 0.10, 10),
        )
    for section, dimension, field, relative, floor in signed_metrics:
        _assert_metric_threshold(
            report,
            baseline,
            (section, dimension, field),
            relative_fraction=relative,
            absolute_floor=floor,
        )
    for section, field, relative, floor in nonnegative_metrics:
        _assert_metric_threshold(
            report,
            baseline,
            (section, field),
            relative_fraction=relative,
            absolute_floor=floor,
        )

    if comparison_role == TRANSPORT_REFERENCE:
        # v1/v2 timeline semantics are not comparable. The transport role is
        # deliberately limited to raw SendInput call duration; command
        # lifecycle, CPU/RSS/startup, wake, pre-send, and core-tail metrics
        # belong to SAME_SEMANTICS only.
        if correctness_only_scenario:
            return
        for polyphony in report["benchmark_config"]["polyphony"]:
            observed_poly = report["by_polyphony"][str(polyphony)]
            baseline_poly = baseline["by_polyphony"][str(polyphony)]
            for section, field, relative, floor in nonnegative_metrics:
                _assert_metric_threshold(
                    observed_poly,
                    baseline_poly,
                    (section, field),
                    relative_fraction=relative,
                    absolute_floor=floor,
                )
        return

    for section, relative, floor in (
        ("command_observation_latency_us", 0.05, 10),
        ("command_completion_latency_us", 0.05, 10),
        ("command_cleanup_cost_us", 0.10, 20),
    ):
        _assert_metric_threshold(
            report,
            baseline,
            (section, "p99"),
            relative_fraction=relative,
            absolute_floor=floor,
        )
    for section in ("worker_cpu_ratio_ppm", "process_cpu_ratio_ppm", "spin_cpu_ratio_ppm"):
        for field, relative in (("p50", 0.05), ("p95", 0.05), ("max", 0.10)):
            _assert_metric_threshold(
                report,
                baseline,
                (section, field),
                relative_fraction=relative,
                absolute_floor=0,
            )
    _assert_metric_threshold(
        report,
        baseline,
        ("peak_rss_bytes", "max"),
        relative_fraction=0.05,
        absolute_floor=2 * 1024 * 1024,
    )
    for field, relative, floor in (("p50", 0.05, 100), ("p99", 0.10, 250), ("max", 0.15, 500)):
        _assert_metric_threshold(
            report,
            baseline,
            ("startup_latency_us", field),
            relative_fraction=relative,
            absolute_floor=floor,
        )

    if correctness_only_scenario:
        return

    for polyphony in report["benchmark_config"]["polyphony"]:
        observed_poly = report["by_polyphony"][str(polyphony)]
        baseline_poly = baseline["by_polyphony"][str(polyphony)]
        for section, dimension, field, relative, floor in signed_metrics:
            _assert_metric_threshold(
                observed_poly,
                baseline_poly,
                (section, dimension, field),
                relative_fraction=relative,
                absolute_floor=floor,
            )
        for section, field, relative, floor in nonnegative_metrics:
            _assert_metric_threshold(
                observed_poly,
                baseline_poly,
                (section, field),
                relative_fraction=relative,
                absolute_floor=floor,
            )
        for section in ("worker_cpu_ratio_ppm", "process_cpu_ratio_ppm", "spin_cpu_ratio_ppm"):
            for field, relative in (("p50", 0.05), ("p95", 0.05), ("max", 0.10)):
                _assert_metric_threshold(
                    observed_poly,
                    baseline_poly,
                    (section, field),
                    relative_fraction=relative,
                    absolute_floor=0,
                )
        _assert_metric_threshold(
            observed_poly,
            baseline_poly,
            ("peak_rss_bytes", "max"),
            relative_fraction=0.05,
            absolute_floor=2 * 1024 * 1024,
        )


def main() -> int:
    args = _parse_args()
    if os.name != "nt":
        raise SystemExit("this acceptance benchmark requires Windows")
    if args.start_delay_us < 0:
        raise SystemExit("--start-delay-us must be non-negative")
    dispatch_repeats, command_samples = _resolve_repeat_counts(args)
    lead_mode, fixed_lead_us = _resolve_lead_config(args)
    if args.actions <= 0:
        raise SystemExit("--actions must be positive")
    if not 15 <= args.game_fps <= 240:
        raise SystemExit("--game-fps must be between 15 and 240")
    if args.warmup_cycles < 0:
        raise SystemExit("--warmup-cycles must be non-negative")
    if (
        isinstance(args.budget_seconds, bool)
        or not math.isfinite(args.budget_seconds)
        or not MIN_BENCHMARK_BUDGET_SECONDS
        <= args.budget_seconds
        <= MAX_BENCHMARK_BUDGET_SECONDS
    ):
        raise SystemExit("--budget-seconds must be between 1 and 600 seconds")
    if args.backend == "sendinput" and not args.allow_real_input:
        raise SystemExit("--backend sendinput requires --allow-real-input")
    mock_base_latency_us, mock_per_key_latency_us = _resolve_mock_latency_values(
        backend=args.backend,
        mock_base_latency_us=args.mock_base_latency_us,
        mock_per_key_latency_us=args.mock_per_key_latency_us,
    )

    git_info = _git_provenance()
    expected_native_commit = args.expected_native_build_commit or git_info["git_sha"]
    native_info = _native_provenance(expected_native_commit)
    if native_info["native_build_commit"] != expected_native_commit:
        raise RuntimeError(
            "native build provenance does not match expected commit: "
            f"native={native_info['native_build_commit']} expected={expected_native_commit}"
        )
    host_info = _host_fingerprint(native_info)
    comparison_metadata = _comparison_metadata(expected_native_commit, args.baseline)

    polyphonies = _parse_polyphony(args.polyphony)
    if args.scenario in {"mixed", "coalesced"} and any(
        polyphony < 2 for polyphony in polyphonies
    ):
        raise SystemExit(
            "mixed/coalesced scenarios require every --polyphony value to be >= 2; "
            "polyphony=1 would silently change packet size"
        )
    benchmark_config = _benchmark_config(
        args=args,
        polyphonies=polyphonies,
        mock_base_latency_us=mock_base_latency_us,
        mock_per_key_latency_us=mock_per_key_latency_us,
    )
    benchmark_config["native_build_flavor"] = _require_native_build_flavor(args.backend)
    _assert_same_key_zero_gap_rejected(
        backend=args.backend,
        mock_base_latency_us=mock_base_latency_us,
        mock_per_key_latency_us=mock_per_key_latency_us,
        adaptive_spin=not args.no_adaptive_spin,
        rt_priority_mode=args.rt_priority_mode,
        game_fps=args.game_fps,
    )
    if args.backend == "sendinput" and command_samples:
        raise SystemExit(
            "SendInput qualification uses a production wheel; use "
            "--skip-command-samples because QPC pause instrumentation is test-support only"
        )
    run_deadline = time.monotonic() + args.budget_seconds

    def next_timeout_ms() -> int:
        remaining = run_deadline - time.monotonic() - 5.0
        if remaining <= 0:
            raise RuntimeError("native acceptance budget expired before cleanup reserve")
        return max(1_000, min(60_000, math.ceil(remaining * 1_000)))

    successful_suites: list[dict[str, Any]] = []
    suite_results: list[BenchmarkRunResult] = []
    failures: list[dict[str, Any]] = []
    for run_index in range(dispatch_repeats):
        suite_runs: dict[str, dict[str, Any]] = {}
        current_polyphony = polyphonies[0]
        current_actions = _actions(
            args.actions + args.warmup_cycles,
            current_polyphony,
            gap_profile=args.gap_profile,
            game_fps=args.game_fps,
            start_delay_us=args.start_delay_us,
            scenario=args.scenario,
            warmup_cycles=args.warmup_cycles,
        )
        try:
            for polyphony in polyphonies:
                current_polyphony = polyphony
                current_actions = _actions(
                    args.actions + args.warmup_cycles,
                    polyphony,
                    gap_profile=args.gap_profile,
                    game_fps=args.game_fps,
                    start_delay_us=args.start_delay_us,
                    scenario=args.scenario,
                    warmup_cycles=args.warmup_cycles,
                )
                run = _run_dispatch(
                    current_actions,
                    polyphony,
                    backend=args.backend,
                    mock_base_latency_us=mock_base_latency_us,
                    mock_per_key_latency_us=mock_per_key_latency_us,
                    adaptive_spin=not args.no_adaptive_spin,
                    rt_priority_mode=args.rt_priority_mode,
                    lead_mode=lead_mode,
                    fixed_lead_us=fixed_lead_us,
                    game_fps=args.game_fps,
                    gap_profile=args.gap_profile,
                    require_focus=benchmark_config["require_focus"],
                    scenario=args.scenario,
                    warmup_cycles=args.warmup_cycles,
                    timeout_ms=next_timeout_ms(),
                    native_build_commit=expected_native_commit,
                )
                _assert_correctness(run)
                run.pop("_snapshot", None)
                run.pop("_telemetry", None)
                suite_runs[str(polyphony)] = run

            successful_suites.append({"dispatch": suite_runs})
            suite_results.append(
                BenchmarkRunResult(
                    run_index=run_index,
                    polyphony=0,
                    result={"dispatch": suite_runs},
                    failure=None,
                )
            )
        except Exception as exc:
            snapshot: dict[str, Any] | None = None
            telemetry: dict[str, Any] | None = None
            diagnostics: dict[str, Any] | None = None
            original: BaseException = exc
            if isinstance(exc, BenchmarkRunFailure):
                snapshot = exc.snapshot
                telemetry = exc.telemetry
                diagnostics = exc.diagnostics
                original = exc.original
            elif suite_runs:
                last_run = suite_runs.get(str(current_polyphony))
                if last_run is not None:
                    snapshot = last_run.get("_snapshot")
                    telemetry = last_run.get("_telemetry")
                    diagnostics = last_run.get("_telemetry_integrity")
            artifact_path = _failed_run_artifact_path(args.output, run_index)
            _write_failed_run_artifact(
                artifact_path,
                git_info=git_info,
                native_info=native_info,
                host_info=host_info,
                run_index=run_index,
                polyphony=current_polyphony,
                actions=current_actions,
                snapshot=snapshot,
                telemetry=telemetry,
                diagnostics=diagnostics,
                exception=original,
            )
            failure = {
                "run_index": run_index,
                "polyphony": current_polyphony,
                "artifact": str(artifact_path),
                "error": f"{type(original).__name__}: {original}",
                "validation_diagnostics": diagnostics,
            }
            failures.append(failure)
            suite_results.append(
                BenchmarkRunResult(
                    run_index=run_index,
                    polyphony=current_polyphony,
                    result=None,
                    failure=failure,
                )
            )
            if not args.continue_after_failure:
                break

    command_runs: list[dict[str, int]] = []
    command_failures: list[dict[str, Any]] = []
    command_actions = _command_interrupt_actions()
    command_polyphony = _command_interrupt_polyphony(command_actions)
    for sample_index in range(command_samples):
        try:
            command_runs.append(
                _measure_command_interrupt(
                    backend=args.backend,
                    mock_base_latency_us=mock_base_latency_us,
                    mock_per_key_latency_us=mock_per_key_latency_us,
                    adaptive_spin=not args.no_adaptive_spin,
                    rt_priority_mode=args.rt_priority_mode,
                    lead_mode=lead_mode,
                    fixed_lead_us=fixed_lead_us,
                    game_fps=args.game_fps,
                    require_focus=benchmark_config["require_focus"],
                )
            )
        except Exception as exc:
            artifact_path = _failed_run_artifact_path(
                args.output, dispatch_repeats + sample_index
            )
            _write_failed_run_artifact(
                artifact_path,
                git_info=git_info,
                native_info=native_info,
                host_info=host_info,
                run_index=dispatch_repeats + sample_index,
                polyphony=command_polyphony,
                actions=command_actions,
                snapshot=None,
                telemetry=None,
                diagnostics=None,
                exception=exc,
            )
            command_failures.append(
                {
                    "sample_index": sample_index,
                    "artifact": str(artifact_path),
                    "error": f"{type(exc).__name__}: {exc}",
                }
            )

    validity = _run_validity_summary(dispatch_repeats, suite_results)
    if failures or command_failures:
        invalid_report: dict[str, Any] = {
            "benchmark_schema_version": BENCHMARK_SCHEMA_VERSION,
            **comparison_metadata,
            "command_timing_domain": COMMAND_TIMING_DOMAIN,
            "latency_segment_domain": LATENCY_SEGMENT_DOMAIN,
            "label": args.label,
            "backend": args.backend,
            "native_build_flavor": benchmark_config["native_build_flavor"],
            "actions_per_polyphony": _actions_per_polyphony(
                actions=args.actions,
                scenario=args.scenario,
            ),
            "polyphony": polyphonies,
            "dispatch_repeats": dispatch_repeats,
            "command_samples": command_samples,
            "warmup_cycles": args.warmup_cycles,
            "budget_seconds": args.budget_seconds,
            "benchmark_config": benchmark_config,
            **validity,
            "requested_dispatch_suites": dispatch_repeats,
            "successful_dispatch_suites": len(successful_suites),
            "failed_dispatch_suites": len(failures),
            "requested_command_samples": command_samples,
            "successful_command_samples": len(command_runs),
            "failed_command_samples": len(command_failures),
            "unattempted_dispatch_suites": dispatch_repeats - len(suite_results),
            "statistics_eligible": False,
            "acceptance_clean": False,
            "acceptance_failure_reasons": ["failed_run"],
            "excluded_runs": 0,
            "failures": failures + command_failures,
            "mock_latency_model": {
                "base_us": mock_base_latency_us,
                "per_key_us": mock_per_key_latency_us,
            }
            if args.backend == "mock"
            else None,
            "git_sha": git_info["git_sha"],
            "native_build_commit": native_info["native_build_commit"],
            "expected_native_build_commit": expected_native_commit,
            "harness_git_sha": git_info["git_sha"],
            "candidate_or_baseline_role": args.label,
            "rustc_version": native_info["rustc_version"],
            "native_schema_version": native_info["schema_version"],
            "host_fingerprint": host_info,
            "dirty_worktree": git_info["dirty_worktree"],
            "command_line": list(sys.argv),
        }
        encoded = json.dumps(invalid_report, indent=2)
        print(encoded)
        if args.output is not None:
            args.output.write_text(encoded + "\n", encoding="utf-8")
        return 1

    dispatch_runs = [
        run
        for suite in successful_suites
        for run in suite["dispatch"].values()
    ]
    physical_boundaries = sum(run["measurement_records"] for run in dispatch_runs)
    by_polyphony: dict[str, Any] = {}
    for polyphony in polyphonies:
        actions = _actions(
            args.actions,
            polyphony,
            gap_profile=args.gap_profile,
            game_fps=args.game_fps,
            start_delay_us=args.start_delay_us,
            scenario=args.scenario,
        )
        runs = [suite["dispatch"][str(polyphony)] for suite in successful_suites]
        poly_report = {
            "polyphony": polyphony,
            "timing_comparison_scope": (
                "correctness_only_requested_suite"
                if args.scenario in {"mixed", "coalesced"}
                else "requested_polyphony"
            ),
            "schema_version": BENCHMARK_SCHEMA_VERSION,
            "timeline_semantics_version": TIMELINE_SEMANTICS_VERSION,
            "candidate_sha": comparison_metadata["candidate_sha"],
            "reference_sha": comparison_metadata["reference_sha"],
            "comparison_role": comparison_metadata["comparison_role"],
            "fps": args.game_fps,
            "latency_class": args.gap_profile,
            "priority_mode": benchmark_config["rt_priority_mode"],
            "lead_mode": benchmark_config["lead_mode"],
            "actions": len(actions),
            "warmup_cycles": args.warmup_cycles,
            "warmup_records": _aggregate_warmup_records(runs),
            "measurement_records": sum(run["measurement_records"] for run in runs),
            "physical_boundaries": sum(run["measurement_records"] for run in runs),
            "native_packet_size_counts": _aggregate_native_packet_size_counts(runs),
            "wake_error_us": _aggregate_metric(runs, "wake_error_us"),
            "pre_send_software_latency_us": _aggregate_metric(
                runs, "pre_send_software_latency_us"
            ),
            "pre_call_lateness_us": _aggregate_metric(
                runs, "pre_call_lateness_us"
            ),
            "sendinput_call_duration_us": _aggregate_metric(
                runs, "sendinput_call_duration_us"
            ),
            "core_post_send_duration_us": _aggregate_metric(runs, "core_post_send_duration_us"),
            "sender_completion_error_us": _aggregate_metric(
                runs, "sender_completion_error_us"
            ),
            "wake_error": _aggregate_metric(runs, "wake_error_us"),
            "pre_send": _aggregate_metric(runs, "pre_send_software_latency_us"),
            "sendinput": _aggregate_metric(runs, "sendinput_call_duration_us"),
            "core_post_send": _aggregate_metric(runs, "core_post_send_duration_us"),
            "observer": _stats([run["observer_duration_max_us"] for run in runs]),
            "sender_completion_error": _aggregate_metric(
                runs, "sender_completion_error_us"
            ),
            "missed_down_boundaries": _aggregate_scalar_sum(
                runs, "missed_down_boundaries"
            ),
            "missed_backlog_boundaries": _aggregate_scalar_sum(
                runs, "missed_backlog_boundaries"
            ),
            "missed_hard_late_boundaries": _aggregate_scalar_sum(
                runs, "missed_hard_late_boundaries"
            ),
            "hold_pair_samples": _aggregate_scalar_sum(runs, "hold_pair_samples"),
            "expected_hold_pair_samples": _aggregate_scalar_sum(
                runs, "expected_hold_pair_samples"
            ),
            "min_pre_call_hold_us": _aggregate_scalar_min_nonzero(
                runs, "min_pre_call_hold_us"
            ),
            "min_completion_hold_us": _aggregate_scalar_min_nonzero(
                runs, "min_completion_hold_us"
            ),
            "max_pre_call_hold_shrink_us": _aggregate_scalar_max(
                runs, "max_pre_call_hold_shrink_us"
            ),
            "max_completion_hold_shrink_us": _aggregate_scalar_max(
                runs, "max_completion_hold_shrink_us"
            ),
            "pre_call_hold_shrink_over_grace_count": _aggregate_scalar_sum(
                runs, "pre_call_hold_shrink_over_grace_count"
            ),
            "hold_unmatched_up_count": _aggregate_scalar_sum(
                runs, "hold_unmatched_up_count"
            ),
            "hold_anchor_overwrite_count": _aggregate_scalar_sum(
                runs, "hold_anchor_overwrite_count"
            ),
            "same_call_retrigger_boundaries": _aggregate_scalar_sum(
                runs, "same_call_retrigger_boundaries"
            ),
            "same_call_retrigger_keys": _aggregate_scalar_sum(
                runs, "same_call_retrigger_keys"
            ),
            **{
                name: _aggregate_scalar_sum(runs, name)
                for name in PRODUCTION_CORRECTNESS_COUNTERS
            },
            "startup_latency_us": _stats([run["startup_latency_us"] for run in runs]),
            "spin_cpu_time_us": _stats([run["spin_cpu_time_us"] for run in runs]),
            "worker_cpu_time_us": _stats([run["worker_cpu_time_us"] for run in runs]),
            "process_cpu_time_us": _stats([run["process_cpu_time_us"] for run in runs]),
            "playback_wall_time_us": _stats([run["playback_wall_time_us"] for run in runs]),
            "spin_duty_cycle_ppm": _stats([run["spin_duty_cycle_ppm"] for run in runs]),
            "worker_cpu_ratio_ppm": _stats([run["worker_cpu_ratio_ppm"] for run in runs]),
            "process_cpu_ratio_ppm": _stats([run["process_cpu_ratio_ppm"] for run in runs]),
            "spin_cpu_ratio_ppm": _stats([run["spin_cpu_ratio_ppm"] for run in runs]),
            "peak_rss_bytes": _required_stats(
                [run["peak_rss_bytes"] for run in runs if run["peak_rss_bytes"] is not None],
                "peak_rss_bytes",
            ),
            "keys_dropped": sum(run["keys_dropped"] for run in runs),
            "failed_release_count": sum(run["failed_release_count"] for run in runs),
            "chord_split_events": sum(run["chord_split_events"] for run in runs),
            "chords_rejected": sum(run["chords_rejected"] for run in runs),
            "authored_keys_rejected": sum(run["authored_keys_rejected"] for run in runs),
            "sendinput_partial_events": sum(
                run["sendinput_partial_events"] for run in runs
            ),
            "sendinput_zero_progress_failures": sum(
                run["sendinput_zero_progress_failures"] for run in runs
            ),
            "sendinput_path_degraded": any(run["sendinput_path_degraded"] for run in runs),
            "core_post_send_degraded": any(run["core_post_send_degraded"] for run in runs),
            "wait_path_degraded": any(run["wait_path_degraded"] for run in runs),
            "sendinput_warn_threshold_us": _stats(
                [run["sendinput_warn_threshold_us"] for run in runs]
            ),
            "core_post_send_warn_threshold_us": _stats(
                [run["core_post_send_warn_threshold_us"] for run in runs]
            ),
            "wait_warn_threshold_us": _stats(
                [run["wait_warn_threshold_us"] for run in runs]
            ),
            "sendinput_degraded_samples": sum(
                run["sendinput_degraded_samples"] for run in runs
            ),
            "core_post_send_degraded_samples": sum(
                run["core_post_send_degraded_samples"] for run in runs
            ),
            "wait_degraded_samples": sum(run["wait_degraded_samples"] for run in runs),
            "positive_residual_at_cap": sum(
                run["positive_residual_at_cap"] for run in runs
            ),
            "lead_by_polyphony": {
                key: max(int(run["lead_by_polyphony"].get(key, 0)) for run in runs)
                for key in {str(polyphony)}
            },
            "outcomes": sorted({run["outcome"] for run in runs}),
            "correctness": _aggregate_correctness(runs),
            "guards": {
                "fixed_hot_60_wake_p99_us": ABSOLUTE_WAKE_P99_LIMIT_US,
                "rt_priority_mode": benchmark_config["rt_priority_mode"],
                "waitable_timer": True,
                "event_wait": True,
                "correctness_checked_before_percentiles": True,
            },
        }
        if args.scenario in {"mixed", "coalesced"}:
            for field in MIXED_POLY_TIMING_FIELDS:
                poly_report.pop(field, None)
        by_polyphony[str(polyphony)] = poly_report
    report: dict[str, Any] = {
        "label": args.label,
        "schema_version": BENCHMARK_SCHEMA_VERSION,
        "backend": args.backend,
        "native_build_flavor": benchmark_config["native_build_flavor"],
        "actions_per_polyphony": _actions_per_polyphony(
            actions=args.actions,
            scenario=args.scenario,
        ),
        "polyphony": polyphonies,
        "dispatch_repeats": dispatch_repeats,
        "command_samples": command_samples,
        "budget_seconds": args.budget_seconds,
        "benchmark_schema_version": BENCHMARK_SCHEMA_VERSION,
        **comparison_metadata,
        "command_timing_domain": COMMAND_TIMING_DOMAIN,
        "latency_segment_domain": LATENCY_SEGMENT_DOMAIN,
        "benchmark_config": benchmark_config,
        "timing_comparison_scope": (
            "aggregate_actual_packets"
            if args.scenario in {"mixed", "coalesced"}
            else "requested_polyphony"
        ),
        **validity,
        "requested_dispatch_suites": dispatch_repeats,
        "successful_dispatch_suites": len(successful_suites),
        "failed_dispatch_suites": 0,
        "requested_command_samples": command_samples,
        "successful_command_samples": len(command_runs),
        "failed_command_samples": 0,
        "statistics_eligible": False,
        "excluded_runs": 0,
        "failures": [],
        "warmup_cycles": args.warmup_cycles,
        "warmup_records": _aggregate_warmup_records(dispatch_runs),
        "measurement_records": physical_boundaries,
        "physical_boundaries": physical_boundaries,
        "qualification_gate": {
            "minimum_physical_boundaries": MIN_QUALIFICATION_PHYSICAL_BOUNDARIES,
            "measured_physical_boundaries": physical_boundaries,
            "passed": _qualification_boundary_gate(
                backend=args.backend, measured_boundaries=physical_boundaries
            ),
        },
        "wake_error_us": _aggregate_metric(dispatch_runs, "wake_error_us"),
        "pre_send_software_latency_us": _aggregate_metric(
            dispatch_runs, "pre_send_software_latency_us"
        ),
        "pre_call_lateness_us": _aggregate_metric(
            dispatch_runs, "pre_call_lateness_us"
        ),
        "sendinput_call_duration_us": _aggregate_metric(
            dispatch_runs, "sendinput_call_duration_us"
        ),
        "core_post_send_duration_us": _aggregate_metric(
            dispatch_runs, "core_post_send_duration_us"
        ),
        "sender_completion_error_us": _aggregate_metric(
            dispatch_runs, "sender_completion_error_us"
        ),
        "wake_error": _aggregate_metric(dispatch_runs, "wake_error_us"),
        "pre_send": _aggregate_metric(
            dispatch_runs, "pre_send_software_latency_us"
        ),
        "sendinput": _aggregate_metric(dispatch_runs, "sendinput_call_duration_us"),
        "core_post_send": _aggregate_metric(
            dispatch_runs, "core_post_send_duration_us"
        ),
        "observer": _stats(
            [run["observer_duration_max_us"] for run in dispatch_runs]
        ),
        "sender_completion_error": _aggregate_metric(
            dispatch_runs, "sender_completion_error_us"
        ),
        **{
            name: _aggregate_scalar_sum(dispatch_runs, name)
            for name in PRODUCTION_CORRECTNESS_COUNTERS
        },
        "correctness": _aggregate_correctness(dispatch_runs),
        "deadline_missed_before_send_count": sum(
            run["missed_down_boundaries"] for run in dispatch_runs
        ),
        "missed_down_boundaries": _aggregate_scalar_sum(
            dispatch_runs, "missed_down_boundaries"
        ),
        "missed_backlog_boundaries": _aggregate_scalar_sum(
            dispatch_runs, "missed_backlog_boundaries"
        ),
        "missed_hard_late_boundaries": _aggregate_scalar_sum(
            dispatch_runs, "missed_hard_late_boundaries"
        ),
        "hold_pair_samples": _aggregate_scalar_sum(dispatch_runs, "hold_pair_samples"),
        "expected_hold_pair_samples": _aggregate_scalar_sum(
            dispatch_runs, "expected_hold_pair_samples"
        ),
        "min_pre_call_hold_us": _aggregate_scalar_min_nonzero(
            dispatch_runs, "min_pre_call_hold_us"
        ),
        "min_completion_hold_us": _aggregate_scalar_min_nonzero(
            dispatch_runs, "min_completion_hold_us"
        ),
        "max_pre_call_hold_shrink_us": _aggregate_scalar_max(
            dispatch_runs, "max_pre_call_hold_shrink_us"
        ),
        "max_completion_hold_shrink_us": _aggregate_scalar_max(
            dispatch_runs, "max_completion_hold_shrink_us"
        ),
        "pre_call_hold_shrink_over_grace_count": _aggregate_scalar_sum(
            dispatch_runs, "pre_call_hold_shrink_over_grace_count"
        ),
        "hold_unmatched_up_count": _aggregate_scalar_sum(
            dispatch_runs, "hold_unmatched_up_count"
        ),
        "hold_anchor_overwrite_count": _aggregate_scalar_sum(
            dispatch_runs, "hold_anchor_overwrite_count"
        ),
        "same_call_retrigger_boundaries": _aggregate_scalar_sum(
            dispatch_runs, "same_call_retrigger_boundaries"
        ),
        "same_call_retrigger_keys": _aggregate_scalar_sum(
            dispatch_runs, "same_call_retrigger_keys"
        ),
        "non_dispatch_count": sum(
            sum(
                int(run["generation_status_counts"].get(name, 0))
                for name in ("dropped_conflict", "dropped_expired", "dropped_backend")
            )
            for run in dispatch_runs
        ),
        "observer_dropped_records": sum(
            run["observer_dropped_records"] for run in dispatch_runs
        ),
        "guards": {
            "fixed_hot_60_wake_p99_us": ABSOLUTE_WAKE_P99_LIMIT_US,
            "rt_priority_mode": benchmark_config["rt_priority_mode"],
            "waitable_timer": True,
            "event_wait": True,
            "correctness_checked_before_percentiles": True,
        },
        "startup_latency_us": _stats(
            [run["startup_latency_us"] for run in dispatch_runs]
        ),
        "spin_cpu_time_us": _stats([run["spin_cpu_time_us"] for run in dispatch_runs]),
        "peak_rss_bytes": _required_stats(
            [run["peak_rss_bytes"] for run in dispatch_runs if run["peak_rss_bytes"] is not None],
            "peak_rss_bytes",
        ),
        "command_observation_latency_us": _stats(
            [run["observation_latency_us"] for run in command_runs]
        ),
        "command_completion_latency_us": _stats(
            [run["completion_latency_us"] for run in command_runs]
        ),
        "command_cleanup_cost_us": _stats(
            [run["cleanup_cost_us"] for run in command_runs]
        ),
        "worker_cpu_ratio_ppm": _stats(
            [run["worker_cpu_ratio_ppm"] for run in dispatch_runs]
        ),
        "process_cpu_ratio_ppm": _stats(
            [run["process_cpu_ratio_ppm"] for run in dispatch_runs]
        ),
        "spin_cpu_ratio_ppm": _stats(
            [run["spin_cpu_ratio_ppm"] for run in dispatch_runs]
        ),
        "keys_dropped": sum(run["keys_dropped"] for run in dispatch_runs),
        "failed_release_count": sum(run["failed_release_count"] for run in dispatch_runs),
        "chord_split_events": sum(run["chord_split_events"] for run in dispatch_runs),
        "chords_rejected": sum(run["chords_rejected"] for run in dispatch_runs),
        "authored_keys_rejected": sum(
            run["authored_keys_rejected"] for run in dispatch_runs
        ),
        "sendinput_partial_events": sum(
            run["sendinput_partial_events"] for run in dispatch_runs
        ),
        "sendinput_zero_progress_failures": sum(
            run["sendinput_zero_progress_failures"] for run in dispatch_runs
        ),
        "sendinput_path_degraded": any(
            run["sendinput_path_degraded"] for run in dispatch_runs
        ),
        "core_post_send_degraded": any(run["core_post_send_degraded"] for run in dispatch_runs),
        "wait_path_degraded": any(run["wait_path_degraded"] for run in dispatch_runs),
        "sendinput_warn_threshold_us": _stats(
            [run["sendinput_warn_threshold_us"] for run in dispatch_runs]
        ),
        "core_post_send_warn_threshold_us": _stats(
            [run["core_post_send_warn_threshold_us"] for run in dispatch_runs]
        ),
        "wait_warn_threshold_us": _stats(
            [run["wait_warn_threshold_us"] for run in dispatch_runs]
        ),
        "sendinput_degraded_samples": sum(
            run["sendinput_degraded_samples"] for run in dispatch_runs
        ),
        "core_post_send_degraded_samples": sum(
            run["core_post_send_degraded_samples"] for run in dispatch_runs
        ),
        "wait_degraded_samples": sum(
            run["wait_degraded_samples"] for run in dispatch_runs
        ),
        "positive_residual_at_cap": sum(
            run["positive_residual_at_cap"] for run in dispatch_runs
        ),
        "outcomes": sorted({run["outcome"] for run in dispatch_runs}),
        "mock_latency_model": {
            "base_us": mock_base_latency_us,
            "per_key_us": mock_per_key_latency_us,
        }
        if args.backend == "mock"
        else None,
        "by_polyphony": by_polyphony,
        "native_packet_size_counts": _aggregate_native_packet_size_counts(dispatch_runs),
        "evidence_scope": (
            "sender_pre_call" if args.backend == "sendinput" else "sender_completion"
        ),
        "git_sha": git_info["git_sha"],
        "native_build_commit": native_info["native_build_commit"],
        "expected_native_build_commit": expected_native_commit,
        "harness_git_sha": git_info["git_sha"],
        "candidate_or_baseline_role": args.label,
        "rustc_version": native_info["rustc_version"],
        "native_schema_version": native_info["schema_version"],
        "backend_evidence": "real_sendinput_strict_diagnostic_pre_call"
        if args.backend == "sendinput"
        else "deterministic_coordinator_delivery_simulation",
        "host_fingerprint": host_info,
        "dirty_worktree": git_info["dirty_worktree"],
        "command_line": list(sys.argv),
    }
    acceptance_failure_reasons = _acceptance_failure_reasons(report)
    report["acceptance_clean"] = not acceptance_failure_reasons
    report["acceptance_failure_reasons"] = acceptance_failure_reasons
    report["statistics_eligible"] = (
        report["acceptance_clean"]
        and _qualification_boundary_gate(
            backend=args.backend, measured_boundaries=physical_boundaries
        )
    )
    encoded = json.dumps(report, indent=2)
    print(encoded)
    if args.output is not None:
        args.output.write_text(encoded + "\n", encoding="utf-8")
    if not report["acceptance_clean"] or not report["statistics_eligible"]:
        return 1
    _assert_report_correctness(report)
    _assert_absolute_wake_slo(report)
    _assert_absolute_pre_call_slo(report)
    if args.baseline is not None:
        _assert_baseline(report, args.baseline)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
