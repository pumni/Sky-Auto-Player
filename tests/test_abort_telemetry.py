"""Phase 0 instrumentation: ``abort_counts_by_reason`` telemetry.

This module is the failing-test-first seed for the SendInput-lifecycle plan. Phase 0 only
adds an additive per-reason abort counter on ``TelemetryLogger`` and exposes it on the
summary JSON. Phase 1 wires the actual ``abort_input_safe`` callers to ``record_abort``;
until then these tests cover the counter API itself in isolation.

Contract:
* Counter starts empty (no aborts -> empty dict, not a key-noise baseline).
* ``record_abort`` is idempotent-per-reason and free-threaded safe under the no-GIL build
  (it is a dict-get + int bump, the same pattern as ``record_pause``).
* The summary surfaces the snapshot dict by value (callers may not mutate it later).
* ``get_send_diagnostics`` (the inputs.py counter block) is unrelated and must NOT be
  extended with abort counts — abort is an orchestration-level event, not an inputs.py one.
"""

from __future__ import annotations

from sky_music.orchestration.telemetry import TelemetryLogger


def _fresh_logger() -> TelemetryLogger:
    # enabled=False keeps it off disk (no log_filepath side effects).
    return TelemetryLogger("phase0-song", enabled=False, run_id="phase0-test")


def test_abort_counts_starts_empty() -> None:
    logger = _fresh_logger()
    assert logger.abort_counts_by_reason == {}


def test_record_abort_tallies_per_reason() -> None:
    logger = _fresh_logger()
    logger.record_abort("manual_pause")
    logger.record_abort("focus_lost")
    logger.record_abort("manual_pause")
    assert logger.abort_counts_by_reason == {"manual_pause": 2, "focus_lost": 1}


def test_record_abort_accepts_unknown_reasons_without_schema_change() -> None:
    # The plan enumerates canonical reasons, but new callers must not require a code change
    # here — stability is a per-call property, not a closed set.
    logger = _fresh_logger()
    logger.record_abort("future_reason_we_must_not_reject")
    assert logger.abort_counts_by_reason == {"future_reason_we_must_not_reject": 1}


def test_record_abort_runs_with_telemetry_disabled() -> None:
    # record_pause guards on `if not self.enabled`. record_abort is additive diagnostic
    # state (no disk); it MUST still tally even when CSV telemetry is disabled, so test
    # fixtures without the log directory get correct counts. The summary is what surfaces
    # them, and the summary is computed regardless of the enabled flag.
    logger = TelemetryLogger("phase0-song", enabled=False, run_id="phase0-test-disabled")
    logger.record_abort("panic")
    assert logger.abort_counts_by_reason == {"panic": 1}


def test_summary_exposes_abort_counts_by_reason_snapshot(tmp_path) -> None:
    # get_summary() returns None when records are empty AND no cached summary exists
    # (see _rows_for_summary contract). One recorded event is enough to force the summary
    # to be computed. We only care that the new key is present and is a value snapshot.
    # `enabled=True` would also mkdir logs/, so we point the summary path at a tmp dir
    # by instead using the no-record shortcut: get_summary tolerates an empty run by
    # returning None; to force computation we need at least one record. The enabled flag
    # is what gates the record acceptance, so we turn it on and contain the side effects
    # by writing into the tmp_path working dir.
    import os
    cwd = os.getcwd()
    os.chdir(tmp_path)
    try:
        logger = TelemetryLogger("phase0-song", enabled=True, run_id="phase0-summary")
        logger.record_abort("focus_lost")
        logger.record_abort("focus_lost")
        logger.record_abort("panic")
        # Force summary computation: one minimal telemetry record is required.
        logger.record(
            event_index=0,
            kind="down",
            scheduled_us=0,
            actual_us=0,
            lateness_us=0,
            send_duration_us=0,
            scan_codes=(0x15,),
            reason="",
            sent_scan_codes=(0x15,),
        )
        logger.save()
    finally:
        os.chdir(cwd)

    summary = logger.get_summary()
    assert summary is not None
    assert summary["abort_counts_by_reason"] == {"focus_lost": 2, "panic": 1}


def test_summary_abort_counts_snapshot_is_a_copy_not_live_view(tmp_path) -> None:
    # Callers must not be able to mutate the live counter via the summary. (We do not
    # promise deep-immutability of every nested dict, but the abort map specifically is
    # returned as a fresh dict via dict(self.abort_counts_by_reason).)
    import os
    cwd = os.getcwd()
    os.chdir(tmp_path)
    try:
        logger = TelemetryLogger("phase0-song", enabled=True, run_id="phase0-snapshot")
        logger.record_abort("manual_pause")
        logger.record(
            event_index=0,
            kind="down",
            scheduled_us=0,
            actual_us=0,
            lateness_us=0,
            send_duration_us=0,
            scan_codes=(0x15,),
            reason="",
            sent_scan_codes=(0x15,),
        )
        logger.save()
    finally:
        os.chdir(cwd)

    summary = logger.get_summary()
    assert summary is not None
    snapshot = summary["abort_counts_by_reason"]
    snapshot["manual_pause"] = 999  # mutate the snapshot
    assert logger.abort_counts_by_reason["manual_pause"] == 1, "live state untouched"
