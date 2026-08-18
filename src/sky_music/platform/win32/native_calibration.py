"""Process-isolated protocol-v4 input-delivery calibration adapter.

The native process owns the Raw Input window and emits signed paired evidence.
This adapter validates the complete result before writing an artifact or the
version-3 production cache. Diagnostic runs may be saved as reports, never as
production cache.
"""

from __future__ import annotations

import hashlib
import importlib
import json
import math
import os
import subprocess
import sys
import time
from dataclasses import dataclass
from dataclasses import field as dataclass_field
from pathlib import Path
from typing import Any

from sky_music.config import DEFAULT_GAME_FPS
from sky_music.domain.hold_timing import (
    DEFAULT_HOLD_FRAMES,
    materialize_hold_us,
)
from sky_music.infrastructure.calibration_loader import (
    MARGIN_CEILING_US,
    MARGIN_FLOOR_US,
    MARGIN_GUARD_US,
    MIN_CALIBRATION_SAMPLE_COUNT,
    REQUIRED_BUCKETS,
    CalibrationCacheSummary,
    CalibrationStatus,
    PairBucketSummary,
    parse_calibration_cache_summary,
    qualify_calibration_margin,
)

SUPPORTED_NATIVE_CALIBRATION_VERSION = 9
SUPPORTED_MEASUREMENT_PROTOCOL_VERSION = 4
CALIBRATION_ARTIFACT_SCHEMA_VERSION = 6
MAX_CALIBRATION_BUDGET_SECONDS = 120
PUBLICATION_RESERVE_SECONDS = 5.0
NATIVE_CLEANUP_RESERVE_SECONDS = 5
MIN_NATIVE_MEASUREMENT_SECONDS = 1
MIN_NATIVE_TOTAL_BUDGET_SECONDS = (
    NATIVE_CLEANUP_RESERVE_SECONDS + MIN_NATIVE_MEASUREMENT_SECONDS
)
MIN_FULL_CALIBRATION_TIMEOUT_SECONDS = (
    PUBLICATION_RESERVE_SECONDS + MIN_NATIVE_TOTAL_BUDGET_SECONDS
)
FULL_POLYPHONIES = (1, 5, 15)
FULL_SAMPLE_COUNT = 100
HOT_GAP_TARGET_US = 5_000
COLD_THRESHOLD_US = 20_000
FULL_COLD_IDLE_GAP_US = 25_000
FULL_WARMUP_SAMPLES = 4
FULL_CHUNK_SAMPLES = 25
CALIBRATION_CLASSES = ("hot", "cold")
MAX_DIAGNOSTIC_SAMPLES = 5_000
MAX_NATIVE_CALIBRATION_STDOUT_BYTES = 8 * 1024 * 1024
QUICK_CALIBRATION_TIMEOUT_SECONDS = 120.0
FULL_CALIBRATION_TIMEOUT_SECONDS = 120.0
NATIVE_RECEIPT_TIMEOUT_MS = 200


@dataclass(frozen=True, slots=True)
class PublishedCalibrationResult:
    status: CalibrationStatus
    margin_us: int | None
    candidate_margin_us: int
    source: str
    sample_count: int
    cache_path: Path
    evidence_kind: str
    source_git_sha: str
    native_build_id: str
    pair_buckets: dict[str, PairBucketSummary] = dataclass_field(default_factory=dict)
    worst_bucket: str = ""
    global_shrink_p99_us: int = 0
    guard_us: int = MARGIN_GUARD_US
    ceiling_us: int = MARGIN_CEILING_US
    effective_min_hold_us: int | None = None


class NativeCalibrationError(RuntimeError):
    """Calibration failed or returned evidence that cannot be trusted."""

    def __init__(
        self, message: str, *, failure_report: dict[str, Any] | None = None
    ) -> None:
        super().__init__(message)
        self.failure_report = failure_report


def _require_mapping(value: object, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise NativeCalibrationError(f"calibration field {name!r} is not an object")
    return value


def _int(value: object, name: str, *, minimum: int = 0) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < minimum:
        raise NativeCalibrationError(f"native calibration has invalid {name}")
    return value


def _finite_timeout(value: float | int | None, *, default: float) -> float:
    actual = default if value is None else value
    if (
        not isinstance(actual, (int, float))
        or isinstance(actual, bool)
        or not math.isfinite(float(actual))
        or float(actual) <= 0
        or float(actual) > MAX_CALIBRATION_BUDGET_SECONDS
    ):
        raise NativeCalibrationError("timeout_seconds must be a finite value in (0, 120]")
    if float(actual) < MIN_NATIVE_TOTAL_BUDGET_SECONDS:
        raise NativeCalibrationError(
            f"timeout_seconds must provide at least {MIN_NATIVE_TOTAL_BUDGET_SECONDS}s"
        )
    return float(actual)


def _native_child_budget(run_deadline: float, now: float) -> tuple[int, float]:
    remaining = run_deadline - now - PUBLICATION_RESERVE_SECONDS
    budget = math.floor(remaining)
    if budget < MIN_NATIVE_TOTAL_BUDGET_SECONDS:
        raise NativeCalibrationError("global calibration budget cannot provide native cleanup reserve")
    return min(MAX_CALIBRATION_BUDGET_SECONDS, budget), remaining


def _candidate_binaries() -> list[Path]:
    candidates: list[Path] = []
    configured = os.environ.get("SKY_NATIVE_CALIBRATION_BIN")
    if configured:
        candidates.append(Path(configured))
    root = Path(__file__).resolve().parents[4]
    for build_dir in ("debug", "release"):
        candidates.extend(
            [
                root / "rust" / "target" / build_dir / "native_calibration.exe",
                root / "rust" / "target" / build_dir / "native_calibration",
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
    raise NativeCalibrationError(
        "native_calibration executable was not found; build the Rust binary or set "
        "SKY_NATIVE_CALIBRATION_BIN"
    )


def _validate_quantiles(value: object, name: str) -> dict[str, int]:
    raw = _require_mapping(value, name)
    fields = ("min", "p50", "p90", "p95", "p99", "max", "mean")
    values = {field: _int(raw.get(field), f"{name}.{field}", minimum=-100_000) for field in fields}
    ordered = [values[field] for field in fields[:-1]]
    if ordered != sorted(ordered) or any(abs(item) > 100_000 for item in ordered):
        raise NativeCalibrationError(f"{name} has invalid signed quantile ordering")
    return values


def _validate_pair_bucket(
    bucket_value: object,
    name: str,
    expected_attempts: int,
    *,
    require_quantiles: bool = True,
) -> dict[str, Any]:
    bucket = _require_mapping(bucket_value, name)
    attempted = _int(bucket.get("attempted"), f"{name}.attempted")
    clean = _int(bucket.get("clean"), f"{name}.clean")
    clean_count = _int(bucket.get("clean_sample_count"), f"{name}.clean_sample_count")
    rejected = _int(bucket.get("rejected"), f"{name}.rejected")
    if attempted != expected_attempts or bucket.get("sample_count") != attempted:
        raise NativeCalibrationError(f"{name} has an unexpected attempt count")
    if clean != clean_count or clean + rejected != attempted:
        raise NativeCalibrationError(f"{name} clean/rejected totals are inconsistent")
    for field in (
        "timeout_count",
        "anomaly_count",
        "class_mismatch_count",
        "partial_send",
        "error_count",
    ):
        if _int(bucket.get(field), f"{name}.{field}") > rejected:
            raise NativeCalibrationError(f"{name}.{field} exceeds rejected pairs")
    pair_quantiles = bucket.get("pair_worst_shrink_us")
    if pair_quantiles is None and not require_quantiles:
        return bucket
    _validate_quantiles(pair_quantiles, f"{name}.pair_worst_shrink_us")
    return bucket


def _validate_common_metadata(data: dict[str, Any]) -> None:
    if data.get("version") != SUPPORTED_NATIVE_CALIBRATION_VERSION:
        raise NativeCalibrationError("unsupported native calibration schema version")
    if data.get("measurement_protocol_version") != SUPPORTED_MEASUREMENT_PROTOCOL_VERSION:
        raise NativeCalibrationError("unsupported native calibration measurement protocol")
    if data.get("evidence_kind") != "injected_raw_input_delivery_proxy":
        raise NativeCalibrationError("native calibration evidence kind is not the expected proxy")
    for name in ("source_git_sha", "native_build_id", "native_source_fingerprint", "rustc_version"):
        value = data.get(name)
        if not isinstance(value, str) or not value.strip() or value == "unknown":
            raise NativeCalibrationError(f"native calibration has invalid {name}")
    if data["source_git_sha"] != data["native_build_id"]:
        raise NativeCalibrationError("native calibration source/build SHA mismatch")
    if data.get("dirty_worktree") is not False:
        raise NativeCalibrationError("native calibration worktree is not clean")
    if getattr(sys, "frozen", False):
        try:
            native_build = importlib.import_module("sky_music._native_build")
            expected_build_id = getattr(native_build, "APP_BUILD_COMMIT", "")
        except (ImportError, AttributeError) as exc:
            raise NativeCalibrationError(
                "frozen release is missing native calibration provenance metadata"
            ) from exc
        if not isinstance(expected_build_id, str) or not expected_build_id:
            raise NativeCalibrationError(
                "frozen release has invalid native calibration provenance metadata"
            )
        if data["native_build_id"] != expected_build_id:
            raise NativeCalibrationError(
                "native calibration build does not match the frozen release"
            )
    host = _require_mapping(data.get("host_fingerprint"), "host_fingerprint")
    _int(host.get("qpc_frequency_hz"), "host_fingerprint.qpc_frequency_hz", minimum=1)
    if not isinstance(host.get("win32_build"), str) or not host["win32_build"].strip():
        raise NativeCalibrationError("native calibration has incomplete Windows fingerprint")
    cleanup = _require_mapping(data.get("cleanup"), "cleanup")
    for field, expected in (
        ("cleanup_success", True),
        ("cleanup_verification_inconclusive", False),
        ("raw_input_restore_failed", False),
        ("pump_thread_failed", False),
    ):
        if cleanup.get(field) is not expected:
            raise NativeCalibrationError(f"native calibration cleanup field {field} failed")


def _validate_configuration(
    data: dict[str, Any], *, expected_polyphonies: tuple[int, ...], expected_samples: int
) -> None:
    config = _require_mapping(data.get("configuration"), "configuration")
    if config.get("polyphonies") != list(expected_polyphonies):
        raise NativeCalibrationError("native calibration polyphony configuration mismatch")
    expected = {
        "samples_per_hot_bucket": expected_samples,
        "samples_per_cold_bucket": expected_samples,
        "warmup_samples": FULL_WARMUP_SAMPLES,
        "receipt_timeout_ms": NATIVE_RECEIPT_TIMEOUT_MS,
        "hot_gap_target_us": HOT_GAP_TARGET_US,
        "cold_threshold_us": COLD_THRESHOLD_US,
        "cold_idle_gap_us": FULL_COLD_IDLE_GAP_US,
    }
    for field, wanted in expected.items():
        if config.get(field) != wanted:
            raise NativeCalibrationError(f"native calibration configuration {field} mismatch")
    budget = _int(config.get("budget_seconds"), "configuration.budget_seconds", minimum=6)
    if budget > MAX_CALIBRATION_BUDGET_SECONDS:
        raise NativeCalibrationError("native calibration budget exceeds global limit")


def _validate_result(
    result: object,
    *,
    expected_budget_seconds: int | None = None,
    expected_samples: int = FULL_SAMPLE_COUNT,
) -> dict[str, Any]:
    data = _require_mapping(result, "root")
    _validate_common_metadata(data)
    _validate_configuration(data, expected_polyphonies=FULL_POLYPHONIES, expected_samples=expected_samples)
    config = _require_mapping(data["configuration"], "configuration")
    if expected_budget_seconds is not None and config.get("budget_seconds") != expected_budget_seconds:
        raise NativeCalibrationError("native calibration budget provenance mismatch")
    raw_buckets = data.get("pair_buckets")
    if not isinstance(raw_buckets, dict):
        raise NativeCalibrationError("native calibration pair_buckets is missing")
    actual_keys: set[str] = set()
    for polyphony in FULL_POLYPHONIES:
        classes = _require_mapping(raw_buckets.get(str(polyphony)), f"pair_buckets.{polyphony}")
        if set(classes) != set(CALIBRATION_CLASSES):
            raise NativeCalibrationError(f"pair_buckets.{polyphony} class matrix is incomplete")
        for class_name in CALIBRATION_CLASSES:
            key = f"{polyphony}/{class_name}"
            actual_keys.add(key)
            _validate_pair_bucket(classes[class_name], key, expected_samples)
    if actual_keys != set(REQUIRED_BUCKETS):
        raise NativeCalibrationError("native calibration pair matrix is incomplete")
    counts = {
        name: _int(data.get(name), name)
        for name in (
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
    }
    if counts["measured_attempted"] != len(REQUIRED_BUCKETS) * expected_samples:
        raise NativeCalibrationError("native calibration measured attempt total is inconsistent")
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
        if counts[f"{scope}_anomalous"] > counts[f"{scope}_attempted"]:
            raise NativeCalibrationError(f"native calibration {scope}_anomalous exceeds attempts")
        if counts[f"{scope}_timed_out"] > counts[f"{scope}_attempted"]:
            raise NativeCalibrationError(f"native calibration {scope}_timed_out exceeds attempts")
    if counts["measured_class_mismatch"] > counts["measured_attempted"]:
        raise NativeCalibrationError("native calibration measured_class_mismatch exceeds attempts")
    if counts["setup_attempted"] != 0:
        raise NativeCalibrationError("protocol-v4 must not publish directional setup attempts")
    bucket_totals = {"anomaly_count": 0, "timeout_count": 0, "class_mismatch_count": 0}
    for polyphony in FULL_POLYPHONIES:
        classes = _require_mapping(raw_buckets[str(polyphony)], f"pair_buckets.{polyphony}")
        for class_name in CALIBRATION_CLASSES:
            bucket = _require_mapping(
                classes[class_name], f"pair_buckets.{polyphony}.{class_name}"
            )
            for name in bucket_totals:
                bucket_totals[name] += _int(bucket.get(name), name)
    if bucket_totals["anomaly_count"] != counts["measured_anomalous"]:
        raise NativeCalibrationError("pair anomaly total does not match measured_anomalous")
    if bucket_totals["timeout_count"] != counts["measured_timed_out"]:
        raise NativeCalibrationError("pair timeout total does not match measured_timed_out")
    if bucket_totals["class_mismatch_count"] != counts["measured_class_mismatch"]:
        raise NativeCalibrationError("pair class mismatch total does not match measured_class_mismatch")
    return data


def _validate_pair_bucket_result(
    result: object,
    *,
    class_name: str,
    polyphony: int,
    samples: int,
    warmup_samples: int,
    budget_seconds: int,
) -> dict[str, Any]:
    data = _require_mapping(result, "pair bucket root")
    _validate_common_metadata(data)
    if data.get("class") != class_name or data.get("polyphony") != polyphony:
        raise NativeCalibrationError("native pair bucket identity does not match request")
    _validate_configuration(data, expected_polyphonies=(polyphony,), expected_samples=samples)
    config = _require_mapping(data["configuration"], "configuration")
    if config.get("warmup_samples") != warmup_samples or config.get("budget_seconds") != budget_seconds:
        raise NativeCalibrationError("native pair bucket configuration provenance mismatch")
    if data.get("attempted_pairs") != samples:
        raise NativeCalibrationError("native pair bucket has the wrong attempt count")
    _validate_pair_bucket(
        data.get("pair_bucket"), "pair_bucket", samples, require_quantiles=False
    )
    if not isinstance(data.get("worst_pairs"), list) or len(data["worst_pairs"]) > 16:
        raise NativeCalibrationError("native pair evidence is not bounded")
    if not isinstance(data.get("anomalous_pairs"), list):
        raise NativeCalibrationError("native pair anomaly evidence is invalid")
    return data


def _write_json_atomically(path: Path, data: object) -> None:
    _write_text_atomically(path, json.dumps(data, indent=2) + "\n")


def _write_text_atomically(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    try:
        temporary.write_text(text, encoding="utf-8")
        with temporary.open("r+b") as handle:
            handle.flush()
            os.fsync(handle.fileno())
        temporary.replace(path)
    except OSError as exc:
        raise NativeCalibrationError(f"could not atomically write {path}: {exc}") from exc
    finally:
        if temporary.exists():
            temporary.unlink(missing_ok=True)


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
    from sky_music.orchestration.native_provenance import native_source_fingerprint

    return {
        "source_git_sha": _git_output("rev-parse", "HEAD"),
        "dirty_worktree": bool(_git_output("status", "--porcelain")),
        "native_source_fingerprint": native_source_fingerprint(
            _repository_root(), "cp314t-win_amd64"
        ),
    }


def _rustc_version() -> str:
    try:
        return subprocess.check_output(["rustc", "--version"], text=True).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        raise NativeCalibrationError(f"could not read rustc provenance: {exc}") from exc


def full_orchestration_configuration(
    *, global_budget_seconds: float = FULL_CALIBRATION_TIMEOUT_SECONDS
) -> dict[str, Any]:
    return {
        "polyphonies": list(FULL_POLYPHONIES),
        "classes": list(CALIBRATION_CLASSES),
        "samples_per_bucket": FULL_SAMPLE_COUNT,
        "minimum_clean_pairs_per_bucket": MIN_CALIBRATION_SAMPLE_COUNT,
        "global_budget_seconds": float(global_budget_seconds),
        "publication_reserve_seconds": PUBLICATION_RESERVE_SECONDS,
        "chunk_samples": FULL_CHUNK_SAMPLES,
    }


def calibration_bucket_keys() -> list[tuple[int, str]]:
    return [(polyphony, class_name) for polyphony in FULL_POLYPHONIES for class_name in CALIBRATION_CLASSES]


def _bucket_key(polyphony: int, class_name: str) -> str:
    return f"{polyphony}/{class_name}"


def _cache_v3(result: dict[str, Any]) -> dict[str, Any]:
    raw_buckets = _require_mapping(result.get("pair_buckets"), "pair_buckets")
    flattened: dict[str, dict[str, Any]] = {}
    for polyphony, class_name in calibration_bucket_keys():
        classes = _require_mapping(raw_buckets.get(str(polyphony)), f"pair_buckets.{polyphony}")
        bucket = _require_mapping(classes.get(class_name), _bucket_key(polyphony, class_name))
        flattened[_bucket_key(polyphony, class_name)] = {
            "attempted": bucket["attempted"],
            "clean_pair_count": bucket["clean"],
            "rejected": bucket["rejected"],
            "pair_worst_shrink_us": bucket["pair_worst_shrink_us"],
        }
    p99_values = {
        key: max(0, int(bucket["pair_worst_shrink_us"]["p99"]))
        for key, bucket in flattened.items()
    }
    global_p99 = max(p99_values.values())
    worst_bucket = max(REQUIRED_BUCKETS, key=lambda key: (p99_values[key], -REQUIRED_BUCKETS.index(key)))
    qualification_result = qualify_calibration_margin(global_p99)
    qualification = {
        "basis": "max_required_bucket_p99_positive_pair_hold_shrink",
        "worst_bucket": worst_bucket,
        "global_shrink_p99_us": global_p99,
        "guard_us": MARGIN_GUARD_US,
        "floor_us": MARGIN_FLOOR_US,
        "ceiling_us": MARGIN_CEILING_US,
        "candidate_margin_us": qualification_result.candidate_margin_us,
        "applied_margin_us": qualification_result.applied_margin_us,
    }
    return {
        "version": 3,
        "source": "device_cache",
        "status": qualification_result.status.value,
        "evidence_kind": result["evidence_kind"],
        "source_formula_version": 3,
        "native_calibration_version": result["version"],
        "measurement_protocol_version": result["measurement_protocol_version"],
        "source_git_sha": result.get("source_git_sha"),
        "native_build_id": result.get("native_build_id"),
        "native_source_fingerprint": result.get("native_source_fingerprint"),
        "rustc_version": result.get("rustc_version"),
        "dirty_worktree": result.get("dirty_worktree"),
        "host_fingerprint": result.get("host_fingerprint"),
        "required_buckets": list(REQUIRED_BUCKETS),
        "pair_buckets": flattened,
        "qualification": qualification,
    }


def _failure_report(*, class_name: str, polyphony: int, detail: str) -> dict[str, Any]:
    return {
        "kind": "pair",
        "class": class_name,
        "polyphony": polyphony,
        "sample_index": 0,
        "phase": "process",
        "exact_error": detail,
        "cleanup_success": False,
        "cleanup_stuck_keys": [],
        "cleanup_verification_inconclusive": True,
        "raw_input_restore_failed": True,
        "pump_thread_failed": False,
    }


def _execute_native_bucket(
    binary: Path,
    *,
    class_name: str,
    polyphony: int,
    samples: int,
    warmup_samples: int,
    budget_seconds: int,
    timeout_seconds: float,
    progress: bool,
    # Kept as a rejected compatibility argument so independent directional
    # evidence cannot silently re-enter the protocol.
    kind: str | None = None,
) -> dict[str, Any]:
    if kind is not None:
        raise NativeCalibrationError("independent directional calibration is retired in protocol v4")
    command = [
        str(binary),
        "--mode",
        "bucket",
        "--class",
        class_name,
        "--polyphony",
        str(polyphony),
        "--samples",
        str(samples),
        "--warmup-samples",
        str(warmup_samples),
        "--budget-seconds",
        str(budget_seconds),
        "--hot-gap-target-us",
        str(HOT_GAP_TARGET_US),
        "--cold-threshold-us",
        str(COLD_THRESHOLD_US),
        "--cold-idle-gap-us",
        str(FULL_COLD_IDLE_GAP_US),
    ]
    try:
        completed = subprocess.run(
            command,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
            check=False,
            shell=False,
        )
    except subprocess.TimeoutExpired as exc:
        output = exc.output.decode("utf-8", errors="replace") if isinstance(exc.output, bytes) else str(exc.output or "")
        report = _failure_report(class_name=class_name, polyphony=polyphony, detail=output or "native calibration timed out")
        raise NativeCalibrationError("native calibration timed out", failure_report=report) from exc
    except OSError as exc:
        report = _failure_report(class_name=class_name, polyphony=polyphony, detail=str(exc))
        raise NativeCalibrationError(str(exc), failure_report=report) from exc
    output = completed.stdout or ""
    diagnostics = completed.stderr or ""
    if progress:
        for line in diagnostics.splitlines():
            if line.startswith("[calibration]"):
                print(line, file=sys.stderr)
    if completed.returncode != 0:
        raise NativeCalibrationError(
            f"native pair bucket failed ({completed.returncode}): {diagnostics[-2000:]}",
            failure_report=_failure_report(class_name=class_name, polyphony=polyphony, detail=diagnostics[-2000:]),
        )
    try:
        payload = json.loads(output)
    except json.JSONDecodeError as exc:
        raise NativeCalibrationError("native pair bucket stdout was not valid JSON") from exc
    return _validate_pair_bucket_result(
        payload,
        class_name=class_name,
        polyphony=polyphony,
        samples=samples,
        warmup_samples=warmup_samples,
        budget_seconds=budget_seconds,
    )


def _run_process(
    binary: Path,
    *,
    budget_seconds: int,
    timeout_seconds: float,
    samples: int,
) -> dict[str, Any]:
    command = [
        str(binary),
        "--mode",
        "quick",
        "--budget-seconds",
        str(budget_seconds),
        "--hot-gap-target-us",
        str(HOT_GAP_TARGET_US),
        "--cold-threshold-us",
        str(COLD_THRESHOLD_US),
        "--cold-idle-gap-us",
        str(FULL_COLD_IDLE_GAP_US),
    ]
    try:
        completed = subprocess.run(
            command,
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
            check=False,
            shell=False,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise NativeCalibrationError("native calibration process failed") from exc
    if completed.returncode != 0:
        raise NativeCalibrationError(
            f"native calibration failed ({completed.returncode}): {completed.stderr[-2000:]}"
        )
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise NativeCalibrationError("native calibration stdout was not valid JSON") from exc
    return _validate_result(payload, expected_budget_seconds=budget_seconds, expected_samples=samples)


def run_diagnostic_calibration(
    *,
    class_name: str,
    polyphony: int,
    samples: int,
    output_path: Path | str = ".cache/calibration-diagnostic.json",
    failure_report_path: Path | str | None = None,
    timeout_seconds: float | None = None,
) -> dict[str, Any]:
    if class_name not in CALIBRATION_CLASSES:
        raise NativeCalibrationError("class_name must be hot or cold")
    if polyphony not in FULL_POLYPHONIES:
        raise NativeCalibrationError("polyphony must be one of 1, 5, or 15")
    if not isinstance(samples, int) or isinstance(samples, bool) or not 1 <= samples <= MAX_DIAGNOSTIC_SAMPLES:
        raise NativeCalibrationError("diagnostic samples must be between 1 and 5000")
    timeout = _finite_timeout(timeout_seconds, default=QUICK_CALIBRATION_TIMEOUT_SECONDS)
    budget = min(MAX_CALIBRATION_BUDGET_SECONDS, max(6, math.floor(timeout)))
    try:
        result = _execute_native_bucket(
            _find_binary(),
            class_name=class_name,
            polyphony=polyphony,
            samples=samples,
            warmup_samples=FULL_WARMUP_SAMPLES,
            budget_seconds=budget,
            timeout_seconds=timeout,
            progress=True,
        )
    except NativeCalibrationError as exc:
        if failure_report_path is not None:
            _write_json_atomically(Path(failure_report_path), exc.failure_report or {})
        raise
    result["acceptance_eligible"] = False
    result["artifact_schema_version"] = CALIBRATION_ARTIFACT_SCHEMA_VERSION
    _write_json_atomically(Path(output_path), result)
    return result


def _artifact_from_bucket(
    bucket: dict[str, Any], *, orchestration: dict[str, Any], key: str
) -> dict[str, Any]:
    return {
        "artifact_type": "native_calibration_bucket",
        "artifact_schema_version": CALIBRATION_ARTIFACT_SCHEMA_VERSION,
        "acceptance_eligible": int(bucket["pair_bucket"]["clean_sample_count"]) >= MIN_CALIBRATION_SAMPLE_COUNT,
        "version": bucket["version"],
        "measurement_protocol_version": bucket["measurement_protocol_version"],
        "evidence_kind": bucket["evidence_kind"],
        "key": key,
        "orchestration_configuration": orchestration,
        "native_configuration": bucket["configuration"],
        "source_git_sha": bucket["source_git_sha"],
        "native_build_id": bucket["native_build_id"],
        "native_source_fingerprint": bucket["native_source_fingerprint"],
        "dirty_worktree": bucket["dirty_worktree"],
        "rustc_version": bucket["rustc_version"],
        "host_fingerprint": bucket["host_fingerprint"],
        "class": bucket["class"],
        "polyphony": bucket["polyphony"],
        "attempted_pairs": bucket["attempted_pairs"],
        "warmup_pairs": bucket["warmup_pairs"],
        "warmup_rejected": bucket["warmup_rejected"],
        "pair_bucket": bucket["pair_bucket"],
        "worst_pairs": bucket["worst_pairs"],
        "anomalous_pairs": bucket["anomalous_pairs"],
        "cleanup": bucket["cleanup"],
        "provenance": _provenance_identity(bucket),
    }


def _manifest_path(checkpoint_dir: Path) -> Path:
    return checkpoint_dir / "MANIFEST.json"


def _provenance_identity(data: dict[str, Any]) -> dict[str, Any]:
    host = _require_mapping(data.get("host_fingerprint"), "host_fingerprint")
    qpc_frequency = _int(host.get("qpc_frequency_hz"), "host_fingerprint.qpc_frequency_hz", minimum=1)
    win32_build = host.get("win32_build")
    if not isinstance(win32_build, str) or not win32_build.strip():
        raise NativeCalibrationError("native calibration has incomplete Windows fingerprint")
    identity = {
        "source_git_sha": data.get("source_git_sha"),
        "native_build_id": data.get("native_build_id"),
        "native_source_fingerprint": data.get("native_source_fingerprint"),
        "dirty_worktree": data.get("dirty_worktree"),
        "rustc_version": data.get("rustc_version"),
        "qpc_frequency_hz": qpc_frequency,
        "win32_build": win32_build,
    }
    for name in ("source_git_sha", "native_build_id", "native_source_fingerprint", "rustc_version"):
        value = identity[name]
        if not isinstance(value, str) or not value.strip():
            raise NativeCalibrationError(f"native calibration provenance has invalid {name}")
    if not isinstance(identity["dirty_worktree"], bool):
        raise NativeCalibrationError("native calibration provenance has invalid dirty_worktree")
    return identity


def _validate_provenance_manifest(value: object) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise NativeCalibrationError("calibration checkpoint provenance manifest is missing")
    expected = {
        "source_git_sha",
        "native_build_id",
        "native_source_fingerprint",
        "dirty_worktree",
        "rustc_version",
        "qpc_frequency_hz",
        "win32_build",
    }
    if set(value) != expected:
        raise NativeCalibrationError("calibration checkpoint provenance manifest is incomplete")
    return _provenance_identity(
        {
            **value,
            "host_fingerprint": {
                "qpc_frequency_hz": value["qpc_frequency_hz"],
                "win32_build": value["win32_build"],
            },
        }
    )


def _static_provenance(identity: dict[str, Any]) -> dict[str, Any]:
    return {
        name: identity[name]
        for name in (
            "source_git_sha",
            "native_build_id",
            "native_source_fingerprint",
            "dirty_worktree",
            "rustc_version",
        )
    }


def _assert_provenance_match(actual: dict[str, Any], expected: dict[str, Any], *, key: str) -> None:
    if actual != expected:
        raise NativeCalibrationError(f"bucket {key} has mismatched calibration provenance")


def _finalize_artifacts(
    artifacts: dict[str, dict[str, Any]],
    *,
    orchestration: dict[str, Any],
    expected_provenance: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if set(artifacts) != set(REQUIRED_BUCKETS):
        raise NativeCalibrationError("calibration artifact matrix is incomplete")
    buckets: dict[str, dict[str, Any]] = {}
    first = next(iter(artifacts.values()))
    common_provenance = expected_provenance or _provenance_identity(first)
    for key, artifact in artifacts.items():
        if artifact.get("key") != key:
            raise NativeCalibrationError(f"bucket {key} has mismatched artifact identity")
        if artifact.get("orchestration_configuration") != orchestration:
            raise NativeCalibrationError(f"bucket {key} has mismatched orchestration configuration")
        _validate_common_metadata(artifact)
        _assert_provenance_match(_provenance_identity(artifact), common_provenance, key=key)
        if artifact.get("acceptance_eligible") is not True:
            raise NativeCalibrationError(f"bucket {key} has insufficient clean pairs")
        pair_bucket = _validate_pair_bucket(artifact.get("pair_bucket"), f"{key}.pair_bucket", FULL_SAMPLE_COUNT)
        buckets.setdefault(str(artifact["polyphony"]), {})[artifact["class"]] = pair_bucket
    final = {
        "artifact_type": "native_calibration_final",
        "artifact_schema_version": CALIBRATION_ARTIFACT_SCHEMA_VERSION,
        "acceptance_eligible": True,
        "version": SUPPORTED_NATIVE_CALIBRATION_VERSION,
        "measurement_protocol_version": SUPPORTED_MEASUREMENT_PROTOCOL_VERSION,
        "evidence_kind": "injected_raw_input_delivery_proxy",
        "source_git_sha": common_provenance["source_git_sha"],
        "native_build_id": common_provenance["native_build_id"],
        "native_source_fingerprint": common_provenance["native_source_fingerprint"],
        "dirty_worktree": common_provenance["dirty_worktree"],
        "rustc_version": common_provenance["rustc_version"],
        "host_fingerprint": first["host_fingerprint"],
        "orchestration_configuration": orchestration,
        "pair_buckets": buckets,
        "measured_attempted": len(REQUIRED_BUCKETS) * FULL_SAMPLE_COUNT,
        "cleanup": {
            "cleanup_attempted": True,
            "cleanup_success": True,
            "cleanup_stuck_keys": [],
            "cleanup_verification_inconclusive": False,
            "raw_input_restore_failed": False,
            "pump_thread_failed": False,
        },
    }
    final["warmup_attempted"] = len(REQUIRED_BUCKETS) * FULL_WARMUP_SAMPLES
    final["setup_attempted"] = 0
    measured_attempted = _int(final["measured_attempted"], "measured_attempted")
    warmup_attempted = _int(final["warmup_attempted"], "warmup_attempted")
    final["total_attempted"] = measured_attempted + warmup_attempted
    warmup_anomalous = sum(int(item["warmup_rejected"]) for item in artifacts.values())
    measured_anomalous = sum(
        int(item["pair_bucket"]["anomaly_count"]) for item in artifacts.values()
    )
    final["warmup_anomalous"] = warmup_anomalous
    final["measured_anomalous"] = measured_anomalous
    final["total_anomalous"] = warmup_anomalous + measured_anomalous
    final["warmup_timed_out"] = 0
    final["measured_timed_out"] = sum(int(item["pair_bucket"]["timeout_count"]) for item in artifacts.values())
    final["total_timed_out"] = final["measured_timed_out"]
    final["measured_class_mismatch"] = sum(int(item["pair_bucket"]["class_mismatch_count"]) for item in artifacts.values())
    return final


def run_full_calibration(
    *,
    checkpoint_dir: Path | str = ".cache/calibration-full",
    resume: bool = False,
    timeout_seconds: float | None = None,
) -> dict[str, Any]:
    timeout = _finite_timeout(timeout_seconds, default=FULL_CALIBRATION_TIMEOUT_SECONDS)
    if timeout < MIN_FULL_CALIBRATION_TIMEOUT_SECONDS:
        raise NativeCalibrationError("full calibration timeout is too short for publication reserve")
    checkpoint = Path(checkpoint_dir)
    checkpoint.mkdir(parents=True, exist_ok=True)
    orchestration = full_orchestration_configuration()
    current_provenance = _current_worktree_provenance()
    current_provenance["rustc_version"] = _rustc_version()
    current_static_provenance = _static_provenance(
        {**current_provenance, "native_build_id": current_provenance["source_git_sha"]}
    )
    manifest_path = _manifest_path(checkpoint)
    stable_provenance: dict[str, Any] | None = None
    if resume and manifest_path.is_file():
        try:
            saved_manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            if saved_manifest.get("orchestration_configuration") != orchestration:
                raise NativeCalibrationError("calibration checkpoint configuration mismatch")
            stable_provenance = _validate_provenance_manifest(saved_manifest.get("provenance"))
            if _static_provenance(stable_provenance) != current_static_provenance:
                raise NativeCalibrationError("calibration checkpoint provenance does not match this build")
        except (OSError, ValueError) as exc:
            raise NativeCalibrationError("calibration checkpoint manifest is unreadable") from exc
    artifacts: dict[str, dict[str, Any]] = {}
    run_deadline = time.monotonic() + timeout
    binary = _find_binary()
    for polyphony, class_name in calibration_bucket_keys():
        key = _bucket_key(polyphony, class_name)
        artifact_path = checkpoint / f"{polyphony}-{class_name}.json"
        if resume and artifact_path.is_file():
            try:
                candidate = json.loads(artifact_path.read_text(encoding="utf-8"))
                digest_path = artifact_path.with_suffix(".sha256")
                digest_ok = (
                    digest_path.is_file()
                    and digest_path.read_text(encoding="utf-8").strip()
                    == hashlib.sha256(artifact_path.read_bytes()).hexdigest()
                )
                if (
                    digest_ok
                    and candidate.get("key") == key
                    and candidate.get("orchestration_configuration") == orchestration
                ):
                    candidate_provenance = _provenance_identity(candidate)
                    if stable_provenance is None:
                        raise NativeCalibrationError(
                            "calibration checkpoint provenance manifest is required for resume"
                        )
                    _assert_provenance_match(candidate_provenance, stable_provenance, key=key)
                    artifacts[key] = candidate
                    continue
            except (OSError, ValueError):
                pass
        budget, child_timeout = _native_child_budget(run_deadline, time.monotonic())
        bucket = _execute_native_bucket(
            binary,
            class_name=class_name,
            polyphony=polyphony,
            samples=FULL_SAMPLE_COUNT,
            warmup_samples=FULL_WARMUP_SAMPLES,
            budget_seconds=budget,
            timeout_seconds=child_timeout,
            progress=True,
        )
        artifact_provenance = _provenance_identity(bucket)
        if stable_provenance is None:
            stable_provenance = artifact_provenance
            if _static_provenance(stable_provenance) != current_static_provenance:
                raise NativeCalibrationError("native bucket provenance does not match this build")
        _assert_provenance_match(artifact_provenance, stable_provenance, key=key)
        artifact = _artifact_from_bucket(bucket, orchestration=orchestration, key=key)
        _write_json_atomically(artifact_path, artifact)
        _write_text_atomically(
            artifact_path.with_suffix(".sha256"), hashlib.sha256(artifact_path.read_bytes()).hexdigest() + "\n"
        )
        artifacts[key] = artifact
        _write_json_atomically(
            manifest_path,
            {
                "orchestration_configuration": orchestration,
                "provenance": stable_provenance,
                "buckets": sorted(artifacts),
            },
        )
    if set(artifacts) != set(REQUIRED_BUCKETS):
        raise NativeCalibrationError("full calibration checkpoint matrix is incomplete")
    if stable_provenance is None:
        raise NativeCalibrationError("calibration checkpoint provenance is missing")
    final = _finalize_artifacts(
        artifacts, orchestration=orchestration, expected_provenance=stable_provenance
    )
    _write_json_atomically(
        manifest_path,
        {
            "orchestration_configuration": orchestration,
            "provenance": stable_provenance,
            "buckets": sorted(artifacts),
        },
    )
    return final


def finalize_native_calibration(
    *, checkpoint_dir: Path | str, output_path: Path | str, cache_path: Path | str
) -> dict[str, Any]:
    checkpoint = Path(checkpoint_dir)
    manifest_path = _manifest_path(checkpoint)
    if not manifest_path.is_file():
        raise NativeCalibrationError("calibration checkpoint manifest is missing")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    orchestration = full_orchestration_configuration()
    if manifest.get("orchestration_configuration") != orchestration:
        raise NativeCalibrationError("calibration checkpoint configuration mismatch")
    if manifest.get("buckets") != sorted(REQUIRED_BUCKETS):
        raise NativeCalibrationError("calibration checkpoint manifest matrix is incomplete")
    stable_provenance = _validate_provenance_manifest(manifest.get("provenance"))
    artifacts: dict[str, dict[str, Any]] = {}
    for polyphony, class_name in calibration_bucket_keys():
        key = _bucket_key(polyphony, class_name)
        path = checkpoint / f"{polyphony}-{class_name}.json"
        if not path.is_file():
            raise NativeCalibrationError(f"calibration checkpoint is missing {key}")
        artifact = json.loads(path.read_text(encoding="utf-8"))
        digest_path = path.with_suffix(".sha256")
        if not digest_path.is_file() or digest_path.read_text(encoding="utf-8").strip() != hashlib.sha256(path.read_bytes()).hexdigest():
            raise NativeCalibrationError(f"calibration checkpoint hash mismatch for {key}")
        artifacts[key] = artifact
    final = _finalize_artifacts(
        artifacts, orchestration=orchestration, expected_provenance=stable_provenance
    )
    cache = _cache_v3(final)
    parse_calibration_cache_summary(cache)
    _write_json_atomically(Path(output_path), final)
    _write_json_atomically(Path(cache_path), cache)
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
    if kind is not None:
        raise NativeCalibrationError("independent directional calibration is retired in protocol v4")
    if mode == "diagnostic":
        if class_name is None or polyphony is None or samples is None:
            raise NativeCalibrationError("diagnostic mode requires class_name, polyphony, and samples")
        return run_diagnostic_calibration(
            class_name=class_name,
            polyphony=polyphony,
            samples=samples,
            output_path=output_path or ".cache/calibration-diagnostic.json",
            failure_report_path=failure_report_path,
            timeout_seconds=timeout_seconds,
        )
    if mode == "full":
        return run_full_calibration(
            checkpoint_dir=checkpoint_dir, resume=resume, timeout_seconds=timeout_seconds
        )
    if mode != "quick":
        raise NativeCalibrationError("mode must be diagnostic, quick, or full")
    if class_name is not None or polyphony is not None or samples is not None:
        raise NativeCalibrationError("quick mode does not accept a directional bucket selector")
    timeout = _finite_timeout(timeout_seconds, default=QUICK_CALIBRATION_TIMEOUT_SECONDS)
    budget = min(MAX_CALIBRATION_BUDGET_SECONDS, max(6, math.floor(timeout)))
    result = _run_process(_find_binary(), budget_seconds=budget, timeout_seconds=timeout, samples=FULL_SAMPLE_COUNT)
    raw_output = Path(output_path) if output_path is not None else Path(".cache/calibration-native.json")
    cache = _cache_v3(result)
    # Validation happens before either write; an invalid run leaves an old
    # cache untouched.
    parse_calibration_cache_summary(cache)
    _write_json_atomically(raw_output, result)
    _write_json_atomically(Path(cache_path), cache)
    return result


def run_published_native_calibration(
    *,
    output_path: Path | str | None = None,
    cache_path: Path | str = ".cache/input_latency.json",
    timeout_seconds: float | None = None,
    fps: int = DEFAULT_GAME_FPS,
    hold_frames: float = DEFAULT_HOLD_FRAMES,
) -> PublishedCalibrationResult:
    result = run_native_calibration(
        mode="quick", output_path=output_path, cache_path=cache_path, timeout_seconds=timeout_seconds
    )
    summary: CalibrationCacheSummary = parse_calibration_cache_summary(_cache_v3(result))
    return PublishedCalibrationResult(
        status=summary.status,
        margin_us=summary.margin_us,
        candidate_margin_us=summary.candidate_margin_us,
        source=summary.source,
        sample_count=summary.sample_count,
        cache_path=Path(cache_path),
        evidence_kind=str(result["evidence_kind"]),
        source_git_sha=str(result["source_git_sha"]),
        native_build_id=str(result["native_build_id"]),
        pair_buckets=summary.pair_buckets,
        worst_bucket=summary.worst_bucket,
        global_shrink_p99_us=summary.global_shrink_p99_us,
        guard_us=summary.guard_us,
        ceiling_us=summary.ceiling_us,
        effective_min_hold_us=(
            materialize_hold_us(hold_frames, fps, summary.margin_us)
            if summary.status is CalibrationStatus.VALID and summary.margin_us is not None
            else None
        ),
    )


__all__ = [
    "CALIBRATION_ARTIFACT_SCHEMA_VERSION",
    "FULL_POLYPHONIES",
    "FULL_SAMPLE_COUNT",
    "NativeCalibrationError",
    "PublishedCalibrationResult",
    "calibration_bucket_keys",
    "finalize_native_calibration",
    "full_orchestration_configuration",
    "run_diagnostic_calibration",
    "run_full_calibration",
    "run_native_calibration",
    "run_published_native_calibration",
]
