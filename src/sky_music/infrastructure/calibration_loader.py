"""Strict loader for the paired input-delivery calibration cache.

The cache is evidence about an app-owned ``WM_INPUT`` delivery proxy.  It is
not game, rendering, audio, or network latency.  Version one contained
independent Down/Up marginals and is intentionally rejected rather than
reinterpreted as paired evidence.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path

DEFAULT_CACHE_FILENAME: str = ".cache/input_latency.json"
SOURCE_DEVICE_CACHE: str = "device_cache"
SOURCE_DEFAULT_500: str = "default_500"
SOURCE_OUT_OF_ENVELOPE_DEFAULT_500: str = "out_of_envelope_default_500"
SOURCE_INVALID_CACHE_DEFAULT_500: str = "invalid_cache_default_500"
SUPPORTED_CACHE_VERSION: int = 3
LEGACY_CACHE_VERSION: int = 2
SUPPORTED_NATIVE_CALIBRATION_VERSION: int = 9
SUPPORTED_MEASUREMENT_PROTOCOL_VERSION: int = 4
SOURCE_FORMULA_VERSION: int = 3
LEGACY_SOURCE_FORMULA_VERSION: int = 2
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
    pair_worst_shrink_us: SignedQuantiles


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
    host = data.get("host_fingerprint")
    if not isinstance(host, dict):
        raise TypeError("host_fingerprint must be an object")
    _int(host.get("qpc_frequency_hz"), "host_fingerprint.qpc_frequency_hz", minimum=1)
    if not isinstance(host.get("win32_build"), str) or not host["win32_build"].strip():
        raise ValueError("host_fingerprint.win32_build is required")


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
            pair_worst_shrink_us=_quantiles(
                bucket.get("pair_worst_shrink_us"), f"{key}.pair_worst_shrink_us"
            ),
        )
    return pair_buckets


def _recompute_qualification(
    pair_buckets: dict[str, PairBucketSummary],
) -> tuple[int, str, CalibrationQualification]:
    p99_values = {
        key: max(0, bucket.pair_worst_shrink_us.p99)
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
    )


def _parse_v3(data: dict[str, object]) -> CalibrationCacheSummary:
    if data.get("source_formula_version") != SOURCE_FORMULA_VERSION:
        raise ValueError("unsupported calibration source formula")
    _validate_provenance(data, require_extended=True)
    pair_buckets = _parse_pair_buckets(data)
    global_p99, worst_bucket, qualification = _recompute_qualification(pair_buckets)

    raw = data.get("qualification")
    if not isinstance(raw, dict):
        raise TypeError("qualification must be an object")
    if raw.get("basis") != "max_required_bucket_p99_positive_pair_hold_shrink":
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
        raise ValueError("v3 cache has a non-publishable status")
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
    )


def _parse_v2(data: dict[str, object]) -> CalibrationCacheSummary:
    if data.get("source_formula_version") != LEGACY_SOURCE_FORMULA_VERSION:
        raise ValueError("unsupported legacy calibration source formula")
    _validate_provenance(data, require_extended=False)
    pair_buckets = _parse_pair_buckets(data)
    global_p99, worst_bucket, qualification = _recompute_qualification(pair_buckets)

    raw = data.get("selected_margin")
    if not isinstance(raw, dict):
        raise TypeError("selected_margin must be an object")
    if raw.get("basis") != "max_required_bucket_p99_positive_pair_hold_shrink":
        raise ValueError("selected margin basis is missing or invalid")
    if raw.get("worst_bucket") != worst_bucket:
        raise ValueError("selected margin worst bucket is inconsistent")
    if raw.get("global_shrink_p99_us") != global_p99:
        raise ValueError("selected margin p99 is inconsistent")
    guard, floor, ceiling = _validate_policy_constants(raw, name="selected_margin")
    legacy_margin = _int(
        raw.get("recommended_margin_us"),
        "selected_margin.recommended_margin_us",
        minimum=0,
    )
    if legacy_margin != _legacy_v2_selected_margin(global_p99):
        raise ValueError("legacy selected margin is inconsistent with paired p99 evidence")

    # The legacy value is integrity evidence only.  Runtime resolution always
    # uses the v3 qualification result, including its out-of-envelope state.
    return _summary(
        status=qualification.status,
        margin_us=qualification.applied_margin_us,
        pair_buckets=pair_buckets,
        worst_bucket=worst_bucket,
        global_p99=global_p99,
        candidate_margin_us=qualification.candidate_margin_us,
        guard=guard,
        floor=floor,
        ceiling=ceiling,
    )


def parse_calibration_cache_summary(data: object) -> CalibrationCacheSummary:
    """Strictly parse canonical v3 evidence or migrate an integrity-checked v2 cache."""

    if not isinstance(data, dict):
        raise TypeError("calibration cache payload must be a dict")
    version = data.get("version")
    if version not in (SUPPORTED_CACHE_VERSION, LEGACY_CACHE_VERSION):
        raise ValueError(f"unsupported calibration cache version: {version}")
    if data.get("evidence_kind") != "injected_raw_input_delivery_proxy":
        raise ValueError("invalid calibration evidence kind")
    if data.get("native_calibration_version") != SUPPORTED_NATIVE_CALIBRATION_VERSION:
        raise ValueError("unsupported native calibration schema")
    if data.get("measurement_protocol_version") != SUPPORTED_MEASUREMENT_PROTOCOL_VERSION:
        raise ValueError("unsupported measurement protocol")
    if data.get("source") != SOURCE_DEVICE_CACHE:
        raise ValueError("invalid calibration source")
    if version == SUPPORTED_CACHE_VERSION:
        return _parse_v3(data)
    return _parse_v2(data)


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
    try:
        summary = parse_calibration_cache_summary(data)
    except (TypeError, ValueError, KeyError):
        return CalibrationLoadResult(
            status=CalibrationStatus.INVALID_CACHE,
            resolved_margin_us=500,
            margin_source=SOURCE_INVALID_CACHE_DEFAULT_500,
            summary=None,
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
    return None, SOURCE_DEFAULT_500


__all__ = [
    "DEFAULT_CACHE_FILENAME",
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
