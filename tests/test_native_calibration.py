from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

from sky_music.platform.win32 import native_calibration


def _native_result(*, clean: int = 64, cleanup_success: bool = True) -> dict[str, object]:
    bucket = {
        "clean_sample_count": clean,
        "first_receipt_us": {"p50": 10, "p90": 20, "p95": 30, "p99": 40},
    }
    return {
        "version": 4,
        "evidence_kind": "injected_raw_input_delivery_proxy",
        "measured_attempted": 100,
        "measured_anomalous": 2,
        "warmup_anomalous": 1,
        "total_anomalous": 3,
        "buckets": {
            "down": {"1": {"hot": bucket}},
            "up": {"1": {"hot": bucket}},
        },
        "cleanup": {
            "cleanup_success": cleanup_success,
            "cleanup_verification_inconclusive": False,
            "raw_input_restore_failed": False,
        },
    }


def test_native_calibration_writes_cache_only_after_valid_clean_result(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    result = _native_result()
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    monkeypatch.setattr(
        native_calibration.subprocess,
        "run",
        lambda *args, **kwargs: subprocess.CompletedProcess(
            args, 0, json.dumps(result), "diagnostic on stderr\n"
        ),
    )

    artifact = tmp_path / "calibration-native.json"
    cache = tmp_path / "input_latency.json"
    returned = native_calibration.run_native_calibration(
        output_path=artifact, cache_path=cache
    )

    assert returned == result
    assert json.loads(artifact.read_text(encoding="utf-8")) == result
    legacy = json.loads(cache.read_text(encoding="utf-8"))
    assert legacy["evidence_kind"] == result["evidence_kind"]
    assert legacy["n"] == 64
    assert legacy["down_us"] == {"p50": 10, "p90": 20, "p99": 40}


@pytest.mark.parametrize(
    "result",
    [
        _native_result(clean=1),
        _native_result(cleanup_success=False),
    ],
)
def test_native_calibration_rejects_untrusted_result_without_mutating_cache(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    result: dict[str, object],
) -> None:
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    monkeypatch.setattr(
        native_calibration.subprocess,
        "run",
        lambda *args, **kwargs: subprocess.CompletedProcess(
            args, 0, json.dumps(result), ""
        ),
    )
    cache = tmp_path / "input_latency.json"
    cache.write_text("sentinel\n", encoding="utf-8")

    with pytest.raises(native_calibration.NativeCalibrationError):
        native_calibration.run_native_calibration(
            output_path=tmp_path / "raw.json", cache_path=cache
        )

    assert cache.read_text(encoding="utf-8") == "sentinel\n"


def test_native_calibration_nonzero_process_exit_is_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(native_calibration, "_find_binary", lambda: tmp_path / "native.exe")
    monkeypatch.setattr(
        native_calibration.subprocess,
        "run",
        lambda *args, **kwargs: subprocess.CompletedProcess(
            args, 7, "{}", "cleanup failed"
        ),
    )

    with pytest.raises(native_calibration.NativeCalibrationError, match="7"):
        native_calibration.run_native_calibration(
            output_path=tmp_path / "raw.json", cache_path=tmp_path / "cache.json"
        )
