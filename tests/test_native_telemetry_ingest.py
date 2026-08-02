from __future__ import annotations

import pytest

from sky_music.orchestration.telemetry import TelemetryLogger, materialize_native_trace


def _compact_output(
    *, completion_error_ticks: int = 250, schema_version: int = 4
) -> dict[str, object]:
    return {
        "schema_version": schema_version,
        "qpc_frequency_hz": 10_000_000,
        "attempted": 1,
        "accepted": 1,
        "dropped": 0,
        "truncated": False,
        "records": [
            {
                "event_index": 0,
                "kind": 0,
                "outcome": 0,
                "polyphony": 1,
                "flags": 1,
                "authored_ticks": 10_000,
                "effective_deadline_ticks": 10_000,
                "wake_ticks": 10_100,
                "send_started_ticks": 10_110,
                "send_completed_ticks": 10_180,
                "completion_error_ticks": completion_error_ticks,
                "applied_lead_ticks": 1_000,
                "win32_error": 1460,
            }
        ],
    }


def test_native_trace_materializer_decodes_current_compact_schema() -> None:
    output = _compact_output()
    record = materialize_native_trace(output)[0]

    assert record.kind == "down"
    assert record.runtime_outcome == "sent"
    assert record.native_polyphony == 1
    assert record.sender_completion_error_us == 25
    assert record.visible_lateness_us == 25
    assert record.dispatch_lateness_us == 25
    assert record.applied_lead_us == 100
    assert record.scan_codes == ()
    assert record.sent_scan_codes == ()
    assert record.skipped_scan_codes == ()


def test_native_telemetry_ingest_preserves_frozen_fields() -> None:
    logger = TelemetryLogger("test", enabled=True, retain_records_after_save=True)
    logger.ingest_native_output(_compact_output())

    row = logger.records[0]._materialize()
    assert row["dispatch_id"] == 0
    assert row["evidence_scope"] == "sender_completion"
    assert row["send_duration_pure_us"] == 7
    assert row["bookkeeping_us"] == 0
    assert row["scheduled_timeline_us"] == 1_000
    assert row["wake_timeline_us"] == 1_010
    assert row["sender_started_us"] == 1_011
    assert row["sender_completed_us"] == 1_018
    assert row["sender_completion_error_us"] == 25
    assert row["completion_error_ticks"] == 250
    assert row["send_operation_duration_us"] == 7
    assert row["sendinput_call_duration_us"] == 7
    assert row["bookkeeping_duration_us"] == 0
    assert row["generation_ids"] == ""
    assert row["first_win32_error"] == 1460
    assert row["last_win32_error"] == 1460
    assert row["send_attempts"] == 0
    assert row["zero_progress_retries"] == 0


def test_native_trace_materializer_preserves_negative_completion_error() -> None:
    record = materialize_native_trace(_compact_output(completion_error_ticks=-250))[0]

    assert record.sender_completion_error_us == -25
    assert record.visible_lateness_us == -25
    assert record.dispatch_lateness_us == -25


@pytest.mark.parametrize(
    "mutate",
    [
        lambda output: output["records"][0].__setitem__(
            "completion_error_ticks", 1 << 63
        ),
        lambda output: output["records"][0].__setitem__(
            "completion_error_ticks", -(1 << 63) - 1
        ),
    ],
)
def test_native_trace_materializer_rejects_invalid_signed_completion_field(
    mutate,
) -> None:
    output = _compact_output()
    mutate(output)

    with pytest.raises(ValueError, match="completion_error_ticks"):
        materialize_native_trace(output)


def test_native_trace_materializer_rejects_legacy_schema() -> None:
    with pytest.raises(ValueError, match="schema version"):
        materialize_native_trace(_compact_output(schema_version=3))


def test_native_terminal_counters_replace_python_placeholders() -> None:
    logger = TelemetryLogger("test", enabled=False)
    logger.record_abort("old")

    logger.record_abort_counts({"finished": 1, "focus_lost": 2})
    logger.record_generation_status_counts({"released": 3, "cancelled": 0})

    assert logger.abort_counts_by_reason == {"finished": 1, "focus_lost": 2}
    assert logger.generation_status_counts == {"released": 3, "cancelled": 0}


def test_summary_counts_zero_insertion_send_attempt() -> None:
    logger = TelemetryLogger(
        "failed-send", enabled=True, retain_records_after_save=True
    )
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
