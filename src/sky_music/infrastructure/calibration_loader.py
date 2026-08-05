"""Device-calibrated margin loader — moved from ``domain/scheduler_types``.

Reads ``.cache/input_latency.json`` (a process-local calibration artefact
written by the latency-calibration workflow) and produces the recommended
device-delivery margin in microseconds together with its source label so
``domain.TimingPolicy.from_dict`` can stay filesystem-free.

Layer direction (AGENTS.md Architecture Invariants): the domain layer must
not import ``ctypes``, ``SendInput``, wall-clock, or Windows-specific
modules. Filesystem I/O and JSON-schema parsing sit at the same
adjacency as the platform layer, so this loader lives in
``infrastructure/`` rather than ``domain/``. The orchestration caller
(runtime_session.RuntimeSessionState.apply_session) resolves the
calibration result once and passes primitives into the domain factory.

Public contract:

    load_calibrated_margin_recommendation() -> (margin_us | None, source_label)

The two source labels are exactly::

    "device_cache"   — a valid .cache/input_latency.json produced a margin
    "default_500"    — cache missing / corrupt / out-of-bounds / under-sampled;
                       the caller falls back to the 500 µs constant

The full recommend formula ``clamp(300, 2000, p99(down_delivery) - p50(up_delivery) + 100)``
and the validation guards against absurd values are preserved bit-for-bit
from the legacy domain function; the test suite (``tests/test_calibration.py``,
``tests/test_core_send_overhaul_invariants.py``) is updated to call this
loader instead of the removed domain function.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

#: Default location of the calibration artefact. Exposed so callers and
#: tests can reference the same path without duplicating the literal.
DEFAULT_CACHE_FILENAME: str = ".cache/input_latency.json"

#: Source-label sentinels for ``load_calibrated_margin_recommendation``.
SOURCE_DEVICE_CACHE: str = "device_cache"
SOURCE_DEFAULT_500: str = "default_500"

#: Calibration artefact schema version this loader understands.
SUPPORTED_CACHE_VERSION: int = 1

#: Minimum sample count the calibration run must produce for the cache to
#: be trusted. Below this the loader returns the fallback rather than a
#: noisy recommendation that the small sample can't defend.
MIN_CALIBRATION_SAMPLE_COUNT: int = 20

#: Hard bound on the p99 down / p50 up the loader accepts. Inputs above
#: this are treated as malformed (a 100 ms first-byte delivery would be a
#: kernel anti-pattern, not a real device signal).
MAX_DELIVERY_US: int = 100_000

#: Margin clamps. Same constants the legacy domain function applied.
MARGIN_FLOOR_US: int = 300
MARGIN_CEILING_US: int = 2000


@dataclass(frozen=True, slots=True)
class CalibrationQuantiles:
    p50: int
    p90: int
    p99: int


@dataclass(frozen=True, slots=True)
class CalibrationCacheSummary:
    margin_us: int
    source: str
    sample_count: int
    down_us: CalibrationQuantiles
    up_us: CalibrationQuantiles


def _compute_recommended_margin_us(p99_down: float, p50_up: float) -> int:
    """Apply the calibration formula and clamp to ``[MARGIN_FLOOR_US,
    MARGIN_CEILING_US]``. Pulled out so the loader is the single owner of
    the formula and the magic constants.
    """
    raw = p99_down - p50_up + 100
    clamped = max(float(MARGIN_FLOOR_US), min(float(MARGIN_CEILING_US), raw))
    return round(clamped)


def parse_calibration_cache_summary(data: object) -> CalibrationCacheSummary:
    """Parse and strictly validate compatible calibration cache payload.

    Validation rules:
    - version == 1 (SUPPORTED_CACHE_VERSION)
    - n >= MIN_CALIBRATION_SAMPLE_COUNT
    - down_us and up_us are dicts
    - p50/p90/p99 are ints (not bools)
    - 0 <= p50 <= p90 <= p99 <= MAX_DELIVERY_US
    - source == SOURCE_DEVICE_CACHE ("device_cache")
    - margin calculated via _compute_recommended_margin_us helper

    Missing or invalid fields raise ValueError or TypeError.
    """
    if not isinstance(data, dict):
        raise TypeError("calibration cache payload must be a dict")

    if data.get("version") != SUPPORTED_CACHE_VERSION:
        raise ValueError(f"unsupported calibration cache version: {data.get('version')}")

    source = data.get("source", SOURCE_DEVICE_CACHE)
    if source != SOURCE_DEVICE_CACHE:
        raise ValueError(f"invalid calibration source: {source!r}")

    n = data.get("n")
    if n is None:
        n = data.get("sample_count")
    if not isinstance(n, int) or isinstance(n, bool):
        raise TypeError("calibration sample count 'n' must be an integer")
    if n < MIN_CALIBRATION_SAMPLE_COUNT:
        raise ValueError(
            f"calibration sample count {n} is less than minimum {MIN_CALIBRATION_SAMPLE_COUNT}"
        )

    down_dict = data.get("down_us")
    up_dict = data.get("up_us")
    if not isinstance(down_dict, dict) or not isinstance(up_dict, dict):
        raise TypeError("down_us and up_us must be dictionaries")

    def _extract_quantiles(qdict: dict, name: str) -> CalibrationQuantiles:
        vals: dict[str, int] = {}
        for key in ("p50", "p90", "p99"):
            if key not in qdict:
                raise ValueError(f"missing quantile {key!r} in {name}")
            val = qdict[key]
            if not isinstance(val, int) or isinstance(val, bool):
                raise TypeError(
                    f"quantile {key!r} in {name} must be an integer, got {type(val).__name__}"
                )
            vals[key] = val
        p50, p90, p99 = vals["p50"], vals["p90"], vals["p99"]
        if not (0 <= p50 <= p90 <= p99 <= MAX_DELIVERY_US):
            raise ValueError(
                f"invalid quantile ordering or bounds in {name}: p50={p50}, p90={p90}, p99={p99}"
            )
        return CalibrationQuantiles(p50=p50, p90=p90, p99=p99)

    down_quantiles = _extract_quantiles(down_dict, "down_us")
    up_quantiles = _extract_quantiles(up_dict, "up_us")

    margin_us = _compute_recommended_margin_us(
        float(down_quantiles.p99), float(up_quantiles.p50)
    )

    return CalibrationCacheSummary(
        margin_us=margin_us,
        source=SOURCE_DEVICE_CACHE,
        sample_count=n,
        down_us=down_quantiles,
        up_us=up_quantiles,
    )


def load_calibrated_margin_recommendation(
    *,
    cache_path: Path | None = None,
    data: dict | None = None,
) -> tuple[int | None, str]:
    """Return ``(margin_us, source_label)``.

    ``cache_path`` defaults to ``.cache/input_latency.json`` relative to
    the current working directory. ``data`` (when provided) short-circuits
    the filesystem read and is the test seam — the legacy tests that
    wrote synthetic JSON into the cache file now pass ``data=`` directly.

    Returns ``(None, SOURCE_DEFAULT_500)`` whenever the cache is missing,
    unreadable, version-incompatible, shape-invalid, out-of-bounds, or
    under-sampled. The caller's fallback is the constant 500 µs.
    """
    if data is None:
        path = Path(cache_path) if cache_path is not None else Path(DEFAULT_CACHE_FILENAME)
        if not path.exists():
            return None, SOURCE_DEFAULT_500
        try:
            with path.open(encoding="utf-8") as f:
                loaded = json.load(f)
        except Exception:
            return None, SOURCE_DEFAULT_500
    else:
        loaded = data

    try:
        summary = parse_calibration_cache_summary(loaded)
        return summary.margin_us, summary.source
    except Exception:
        return None, SOURCE_DEFAULT_500


__all__ = [
    "DEFAULT_CACHE_FILENAME",
    "MARGIN_CEILING_US",
    "MARGIN_FLOOR_US",
    "MAX_DELIVERY_US",
    "MIN_CALIBRATION_SAMPLE_COUNT",
    "SOURCE_DEFAULT_500",
    "SOURCE_DEVICE_CACHE",
    "SUPPORTED_CACHE_VERSION",
    "CalibrationCacheSummary",
    "CalibrationQuantiles",
    "load_calibrated_margin_recommendation",
    "parse_calibration_cache_summary",
]

