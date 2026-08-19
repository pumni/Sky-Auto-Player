"""Strict loader for the paired total-hold calibration cache.

The cache is host-side target-to-receipt evidence for an app-owned
``WM_INPUT`` path.  It is not game, rendering, audio, or network latency.
Legacy independent Down/Up evidence is intentionally rejected rather than
reinterpreted as paired total-hold evidence.
"""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import cast

DEFAULT_CACHE_FILENAME: str = ".cache/input_latency.json"
SOURCE_DEVICE_CACHE: str = "device_cache"
SOURCE_DEFAULT_500: str = "default_500"
SOURCE_OUT_OF_ENVELOPE_DEFAULT_500: str = "out_of_envelope_default_500"
SOURCE_INVALID_CACHE_DEFAULT_500: str = "invalid_cache_default_500"
SOURCE_INCOMPATIBLE_HOST_DEFAULT_500: str = "incompatible_host_default_500"
SUPPORTED_CACHE_VERSION: int = 5
LEGACY_CACHE_VERSION: int = 3
PREVIOUS_CACHE_VERSION: int = 4
SUPPORTED_NATIVE_CALIBRATION_VERSION: int = 11
SUPPORTED_MEASUREMENT_PROTOCOL_VERSION: int = 5
SOURCE_FORMULA_VERSION: int = 4
LEGACY_SOURCE_FORMULA_VERSION: int = 3
HOST_FINGERPRINT_VERSION: int = 2
CALIBRATION_ARTIFACT_SCHEMA_VERSION: int = 8
CALIBRATION_EVIDENCE_KIND: str = "injected_raw_input_total_hold_proxy"
MIN_CALIBRATION_SAMPLE_COUNT: int = 100
MAX_SHRINK_US: int = 100_000
MARGIN_GUARD_US: int = 100
MARGIN_FLOOR_US: int = 300
# This is a qualification ceiling, not a saturation/clamp target.
MARGIN_CEILING_US: int = 2_000
REQUIRED_BUCKETS: tuple[str, ...] = (
    "1/hot",
    "1/cold",
    "5/hot",
    "5/cold",
    "15/hot",
    "15/cold",
)


class CalibrationStatus(StrEnum):
    VALID = "valid"
    OUT_OF_ENVELOPE = "out_of_envelope"
    UNCALIBRATED = "uncalibrated"
    INVALID_CACHE = "invalid_cache"
    INCOMPATIBLE = "incompatible"


@dataclass(frozen=True, slots=True)
class CalibrationQualification:
    status: CalibrationStatus
    candidate_margin_us: int
    applied_margin_us: int | None


def qualify_calibration_margin(
    global_shrink_p99_us: int,
) -> CalibrationQualification:
    """Apply the one authoritative host-delivery qualification formula."""

    if not isinstance(global_shrink_p99_us, int) or isinstance(
        global_shrink_p99_us, bool
    ):
        raise TypeError("global_shrink_p99_us must be an integer")
    positive_p99 = max(0, global_shrink_p99_us)
    candidate = positive_p99 + MARGIN_GUARD_US
    if candidate > MARGIN_CEILING_US:
        return CalibrationQualification(
            status=CalibrationStatus.OUT_OF_ENVELOPE,
            candidate_margin_us=candidate,
            applied_margin_us=None,
        )
    return CalibrationQualification(
        status=CalibrationStatus.VALID,
        candidate_margin_us=candidate,
        applied_margin_us=max(MARGIN_FLOOR_US, candidate),
    )


@dataclass(frozen=True, slots=True)
class CalibrationQuantiles:
    """Three diagnostic quantiles retained for compatibility/UI details."""

    p50: int
    p90: int
    p99: int


@dataclass(frozen=True, slots=True)
class SignedQuantiles:
    min: int
    p50: int
    p90: int
    p95: int
    p99: int
    max: int
    mean: int


@dataclass(frozen=True, slots=True)
class PairBucketSummary:
    attempted: int
    clean_pair_count: int
    rejected: int
    pair_worst_total_proxy_shrink_us: SignedQuantiles
    scheduler_shrink_us: SignedQuantiles
    sendinput_shrink_us: SignedQuantiles
    delivery_shrink_us: SignedQuantiles
    # Legacy delivery-only diagnostics are retained for display/audit only.
    pair_worst_shrink_us: SignedQuantiles | None = None


@dataclass(frozen=True, slots=True)
class CalibrationCacheSummary:
    status: CalibrationStatus
    margin_us: int | None
    source: str
    sample_count: int
    pair_buckets: dict[str, PairBucketSummary]
    worst_bucket: str
    global_shrink_p99_us: int
    guard_us: int
    floor_us: int
    ceiling_us: int
    candidate_margin_us: int
    host_fingerprint: dict[str, object]
    scheduling_aids: dict[str, object]
    down_us: CalibrationQuantiles | None = None
    up_us: CalibrationQuantiles | None = None


@dataclass(frozen=True, slots=True)
class CalibrationLoadResult:
    status: CalibrationStatus
    resolved_margin_us: int
    margin_source: str
    summary: CalibrationCacheSummary | None


def _int(value: object, name: str, *, minimum: int | None = None) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError(f"{name} must be an integer")
    if minimum is not None and value < minimum:
        raise ValueError(f"{name} must be >= {minimum}")
    return value


def _quantiles(value: object, name: str) -> SignedQuantiles:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    values = {
        field: _int(value.get(field), f"{name}.{field}")
        for field in ("min", "p50", "p90", "p95", "p99", "max", "mean")
    }
    ordered = [values[field] for field in ("min", "p50", "p90", "p95", "p99", "max")]
    if ordered != sorted(ordered):
        raise ValueError(f"{name} quantiles are not ordered")
    if any(abs(item) > MAX_SHRINK_US for item in ordered):
        raise ValueError(f"{name} exceeds the signed evidence bound")
    return SignedQuantiles(**values)


def _legacy_v2_selected_margin(global_p99: int) -> int:
    """Validate only the selected value serialized by a legacy v2 cache."""

    return max(
        MARGIN_FLOOR_US,
        min(MARGIN_CEILING_US, global_p99 + MARGIN_GUARD_US),
    )


def _validate_host_fingerprint(value: object, *, name: str = "host_fingerprint") -> dict[str, object]:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    if value.get("host_fingerprint_version") != HOST_FINGERPRINT_VERSION:
        raise ValueError(f"{name}.host_fingerprint_version is unsupported")
    _int(value.get("qpc_frequency_hz"), f"{name}.qpc_frequency_hz", minimum=1)
    for field in ("win32_build", "processor_architecture", "cpu_vendor"):
        field_value = value.get(field)
        if not isinstance(field_value, str) or not field_value.strip():
            raise ValueError(f"{name}.{field} is required")
    for field in (
        "cpu_family",
        "cpu_model",
        "cpu_stepping",
        "logical_processor_count",
        "processor_group_count",
    ):
        _int(value.get(field), f"{name}.{field}", minimum=0)
    efficiency = value.get("cpu_set_efficiency_classes")
    if not isinstance(efficiency, list) or any(
        not isinstance(item, int) or isinstance(item, bool) or item < 0 for item in efficiency
    ):
        raise ValueError(f"{name}.cpu_set_efficiency_classes is invalid")
    for field in ("highest_efficiency_class", "lowest_efficiency_class"):
        field_value = value.get(field)
        if field_value is not None:
            _int(field_value, f"{name}.{field}", minimum=0)
    sampled_at = value.get("sampled_at_us")
    if sampled_at is not None:
        _int(sampled_at, f"{name}.sampled_at_us", minimum=0)
    return value


def _validate_scheduling_aids(
    value: object, *, name: str = "scheduling_aids"
) -> dict[str, object]:
    if not isinstance(value, dict):
        raise TypeError(f"{name} must be an object")
    mmcss = value.get("mmcss_acquired")
    if not isinstance(mmcss, str) or mmcss not in {
        "off",
        "mmcss:Games",
        "thread:highest",
        "thread:time_critical",
    }:
        raise ValueError(f"{name}.mmcss_acquired is invalid")
    mmcss_active = value.get("mmcss_active")
    if not isinstance(mmcss_active, bool) or mmcss_active != (mmcss != "off"):
        raise ValueError(f"{name}.mmcss_active is inconsistent")
    power_active = value.get("power_throttling_active")
    if not isinstance(power_active, bool):
        raise ValueError(f"{name}.power_throttling_active is invalid")
    waiter_mode = value.get("waiter_mode")
    if not isinstance(waiter_mode, str) or waiter_mode not in {
        "event+high_resolution_timer",
        "high_resolution_timer",
        "event+timer_resolution_fallback",
        "timer_resolution_fallback",
    }:
        raise ValueError(f"{name}.waiter_mode is invalid")
    return value


def _validate_publishable_scheduling_aids(
    value: object, *, name: str = "scheduling_aids"
) -> dict[str, object]:
    scheduling_aids = _validate_scheduling_aids(value, name=name)
    if scheduling_aids["waiter_mode"] != "event+high_resolution_timer":
        raise ValueError(
            f"{name}.waiter_mode is not publishable: "
            "production calibration requires event+high_resolution_timer"
        )
    if scheduling_aids["mmcss_acquired"] not in {
        "mmcss:Games",
        "thread:highest",
        "off",
    }:
        raise ValueError(f"{name}.mmcss_acquired is not publishable")
    return scheduling_aids


def _host_identity(value: dict[str, object]) -> tuple[object, ...]:
    efficiency = cast(list[int], value["cpu_set_efficiency_classes"])
    return (
        value["host_fingerprint_version"],
        value["qpc_frequency_hz"],
        value["win32_build"],
        value["processor_architecture"],
        value["cpu_vendor"],
        value["cpu_family"],
        value["cpu_model"],
        value["cpu_stepping"],
        value["logical_processor_count"],
        value["processor_group_count"],
        tuple(efficiency),
        value["highest_efficiency_class"],
        value["lowest_efficiency_class"],
    )


def _current_host_fingerprint() -> dict[str, object] | None:
    """Read the current native host identity without a Python Win32 seam."""

    if sys.platform != "win32":
        return None
    try:
        import sky_player_rs  # type: ignore[import-not-found]

        provider = getattr(sky_player_rs, "host_timing_fingerprint_json", None)
        if not callable(provider):
            return None
        raw = provider()
        if not isinstance(raw, str):
            return None
        return _validate_host_fingerprint(json.loads(raw), name="current_host_fingerprint")
    except (ImportError, OSError, TypeError, ValueError, json.JSONDecodeError):
        return None


def _host_matches_current(value: dict[str, object]) -> bool:
    current = _current_host_fingerprint()
    return current is not None and _host_identity(value) == _host_identity(current)


def _validate_provenance(data: dict[str, object], *, require_extended: bool) -> None:
    fields = ["source_git_sha", "native_build_id"]
    if require_extended:
        fields.extend(("native_source_fingerprint", "rustc_version"))
    for field in fields:
        value = data.get(field)
        if not isinstance(value, str) or not value.strip() or value == "unknown":
            raise ValueError(f"invalid calibration provenance field: {field}")
    if data["source_git_sha"] != data["native_build_id"]:
        raise ValueError("calibration source/build provenance mismatch")
    if data.get("dirty_worktree") is not False:
        raise ValueError("calibration provenance is dirty")
    _validate_host_fingerprint(data.get("host_fingerprint"))
    _validate_scheduling_aids(data.get("scheduling_aids"))


def _parse_pair_buckets(data: dict[str, object]) -> dict[str, PairBucketSummary]:
    required = data.get("required_buckets")
    if not isinstance(required, list) or tuple(required) != REQUIRED_BUCKETS:
        raise ValueError("calibration required bucket matrix is incomplete")
    raw_buckets = data.get("pair_buckets")
    if not isinstance(raw_buckets, dict) or set(raw_buckets) != set(REQUIRED_BUCKETS):
        raise ValueError("calibration pair bucket matrix is incomplete")

    pair_buckets: dict[str, PairBucketSummary] = {}
    for key in REQUIRED_BUCKETS:
        bucket = raw_buckets.get(key)
        if not isinstance(bucket, dict):
            raise TypeError(f"pair bucket {key} must be an object")
        attempted = _int(bucket.get("attempted"), f"{key}.attempted", minimum=0)
        clean = _int(bucket.get("clean_pair_count"), f"{key}.clean_pair_count", minimum=0)
        rejected = _int(bucket.get("rejected"), f"{key}.rejected", minimum=0)
        if clean < MIN_CALIBRATION_SAMPLE_COUNT:
            raise ValueError(
                f"pair bucket {key} has only {clean} clean pairs; "
                f"at least {MIN_CALIBRATION_SAMPLE_COUNT} are required"
            )
        if clean > attempted or rejected != attempted - clean:
            raise ValueError(f"pair bucket {key} counts are inconsistent")
        pair_buckets[key] = PairBucketSummary(
            attempted=attempted,
            clean_pair_count=clean,
            rejected=rejected,
            pair_worst_total_proxy_shrink_us=_quantiles(
                bucket.get("pair_worst_total_proxy_shrink_us"),
                f"{key}.pair_worst_total_proxy_shrink_us",
            ),
            scheduler_shrink_us=_quantiles(
                bucket.get("scheduler_shrink_us"), f"{key}.scheduler_shrink_us"
            ),
            sendinput_shrink_us=_quantiles(
                bucket.get("sendinput_shrink_us"), f"{key}.sendinput_shrink_us"
            ),
            delivery_shrink_us=_quantiles(
                bucket.get("delivery_shrink_us"), f"{key}.delivery_shrink_us"
            ),
            pair_worst_shrink_us=_quantiles(
                bucket.get("pair_worst_shrink_us"), f"{key}.pair_worst_shrink_us"
            )
            if bucket.get("pair_worst_shrink_us") is not None
            else None,
        )
    return pair_buckets


def _recompute_qualification(
    pair_buckets: dict[str, PairBucketSummary],
) -> tuple[int, str, CalibrationQualification]:
    p99_values = {
        key: max(0, bucket.pair_worst_total_proxy_shrink_us.p99)
        for key, bucket in pair_buckets.items()
    }
    global_p99 = max(0, *(p99_values[key] for key in REQUIRED_BUCKETS))
    worst_bucket = max(
        REQUIRED_BUCKETS,
        key=lambda key: (p99_values[key], -REQUIRED_BUCKETS.index(key)),
    )
    return global_p99, worst_bucket, qualify_calibration_margin(global_p99)


def _validate_policy_constants(
    payload: dict[str, object], *, name: str
) -> tuple[int, int, int]:
    guard = _int(payload.get("guard_us"), f"{name}.guard_us", minimum=0)
    floor = _int(payload.get("floor_us"), f"{name}.floor_us", minimum=0)
    ceiling = _int(payload.get("ceiling_us"), f"{name}.ceiling_us", minimum=floor)
    if (guard, floor, ceiling) != (
        MARGIN_GUARD_US,
        MARGIN_FLOOR_US,
        MARGIN_CEILING_US,
    ):
        raise ValueError("calibration policy constants do not match the correction contract")
    return guard, floor, ceiling


def _summary(
    *,
    status: CalibrationStatus,
    margin_us: int | None,
    pair_buckets: dict[str, PairBucketSummary],
    worst_bucket: str,
    global_p99: int,
    candidate_margin_us: int,
    guard: int,
    floor: int,
    ceiling: int,
    host_fingerprint: dict[str, object],
    scheduling_aids: dict[str, object],
) -> CalibrationCacheSummary:
    return CalibrationCacheSummary(
        status=status,
        margin_us=margin_us,
        source=SOURCE_DEVICE_CACHE,
        sample_count=min(bucket.clean_pair_count for bucket in pair_buckets.values()),
        pair_buckets=pair_buckets,
        worst_bucket=worst_bucket,
        global_shrink_p99_us=global_p99,
        guard_us=guard,
        floor_us=floor,
        ceiling_us=ceiling,
        candidate_margin_us=candidate_margin_us,
        host_fingerprint=host_fingerprint,
        scheduling_aids=scheduling_aids,
    )


def _parse_v5(data: dict[str, object]) -> CalibrationCacheSummary:
    if data.get("source_formula_version") != SOURCE_FORMULA_VERSION:
        raise ValueError("unsupported calibration source formula")
    _validate_provenance(data, require_extended=True)
    scheduling_aids = _validate_publishable_scheduling_aids(data.get("scheduling_aids"))
    pair_buckets = _parse_pair_buckets(data)
    global_p99, worst_bucket, qualification = _recompute_qualification(pair_buckets)

    raw = data.get("qualification")
    if not isinstance(raw, dict):
        raise TypeError("qualification must be an object")
    if raw.get("basis") != "max_required_bucket_p99_positive_pair_total_proxy_hold_shrink":
        raise ValueError("qualification basis is missing or invalid")
    if raw.get("worst_bucket") != worst_bucket:
        raise ValueError("qualification worst bucket is inconsistent")
    if raw.get("global_shrink_p99_us") != global_p99:
        raise ValueError("qualification p99 is inconsistent")
    guard, floor, ceiling = _validate_policy_constants(raw, name="qualification")
    candidate = _int(raw.get("candidate_margin_us"), "qualification.candidate_margin_us", minimum=0)
    if candidate != qualification.candidate_margin_us:
        raise ValueError("qualification candidate is inconsistent")
    serialized_status = raw.get("status", data.get("status"))
    if serialized_status != data.get("status"):
        raise ValueError("cache status and qualification status disagree")
    try:
        status = CalibrationStatus(serialized_status)
    except (TypeError, ValueError) as exc:
        raise ValueError("invalid calibration cache status") from exc
    if status not in (CalibrationStatus.VALID, CalibrationStatus.OUT_OF_ENVELOPE):
        raise ValueError("v4 cache has a non-publishable status")
    if status is not qualification.status:
        raise ValueError("qualification status is inconsistent")

    applied_value = raw.get("applied_margin_us")
    if applied_value is None:
        applied = None
    else:
        applied = _int(applied_value, "qualification.applied_margin_us", minimum=0)
    if applied != qualification.applied_margin_us:
        raise ValueError("qualification applied margin is inconsistent")

    return _summary(
        status=status,
        margin_us=applied,
        pair_buckets=pair_buckets,
        worst_bucket=worst_bucket,
        global_p99=global_p99,
        candidate_margin_us=candidate,
        guard=guard,
        floor=floor,
        ceiling=ceiling,
        host_fingerprint=_validate_host_fingerprint(data.get("host_fingerprint")),
        scheduling_aids=scheduling_aids,
    )


def parse_calibration_cache_summary(data: object) -> CalibrationCacheSummary:
    """Strictly parse the canonical vNext cache; legacy evidence is rejected."""

    if not isinstance(data, dict):
        raise TypeError("calibration cache payload must be a dict")
    version = data.get("version")
    if version != SUPPORTED_CACHE_VERSION:
        if version == LEGACY_CACHE_VERSION:
            raise ValueError("legacy calibration cache is incompatible with vNext")
        raise ValueError(f"unsupported calibration cache version: {version}")
    if data.get("evidence_kind") != CALIBRATION_EVIDENCE_KIND:
        raise ValueError("invalid calibration evidence kind")
    if data.get("artifact_schema_version") != CALIBRATION_ARTIFACT_SCHEMA_VERSION:
        raise ValueError("unsupported calibration artifact schema")
    if data.get("native_calibration_version") != SUPPORTED_NATIVE_CALIBRATION_VERSION:
        raise ValueError("unsupported native calibration schema")
    if data.get("measurement_protocol_version") != SUPPORTED_MEASUREMENT_PROTOCOL_VERSION:
        raise ValueError("unsupported measurement protocol")
    if data.get("source") != SOURCE_DEVICE_CACHE:
        raise ValueError("invalid calibration source")
    return _parse_v5(data)


def load_calibration_resolution(
    *, cache_path: Path | None = None, data: dict | None = None
) -> CalibrationLoadResult:
    """Resolve the playback hold margin and preserve cache health semantics."""

    if data is None:
        path = Path(cache_path) if cache_path is not None else Path(DEFAULT_CACHE_FILENAME)
        if not path.exists():
            return CalibrationLoadResult(
                status=CalibrationStatus.UNCALIBRATED,
                resolved_margin_us=500,
                margin_source=SOURCE_DEFAULT_500,
                summary=None,
            )
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError, TypeError):
            return CalibrationLoadResult(
                status=CalibrationStatus.INVALID_CACHE,
                resolved_margin_us=500,
                margin_source=SOURCE_INVALID_CACHE_DEFAULT_500,
                summary=None,
            )
    if isinstance(data, dict) and (
        data.get("version") in (2, LEGACY_CACHE_VERSION, PREVIOUS_CACHE_VERSION)
        or data.get("measurement_protocol_version") == 4
        or data.get("evidence_kind") == "injected_raw_input_delivery_proxy"
    ):
        return CalibrationLoadResult(
            status=CalibrationStatus.INCOMPATIBLE,
            resolved_margin_us=500,
            margin_source=SOURCE_INCOMPATIBLE_HOST_DEFAULT_500,
            summary=None,
        )
    try:
        summary = parse_calibration_cache_summary(data)
    except (TypeError, ValueError, KeyError):
        return CalibrationLoadResult(
            status=CalibrationStatus.INVALID_CACHE,
            resolved_margin_us=500,
            margin_source=SOURCE_INVALID_CACHE_DEFAULT_500,
            summary=None,
        )
    if sys.platform == "win32" and not _host_matches_current(summary.host_fingerprint):
        return CalibrationLoadResult(
            status=CalibrationStatus.INCOMPATIBLE,
            resolved_margin_us=500,
            margin_source=SOURCE_INCOMPATIBLE_HOST_DEFAULT_500,
            summary=summary,
        )
    if summary.status is CalibrationStatus.OUT_OF_ENVELOPE:
        return CalibrationLoadResult(
            status=summary.status,
            resolved_margin_us=500,
            margin_source=SOURCE_OUT_OF_ENVELOPE_DEFAULT_500,
            summary=summary,
        )
    if summary.margin_us is None:
        raise ValueError("valid calibration summary has no applied margin")
    return CalibrationLoadResult(
        status=CalibrationStatus.VALID,
        resolved_margin_us=summary.margin_us,
        margin_source=SOURCE_DEVICE_CACHE,
        summary=summary,
    )


def load_calibrated_margin_recommendation(
    *, cache_path: Path | None = None, data: dict | None = None
) -> tuple[int | None, str]:
    """Compatibility wrapper; production policy resolution uses the typed result."""

    resolution = load_calibration_resolution(cache_path=cache_path, data=data)
    if resolution.status is CalibrationStatus.VALID:
        return resolution.resolved_margin_us, SOURCE_DEVICE_CACHE
    if resolution.status is CalibrationStatus.OUT_OF_ENVELOPE:
        return None, SOURCE_OUT_OF_ENVELOPE_DEFAULT_500
    if resolution.status is CalibrationStatus.INVALID_CACHE:
        return None, SOURCE_INVALID_CACHE_DEFAULT_500
    if resolution.status is CalibrationStatus.INCOMPATIBLE:
        return None, SOURCE_INCOMPATIBLE_HOST_DEFAULT_500
    return None, SOURCE_DEFAULT_500


__all__ = [
    "CALIBRATION_ARTIFACT_SCHEMA_VERSION",
    "CALIBRATION_EVIDENCE_KIND",
    "DEFAULT_CACHE_FILENAME",
    "HOST_FINGERPRINT_VERSION",
    "LEGACY_CACHE_VERSION",
    "LEGACY_SOURCE_FORMULA_VERSION",
    "MARGIN_CEILING_US",
    "MARGIN_FLOOR_US",
    "MAX_SHRINK_US",
    "MIN_CALIBRATION_SAMPLE_COUNT",
    "REQUIRED_BUCKETS",
    "SOURCE_DEFAULT_500",
    "SOURCE_DEVICE_CACHE",
    "SOURCE_FORMULA_VERSION",
    "SOURCE_INCOMPATIBLE_HOST_DEFAULT_500",
    "SOURCE_INVALID_CACHE_DEFAULT_500",
    "SOURCE_OUT_OF_ENVELOPE_DEFAULT_500",
    "SUPPORTED_CACHE_VERSION",
    "SUPPORTED_MEASUREMENT_PROTOCOL_VERSION",
    "SUPPORTED_NATIVE_CALIBRATION_VERSION",
    "CalibrationCacheSummary",
    "CalibrationLoadResult",
    "CalibrationQualification",
    "CalibrationQuantiles",
    "CalibrationStatus",
    "PairBucketSummary",
    "load_calibrated_margin_recommendation",
    "load_calibration_resolution",
    "parse_calibration_cache_summary",
    "qualify_calibration_margin",
]
