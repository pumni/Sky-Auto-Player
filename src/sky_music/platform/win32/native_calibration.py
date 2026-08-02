"""Process-isolated native Raw Input calibration adapter.

The player must never register Raw Input for calibration in its own process.
This module locates the dedicated Rust calibration executable, validates its
structured result, and writes the legacy margin cache only after cleanup and
clean-sample gates have passed.
"""

from __future__ import annotations

import hashlib
import importlib
import json
import math
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

from sky_music.infrastructure.calibration_loader import MIN_CALIBRATION_SAMPLE_COUNT

SUPPORTED_NATIVE_CALIBRATION_VERSION = 6
SUPPORTED_MEASUREMENT_PROTOCOL_VERSION = 3
CALIBRATION_ARTIFACT_SCHEMA_VERSION = 1
FULL_POLYPHONIES = (1, 2, 3, 5, 8, 15)
FULL_SAMPLE_COUNT = 5_000
FULL_COLD_IDLE_GAP_US = 100_000
FULL_WARMUP_SAMPLES = 50
CALIBRATION_KINDS = ("down", "up")
CALIBRATION_CLASSES = ("hot", "cold")
MAX_NATIVE_CALIBRATION_STDOUT_BYTES = 8 * 1024 * 1024
QUICK_CALIBRATION_TIMEOUT_SECONDS = 1_800.0
FULL_CALIBRATION_TIMEOUT_SECONDS = 14_400.0


class NativeCalibrationError(RuntimeError):
    """Calibration failed or returned evidence that cannot be trusted."""

    def __init__(self, message: str, *, failure_report: dict[str, Any] | None = None) -> None:
        super().__init__(message)
        self.failure_report = failure_report


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
    win32_build = host.get("win32_build")
    if not isinstance(win32_build, str) or not win32_build.strip() or win32_build == "unknown":
        raise NativeCalibrationError("native calibration has incomplete Windows host fingerprint")
    sampled_at_us = host.get("sampled_at_us")
    if not isinstance(sampled_at_us, int) or isinstance(sampled_at_us, bool) or sampled_at_us <= 0:
        raise NativeCalibrationError("native calibration has invalid host sample timestamp")

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
    for scope in ("warmup", "measured", "setup"):
        attempted = counts[f"{scope}_attempted"]
        for metric in ("anomalous", "timed_out"):
            if counts[f"{scope}_{metric}"] > attempted:
                raise NativeCalibrationError(
                    f"native calibration {scope}_{metric} exceeds {scope}_attempted"
                )
    if counts["measured_class_mismatch"] > counts["measured_attempted"]:
        raise NativeCalibrationError(
            "native calibration measured_class_mismatch exceeds measured_attempted"
        )

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


def _write_text_atomically(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp")
    try:
        temporary.write_text(text, encoding="utf-8")
        temporary.replace(path)
    except OSError as exc:
        raise NativeCalibrationError(f"could not write calibration text {path}: {exc}") from exc


def _repository_root() -> Path:
    return Path(__file__).resolve().parents[4]


def _git_output(*arguments: str) -> str:
    try:
        return subprocess.check_output(
            ["git", *arguments], cwd=_repository_root(), stderr=subprocess.DEVNULL, text=True
        ).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        raise NativeCalibrationError(f"could not read git provenance: {exc}") from exc


def _current_worktree_provenance() -> dict[str, Any]:
    source_git_sha = _git_output("rev-parse", "HEAD")
    dirty_worktree = bool(_git_output("status", "--porcelain"))
    from sky_music.orchestration.native_provenance import native_source_fingerprint

    return {
        "source_git_sha": source_git_sha,
        "dirty_worktree": dirty_worktree,
        "native_source_fingerprint": native_source_fingerprint(
            _repository_root(), "cp314t-win_amd64"
        ),
    }


def _rustc_version() -> str:
    try:
        return subprocess.check_output(["rustc", "--version"], text=True).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        raise NativeCalibrationError(f"could not read rustc provenance: {exc}") from exc


def full_calibration_configuration() -> dict[str, Any]:
    """Return the immutable full-calibration contract."""

    return {
        "polyphonies": list(FULL_POLYPHONIES),
        "samples_per_hot_bucket": FULL_SAMPLE_COUNT,
        "samples_per_cold_bucket": FULL_SAMPLE_COUNT,
        "warmup_samples": FULL_WARMUP_SAMPLES,
        "receipt_timeout_ms": 200,
        "hot_gap_target_us": 5_000,
        "cold_idle_gap_us": FULL_COLD_IDLE_GAP_US,
        "cold_threshold_us": 20_000,
    }


def calibration_bucket_keys() -> list[tuple[str, int, str]]:
    return [
        (kind, polyphony, class_name)
        for polyphony in FULL_POLYPHONIES
        for kind in CALIBRATION_KINDS
        for class_name in CALIBRATION_CLASSES
    ]


def _bucket_key(kind: str, polyphony: int, class_name: str) -> str:
    return f"{kind}/{polyphony}/{class_name}"


def _stable_host_fingerprint(value: object) -> dict[str, Any]:
    host = _require_mapping(value, "host_fingerprint")
    frequency = host.get("qpc_frequency_hz")
    build = host.get("win32_build")
    if not isinstance(frequency, int) or isinstance(frequency, bool) or frequency <= 0:
        raise NativeCalibrationError("native calibration has invalid QPC frequency")
    if not isinstance(build, str) or not build.strip() or build == "unknown":
        raise NativeCalibrationError("native calibration has incomplete Windows host fingerprint")
    # sampled_at_us is observation time, not host identity.  Keeping it out of
    # the checkpoint provenance lets separate bucket processes agree on the
    # same machine while retaining the stable QPC/Windows identity.
    return {"qpc_frequency_hz": frequency, "win32_build": build}


def _validate_bucket_result(
    result: object,
    *,
    kind: str,
    class_name: str,
    polyphony: int,
    samples: int,
) -> dict[str, Any]:
    data = _require_mapping(result, "bucket root")
    if data.get("version") != SUPPORTED_NATIVE_CALIBRATION_VERSION:
        raise NativeCalibrationError("unsupported native calibration schema version")
    if data.get("measurement_protocol_version") != SUPPORTED_MEASUREMENT_PROTOCOL_VERSION:
        raise NativeCalibrationError("unsupported native calibration measurement protocol")
    if data.get("evidence_kind") != "injected_raw_input_delivery_proxy":
        raise NativeCalibrationError("native calibration evidence kind is not the expected proxy")
    if data.get("kind") != kind or data.get("class") != class_name:
        raise NativeCalibrationError("native calibration bucket identity does not match the request")
    if data.get("polyphony") != polyphony:
        raise NativeCalibrationError("native calibration polyphony does not match the request")
    for name in (
        "source_git_sha",
        "native_build_id",
        "native_source_fingerprint",
        "rustc_version",
    ):
        value = data.get(name)
        if not isinstance(value, str) or not value.strip() or value == "unknown":
            raise NativeCalibrationError(f"native calibration has invalid {name}")
    if data["source_git_sha"] != data["native_build_id"]:
        raise NativeCalibrationError("native calibration source/build SHA mismatch")
    _stable_host_fingerprint(data.get("host_fingerprint"))

    configuration = _require_mapping(data.get("configuration"), "configuration")
    if configuration.get("polyphonies") != [polyphony]:
        raise NativeCalibrationError("native bucket configuration has the wrong polyphony")
    selected_samples = (
        configuration.get("samples_per_hot_bucket")
        if class_name == "hot"
        else configuration.get("samples_per_cold_bucket")
    )
    if selected_samples != samples:
        raise NativeCalibrationError("native bucket configuration has the wrong sample count")
    if configuration.get("cold_idle_gap_us") != FULL_COLD_IDLE_GAP_US:
        raise NativeCalibrationError("native bucket changed the cold idle gap contract")

    bucket = _require_mapping(data.get("bucket"), "bucket")
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
    counts = {
        field: _nonnegative_int(bucket.get(field), f"bucket.{field}") for field in fields
    }
    if counts["attempted"] != samples or counts["sample_count"] != samples:
        raise NativeCalibrationError("native bucket does not contain the requested samples")
    if counts["clean"] + counts["rejected"] != samples:
        raise NativeCalibrationError("native bucket clean/rejected totals are inconsistent")
    for field in ("timeout_count", "anomaly_count", "class_mismatch_count", "partial_send", "error_count"):
        if counts[field] > counts["rejected"]:
            raise NativeCalibrationError(f"bucket.{field} exceeds rejected samples")

    cleanup = _require_mapping(data.get("cleanup"), "cleanup")
    for field in (
        "cleanup_success",
        "cleanup_verification_inconclusive",
        "raw_input_restore_failed",
        "pump_thread_failed",
    ):
        if not isinstance(cleanup.get(field), bool):
            raise NativeCalibrationError(f"cleanup.{field} must be boolean")
    return data


def _native_failure_report(
    *,
    kind: str,
    class_name: str,
    polyphony: int,
    exact_error: str,
    native_stderr: str = "",
    phase: str = "process",
) -> dict[str, Any]:
    for line in reversed(native_stderr.splitlines()):
        marker = "CALIBRATION_FAILURE_JSON:"
        if marker in line:
            try:
                parsed = json.loads(line.split(marker, 1)[1])
            except json.JSONDecodeError:
                break
            if isinstance(parsed, dict):
                return parsed
    return {
        "kind": kind,
        "class": class_name,
        "polyphony": polyphony,
        "sample_index": 0,
        "phase": phase,
        "exact_error": exact_error,
        "win32_error": None,
        "cleanup_success": False,
        "cleanup_stuck_keys": [],
        "cleanup_verification_inconclusive": True,
        "raw_input_restore_failed": True,
        "pump_thread_failed": False,
    }


def _execute_native_bucket(
    binary: Path,
    *,
    kind: str,
    class_name: str,
    polyphony: int,
    samples: int,
    warmup_samples: int,
    timeout_seconds: float,
    progress: bool,
) -> dict[str, Any]:
    command = [
        str(binary),
        "--mode",
        "bucket",
        "--kind",
        kind,
        "--class",
        class_name,
        "--polyphony",
        str(polyphony),
        "--samples",
        str(samples),
        "--warmup-samples",
        str(warmup_samples),
    ]
    try:
        process = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            # The native process emits progress on stderr and its final JSON
            # on stdout.  Merging the streams lets this single owner surface
            # progress without creating another Python thread in the platform
            # boundary.
            stderr=subprocess.STDOUT,
            shell=False,
        )
    except OSError as exc:
        report = _native_failure_report(
            kind=kind,
            class_name=class_name,
            polyphony=polyphony,
            exact_error=f"could not start native calibration: {exc}",
        )
        raise NativeCalibrationError(str(exc), failure_report=report) from exc

    output_lines: list[str] = []
    started = time.monotonic()
    timed_out = False
    while process.poll() is None:
        stream = process.stdout
        line = stream.readline() if stream is not None else b""
        if line:
            text_line = line.decode("utf-8", errors="replace").rstrip()
            output_lines.append(text_line)
            if progress and text_line.startswith("[calibration]"):
                print(text_line, flush=True)
        if time.monotonic() - started >= timeout_seconds:
            timed_out = True
            process.kill()
            process.wait()
            break
    if process.stdout is not None:
        tail = process.stdout.read()
        if tail:
            for text_line in tail.decode("utf-8", errors="replace").splitlines():
                output_lines.append(text_line)
                if progress and text_line.startswith("[calibration]"):
                    print(text_line, flush=True)
    stderr = "\n".join(output_lines)
    if timed_out:
        report = _native_failure_report(
            kind=kind,
            class_name=class_name,
            polyphony=polyphony,
            exact_error="native calibration timed out",
            native_stderr=stderr,
        )
        raise NativeCalibrationError("native calibration timed out", failure_report=report)
    if len(stderr.encode("utf-8")) > MAX_NATIVE_CALIBRATION_STDOUT_BYTES:
        raise NativeCalibrationError("native calibration stdout exceeded the size limit")
    return_code = process.returncode
    if return_code != 0:
        detail = stderr.strip() or "native calibration exited without diagnostics"
        report = _native_failure_report(
            kind=kind,
            class_name=class_name,
            polyphony=polyphony,
            exact_error=f"native calibration failed ({return_code}): {detail}",
            native_stderr=stderr,
        )
        raise NativeCalibrationError(
            f"native calibration failed ({return_code}): {detail}", failure_report=report
        )
    json_start = next(
        (index for index, line in enumerate(output_lines) if line.strip() == "{"),
        None,
    )
    json_text = "\n".join(output_lines[json_start:]) if json_start is not None else ""
    try:
        parsed = json.loads(json_text)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        report = _native_failure_report(
            kind=kind,
            class_name=class_name,
            polyphony=polyphony,
            exact_error="native calibration stdout was not valid JSON",
            native_stderr=stderr,
        )
        raise NativeCalibrationError(
            "native calibration stdout was not valid JSON", failure_report=report
        ) from exc
    if not isinstance(parsed, dict):
        raise NativeCalibrationError("native calibration bucket output was not an object")
    return parsed


def _write_sha256(path: Path) -> str:
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    _write_text_atomically(path.with_suffix(path.suffix + ".sha256"), digest + "  " + path.name + "\n")
    return digest


def _read_sha256_sidecar(path: Path) -> str | None:
    try:
        parts = path.read_text(encoding="utf-8").split()
    except (OSError, UnicodeError):
        return None
    return parts[0] if parts else None


def _write_failure_report(
    path: Path,
    report: dict[str, Any],
    *,
    command: list[str],
    acceptance_eligible: bool = False,
) -> None:
    data = {
        "artifact_type": "native_calibration_failure_report",
        "schema_version": CALIBRATION_ARTIFACT_SCHEMA_VERSION,
        "acceptance_eligible": acceptance_eligible,
        "command": command,
        **report,
    }
    cleanup = {
        key: data.pop(key)
        for key in (
            "cleanup_success",
            "cleanup_stuck_keys",
            "cleanup_verification_inconclusive",
            "raw_input_restore_failed",
            "pump_thread_failed",
        )
        if key in data
    }
    data["cleanup"] = cleanup
    _write_json_atomically(path, data)


def _bucket_artifact(
    result: dict[str, Any],
    *,
    configuration: dict[str, Any],
    kind: str,
    class_name: str,
    polyphony: int,
    acceptance_eligible: bool,
) -> dict[str, Any]:
    return {
        "artifact_type": "native_calibration_bucket",
        "schema_version": CALIBRATION_ARTIFACT_SCHEMA_VERSION,
        "native_schema_version": result["version"],
        "measurement_protocol_version": result["measurement_protocol_version"],
        "acceptance_eligible": acceptance_eligible,
        "source_git_sha": result["source_git_sha"],
        "native_build_id": result["native_build_id"],
        "native_source_fingerprint": result["native_source_fingerprint"],
        "dirty_worktree": result["dirty_worktree"],
        "rustc_version": result["rustc_version"],
        "host_fingerprint": _stable_host_fingerprint(result["host_fingerprint"]),
        "configuration": configuration,
        "kind": kind,
        "class": class_name,
        "polyphony": polyphony,
        "attempted": result["attempted"],
        "setup_attempted": result["setup_attempted"],
        "setup_anomalous": result["setup_anomalous"],
        "setup_timed_out": result["setup_timed_out"],
        "warmup_attempted": result["warmup_attempted"],
        "warmup_anomalous": result["warmup_anomalous"],
        "warmup_timed_out": result["warmup_timed_out"],
        "total_attempted": result["total_attempted"],
        "total_anomalous": result["total_anomalous"],
        "total_timed_out": result["total_timed_out"],
        "bucket": result["bucket"],
        "cleanup": result["cleanup"],
    }


def _native_metadata(binary: Path) -> dict[str, Any]:
    try:
        completed = subprocess.run(
            [str(binary), "--metadata"],
            capture_output=True,
            text=True,
            check=False,
            timeout=10.0,
            shell=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise NativeCalibrationError(f"could not read native build metadata: {exc}") from exc
    if completed.returncode != 0:
        raise NativeCalibrationError(
            "native build metadata command failed: "
            + (completed.stderr.strip() or str(completed.returncode))
        )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise NativeCalibrationError("native build metadata was not valid JSON") from exc
    if not isinstance(value, dict):
        raise NativeCalibrationError("native build metadata was not an object")
    return value


def _current_full_provenance(binary: Path) -> dict[str, Any]:
    worktree = _current_worktree_provenance()
    if worktree["dirty_worktree"]:
        raise NativeCalibrationError("full calibration requires a clean worktree")
    native = _native_metadata(binary)
    for field in ("source_git_sha", "native_build_id", "native_source_fingerprint", "rustc_version"):
        if not isinstance(native.get(field), str) or not native[field].strip():
            raise NativeCalibrationError(f"native build metadata is missing {field}")
    if native["source_git_sha"] != worktree["source_git_sha"]:
        raise NativeCalibrationError("native build SHA does not match the current Git SHA")
    if native["native_build_id"] != worktree["source_git_sha"]:
        raise NativeCalibrationError("native build ID does not match the current Git SHA")
    if native["native_source_fingerprint"] != worktree["native_source_fingerprint"]:
        raise NativeCalibrationError("native source fingerprint does not match the current source")
    if native["rustc_version"] != _rustc_version():
        raise NativeCalibrationError("rustc version does not match the native build")
    host = _stable_host_fingerprint(native.get("host_fingerprint"))
    return {
        **worktree,
        "native_build_id": native["native_build_id"],
        "rustc_version": native["rustc_version"],
        "host_fingerprint": host,
    }


def _artifact_provenance(artifact: dict[str, Any]) -> dict[str, Any]:
    return {
        field: artifact[field]
        for field in (
            "source_git_sha",
            "native_build_id",
            "native_source_fingerprint",
            "dirty_worktree",
            "rustc_version",
            "host_fingerprint",
        )
    }


def _checkpoint_manifest(configuration: dict[str, Any]) -> dict[str, Any]:
    return {
        "artifact_type": "native_calibration_checkpoint",
        "schema_version": CALIBRATION_ARTIFACT_SCHEMA_VERSION,
        "measurement_protocol_version": SUPPORTED_MEASUREMENT_PROTOCOL_VERSION,
        "acceptance_eligible": False,
        "configuration": configuration,
        "provenance": None,
        "buckets": [],
    }


def _checkpoint_path(checkpoint_dir: Path) -> Path:
    return checkpoint_dir / "checkpoint.json"


def _load_json_file(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise NativeCalibrationError(f"could not read JSON artifact {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise NativeCalibrationError(f"JSON artifact {path} was not an object")
    return value


def _validate_checkpoint_manifest(
    manifest: dict[str, Any],
    *,
    checkpoint_dir: Path,
    configuration: dict[str, Any],
    provenance: dict[str, Any],
) -> dict[str, dict[str, Any]]:
    if manifest.get("artifact_type") != "native_calibration_checkpoint":
        raise NativeCalibrationError("checkpoint manifest has the wrong artifact type")
    if manifest.get("schema_version") != CALIBRATION_ARTIFACT_SCHEMA_VERSION:
        raise NativeCalibrationError("checkpoint schema version mismatch")
    if manifest.get("measurement_protocol_version") != SUPPORTED_MEASUREMENT_PROTOCOL_VERSION:
        raise NativeCalibrationError("checkpoint measurement protocol mismatch")
    if manifest.get("configuration") != configuration:
        raise NativeCalibrationError("checkpoint configuration mismatch")
    if manifest.get("provenance") != provenance:
        raise NativeCalibrationError("checkpoint provenance mismatch")
    entries = manifest.get("buckets")
    if not isinstance(entries, list):
        raise NativeCalibrationError("checkpoint bucket list is invalid")
    expected = {_bucket_key(*key) for key in calibration_bucket_keys()}
    found: dict[str, dict[str, Any]] = {}
    for entry in entries:
        if not isinstance(entry, dict):
            raise NativeCalibrationError("checkpoint contains a non-object bucket entry")
        key = entry.get("key")
        if not isinstance(key, str) or key not in expected or key in found:
            raise NativeCalibrationError("checkpoint contains a duplicate or unknown bucket")
        relative = entry.get("artifact")
        digest = entry.get("sha256")
        if not isinstance(relative, str) or Path(relative).is_absolute() or ".." in Path(relative).parts:
            raise NativeCalibrationError("checkpoint artifact path is unsafe")
        if not isinstance(digest, str) or len(digest) != 64:
            raise NativeCalibrationError("checkpoint artifact SHA-256 is invalid")
        artifact_path = checkpoint_dir / relative
        if not artifact_path.is_file():
            raise NativeCalibrationError(f"checkpoint artifact is missing: {relative}")
        actual = hashlib.sha256(artifact_path.read_bytes()).hexdigest()
        if actual != digest:
            raise NativeCalibrationError(f"checkpoint artifact SHA-256 mismatch: {relative}")
        sha_path = artifact_path.with_suffix(artifact_path.suffix + ".sha256")
        if not sha_path.is_file() or _read_sha256_sidecar(sha_path) != digest:
            raise NativeCalibrationError(f"checkpoint artifact SHA-256 sidecar mismatch: {relative}")
        artifact = _load_json_file(artifact_path)
        kind, polyphony_text, class_name = key.split("/")
        polyphony = int(polyphony_text)
        if artifact.get("artifact_type") != "native_calibration_bucket":
            raise NativeCalibrationError(f"checkpoint artifact {key} has the wrong type")
        if artifact.get("acceptance_eligible") is not True:
            raise NativeCalibrationError(f"checkpoint artifact {key} is not acceptance eligible")
        if (
            artifact.get("kind"),
            artifact.get("polyphony"),
            artifact.get("class"),
        ) != (kind, polyphony, class_name):
            raise NativeCalibrationError(f"checkpoint artifact {key} identity mismatch")
        if artifact.get("configuration") != configuration:
            raise NativeCalibrationError(f"checkpoint artifact {key} configuration mismatch")
        if _artifact_provenance(artifact) != provenance:
            raise NativeCalibrationError(f"checkpoint artifact {key} provenance mismatch")
        found[key] = artifact
    return found


def run_diagnostic_calibration(
    *,
    kind: str,
    class_name: str,
    polyphony: int,
    samples: int,
    output_path: Path | str = ".cache/calibration-diagnostic.json",
    failure_report_path: Path | str | None = None,
    timeout_seconds: float | None = None,
) -> dict[str, Any]:
    if kind not in CALIBRATION_KINDS:
        raise NativeCalibrationError("kind must be down or up")
    if class_name not in CALIBRATION_CLASSES:
        raise NativeCalibrationError("class must be hot or cold")
    if not isinstance(polyphony, int) or isinstance(polyphony, bool) or not 1 <= polyphony <= 15:
        raise NativeCalibrationError("polyphony must be an integer from 1 through 15")
    if not isinstance(samples, int) or isinstance(samples, bool) or not 1 <= samples <= FULL_SAMPLE_COUNT:
        raise NativeCalibrationError("samples must be an integer from 1 through 5000")
    if timeout_seconds is None:
        timeout_seconds = QUICK_CALIBRATION_TIMEOUT_SECONDS
    if not isinstance(timeout_seconds, (int, float)) or isinstance(timeout_seconds, bool):
        raise NativeCalibrationError("timeout_seconds must be a finite positive number")
    if not math.isfinite(float(timeout_seconds)) or timeout_seconds <= 0:
        raise NativeCalibrationError("timeout_seconds must be a finite positive number")

    output = Path(output_path)
    report_path = Path(failure_report_path) if failure_report_path is not None else output.with_suffix(
        output.suffix + ".failure.json"
    )
    binary: Path | None = None
    command = [
        "native_calibration.exe",
        "--mode",
        "bucket",
        "--kind",
        kind,
        "--class",
        class_name,
        "--polyphony",
        str(polyphony),
        "--samples",
        str(samples),
        "--warmup-samples",
        "1",
    ]
    try:
        binary = _find_binary()
        command[0] = str(binary)
        result = _execute_native_bucket(
            binary,
            kind=kind,
            class_name=class_name,
            polyphony=polyphony,
            samples=samples,
            # One setup cycle seeds the previous exact SendInput completion so
            # the first measured cold gap has a real classification anchor.
            warmup_samples=1,
            timeout_seconds=float(timeout_seconds),
            progress=True,
        )
        _validate_bucket_result(
            result,
            kind=kind,
            class_name=class_name,
            polyphony=polyphony,
            samples=samples,
        )
        configuration = {
            "polyphonies": [polyphony],
            "samples_per_hot_bucket": samples,
            "samples_per_cold_bucket": samples,
            "warmup_samples": 0,
            "receipt_timeout_ms": 200,
            "hot_gap_target_us": 5_000,
            "cold_idle_gap_us": FULL_COLD_IDLE_GAP_US,
            "cold_threshold_us": 20_000,
        }
        artifact = _bucket_artifact(
            result,
            configuration=configuration,
            kind=kind,
            class_name=class_name,
            polyphony=polyphony,
            acceptance_eligible=False,
        )
        _write_json_atomically(output, artifact)
        _write_sha256(output)
        return artifact
    except NativeCalibrationError as exc:
        report = exc.failure_report or _native_failure_report(
            kind=kind,
            class_name=class_name,
            polyphony=polyphony,
            exact_error=str(exc),
        )
        _write_failure_report(report_path, report, command=command)
        raise


def run_full_calibration(
    *,
    checkpoint_dir: Path | str = ".cache/calibration-full",
    resume: bool = False,
    timeout_seconds: float | None = None,
) -> dict[str, Any]:
    configuration = full_calibration_configuration()
    checkpoint = Path(checkpoint_dir)
    manifest_path = _checkpoint_path(checkpoint)
    binary = _find_binary()
    provenance = _current_full_provenance(binary)
    if timeout_seconds is None:
        timeout_seconds = FULL_CALIBRATION_TIMEOUT_SECONDS
    if not isinstance(timeout_seconds, (int, float)) or isinstance(timeout_seconds, bool):
        raise NativeCalibrationError("timeout_seconds must be a finite positive number")
    if not math.isfinite(float(timeout_seconds)) or timeout_seconds <= 0:
        raise NativeCalibrationError("timeout_seconds must be a finite positive number")

    if resume:
        if not manifest_path.is_file():
            raise NativeCalibrationError("--resume requested but checkpoint manifest is missing")
        manifest = _load_json_file(manifest_path)
        completed = _validate_checkpoint_manifest(
            manifest,
            checkpoint_dir=checkpoint,
            configuration=configuration,
            provenance=provenance,
        )
    else:
        if manifest_path.exists():
            raise NativeCalibrationError(
                "checkpoint already exists; use --resume or choose a new checkpoint directory"
            )
        checkpoint.mkdir(parents=True, exist_ok=True)
        manifest = _checkpoint_manifest(configuration)
        manifest["provenance"] = provenance
        completed = {}
        _write_json_atomically(manifest_path, manifest)

    for kind, polyphony, class_name in calibration_bucket_keys():
        key = _bucket_key(kind, polyphony, class_name)
        if key in completed:
            continue
        failure_path = checkpoint / "failures" / f"{kind}-{polyphony}-{class_name}.json"
        command = [
            str(binary),
            "--mode",
            "bucket",
            "--kind",
            kind,
            "--class",
            class_name,
            "--polyphony",
            str(polyphony),
            "--samples",
            str(FULL_SAMPLE_COUNT),
            "--warmup-samples",
            str(FULL_WARMUP_SAMPLES),
        ]
        try:
            raw = _execute_native_bucket(
                binary,
                kind=kind,
                class_name=class_name,
                polyphony=polyphony,
                samples=FULL_SAMPLE_COUNT,
                warmup_samples=FULL_WARMUP_SAMPLES,
                timeout_seconds=float(timeout_seconds),
                progress=False,
            )
            _validate_bucket_result(
                raw,
                kind=kind,
                class_name=class_name,
                polyphony=polyphony,
                samples=FULL_SAMPLE_COUNT,
            )
            artifact = _bucket_artifact(
                raw,
                configuration=configuration,
                kind=kind,
                class_name=class_name,
                polyphony=polyphony,
                acceptance_eligible=True,
            )
            artifact_path = checkpoint / "buckets" / f"{kind}-{polyphony}-{class_name}.json"
            _write_json_atomically(artifact_path, artifact)
            digest = _write_sha256(artifact_path)
        except NativeCalibrationError as exc:
            report = exc.failure_report or _native_failure_report(
                kind=kind,
                class_name=class_name,
                polyphony=polyphony,
                exact_error=str(exc),
            )
            _write_failure_report(failure_path, report, command=command)
            raise

        manifest["provenance"] = provenance
        entries = [entry for entry in manifest["buckets"] if entry["key"] != key]
        entries.append(
            {
                "key": key,
                "artifact": str(artifact_path.relative_to(checkpoint)).replace("\\", "/"),
                "sha256": digest,
            }
        )
        manifest["buckets"] = entries
        _write_json_atomically(manifest_path, manifest)
        completed[key] = artifact

    return {
        "artifact_type": "native_calibration_checkpoint",
        "checkpoint_dir": str(checkpoint),
        "completed_buckets": len(completed),
        "total_buckets": len(calibration_bucket_keys()),
        "acceptance_eligible": False,
    }


def _finalizer_artifact(
    manifest: dict[str, Any], artifacts: dict[str, dict[str, Any]]
) -> dict[str, Any]:
    configuration = full_calibration_configuration()
    expected_keys = {_bucket_key(*key) for key in calibration_bucket_keys()}
    if set(artifacts) != expected_keys:
        raise NativeCalibrationError("finalizer requires exactly 24 known buckets")
    provenance: dict[str, Any] | None = None
    nested: dict[str, dict[str, dict[str, Any]]] = {"down": {}, "up": {}}
    totals = {
        "measured_attempted": 0,
        "setup_attempted": 0,
        "setup_anomalous": 0,
        "setup_timed_out": 0,
        "warmup_attempted": 0,
        "warmup_anomalous": 0,
        "warmup_timed_out": 0,
        "measured_anomalous": 0,
        "measured_timed_out": 0,
        "measured_class_mismatch": 0,
        "total_attempted": 0,
        "total_anomalous": 0,
        "total_timed_out": 0,
    }
    for kind, polyphony, class_name in calibration_bucket_keys():
        key = _bucket_key(kind, polyphony, class_name)
        artifact = artifacts[key]
        if artifact.get("acceptance_eligible") is not True:
            raise NativeCalibrationError(f"finalizer rejects ineligible bucket {key}")
        if artifact.get("configuration") != configuration:
            raise NativeCalibrationError(f"finalizer configuration mismatch in {key}")
        if _artifact_provenance(artifact) != (provenance or _artifact_provenance(artifact)):
            raise NativeCalibrationError(f"finalizer provenance mismatch in {key}")
        provenance = provenance or _artifact_provenance(artifact)
        if artifact.get("dirty_worktree") is not False:
            raise NativeCalibrationError("finalizer rejects dirty provenance")
        bucket = _require_mapping(artifact.get("bucket"), f"{key}.bucket")
        if bucket.get("attempted") != FULL_SAMPLE_COUNT or bucket.get("sample_count") != FULL_SAMPLE_COUNT:
            raise NativeCalibrationError(f"finalizer requires 5000 samples in {key}")
        if bucket.get("clean_sample_count", 0) < MIN_CALIBRATION_SAMPLE_COUNT:
            raise NativeCalibrationError(f"finalizer has too few clean samples in {key}")
        cleanup = _require_mapping(artifact.get("cleanup"), f"{key}.cleanup")
        if (
            cleanup.get("cleanup_success") is not True
            or cleanup.get("cleanup_verification_inconclusive") is not False
            or cleanup.get("raw_input_restore_failed") is not False
            or cleanup.get("pump_thread_failed") is not False
        ):
            raise NativeCalibrationError(f"finalizer rejects unsuccessful cleanup in {key}")
        nested[kind].setdefault(str(polyphony), {})[class_name] = bucket
        totals["measured_attempted"] += int(artifact["attempted"])
        totals["setup_attempted"] += int(artifact["setup_attempted"])
        totals["setup_anomalous"] += int(artifact["setup_anomalous"])
        totals["setup_timed_out"] += int(artifact["setup_timed_out"])
        totals["warmup_attempted"] += int(artifact["warmup_attempted"])
        totals["warmup_anomalous"] += int(artifact["warmup_anomalous"])
        totals["warmup_timed_out"] += int(artifact["warmup_timed_out"])
        totals["measured_anomalous"] += int(bucket["anomaly_count"])
        totals["measured_timed_out"] += int(bucket["timeout_count"])
        totals["measured_class_mismatch"] += int(bucket["class_mismatch_count"])
        totals["total_attempted"] += int(artifact["total_attempted"])
        totals["total_anomalous"] += int(artifact["total_anomalous"])
        totals["total_timed_out"] += int(artifact["total_timed_out"])

    expected_measured = len(expected_keys) * FULL_SAMPLE_COUNT
    expected_warmup = len(expected_keys) * FULL_WARMUP_SAMPLES
    expected_setup = 2 * len(FULL_POLYPHONIES) * (FULL_SAMPLE_COUNT + FULL_WARMUP_SAMPLES)
    if totals["measured_attempted"] != expected_measured:
        raise NativeCalibrationError("finalizer measured aggregate total mismatch")
    if totals["warmup_attempted"] != expected_warmup:
        raise NativeCalibrationError("finalizer warmup aggregate total mismatch")
    if totals["setup_attempted"] != expected_setup:
        raise NativeCalibrationError("finalizer setup aggregate total mismatch")
    if totals["total_attempted"] != expected_measured + expected_warmup + expected_setup:
        raise NativeCalibrationError("finalizer total aggregate mismatch")
    if provenance is None:
        raise NativeCalibrationError("finalizer has no provenance")
    if manifest.get("provenance") != provenance:
        raise NativeCalibrationError("finalizer manifest provenance mismatch")

    final = {
        "artifact_type": "native_calibration_final",
        "acceptance_eligible": True,
        "version": SUPPORTED_NATIVE_CALIBRATION_VERSION,
        "schema_version": CALIBRATION_ARTIFACT_SCHEMA_VERSION,
        "measurement_protocol_version": SUPPORTED_MEASUREMENT_PROTOCOL_VERSION,
        "evidence_kind": "injected_raw_input_delivery_proxy",
        "source_git_sha": provenance["source_git_sha"],
        "native_build_id": provenance["native_build_id"],
        "native_source_fingerprint": provenance["native_source_fingerprint"],
        "dirty_worktree": provenance["dirty_worktree"],
        "rustc_version": provenance["rustc_version"],
        "host_fingerprint": {**provenance["host_fingerprint"], "sampled_at_us": 1},
        "configuration": configuration,
        "buckets": nested,
        **totals,
        "cleanup": {
            "cleanup_attempted": True,
            "cleanup_success": True,
            "cleanup_stuck_keys": [],
            "cleanup_verification_inconclusive": False,
            "raw_input_restore_failed": False,
            "pump_thread_failed": False,
        },
    }
    final["total_anomalous"] = totals["warmup_anomalous"] + totals["measured_anomalous"] + totals["setup_anomalous"]
    final["total_timed_out"] = totals["warmup_timed_out"] + totals["measured_timed_out"] + totals["setup_timed_out"]
    _validate_result(final)
    return final


def finalize_native_calibration(
    *,
    checkpoint_dir: Path | str,
    output_path: Path | str,
    cache_path: Path | str,
) -> dict[str, Any]:
    checkpoint = Path(checkpoint_dir)
    manifest = _load_json_file(_checkpoint_path(checkpoint))
    configuration = full_calibration_configuration()
    if manifest.get("configuration") != configuration:
        raise NativeCalibrationError("finalizer configuration mismatch")
    entries = manifest.get("buckets")
    if not isinstance(entries, list) or len(entries) != len(calibration_bucket_keys()):
        raise NativeCalibrationError("finalizer requires exactly 24 bucket entries")
    artifacts: dict[str, dict[str, Any]] = {}
    expected_keys = {_bucket_key(*key) for key in calibration_bucket_keys()}
    for entry in entries:
        if not isinstance(entry, dict):
            raise NativeCalibrationError("finalizer found a malformed bucket entry")
        key = entry.get("key")
        if not isinstance(key, str) or key not in expected_keys or key in artifacts:
            raise NativeCalibrationError("finalizer found a duplicate or unknown bucket")
        relative = entry.get("artifact")
        digest = entry.get("sha256")
        if not isinstance(relative, str) or Path(relative).is_absolute() or ".." in Path(relative).parts:
            raise NativeCalibrationError("finalizer found an unsafe artifact path")
        artifact_path = checkpoint / relative
        sha_path = artifact_path.with_suffix(artifact_path.suffix + ".sha256")
        if (
            not artifact_path.is_file()
            or not isinstance(digest, str)
            or hashlib.sha256(artifact_path.read_bytes()).hexdigest() != digest
            or not sha_path.is_file()
            or _read_sha256_sidecar(sha_path) != digest
        ):
            raise NativeCalibrationError(f"finalizer artifact hash mismatch: {key}")
        artifacts[key] = _load_json_file(artifact_path)
    if set(artifacts) != expected_keys:
        raise NativeCalibrationError("finalizer bucket set is incomplete")
    final = _finalizer_artifact(manifest, artifacts)
    _write_json_atomically(Path(output_path), final)
    _write_json_atomically(Path(cache_path), _legacy_cache(final))
    return final


def run_native_calibration(
    *,
    mode: str = "quick",
    output_path: Path | str | None = None,
    cache_path: Path | str = ".cache/input_latency.json",
    timeout_seconds: float | None = None,
    checkpoint_dir: Path | str = ".cache/calibration-full",
    resume: bool = False,
    kind: str | None = None,
    class_name: str | None = None,
    polyphony: int | None = None,
    samples: int | None = None,
    failure_report_path: Path | str | None = None,
) -> dict[str, Any]:
    """Run the dedicated calibration process and return validated raw JSON."""

    if mode == "diagnostic":
        if kind is None or class_name is None or polyphony is None or samples is None:
            raise NativeCalibrationError(
                "diagnostic mode requires kind, class_name, polyphony, and samples"
            )
        return run_diagnostic_calibration(
            kind=kind,
            class_name=class_name,
            polyphony=polyphony,
            samples=samples,
            output_path=output_path or ".cache/calibration-diagnostic.json",
            failure_report_path=failure_report_path,
            timeout_seconds=timeout_seconds,
        )
    if mode == "full":
        return run_full_calibration(
            checkpoint_dir=checkpoint_dir,
            resume=resume,
            timeout_seconds=timeout_seconds,
        )
    if mode != "quick":
        raise NativeCalibrationError("mode must be diagnostic, quick, or full")
    if timeout_seconds is None:
        timeout_seconds = (
            FULL_CALIBRATION_TIMEOUT_SECONDS
            if mode == "full"
            else QUICK_CALIBRATION_TIMEOUT_SECONDS
        )
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


__all__ = [
    "NativeCalibrationError",
    "calibration_bucket_keys",
    "finalize_native_calibration",
    "full_calibration_configuration",
    "run_diagnostic_calibration",
    "run_full_calibration",
    "run_native_calibration",
]
