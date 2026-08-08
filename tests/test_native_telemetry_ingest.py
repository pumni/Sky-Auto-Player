from __future__ import annotations

from typing import Any

import pytest

from sky_music.orchestration.telemetry import TelemetryLogger, materialize_native_trace


def _compact_output(
    *,
    completion_error_ticks: int = 250,
    authored_completion_error_ticks: int = 250,
    schema_version: int = 7,
    requested_count: int = 1,
    sent_count: int = 1,
    skipped_count: int = 0,
    send_attempts: int = 1,
) -> dict[str, Any]:
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
                "core_post_send_duration_us": 4,
                "completion_error_ticks": completion_error_ticks,
                "authored_completion_error_ticks": authored_completion_error_ticks,
                "applied_lead_ticks": 1_000,
                "win32_error": 1460,
                "requested_count": requested_count,
                "sent_count": sent_count,
                "skipped_count": skipped_count,
                "send_attempts": send_attempts,
            }
        ],
    }


def test_native_trace_materializer_decodes_current_compact_schema() -> None:
    output = _compact_output()
    record = materialize_native_trace(output)[0]

    assert record.kind == "down"
    assert record.runtime_outcome == "sent"
    assert record.native_polyphony == 1
    assert record.native_requested_count == 1
    assert record.native_sent_count == 1
    assert record.native_skipped_count == 0
    assert record.wake_error_us == 10
    assert record.sender_completion_error_us == 25
    assert record.core_post_send_duration_us == 4
    assert record.visible_lateness_us == 25
    assert record.dispatch_lateness_us == 25
    assert record.applied_lead_us == 100
    assert record.scan_codes == ()
    assert record.sent_scan_codes == ()
    assert record.skipped_scan_codes == ()


def test_native_trace_materializer_preserves_zero_relative_send_start() -> None:
    output = _compact_output()
    output["records"][0]["authored_ticks"] = 0
    output["records"][0]["effective_deadline_ticks"] = 0
    output["records"][0]["wake_ticks"] = 0
    output["records"][0]["send_started_ticks"] = 0
    output["records"][0]["send_completed_ticks"] = 25

    record = materialize_native_trace(output)[0]

    assert record.sender_started_us == 0
    assert record.sender_completed_us == 2
    assert record.send_duration_us == 2


def test_native_trace_materializer_rejects_missing_core_post_send_duration() -> None:
    output = _compact_output()
    del output["records"][0]["core_post_send_duration_us"]

    with pytest.raises(ValueError, match="core_post_send_duration_us"):
        materialize_native_trace(output)


def test_native_telemetry_ingest_preserves_frozen_fields() -> None:
    logger = TelemetryLogger("test", enabled=True, retain_records_after_save=True)
    logger.ingest_native_output(_compact_output())

    row = logger.records[0]._materialize()
    assert row["dispatch_id"] == 0
    assert row["evidence_scope"] == "sender_completion"
    assert row["send_duration_pure_us"] == 7
    assert row["bookkeeping_us"] == 4
    assert row["scheduled_timeline_us"] == 1_000
    assert row["wake_timeline_us"] == 1_010
    assert row["wake_error_us"] == 10
    assert row["sender_started_us"] == 1_011
    assert row["sender_completed_us"] == 1_018
    assert row["sender_completion_error_us"] == 25
    assert row["completion_error_ticks"] == 250
    assert row["authored_completion_error_ticks"] == 250
    assert row["send_operation_duration_us"] == 7
    assert row["sendinput_call_duration_us"] == 7
    assert row["core_post_send_duration_us"] == 4
    assert row["generation_ids"] == ""
    assert row["first_win32_error"] == 1460
    assert row["last_win32_error"] == 1460
    assert row["send_attempts"] == 1
    assert row["native_requested_count"] == 1
    assert row["native_sent_count"] == 1
    assert row["native_skipped_count"] == 0
    assert row["zero_progress_retries"] == 0


def test_native_truncated_clean_looking_records_fail_closed() -> None:
    output = _compact_output()
    output.update({"attempted": 2, "accepted": 1, "dropped": 1, "truncated": True})

    logger = TelemetryLogger("truncated", enabled=True, retain_records_after_save=True)
    logger.ingest_native_output(output)

    summary = logger.get_summary()
    assert summary is not None
    assert summary["telemetry_truncated"] is True
    assert summary["telemetry_dropped_count"] >= 1
    assert summary["sender_clean_known"] is False
    assert summary["sender_clean"] is False
    assert (
        summary["evidence_boundaries"]["sender_completion"]["sender_clean_known"]
        is False
    )
    assert summary["evidence_boundaries"]["sender_completion"]["sender_clean"] is False


def test_native_nontruncated_clean_records_are_known_clean() -> None:
    logger = TelemetryLogger("clean", enabled=True, retain_records_after_save=True)
    logger.ingest_native_output(_compact_output())

    summary = logger.get_summary()
    assert summary is not None
    assert summary["sender_clean_known"] is True
    assert summary["sender_clean"] is True
    assert (
        summary["evidence_boundaries"]["sender_completion"]["sender_clean_known"]
        is True
    )


@pytest.mark.parametrize(
    ("mutate", "message"),
    [
        (lambda output: output.__setitem__("attempted", -1), "attempted"),
        (lambda output: output.__setitem__("accepted", -1), "accepted"),
        (lambda output: output.__setitem__("dropped", -1), "dropped"),
        (lambda output: output.__setitem__("attempted", True), "attempted"),
        (lambda output: output.__setitem__("accepted", True), "accepted"),
        (lambda output: output.__setitem__("dropped", True), "dropped"),
        (lambda output: output.__setitem__("accepted", 0), "accepted != len"),
        (
            lambda output: output.update({"attempted": 2, "accepted": 1, "dropped": 1}),
            "dropped > 0",
        ),
        (lambda output: output.update({"attempted": 0, "accepted": 1}), "attempted < accepted"),
    ],
)
def test_native_telemetry_ingest_rejects_invalid_envelope(mutate, message: str) -> None:
    output = _compact_output()
    mutate(output)

    logger = TelemetryLogger("invalid", enabled=True)
    with pytest.raises(ValueError, match=message):
        logger.ingest_native_output(output)

    assert logger.record_stats() == {
        "attempted": 0,
        "accepted": 0,
        "written": 0,
        "dropped": 0,
        "retained": 0,
    }


def test_python_and_native_truncation_drop_counts_merge_without_double_counting() -> None:
    logger = TelemetryLogger("merge", enabled=True, retain_records_after_save=True)
    logger._telemetry_capacity = 2
    for event_index in range(3):
        logger.record(
            event_index=event_index,
            kind="down",
            scheduled_us=event_index * 1_000,
            actual_us=event_index * 1_000 + 10,
            lateness_us=10,
            send_duration_us=5,
            scan_codes=(21 + event_index,),
            sent_scan_codes=(21 + event_index,),
            reason="merge",
        )

    native = _compact_output()
    native.update(
        {"attempted": 1, "accepted": 0, "dropped": 1, "truncated": True, "records": []}
    )
    logger.ingest_native_output(native)

    stats = logger.record_stats()
    assert stats["dropped"] == 2
    assert stats["retained"] == 2
    assert stats["accepted"] == 2
    summary = logger.get_summary()
    assert summary is not None
    assert summary["telemetry_dropped_count"] == 2
    assert summary["sender_clean_known"] is False
    assert summary["sender_clean"] is False


def test_long_telemetry_session_remains_bounded_and_fails_closed() -> None:
    logger = TelemetryLogger("long", enabled=True, retain_records_after_save=True)
    for event_index in range(2 * 1_024):
        logger.record(
            event_index=event_index,
            kind="down" if event_index % 2 == 0 else "up",
            scheduled_us=event_index * 1_000,
            actual_us=event_index * 1_000 + 10,
            lateness_us=10,
            send_duration_us=5,
            scan_codes=(21,),
            sent_scan_codes=(21,),
            reason="long-session",
        )

    summary = logger.get_summary()
    assert summary is not None
    assert len(logger.records) == 1_024
    assert summary["telemetry_truncated"] is True
    assert summary["sender_clean_known"] is False
    assert summary["sender_clean"] is False


def test_native_trace_materializer_preserves_negative_completion_error() -> None:
    record = materialize_native_trace(
        _compact_output(
            completion_error_ticks=-250,
            authored_completion_error_ticks=-250,
        )
    )[0]

    assert record.sender_completion_error_us == -25
    assert record.visible_lateness_us == -25
    assert record.dispatch_lateness_us == -25


def test_native_trace_keeps_authored_and_effective_completion_residuals_distinct() -> None:
    record = materialize_native_trace(
        _compact_output(
            completion_error_ticks=500,
            authored_completion_error_ticks=-500,
        )
    )[0]

    assert record.sender_completion_error_us == 50
    assert record.visible_lateness_us == -50
    assert record.dispatch_lateness_us == -50


@pytest.mark.parametrize(
    "mutate",
    [
        lambda output: output["records"][0].__setitem__(
            "completion_error_ticks", 1 << 63
        ),
        lambda output: output["records"][0].__setitem__(
            "completion_error_ticks", -(1 << 63) - 1
        ),
        lambda output: output["records"][0].__setitem__(
            "authored_completion_error_ticks", 1 << 63
        ),
        lambda output: output["records"][0].__setitem__(
            "authored_completion_error_ticks", -(1 << 63) - 1
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


@pytest.mark.parametrize(
    "field_value",
    [
        {"requested_count": 2},
        {"sent_count": 2},
        {"skipped_count": 2},
        {"send_attempts": 256},
    ],
)
def test_native_trace_materializer_rejects_invalid_delivery_counts(
    field_value: dict[str, int],
) -> None:
    output = _compact_output()
    output["records"][0].update(field_value)

    with pytest.raises(ValueError, match="delivery counts"):
        materialize_native_trace(output)


@pytest.mark.parametrize("schema_version", [3, 4, 5, 6])
def test_native_trace_materializer_rejects_legacy_schema(schema_version: int) -> None:
    with pytest.raises(ValueError, match="schema version"):
        materialize_native_trace(_compact_output(schema_version=schema_version))


def test_native_records_are_included_in_summary_with_semantic_counts() -> None:
    logger = TelemetryLogger("native", enabled=True, retain_records_after_save=True)
    logger.ingest_native_output(
        {
            "schema_version": 7,
            "qpc_frequency_hz": 10_000_000,
            "attempted": 2,
            "accepted": 2,
            "dropped": 0,
            "truncated": False,
            "records": [
                _compact_output()["records"][0],
                {
                    **_compact_output(
                        completion_error_ticks=-250,
                        authored_completion_error_ticks=250,
                        requested_count=3,
                        sent_count=2,
                        skipped_count=1,
                    )["records"][0],
                    "event_index": 1,
                    "kind": 1,
                    "polyphony": 3,
                },
            ],
        }
    )

    summary = logger.get_summary()
    assert summary is not None
    assert summary["attempted_dispatches"] == 2
    assert summary["intended_down_count"] == 1
    assert summary["intended_up_count"] == 3
    assert summary["sent_down_count"] == 1
    assert summary["sent_up_count"] == 2
    assert summary["backend_skipped_up_count"] == 1
    assert summary["send_duration_us"]["p50_us"] > 0
    assert summary["visible_lateness_us"]["p50_us"] == 25.0


@pytest.mark.parametrize(
    ("requested_count", "sent_count", "skipped_count", "send_attempts"),
    [(1, 0, 0, 2), (1, 0, 1, 1)],
)
def test_native_zero_or_partial_send_is_counted_without_fake_scan_codes(
    requested_count: int, sent_count: int, skipped_count: int, send_attempts: int
) -> None:
    logger = TelemetryLogger("native", enabled=True, retain_records_after_save=True)
    logger.ingest_native_output(
        {
            "schema_version": 7,
            "qpc_frequency_hz": 10_000_000,
            "attempted": 1,
            "accepted": 1,
            "dropped": 0,
            "truncated": False,
            "records": [
                _compact_output(
                    requested_count=requested_count,
                    sent_count=sent_count,
                    skipped_count=skipped_count,
                    send_attempts=send_attempts,
                )["records"][0]
            ],
        }
    )
    summary = logger.get_summary()
    assert summary is not None
    assert summary["attempted_dispatches"] == 1
    assert summary["intended_down_count"] == requested_count
    assert summary["sent_down_count"] == sent_count
    assert summary["backend_skipped_down_count"] == skipped_count


def test_focus_blocked_trace_is_not_counted_as_backend_dispatch() -> None:
    blocked = _compact_output(
        authored_completion_error_ticks=0,
        requested_count=0,
        sent_count=0,
        skipped_count=0,
        send_attempts=0,
    )["records"][0]
    blocked.update({"outcome": 3, "polyphony": 1, "flags": 8})
    sent = _compact_output()["records"][0]
    sent["event_index"] = 1

    logger = TelemetryLogger("focus", enabled=True, retain_records_after_save=True)
    logger.ingest_native_output(
        {
            "schema_version": 7,
            "qpc_frequency_hz": 10_000_000,
            "attempted": 2,
            "accepted": 2,
            "dropped": 0,
            "truncated": False,
            "records": [blocked, sent],
        }
    )

    summary = logger.get_summary()
    assert summary is not None
    assert summary["attempted_dispatches"] == 1
    assert summary["intended_down_count"] == 1
    assert summary["sent_down_count"] == 1
    assert summary["sender_clean"] is True
    assert summary["send_duration_us"]["p50_us"] > 0


def test_suppressed_stale_up_is_not_counted_as_backend_dispatch() -> None:
    suppressed = _compact_output(
        authored_completion_error_ticks=0,
        requested_count=0,
        sent_count=0,
        skipped_count=0,
        send_attempts=0,
    )["records"][0]
    suppressed.update({"kind": 1, "outcome": 4, "polyphony": 1, "flags": 8})
    sent = _compact_output()["records"][0]
    sent["event_index"] = 1

    logger = TelemetryLogger("stale-up", enabled=True, retain_records_after_save=True)
    logger.ingest_native_output(
        {
            "schema_version": 7,
            "qpc_frequency_hz": 10_000_000,
            "attempted": 2,
            "accepted": 2,
            "dropped": 0,
            "truncated": False,
            "records": [suppressed, sent],
        }
    )

    summary = logger.get_summary()
    assert summary is not None
    assert summary["attempted_dispatches"] == 1


@pytest.mark.parametrize(
    ("sent_count", "skipped_count", "expected_events", "expected_keys"),
    [(2, 1, 0, 0), (0, 1, 1, 1), (0, 3, 1, 3)],
)
def test_noop_skipped_count_has_event_semantics(
    sent_count: int,
    skipped_count: int,
    expected_events: int,
    expected_keys: int,
) -> None:
    output = _compact_output(
        requested_count=max(3, sent_count + skipped_count),
        sent_count=sent_count,
        skipped_count=skipped_count,
        send_attempts=1,
    )
    output["records"][0]["polyphony"] = 3
    logger = TelemetryLogger("noop", enabled=True, retain_records_after_save=True)
    logger.ingest_native_output(output)

    summary = logger.get_summary()
    assert summary is not None
    assert summary["noop_skipped_count"] == expected_events
    assert summary["noop_skipped_key_count"] == expected_keys


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
