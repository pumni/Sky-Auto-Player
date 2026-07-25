from __future__ import annotations

import csv
import json
from pathlib import Path
from typing import SupportsIndex, cast, overload
from unittest.mock import patch

from sky_music.orchestration import telemetry as telemetry_module
from sky_music.orchestration.telemetry import TelemetryLogger, TelemetryRecord


def _record(logger: TelemetryLogger, event_index: int) -> None:
    logger.record(
        event_index=event_index,
        kind="down",
        scheduled_us=event_index * 1_000,
        actual_us=event_index * 1_000,
        lateness_us=0,
        send_duration_us=1,
        scan_codes=(0x1E,),
        reason="note",
        dispatch_completed_us=event_index * 1_000 + 1,
        sent_scan_codes=(0x1E,),
        visible_lateness_us=1,
    )


class _NoSliceRecords(list[TelemetryRecord]):
    @overload
    def __getitem__(self, index: SupportsIndex) -> TelemetryRecord: ...

    @overload
    def __getitem__(self, index: slice[SupportsIndex | None, SupportsIndex | None, SupportsIndex | None]) -> list[TelemetryRecord]: ...

    def __getitem__(
        self, index: SupportsIndex | slice[SupportsIndex | None, SupportsIndex | None, SupportsIndex | None],
    ) -> TelemetryRecord | list[TelemetryRecord]:
        if isinstance(index, slice):
            raise AssertionError("dispatch record path performed O(n) list slicing")
        return cast(TelemetryRecord, super().__getitem__(index))


def test_telemetry_cap_stops_accepting_in_o1_without_slicing() -> None:
    with (
        patch.object(telemetry_module, "_TELEMETRY_MAX_BUFFER", 3),
    ):
        logger = TelemetryLogger("bounded", enabled=True)
    logger.records = _NoSliceRecords()

    with (
        patch.object(
            logger,
            "_ensure_csv_open",
            side_effect=AssertionError("record() attempted filesystem export"),
        ),
    ):
        for event_index in range(5):
            _record(logger, event_index)

    assert len(logger.records) == 3
    assert logger._dropped_count == 2
    assert logger._truncated is True


def test_telemetry_summary_discloses_exact_truncation_policy() -> None:
    with patch.object(telemetry_module, "_TELEMETRY_MAX_BUFFER", 3):
        logger = TelemetryLogger("summary", enabled=True)
        for event_index in range(5):
            _record(logger, event_index)

    summary = logger.get_summary()

    assert summary is not None
    assert summary["telemetry_truncated"] is True
    assert summary["telemetry_dropped_count"] == 2
    assert summary["telemetry_buffer"] == {
        "policy": "retain_first_n_stop_accepting",
        "capacity": 3,
        "attempted_count": 5,
        "retained_count": 3,
        "dropped_count": 2,
        "truncated": True,
        "marker_count": 1,
    }


def test_truncated_telemetry_exports_once_at_playback_lifecycle(tmp_path: Path) -> None:
    with patch.object(telemetry_module, "_TELEMETRY_MAX_BUFFER", 3):
        logger = TelemetryLogger("export", enabled=True, run_id="bounded-export")
        logger.log_filepath = tmp_path / "bounded.csv"
        for event_index in range(5):
            _record(logger, event_index)
        assert not logger.log_filepath.exists()
        logger.save()

    with logger.log_filepath.open(newline="", encoding="utf-8") as csv_file:
        assert len(list(csv.DictReader(csv_file))) == 3
    summary_path = logger.log_filepath.with_suffix(".summary.json")
    with summary_path.open(encoding="utf-8") as summary_file:
        summary = json.load(summary_file)
    assert summary["telemetry_dropped_count"] == 2
    assert summary["telemetry_buffer"]["marker_count"] == 1
