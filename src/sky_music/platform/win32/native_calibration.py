"""Process-isolated native Raw Input calibration adapter.

The player must never register Raw Input for calibration in its own process.
This module locates the dedicated Rust calibration executable, validates its
structured result, and writes the legacy margin cache only after cleanup and
clean-sample gates have passed.
"""

from __future__ import annotations

import importlib
import json
import math
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

from sky_music.infrastructure.calibration_loader import MIN_CALIBRATION_SAMPLE_COUNT

SUPPORTED_NATIVE_CALIBRATION_VERSION = 6
SUPPORTED_MEASUREMENT_PROTOCOL_VERSION = 3
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


def _nonnegative_int(value: object, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise NativeCalibrationError(f"native calibration has invalid {name}")
    return value


def _positive_int(value: object, name: str) -> int:
    result = _nonnegative_int(value, name)
    if result == 0:
        raise NativeCalibrationError(f"native calibration has invalid {name}")
    return result


def _validate_configuration(result: dict[str, Any]) -> tuple[tuple[int, ...], int, int]:
    configuration = _require_mapping(result.get("configuration"), "configuration")
    raw_polyphonies = configuration.get("polyphonies")
    if not isinstance(raw_polyphonies, list) or not raw_polyphonies:
        raise NativeCalibrationError("calibration configuration polyphonies must not be empty")
    polyphonies: list[int] = []
    for value in raw_polyphonies:
        polyphony = _positive_int(value, "configuration.polyphonies")
        if polyphony > 15:
            raise NativeCalibrationError("calibration configuration polyphony is out of range")
        if polyphony in polyphonies:
            raise NativeCalibrationError("calibration configuration has duplicate polyphony")
        polyphonies.append(polyphony)

    hot_target = _nonnegative_int(
        configuration.get("hot_gap_target_us"), "configuration.hot_gap_target_us"
    )
    cold_threshold = _positive_int(
        configuration.get("cold_threshold_us"), "configuration.cold_threshold_us"
    )
    cold_idle = _nonnegative_int(
        configuration.get("cold_idle_gap_us"), "configuration.cold_idle_gap_us"
    )
    if hot_target >= cold_threshold:
        raise NativeCalibrationError("hot gap target must be shorter than cold threshold")
    if cold_idle < cold_threshold:
        raise NativeCalibrationError("cold idle gap is shorter than cold threshold")
    hot_samples = _positive_int(
        configuration.get("samples_per_hot_bucket"),
        "configuration.samples_per_hot_bucket",
    )
    cold_samples = _positive_int(
        configuration.get("samples_per_cold_bucket"),
        "configuration.samples_per_cold_bucket",
    )
    return tuple(polyphonies), hot_samples, cold_samples


def _validate_bucket(
    bucket_value: object,
    name: str,
    expected_attempts: int,
) -> dict[str, Any]:
    bucket = _require_mapping(bucket_value, name)
    fields = (
        "attempted",
        "clean",
        "rejected",
        "clean_sample_count",
        "sample_count",
        "timeout_count",
        "anomaly_count",
        "class_mismatch_count",
        "partial_send",
        "error_count",
    )
    counts = {field: _nonnegative_int(bucket.get(field), f"{name}.{field}") for field in fields}
    if counts["attempted"] != expected_attempts:
        raise NativeCalibrationError(f"{name} has an unexpected attempt count")
    if counts["clean_sample_count"] != counts["clean"]:
        raise NativeCalibrationError(f"{name} clean sample count is inconsistent")
    if counts["clean"] + counts["rejected"] != counts["attempted"]:
        raise NativeCalibrationError(f"{name} clean/rejected totals are inconsistent")
    if counts["sample_count"] != counts["attempted"]:
        raise NativeCalibrationError(f"{name} sample count is inconsistent")
    for field in ("timeout_count", "anomaly_count", "class_mismatch_count", "partial_send", "error_count"):
        if counts[field] > counts["rejected"]:
            raise NativeCalibrationError(f"{name}.{field} exceeds rejected count")
    return bucket


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
        if value.endswith("-dirty"):
            raise NativeCalibrationError("native calibration provenance is dirty")
    if data.get("dirty_worktree") is not False:
        raise NativeCalibrationError("native calibration worktree is not clean")
    if data["source_git_sha"] != data["native_build_id"]:
        raise NativeCalibrationError("native calibration source/build SHA mismatch")

    polyphonies, hot_attempts, cold_attempts = _validate_configuration(data)

    if getattr(sys, "frozen", False):
        try:
            native_build = importlib.import_module("sky_music._native_build")
        except (ImportError, AttributeError) as exc:
            raise NativeCalibrationError(
                "frozen release is missing native calibration provenance metadata"
            ) from exc
        expected_build_id = getattr(native_build, "EXPECTED_NATIVE_BUILD_ID", "")
        expected_fingerprint = getattr(
            native_build, "EXPECTED_NATIVE_SOURCE_FINGERPRINT", ""
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
        "measured_class_mismatch",
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
    if set(buckets) != {"down", "up"}:
        raise NativeCalibrationError("native calibration bucket kinds are incomplete or unknown")
    bucket_totals = {
        "attempted": 0,
        "timeout_count": 0,
        "anomaly_count": 0,
        "class_mismatch_count": 0,
    }
    for kind in ("down", "up"):
        by_polyphony = _require_mapping(buckets.get(kind), f"buckets.{kind}")
        expected_polys = {str(polyphony) for polyphony in polyphonies}
        if set(by_polyphony) != expected_polys:
            raise NativeCalibrationError(f"native calibration {kind} bucket matrix is incomplete")
        for polyphony in polyphonies:
            classes = _require_mapping(by_polyphony.get(str(polyphony)), f"buckets.{kind}.{polyphony}")
            if set(classes) != {"hot", "cold"}:
                raise NativeCalibrationError(
                    f"native calibration {kind}.{polyphony} class matrix is incomplete"
                )
            expected = {"hot": hot_attempts, "cold": cold_attempts}
            for class_name in ("hot", "cold"):
                name = f"buckets.{kind}.{polyphony}.{class_name}"
                bucket = _validate_bucket(classes.get(class_name), name, expected[class_name])
                for field in bucket_totals:
                    bucket_totals[field] += int(bucket[field])

    if bucket_totals["attempted"] != counts["measured_attempted"]:
        raise NativeCalibrationError("bucket attempts do not equal measured_attempted")
    for field, top_level in (
        ("timeout_count", "measured_timed_out"),
        ("anomaly_count", "measured_anomalous"),
        ("class_mismatch_count", "measured_class_mismatch"),
    ):
        if bucket_totals[field] != counts[top_level]:
            raise NativeCalibrationError(f"bucket {field} total does not match {top_level}")
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
        "dirty_worktree": result.get("dirty_worktree"),
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
