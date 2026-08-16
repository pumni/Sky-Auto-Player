"""Strict loader for the paired input-delivery calibration cache.

The cache is evidence about an app-owned ``WM_INPUT`` delivery proxy.  It is
not game, rendering, audio, or network latency.  Version one contained
independent Down/Up marginals and is intentionally rejected rather than
reinterpreted as paired evidence.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

DEFAULT_CACHE_FILENAME: str = ".cache/input_latency.json"
SOURCE_DEVICE_CACHE: str = "device_cache"
SOURCE_DEFAULT_500: str = "default_500"
SUPPORTED_CACHE_VERSION: int = 2
SUPPORTED_NATIVE_CALIBRATION_VERSION: int = 9
SUPPORTED_MEASUREMENT_PROTOCOL_VERSION: int = 4
SOURCE_FORMULA_VERSION: int = 2
MIN_CALIBRATION_SAMPLE_COUNT: int = 100
MAX_SHRINK_US: int = 100_000
MARGIN_GUARD_US: int = 100
MARGIN_FLOOR_US: int = 300
MARGIN_CEILING_US: int = 2_000
REQUIRED_BUCKETS: tuple[str, ...] = (
    "1/hot",
    "1/cold",
    "5/hot",
    "5/cold",
    "15/hot",
    "15/cold",
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
    margin_us: int
    source: str
    sample_count: int
    pair_buckets: dict[str, PairBucketSummary]
    worst_bucket: str
    global_shrink_p99_us: int
    guard_us: int
    floor_us: int
    ceiling_us: int
    down_us: CalibrationQuantiles | None = None
    up_us: CalibrationQuantiles | None = None


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


def _selected_margin(global_shrink_p99_us: int) -> int:
    return max(
        MARGIN_FLOOR_US,
        min(MARGIN_CEILING_US, global_shrink_p99_us + MARGIN_GUARD_US),
    )


def parse_calibration_cache_summary(data: object) -> CalibrationCacheSummary:
    """Strictly parse cache-v2 paired evidence.

    This function deliberately does not accept cache-v1.  The old marginal
    formula cannot be converted into per-pair ``D-U`` evidence safely.
    """

    if not isinstance(data, dict):
        raise TypeError("calibration cache payload must be a dict")
    if data.get("version") != SUPPORTED_CACHE_VERSION:
        raise ValueError(f"unsupported calibration cache version: {data.get('version')}")
    if data.get("evidence_kind") != "injected_raw_input_delivery_proxy":
        raise ValueError("invalid calibration evidence kind")
    if data.get("source_formula_version") != SOURCE_FORMULA_VERSION:
        raise ValueError("unsupported calibration source formula")
    if data.get("native_calibration_version") != SUPPORTED_NATIVE_CALIBRATION_VERSION:
        raise ValueError("unsupported native calibration schema")
    if data.get("measurement_protocol_version") != SUPPORTED_MEASUREMENT_PROTOCOL_VERSION:
        raise ValueError("unsupported measurement protocol")
    if data.get("source") != SOURCE_DEVICE_CACHE:
        raise ValueError("invalid calibration source")
    if data.get("dirty_worktree") is not False:
        raise ValueError("calibration provenance is dirty")
    for field in ("source_git_sha", "native_build_id"):
        value = data.get(field)
        if not isinstance(value, str) or not value.strip() or value == "unknown":
            raise ValueError(f"invalid calibration provenance field: {field}")
    if data["source_git_sha"] != data["native_build_id"]:
        raise ValueError("calibration source/build provenance mismatch")
    host = data.get("host_fingerprint")
    if not isinstance(host, dict):
        raise TypeError("host_fingerprint must be an object")
    _int(host.get("qpc_frequency_hz"), "host_fingerprint.qpc_frequency_hz", minimum=1)
    if not isinstance(host.get("win32_build"), str) or not host["win32_build"].strip():
        raise ValueError("host_fingerprint.win32_build is required")

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
        clean = _int(
            bucket.get("clean_pair_count"), f"{key}.clean_pair_count", minimum=0
        )
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

    selected = data.get("selected_margin")
    if not isinstance(selected, dict):
        raise TypeError("selected_margin must be an object")
    p99_values = {
        key: max(0, bucket.pair_worst_shrink_us.p99)
        for key, bucket in pair_buckets.items()
    }
    global_p99 = max(p99_values.values())
    worst_bucket = max(REQUIRED_BUCKETS, key=lambda key: (p99_values[key], -REQUIRED_BUCKETS.index(key)))
    if selected.get("basis") != "max_required_bucket_p99_positive_pair_hold_shrink":
        raise ValueError("selected margin basis is missing or invalid")
    if selected.get("worst_bucket") != worst_bucket:
        raise ValueError("selected margin worst bucket is inconsistent")
    if selected.get("global_shrink_p99_us") != global_p99:
        raise ValueError("selected margin p99 is inconsistent")
    guard = _int(selected.get("guard_us"), "selected_margin.guard_us", minimum=0)
    floor = _int(selected.get("floor_us"), "selected_margin.floor_us", minimum=0)
    ceiling = _int(selected.get("ceiling_us"), "selected_margin.ceiling_us", minimum=floor)
    if (guard, floor, ceiling) != (
        MARGIN_GUARD_US,
        MARGIN_FLOOR_US,
        MARGIN_CEILING_US,
    ):
        raise ValueError("calibration policy constants do not match the correction contract")
    margin = _int(selected.get("recommended_margin_us"), "selected_margin.recommended_margin_us", minimum=0)
    if margin != _selected_margin(global_p99):
        raise ValueError("selected margin is inconsistent with paired p99 evidence")

    return CalibrationCacheSummary(
        margin_us=margin,
        source=SOURCE_DEVICE_CACHE,
        sample_count=min(bucket.clean_pair_count for bucket in pair_buckets.values()),
        pair_buckets=pair_buckets,
        worst_bucket=worst_bucket,
        global_shrink_p99_us=global_p99,
        guard_us=guard,
        floor_us=floor,
        ceiling_us=ceiling,
    )


def load_calibrated_margin_recommendation(
    *, cache_path: Path | None = None, data: dict | None = None
) -> tuple[int | None, str]:
    """Return a validated static margin or the unchanged 500 µs fallback."""

    if data is None:
        path = Path(cache_path) if cache_path is not None else Path(DEFAULT_CACHE_FILENAME)
        if not path.exists():
            return None, SOURCE_DEFAULT_500
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return None, SOURCE_DEFAULT_500
    try:
        summary = parse_calibration_cache_summary(data)
    except (TypeError, ValueError, KeyError):
        return None, SOURCE_DEFAULT_500
    return summary.margin_us, SOURCE_DEVICE_CACHE


__all__ = [
    "DEFAULT_CACHE_FILENAME",
    "MARGIN_CEILING_US",
    "MARGIN_FLOOR_US",
    "MAX_SHRINK_US",
    "MIN_CALIBRATION_SAMPLE_COUNT",
    "REQUIRED_BUCKETS",
    "SOURCE_DEFAULT_500",
    "SOURCE_DEVICE_CACHE",
    "SUPPORTED_CACHE_VERSION",
    "SUPPORTED_MEASUREMENT_PROTOCOL_VERSION",
    "SUPPORTED_NATIVE_CALIBRATION_VERSION",
    "CalibrationCacheSummary",
    "CalibrationQuantiles",
    "PairBucketSummary",
    "load_calibrated_margin_recommendation",
    "parse_calibration_cache_summary",
]
