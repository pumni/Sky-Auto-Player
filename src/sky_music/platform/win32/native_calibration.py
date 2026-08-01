"""Process-isolated native Raw Input calibration adapter.

The player must never register Raw Input for calibration in its own process.
This module locates the dedicated Rust calibration executable, validates its
structured result, and writes the legacy margin cache only after cleanup and
clean-sample gates have passed.
"""

from __future__ import annotations

import json
import math
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, cast

from sky_music.infrastructure.calibration_loader import MIN_CALIBRATION_SAMPLE_COUNT

SUPPORTED_NATIVE_CALIBRATION_VERSION = 5
SUPPORTED_MEASUREMENT_PROTOCOL_VERSION = 2
MAX_NATIVE_CALIBRATION_STDOUT_BYTES = 8 * 1024 * 1024


class NativeCalibrationError(RuntimeError):
    """Calibration failed or returned evidence that cannot be trusted."""


def _candidate_binaries() -> list[Path]:
    configured = os.environ.get("SKY_NATIVE_CALIBRATION_BIN")
    candidates: list[Path] = []
    if configured:
        candidates.append(Path(configured))

    repository_root = Path(__file__).resolve().parents[4]
    for build_dir in ("debug", "release"):
        candidates.extend(
            [
                repository_root / "rust" / "target" / build_dir / "native_calibration.exe",
                repository_root / "rust" / "target" / build_dir / "native_calibration",
            ]
        )

    candidates.extend(
        [
            Path(sys.executable).resolve().parent / "native_calibration.exe",
            Path(__file__).resolve().parent / "native_calibration.exe",
        ]
    )
    return candidates


def _find_binary() -> Path:
    for candidate in _candidate_binaries():
        if candidate.is_file():
            return candidate
    searched = ", ".join(str(path) for path in _candidate_binaries())
    raise NativeCalibrationError(
        "native_calibration executable was not found; build the Rust binary "
        f"or set SKY_NATIVE_CALIBRATION_BIN (searched: {searched})"
    )


def _require_mapping(value: object, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise NativeCalibrationError(f"calibration field {name!r} is not an object")
    return value


def _bucket(result: dict[str, Any], kind: str) -> dict[str, Any]:
    buckets = _require_mapping(result.get("buckets"), "buckets")
    by_kind = _require_mapping(buckets.get(kind), f"buckets.{kind}")
    polyphony = _require_mapping(by_kind.get("1"), f"buckets.{kind}.1")
    return _require_mapping(polyphony.get("hot"), f"buckets.{kind}.1.hot")


def _clean_quantile(bucket: dict[str, Any], kind: str) -> dict[str, int]:
    clean = bucket.get("clean_sample_count")
    if not isinstance(clean, int) or isinstance(clean, bool):
        raise NativeCalibrationError(f"{kind} bucket has invalid clean_sample_count")
    if clean < MIN_CALIBRATION_SAMPLE_COUNT:
        raise NativeCalibrationError(
            f"{kind} bucket has only {clean} clean samples; "
            f"at least {MIN_CALIBRATION_SAMPLE_COUNT} are required"
        )
    quantiles = _require_mapping(bucket.get("first_receipt_us"), f"{kind}.first_receipt_us")
    values: dict[str, int] = {}
    for name in ("p50", "p90", "p95", "p99"):
        value = quantiles.get(name)
        if not isinstance(value, int) or isinstance(value, bool):
            raise NativeCalibrationError(f"{kind} bucket has invalid {name} quantile")
        values[name] = value
    return values


def _validate_result(result: object) -> dict[str, Any]:
    data = _require_mapping(result, "root")
    if data.get("version") != SUPPORTED_NATIVE_CALIBRATION_VERSION:
        raise NativeCalibrationError("unsupported native calibration schema version")
    if data.get("measurement_protocol_version") != SUPPORTED_MEASUREMENT_PROTOCOL_VERSION:
        raise NativeCalibrationError("unsupported native calibration measurement protocol")
    if data.get("evidence_kind") != "injected_raw_input_delivery_proxy":
        raise NativeCalibrationError("native calibration evidence kind is not the expected proxy")
    for name in (
        "source_git_sha",
        "native_build_id",
        "native_source_fingerprint",
        "rustc_version",
    ):
        value = data.get(name)
        if not isinstance(value, str) or not value.strip() or value == "unknown":
            raise NativeCalibrationError(f"native calibration has invalid {name}")

    if getattr(sys, "frozen", False):
        try:
            from sky_music import _native_build
        except (ImportError, AttributeError) as exc:
            raise NativeCalibrationError(
                "frozen release is missing native calibration provenance metadata"
            ) from exc
        expected_build_id = getattr(_native_build, "EXPECTED_NATIVE_BUILD_ID", "")
        expected_fingerprint = getattr(
            _native_build, "EXPECTED_NATIVE_SOURCE_FINGERPRINT", ""
        )
        if not isinstance(expected_build_id, str) or not isinstance(expected_fingerprint, str):
            raise NativeCalibrationError(
                "frozen release has invalid native calibration provenance metadata"
            )
        if data["native_build_id"] != expected_build_id:
            raise NativeCalibrationError(
                "native calibration build does not match the frozen release"
            )
        if data["native_source_fingerprint"] != expected_fingerprint:
            raise NativeCalibrationError(
                "native calibration source fingerprint does not match the frozen release"
            )

    host = _require_mapping(data.get("host_fingerprint"), "host_fingerprint")
    frequency = host.get("qpc_frequency_hz")
    if not isinstance(frequency, int) or isinstance(frequency, bool) or frequency <= 0:
        raise NativeCalibrationError("native calibration has invalid QPC frequency")

    cleanup = _require_mapping(data.get("cleanup"), "cleanup")
    if cleanup.get("cleanup_success") is not True:
        raise NativeCalibrationError("native calibration cleanup did not succeed")
    if cleanup.get("cleanup_verification_inconclusive") is not False:
        raise NativeCalibrationError("native calibration cleanup could not be verified")
    if cleanup.get("raw_input_restore_failed") is not False:
        raise NativeCalibrationError(
            "native calibration did not verify Raw Input registration restoration"
        )
    if cleanup.get("pump_thread_failed") is not False:
        raise NativeCalibrationError("native calibration pump thread did not exit cleanly")

    count_names = (
        "warmup_attempted",
        "measured_attempted",
        "setup_attempted",
        "warmup_anomalous",
        "measured_anomalous",
        "setup_anomalous",
        "warmup_timed_out",
        "measured_timed_out",
        "setup_timed_out",
        "total_attempted",
        "total_anomalous",
        "total_timed_out",
    )
    counts: dict[str, int] = {}
    for name in count_names:
        value = data.get(name)
        if not isinstance(value, int) or isinstance(value, bool) or value < 0:
            raise NativeCalibrationError(f"native calibration has invalid {name}")
        counts[name] = value
    if counts["measured_attempted"] <= 0:
        raise NativeCalibrationError("native calibration has no measured attempts")
    if counts["total_attempted"] != sum(
        counts[name] for name in ("warmup_attempted", "measured_attempted", "setup_attempted")
    ):
        raise NativeCalibrationError("native calibration attempt totals are inconsistent")
    if counts["total_anomalous"] != sum(
        counts[name] for name in ("warmup_anomalous", "measured_anomalous", "setup_anomalous")
    ):
        raise NativeCalibrationError("native calibration anomaly totals are inconsistent")
    if counts["total_timed_out"] != sum(
        counts[name] for name in ("warmup_timed_out", "measured_timed_out", "setup_timed_out")
    ):
        raise NativeCalibrationError("native calibration timeout totals are inconsistent")

    buckets = _require_mapping(data.get("buckets"), "buckets")
    for kind in ("down", "up"):
        by_polyphony = _require_mapping(buckets.get(kind), f"buckets.{kind}")
        for polyphony, classes in by_polyphony.items():
            _require_mapping(classes, f"buckets.{kind}.{polyphony}")
            for class_name, bucket_value in classes.items():
                bucket = _require_mapping(
                    bucket_value, f"buckets.{kind}.{polyphony}.{class_name}"
                )
                attempted = bucket.get("attempted")
                clean = bucket.get("clean")
                rejected = bucket.get("rejected")
                clean_samples = bucket.get("clean_sample_count")
                if not all(
                    isinstance(value, int) and not isinstance(value, bool) and value >= 0
                    for value in (attempted, clean, rejected, clean_samples)
                ):
                    raise NativeCalibrationError("native calibration bucket counts are invalid")
                attempted_value = cast(int, attempted)
                clean_value = cast(int, clean)
                rejected_value = cast(int, rejected)
                clean_samples_value = cast(int, clean_samples)
                if clean_samples_value != clean_value or clean_value + rejected_value != attempted_value:
                    raise NativeCalibrationError("native calibration bucket totals are inconsistent")
    return data


def _legacy_cache(result: dict[str, Any]) -> dict[str, Any]:
    down = _clean_quantile(_bucket(result, "down"), "down")
    up = _clean_quantile(_bucket(result, "up"), "up")
    clean_count = min(
        _bucket(result, "down").get("clean_sample_count", 0),
        _bucket(result, "up").get("clean_sample_count", 0),
    )
    return {
        "version": 1,
        "evidence_kind": result["evidence_kind"],
        "source_formula_version": 1,
        "down_us": {name: down[name] for name in ("p50", "p90", "p99")},
        "up_us": {name: up[name] for name in ("p50", "p90", "p99")},
        "n": clean_count,
        "sample_count": clean_count,
        "native_calibration_version": result["version"],
        "measurement_protocol_version": result["measurement_protocol_version"],
        "source_git_sha": result.get("source_git_sha"),
        "native_build_id": result.get("native_build_id"),
        "host_fingerprint": result.get("host_fingerprint"),
        "anomaly_counts": {
            "warmup": result.get("warmup_anomalous"),
            "measured": result.get("measured_anomalous"),
            "setup": result.get("setup_anomalous"),
            "total": result.get("total_anomalous"),
        },
    }


def _write_json_atomically(path: Path, data: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    try:
        temporary.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
        temporary.replace(path)
    except OSError as exc:
        raise NativeCalibrationError(f"could not write calibration artifact {path}: {exc}") from exc


def run_native_calibration(
    *,
    mode: str = "quick",
    output_path: Path | str | None = None,
    cache_path: Path | str = ".cache/input_latency.json",
    timeout_seconds: float = 1800.0,
) -> dict[str, Any]:
    """Run the dedicated calibration process and return validated raw JSON."""

    if mode not in {"quick", "full"}:
        raise NativeCalibrationError("mode must be quick or full")
    if (
        not isinstance(timeout_seconds, (int, float))
        or isinstance(timeout_seconds, bool)
        or not math.isfinite(float(timeout_seconds))
    ):
        raise NativeCalibrationError("timeout_seconds must be a finite positive number")
    if timeout_seconds <= 0:
        raise NativeCalibrationError("timeout_seconds must be a finite positive number")

    binary = _find_binary()
    with tempfile.TemporaryFile() as stdout_file, tempfile.TemporaryFile() as stderr_file:
        try:
            process = subprocess.Popen(
                [str(binary), "--mode", mode],
                stdout=stdout_file,
                stderr=stderr_file,
                shell=False,
            )
            try:
                return_code = process.wait(timeout=float(timeout_seconds))
            except subprocess.TimeoutExpired as exc:
                process.kill()
                process.wait()
                raise NativeCalibrationError("native calibration timed out") from exc
        except OSError as exc:
            raise NativeCalibrationError(f"could not start native calibration: {exc}") from exc

        stdout_size = stdout_file.tell()
        if stdout_size > MAX_NATIVE_CALIBRATION_STDOUT_BYTES:
            raise NativeCalibrationError("native calibration stdout exceeded the size limit")
        stdout_file.seek(0)
        stderr_file.seek(0)
        stdout = stdout_file.read().decode("utf-8", errors="strict")
        stderr = stderr_file.read().decode("utf-8", errors="replace")

    if return_code != 0:
        detail = stderr.strip() or "native calibration exited without diagnostics"
        raise NativeCalibrationError(f"native calibration failed ({return_code}): {detail}")
    if stderr.strip():
        # Diagnostics are allowed on stderr, but stdout must remain JSON-only.
        pass
    try:
        result = _validate_result(json.loads(stdout))
    except (json.JSONDecodeError, TypeError, ValueError) as exc:
        raise NativeCalibrationError("native calibration stdout was not valid JSON") from exc

    raw_output = Path(output_path) if output_path is not None else Path(".cache/calibration-native.json")
    _write_json_atomically(raw_output, result)
    _write_json_atomically(Path(cache_path), _legacy_cache(result))
    return result


__all__ = ["NativeCalibrationError", "run_native_calibration"]
