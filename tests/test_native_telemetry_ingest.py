from __future__ import annotations

from sky_music.orchestration.telemetry import TelemetryLogger


def test_native_telemetry_ingest_preserves_frozen_fields() -> None:
    logger = TelemetryLogger("test", enabled=True, retain_records_after_save=True)
    logger.ingest_native_output(
        {
            "attempted": 1,
            "accepted": 1,
            "dropped": 0,
            "truncated": False,
            "records": [
                {
                    "event_index": 0,
                    "dispatch_id": 7,
                    "kind": "down",
                    "scheduled_us": 1_000,
                    "actual_us": 1_010,
                    "dispatch_completed_us": 1_020,
                    "lateness_us": 10,
                    "visible_lateness_us": 20,
                    "send_duration_us": 12,
                    "send_duration_pure_us": 10,
                    "bookkeeping_us": 2,
                    "dispatch_lateness_us": 20,
                    "scan_codes": [0x15],
                    "sent_scan_codes": [0x15],
                    "skipped_scan_codes": [],
                    "generation_ids": [1],
                    "runtime_outcome": "sent",
                    "deferred_by_us": 0,
                    "pre_send_spin_us": 0,
                    "idle_gap_us": 0,
                    "reason": "note",
                    "applied_lead_us": 100,
                    "first_win32_error": 5,
                    "last_win32_error": 1460,
                    "send_attempts": 3,
                    "zero_progress_retries": 2,
                }
            ],
        }
    )

    row = logger.records[0]._materialize()
    assert row["dispatch_id"] == 7
    assert row["evidence_scope"] == "sendinput_side"
    assert row["send_duration_pure_us"] == 10
    assert row["bookkeeping_us"] == 2
    assert row["generation_ids"] == "1"
    assert row["first_win32_error"] == 5
    assert row["last_win32_error"] == 1460
    assert row["send_attempts"] == 3
    assert row["zero_progress_retries"] == 2


def test_native_terminal_counters_replace_python_placeholders() -> None:
    logger = TelemetryLogger("test", enabled=False)
    logger.record_abort("old")

    logger.record_abort_counts({"finished": 1, "focus_lost": 2})
    logger.record_generation_status_counts({"released": 3, "cancelled": 0})

    assert logger.abort_counts_by_reason == {"finished": 1, "focus_lost": 2}
    assert logger.generation_status_counts == {"released": 3, "cancelled": 0}


def test_summary_counts_zero_insertion_send_attempt() -> None:
    logger = TelemetryLogger("failed-send", enabled=True, retain_records_after_save=True)
    logger.record(
        event_index=0,
        kind="up",
        scheduled_us=1_000,
        actual_us=1_010,
        lateness_us=10,
        send_duration_us=6_000,
        scan_codes=(0x15,),
        reason="release",
        sent_scan_codes=(),
        send_attempts=3,
        first_win32_error=5,
        last_win32_error=1460,
        zero_progress_retries=2,
        runtime_outcome="failed_note_off",
    )

    summary = logger.get_summary()
    assert summary is not None
    assert summary["attempted_dispatches"] == 1
    assert summary["successful_dispatches"] == 0
    assert summary["send_duration_us"]["p50_us"] == 6_000.0
