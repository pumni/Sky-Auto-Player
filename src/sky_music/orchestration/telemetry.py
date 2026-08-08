import contextlib
import csv
import itertools
import json
import math
import random
import sys
import time
from collections.abc import Mapping
from pathlib import Path
from typing import Any, TextIO

from sky_music.orchestration.native_models import BackendHealth

# Soft threshold reported by flush_if_large(). Runtime callers never export or
# truncate here; lifecycle save() owns file I/O after timed dispatch has ended.
_TELEMETRY_FLUSH_CHUNK = 10_000
# Hard cap for the retain-first policy. Once full, record() performs only O(1)
# counter updates and stops accepting detail records.
_TELEMETRY_MAX_BUFFER = 1_024
NATIVE_TELEMETRY_SCHEMA_VERSION = 9


def _optional_int(value: Any) -> int | None:
    if value is None or value == "":
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError("native telemetry integer field is invalid")
    return value


def _required_nonnegative_int(value: Any, field: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"native telemetry field {field} is invalid")
    return value


def _required_signed_int64(value: Any, field: str) -> int:
    if (
        isinstance(value, bool)
        or not isinstance(value, int)
        or not -(1 << 63) <= value <= (1 << 63) - 1
    ):
        raise ValueError(f"native telemetry field {field} is invalid")
    return value


_CSV_FIELDS: list[str] = [
    "song",
    "event_index",
    "authored_ticks",
    "effective_deadline_ticks",
    "wake_ticks",
    "send_started_ticks",
    "send_completed_ticks",
    "completion_error_ticks",
    "authored_completion_error_ticks",
    "native_requested_count",
    "native_sent_count",
    "native_skipped_count",
    "applied_lead_ticks",
    "win32_error",
    "dispatch_id",
    "packet_id",
    "kind",
    "scheduled_us",
    "scheduled_timeline_us",
    "effective_deadline_us",
    "actual_us",
    "wake_timeline_us",
    "dispatch_completed_us",
    "evidence_scope",
    "lateness_us",
    "visible_lateness_us",
    "send_duration_us",
    "send_duration_pure_us",
    "bookkeeping_us",
    "dispatch_lateness_us",
    "scan_codes",
    "sent_scan_codes",
    "skipped_scan_codes",
    "generation_ids",
    "runtime_outcome",
    "deferred_by_us",
    "pre_send_spin_us",
    "idle_gap_us",
    "reason",
    "applied_lead_us",
    "first_win32_error",
    "last_win32_error",
    "send_attempts",
    "zero_progress_retries",
    "head_of_line_delay_us",
    "same_timestamp_release_before_down",
    "authored_us",
    "wait_target_us",
    "wake_us",
    "wake_error_us",
    "send_started_us",
    "send_completed_us",
    "sender_completion_error_us",
    "send_operation_duration_us",
    "sender_started_us",
    "sender_completed_us",
    "sendinput_call_duration_us",
    "core_post_send_duration_us",
    "delivery_first_us",
    "delivery_last_us",
    "delivery_last_error_us",
    "intra_chord_delivery_spread_us",
    "lead_components",
]

_CSV_INT_FIELDS: frozenset[str] = frozenset(
    {
        "event_index",
        "dispatch_id",
        "packet_id",
        "scheduled_us",
        "scheduled_timeline_us",
        "actual_us",
        "authored_ticks",
        "effective_deadline_ticks",
        "effective_deadline_us",
        "wake_ticks",
        "send_started_ticks",
        "send_completed_ticks",
        "completion_error_ticks",
        "authored_completion_error_ticks",
        "native_requested_count",
        "native_sent_count",
        "native_skipped_count",
        "applied_lead_ticks",
        "win32_error",
        "wake_timeline_us",
        "dispatch_completed_us",
        "lateness_us",
        "visible_lateness_us",
        "send_duration_us",
        "send_duration_pure_us",
        "bookkeeping_us",
        "dispatch_lateness_us",
        "deferred_by_us",
        "pre_send_spin_us",
        "idle_gap_us",
        "applied_lead_us",
        "first_win32_error",
        "last_win32_error",
        "send_attempts",
        "zero_progress_retries",
        "head_of_line_delay_us",
        "authored_us",
        "wait_target_us",
        "wake_us",
        "wake_error_us",
        "send_started_us",
        "send_completed_us",
        "sender_completion_error_us",
        "send_operation_duration_us",
        "sender_started_us",
        "sender_completed_us",
        "sendinput_call_duration_us",
        "core_post_send_duration_us",
        "delivery_first_us",
        "delivery_last_us",
        "delivery_last_error_us",
        "intra_chord_delivery_spread_us",
    }
)


class TelemetryRecord:
    __slots__ = (
        "_dict",
        "actual_us",
        "applied_lead_ticks",
        "applied_lead_us",
        "authored_completion_error_ticks",
        "authored_ticks",
        "authored_us",
        "bookkeeping_us",
        "completion_error_ticks",
        "core_post_send_duration_us",
        "deferred_by_us",
        "delivery_first_us",
        "delivery_last_error_us",
        "delivery_last_us",
        "dispatch_completed_us",
        "dispatch_id",
        "dispatch_lateness_us",
        "effective_deadline_ticks",
        "effective_deadline_us",
        "event_index",
        "first_win32_error",
        "generation_ids",
        "head_of_line_delay_us",
        "idle_gap_us",
        "intra_chord_delivery_spread_us",
        "kind",
        "last_win32_error",
        "lateness_us",
        "lead_components",
        "native_polyphony",
        "native_requested_count",
        "native_sent_count",
        "native_skipped_count",
        "packet_id",
        "pre_send_spin_us",
        "reason",
        "runtime_outcome",
        "same_timestamp_release_before_down",
        "scan_codes",
        "scheduled_timeline_us",
        "scheduled_us",
        "send_attempts",
        "send_completed_ticks",
        "send_completed_us",
        "send_duration_pure_us",
        "send_duration_us",
        "send_operation_duration_us",
        "send_started_ticks",
        "send_started_us",
        "sender_completed_us",
        "sender_completion_error_us",
        "sender_started_us",
        "sendinput_call_duration_us",
        "sent_scan_codes",
        "skipped_scan_codes",
        "song_name",
        "visible_lateness_us",
        "wait_target_us",
        "wake_error_us",
        "wake_ticks",
        "wake_timeline_us",
        "wake_us",
        "win32_error",
        "zero_progress_retries",
    )

    def __init__(
        self,
        song_name: str,
        event_index: int,
        kind: str,
        scheduled_us: int,
        actual_us: int,
        lateness_us: int,
        send_duration_us: int,
        scan_codes: tuple[int, ...],
        reason: str,
        dispatch_id: int | None,
        dispatch_completed_us: int | None,
        sent_scan_codes: tuple[int, ...] | None,
        skipped_scan_codes: tuple[int, ...],
        generation_ids: tuple[int, ...],
        runtime_outcome: str,
        deferred_by_us: int,
        pre_send_spin_us: int,
        idle_gap_us: int,
        visible_lateness_us: int | None,
        applied_lead_us: int = 0,
        send_duration_pure_us: int = 0,
        bookkeeping_us: int = 0,
        dispatch_lateness_us: int = 0,
        first_win32_error: int | None = None,
        last_win32_error: int | None = None,
        send_attempts: int = 0,
        zero_progress_retries: int = 0,
        head_of_line_delay_us: int | None = None,
        same_timestamp_release_before_down: bool | None = None,
        packet_id: int | None = None,
        authored_us: int | None = None,
        wait_target_us: int | None = None,
        wake_us: int | None = None,
        wake_error_us: int | None = None,
        send_started_us: int | None = None,
        send_completed_us: int | None = None,
        sender_completion_error_us: int | None = None,
        delivery_first_us: int | None = None,
        delivery_last_us: int | None = None,
        delivery_last_error_us: int | None = None,
        intra_chord_delivery_spread_us: int | None = None,
        lead_components: int | None = None,
        scheduled_timeline_us: int | None = None,
        wake_timeline_us: int | None = None,
        sender_started_us: int | None = None,
        sender_completed_us: int | None = None,
        sendinput_call_duration_us: int | None = None,
        core_post_send_duration_us: int | None = None,
        send_operation_duration_us: int | None = None,
        authored_ticks: int = 0,
        effective_deadline_ticks: int = 0,
        wake_ticks: int = 0,
        send_started_ticks: int = 0,
        send_completed_ticks: int = 0,
        completion_error_ticks: int = 0,
        applied_lead_ticks: int = 0,
        win32_error: int = 0,
        native_polyphony: int | None = None,
        native_requested_count: int | None = None,
        native_sent_count: int | None = None,
        native_skipped_count: int | None = None,
        authored_completion_error_ticks: int = 0,
    ) -> None:
        self._dict = None
        self.authored_ticks = authored_ticks
        self.effective_deadline_ticks = effective_deadline_ticks
        self.wake_ticks = wake_ticks
        self.send_started_ticks = send_started_ticks
        self.send_completed_ticks = send_completed_ticks
        self.completion_error_ticks = completion_error_ticks
        self.authored_completion_error_ticks = authored_completion_error_ticks
        self.applied_lead_ticks = applied_lead_ticks
        self.win32_error = win32_error
        # Native compact telemetry carries polyphony, but it intentionally is
        # not part of the legacy CSV schema.  Keep it only as a consumer-side
        # derived value for acceptance reports; never infer it from scan codes.
        self.native_polyphony = native_polyphony
        self.native_requested_count = native_requested_count
        self.native_sent_count = native_sent_count
        self.native_skipped_count = native_skipped_count
        self.song_name = song_name
        self.event_index = event_index
        self.kind = kind
        self.scheduled_us = scheduled_us
        self.scheduled_timeline_us = (
            scheduled_us if scheduled_timeline_us is None else scheduled_timeline_us
        )
        self.effective_deadline_us = self.scheduled_timeline_us
        self.actual_us = actual_us
        self.lateness_us = lateness_us
        self.send_duration_us = send_duration_us
        self.scan_codes = scan_codes
        self.reason = reason
        self.dispatch_id = dispatch_id
        self.dispatch_completed_us = dispatch_completed_us
        self.sent_scan_codes = sent_scan_codes
        self.skipped_scan_codes = skipped_scan_codes
        self.generation_ids = generation_ids
        self.runtime_outcome = runtime_outcome
        self.deferred_by_us = deferred_by_us
        self.pre_send_spin_us = pre_send_spin_us
        self.idle_gap_us = idle_gap_us
        self.visible_lateness_us = visible_lateness_us
        self.applied_lead_us = applied_lead_us
        self.send_duration_pure_us = send_duration_pure_us
        self.bookkeeping_us = bookkeeping_us
        self.dispatch_lateness_us = dispatch_lateness_us
        self.first_win32_error = first_win32_error
        self.last_win32_error = last_win32_error
        self.send_attempts = send_attempts
        self.zero_progress_retries = zero_progress_retries
        self.head_of_line_delay_us = head_of_line_delay_us
        self.same_timestamp_release_before_down = same_timestamp_release_before_down
        self.packet_id = packet_id
        self.authored_us = authored_us
        self.wait_target_us = wait_target_us
        self.wake_us = wake_us
        self.wake_error_us = wake_error_us
        self.wake_timeline_us = (
            actual_us if wake_timeline_us is None else wake_timeline_us
        )
        self.send_started_us = send_started_us
        self.send_completed_us = send_completed_us
        self.sender_completion_error_us = sender_completion_error_us
        self.sender_started_us = (
            send_started_us if sender_started_us is None else sender_started_us
        )
        self.sender_completed_us = (
            send_completed_us if sender_completed_us is None else sender_completed_us
        )
        self.sendinput_call_duration_us: int | None = (
            send_duration_pure_us
            if sendinput_call_duration_us is None
            else sendinput_call_duration_us
        )
        self.core_post_send_duration_us = (
            bookkeeping_us
            if core_post_send_duration_us is None
            else core_post_send_duration_us
        )
        self.send_operation_duration_us = send_operation_duration_us
        self.delivery_first_us = delivery_first_us
        self.delivery_last_us = delivery_last_us
        self.delivery_last_error_us = delivery_last_error_us
        self.intra_chord_delivery_spread_us = intra_chord_delivery_spread_us
        self.lead_components = lead_components

    def _materialize(self) -> dict:
        if self._dict is None:
            scan_codes_str = ";".join(str(sc) for sc in self.scan_codes)
            sent_scan_codes = (
                self.scan_codes
                if self.sent_scan_codes is None
                else self.sent_scan_codes
            )
            visible_lat = self.visible_lateness_us
            if visible_lat is None:
                completed_us = self.dispatch_completed_us
                visible_lat = (
                    completed_us - self.scheduled_us
                    if completed_us is not None
                    else self.actual_us + self.send_duration_us - self.scheduled_us
                )
            self._dict = {
                "song": self.song_name,
                "event_index": self.event_index,
                "authored_ticks": self.authored_ticks,
                "effective_deadline_ticks": self.effective_deadline_ticks,
                "wake_ticks": self.wake_ticks,
                "send_started_ticks": self.send_started_ticks,
                "send_completed_ticks": self.send_completed_ticks,
                "completion_error_ticks": self.completion_error_ticks,
                "authored_completion_error_ticks": self.authored_completion_error_ticks,
                "native_requested_count": self.native_requested_count,
                "native_sent_count": self.native_sent_count,
                "native_skipped_count": self.native_skipped_count,
                "applied_lead_ticks": self.applied_lead_ticks,
                "win32_error": self.win32_error,
                "dispatch_id": self.event_index
                if self.dispatch_id is None
                else self.dispatch_id,
                "packet_id": 0 if self.packet_id is None else self.packet_id,
                "evidence_scope": "sender_completion",
                "kind": self.kind,
                "scheduled_us": self.scheduled_us,
                "scheduled_timeline_us": self.scheduled_timeline_us,
                "effective_deadline_us": self.effective_deadline_us,
                "actual_us": self.actual_us,
                "wake_timeline_us": self.wake_timeline_us,
                "dispatch_completed_us": (
                    self.actual_us + self.send_duration_us
                    if self.dispatch_completed_us is None
                    else self.dispatch_completed_us
                ),
                "lateness_us": self.lateness_us,
                "visible_lateness_us": visible_lat,
                "send_duration_us": self.send_duration_us,
                "scan_codes": scan_codes_str,
                "sent_scan_codes": ";".join(str(sc) for sc in sent_scan_codes),
                "skipped_scan_codes": ";".join(
                    str(sc) for sc in self.skipped_scan_codes
                ),
                "generation_ids": ";".join(
                    str(generation_id) for generation_id in self.generation_ids
                ),
                "runtime_outcome": self.runtime_outcome,
                "deferred_by_us": self.deferred_by_us,
                "pre_send_spin_us": self.pre_send_spin_us,
                "idle_gap_us": self.idle_gap_us,
                "reason": self.reason,
                "applied_lead_us": self.applied_lead_us,
                "first_win32_error": self.first_win32_error,
                "last_win32_error": self.last_win32_error,
                "send_attempts": self.send_attempts,
                "zero_progress_retries": self.zero_progress_retries,
                "send_duration_pure_us": self.send_duration_pure_us,
                "bookkeeping_us": self.bookkeeping_us,
                "dispatch_lateness_us": self.dispatch_lateness_us,
                "head_of_line_delay_us": self.head_of_line_delay_us,
                "same_timestamp_release_before_down": self.same_timestamp_release_before_down,
                "authored_us": self.authored_us,
                "wait_target_us": self.wait_target_us,
                "wake_us": self.wake_us,
                "wake_error_us": self.wake_error_us,
                "send_started_us": self.send_started_us,
                "send_completed_us": self.send_completed_us,
                "sender_completion_error_us": self.sender_completion_error_us,
                "send_operation_duration_us": self.send_operation_duration_us,
                "sender_started_us": self.sender_started_us,
                "sender_completed_us": self.sender_completed_us,
                "sendinput_call_duration_us": self.sendinput_call_duration_us,
                "core_post_send_duration_us": self.core_post_send_duration_us,
                "delivery_first_us": self.delivery_first_us,
                "delivery_last_us": self.delivery_last_us,
                "delivery_last_error_us": self.delivery_last_error_us,
                "intra_chord_delivery_spread_us": self.intra_chord_delivery_spread_us,
                "lead_components": self.lead_components,
            }
        return self._dict

    def __getitem__(self, key):
        return self._materialize()[key]

    def get(self, key, default=None):
        return self._materialize().get(key, default)

    def __contains__(self, key):
        return key in self._materialize()

    def __iter__(self):
        return iter(self._materialize())

    def __len__(self):
        return len(self._materialize())

    def keys(self):
        return self._materialize().keys()

    def values(self):
        return self._materialize().values()

    def items(self):
        return self._materialize().items()


_NATIVE_TRACE_OUTCOMES: dict[int, str] = {
    0: "sent",
    1: "deferred_release",
    2: "failed_note_off",
    3: "blocked_unfocused",
    4: "suppressed_stale_up",
    5: "recovered_zero_progress_but_late",
    6: "strict_completion_slo_exceeded",
    7: "chord_integrity_lost",
    8: "aborted",
}


def materialize_native_trace(
    output: dict[str, Any], *, song_name: str = "native"
) -> list[TelemetryRecord]:
    """Decode the current compact QPC-tick native trace exactly once.

    The native schema deliberately contains only fixed-width fields.  All
    microsecond values and human-readable names here are derived consumer
    values; callers must not expect them in the native JSON envelope.
    """

    # Schemas 7 and 8 remain readable for historical native artifacts; Rust
    # emits only the current schema 9.
    if output.get("schema_version") not in (7, 8, NATIVE_TELEMETRY_SCHEMA_VERSION):
        raise ValueError("unsupported native telemetry schema version")
    records = output.get("records")
    if not isinstance(records, list):
        raise ValueError("invalid native telemetry envelope")
    frequency = output.get("qpc_frequency_hz")
    if isinstance(frequency, bool) or not isinstance(frequency, int) or frequency <= 0:
        raise ValueError("native telemetry envelope is missing qpc_frequency_hz")

    def ticks_to_us(ticks: Any, field: str) -> int:
        value = _required_nonnegative_int(ticks, field)
        return (value * 1_000_000) // frequency

    def signed_ticks_to_us(ticks: int) -> int:
        sign = -1 if ticks < 0 else 1
        return sign * ((abs(ticks) * 1_000_000) // frequency)

    materialized: list[TelemetryRecord] = []
    for row in records:
        if not isinstance(row, dict):
            raise ValueError("invalid native telemetry record")
        event_index = _required_nonnegative_int(row.get("event_index"), "event_index")
        kind_code = _required_nonnegative_int(row.get("kind"), "kind")
        outcome_code = _required_nonnegative_int(row.get("outcome"), "outcome")
        polyphony = _required_nonnegative_int(row.get("polyphony"), "polyphony")
        _required_nonnegative_int(row.get("flags"), "flags")
        if kind_code not in (0, 1) or not 0 <= polyphony <= 15:
            raise ValueError("native telemetry record has invalid kind/polyphony")
        outcome = _NATIVE_TRACE_OUTCOMES.get(outcome_code)
        if outcome is None:
            raise ValueError("native telemetry record has unknown outcome code")

        authored_ticks = _required_nonnegative_int(
            row.get("authored_ticks"), "authored_ticks"
        )
        effective_ticks = _required_nonnegative_int(
            row.get("effective_deadline_ticks"), "effective_deadline_ticks"
        )
        wake_ticks = _required_nonnegative_int(row.get("wake_ticks"), "wake_ticks")
        started_ticks = _required_nonnegative_int(
            row.get("send_started_ticks"), "send_started_ticks"
        )
        completed_ticks = _required_nonnegative_int(
            row.get("send_completed_ticks"), "send_completed_ticks"
        )
        core_post_send_duration_us = _required_nonnegative_int(
            row.get("core_post_send_duration_us"), "core_post_send_duration_us"
        )
        completion_error_ticks = _required_signed_int64(
            row.get("completion_error_ticks"), "completion_error_ticks"
        )
        authored_completion_error_ticks = _required_signed_int64(
            row.get("authored_completion_error_ticks"),
            "authored_completion_error_ticks",
        )
        lead_ticks = _required_nonnegative_int(
            row.get("applied_lead_ticks"), "applied_lead_ticks"
        )
        win32_error = _required_nonnegative_int(row.get("win32_error"), "win32_error")
        requested_count = _required_nonnegative_int(
            row.get("requested_count"), "requested_count"
        )
        sent_count = _required_nonnegative_int(row.get("sent_count"), "sent_count")
        skipped_count = _required_nonnegative_int(
            row.get("skipped_count"), "skipped_count"
        )
        send_attempts = _required_nonnegative_int(
            row.get("send_attempts"), "send_attempts"
        )
        if (
            send_attempts > 255
            or requested_count > polyphony
            or sent_count > requested_count
            or skipped_count > requested_count
            or sent_count + skipped_count > requested_count
        ):
            raise ValueError("native telemetry record has invalid delivery counts")

        authored_us = ticks_to_us(authored_ticks, "authored_ticks")
        effective_us = ticks_to_us(effective_ticks, "effective_deadline_ticks")
        wake_us = ticks_to_us(wake_ticks, "wake_ticks")
        # Zero is a valid QPC-relative timestamp for the first dispatch.  The
        # compact native schema stores timestamps as fixed-width integers, so
        # it must not be treated as an absent optional value here.
        started_us = ticks_to_us(started_ticks, "send_started_ticks")
        completed_us = ticks_to_us(completed_ticks, "send_completed_ticks")
        sender_completion_error_us = signed_ticks_to_us(completion_error_ticks)
        authored_completion_error_us = signed_ticks_to_us(authored_completion_error_ticks)
        send_duration_us = (
            max(0, completed_us - started_us) if started_us is not None else 0
        )
        materialized.append(
            TelemetryRecord(
                song_name=song_name,
                event_index=event_index,
                kind="up" if kind_code else "down",
                scheduled_us=authored_us,
                actual_us=wake_us,
                lateness_us=wake_us - effective_us,
                send_duration_us=send_duration_us,
                scan_codes=(),
                reason="",
                dispatch_id=event_index,
                dispatch_completed_us=completed_us,
                sent_scan_codes=(),
                skipped_scan_codes=(),
                generation_ids=(),
                runtime_outcome=outcome,
                deferred_by_us=0,
                pre_send_spin_us=0,
                idle_gap_us=0,
                visible_lateness_us=authored_completion_error_us,
                applied_lead_us=ticks_to_us(lead_ticks, "applied_lead_ticks"),
                send_duration_pure_us=send_duration_us,
                dispatch_lateness_us=authored_completion_error_us,
                first_win32_error=win32_error or None,
                last_win32_error=win32_error or None,
                send_attempts=send_attempts,
                packet_id=event_index,
                authored_us=authored_us,
                scheduled_timeline_us=effective_us,
                wake_us=wake_us,
                wake_error_us=wake_us - effective_us,
                wake_timeline_us=wake_us,
                send_started_us=started_us,
                send_completed_us=completed_us,
                sender_completion_error_us=sender_completion_error_us,
                send_operation_duration_us=send_duration_us,
                sendinput_call_duration_us=send_duration_us,
                bookkeeping_us=core_post_send_duration_us,
                core_post_send_duration_us=core_post_send_duration_us,
                authored_ticks=authored_ticks,
                effective_deadline_ticks=effective_ticks,
                wake_ticks=wake_ticks,
                send_started_ticks=started_ticks,
                send_completed_ticks=completed_ticks,
                completion_error_ticks=completion_error_ticks,
                authored_completion_error_ticks=authored_completion_error_ticks,
                applied_lead_ticks=lead_ticks,
                win32_error=win32_error,
                native_polyphony=polyphony,
                native_requested_count=requested_count,
                native_sent_count=sent_count,
                native_skipped_count=skipped_count,
            )
        )
    return materialized


class TelemetryLogger:
    """Records precise microsecond timing metrics into clean CSV and companion summary JSON files for calibration."""

    last_picker_cleanup: dict | None = None
    last_thread_census: dict | None = None

    def __init__(
        self,
        song_name: str,
        enabled: bool = False,
        hold_frames: float = 1.0,
        hold_label: str = "hold 1.00f",
        tempo_scale: float = 1.0,
        run_id: str | None = None,
        fps: int | None = None,
        min_hold_us: int = 0,
        min_hold_margin_us: int = 500,
        min_hold_margin_source: str = "default_500",
        *,
        retain_records_after_save: bool = False,
    ):
        self.song_name = song_name
        self.enabled = enabled
        self.hold_frames = hold_frames
        self.hold_label = hold_label
        self.tempo_scale = tempo_scale
        self.fps = fps
        self.min_hold_us = max(0, min_hold_us)
        self.min_hold_margin_us = max(0, int(min_hold_margin_us))
        self.min_hold_margin_source = str(min_hold_margin_source)
        self.records: list[TelemetryRecord] = []
        # Summary computed by save()/get_summary() before records are dropped for hygiene.
        # Keeps get_summary() callable for late callers (engine _log_timing_summary, CLI,
        # tests) after save() clears the (often multi-MB) records list — otherwise they'd
        # see None because the early-empty guard in get_summary was the only signal.
        # Read-only contract: callers read keys, never mutate.
        self._last_summary: dict | None = None
        # Test-only hook: keeps ``records`` populated after save() so tests that assert
        # on raw per-event fields after play() keep working without re-architecting onto
        # the CSV/summary path. Production always leaves this False.
        self._retain_records_after_save = retain_records_after_save
        self.log_filepath: Path | None = None
        self._csv_file: TextIO | None = None
        self._csv_writer: csv.DictWriter | None = None
        # Count of events already written to the open CSV (for summary re-read path).
        self._csv_events_written: int = 0
        # Offset into self.records of the first not-yet-written entry (retain mode).
        self._records_written_offset: int = 0
        # True once a mid-play flush cleared the in-memory list (disk becomes authoritative).
        self._cleared_mid_play: bool = False
        self._dropped_count: int = 0
        self._attempted_record_count: int = 0
        self._accepted_record_count: int = 0
        self._truncated: bool = False
        self._telemetry_capacity: int = _TELEMETRY_MAX_BUFFER
        self.backend_health: BackendHealth | None = None
        self.release_outcome = None
        self.input_path_degraded: bool = False
        self.input_path_warn_us: int = 300
        self.runtime_options: dict[str, object] = {}
        self.schedule_summary: dict | None = None
        self.generation_status_counts: dict[str, int] = {}
        self.pause_durations_us: dict[str, list[int]] = {
            "manual": [],
            "focus": [],
        }
        # Abort-reason telemetry (Phase 0 of the SendInput lifecycle plan). Pure additive
        # instrumentation: counts how many times each interrupt path fires the unified abort
        # helper. Reasons are stable strings ("manual_pause" | "focus_lost" | "panic" |
        # "quit" | "finished" | "error"). Empty unless an abort happens, so tests that do not
        # exercise aborts see an empty dict — never a key-noise baseline.
        self.abort_counts_by_reason: dict[str, int] = {}
        # Unique run ID generation
        if run_id is None:
            self.run_id = (
                f"{time.strftime('%Y%m%d-%H%M%S')}-{random.randint(1000, 9999)}"
            )
        else:
            self.run_id = run_id

        if self.enabled:
            logs_dir = Path("logs")
            logs_dir.mkdir(parents=True, exist_ok=True)
            self.log_filepath = logs_dir / f"playback_telemetry_{self.run_id}.csv"

    def record(
        self,
        event_index: int | None = None,
        kind: str | None = None,
        scheduled_us: int | None = None,
        actual_us: int | None = None,
        lateness_us: int | None = None,
        send_duration_us: int | None = None,
        scan_codes: tuple[int, ...] | None = None,
        reason: str | None = None,
        *,
        result: Any = None,
        dispatch_id: int | None = None,
        dispatch_completed_us: int | None = None,
        sent_scan_codes: tuple[int, ...] | None = None,
        skipped_scan_codes: tuple[int, ...] = (),
        generation_ids: tuple[int, ...] = (),
        runtime_outcome: str = "sent",
        deferred_by_us: int = 0,
        pre_send_spin_us: int = 0,
        idle_gap_us: int = 0,
        visible_lateness_us: int | None = None,
        applied_lead_us: int = 0,
        first_win32_error: int | None = None,
        last_win32_error: int | None = None,
        send_attempts: int = 0,
        zero_progress_retries: int = 0,
        head_of_line_delay_us: int | None = None,
        same_timestamp_release_before_down: bool | None = None,
        packet_id: int | None = None,
        authored_us: int | None = None,
        wait_target_us: int | None = None,
        wake_us: int | None = None,
        wake_error_us: int | None = None,
        send_started_us: int | None = None,
        send_completed_us: int | None = None,
        sender_completion_error_us: int | None = None,
        authored_completion_error_ticks: int = 0,
        send_operation_duration_us: int | None = None,
        delivery_first_us: int | None = None,
        delivery_last_us: int | None = None,
        delivery_last_error_us: int | None = None,
        intra_chord_delivery_spread_us: int | None = None,
        lead_components: int | None = None,
    ) -> None:
        send_duration_pure_us = 0
        bookkeeping_us = 0
        dispatch_lateness_us = 0

        if not self.enabled:
            return

        if result is not None:
            event_index = result.event_index
            scheduled_us = result.scheduled_us
            actual_us = result.actual_us
            lateness_us = result.lateness_us
            send_duration_us = result.send_duration_us
            dispatch_completed_us = result.dispatch_completed_us
            sent_scan_codes = result.sent_scan_codes
            skipped_scan_codes = result.skipped_scan_codes
            runtime_outcome = result.runtime_outcome
            deferred_by_us = getattr(result, "deferred_by_us", 0)
            visible_lateness_us = result.visible_lateness_us
            applied_lead_us = result.applied_lead_us
            first_win32_error = getattr(result, "first_win32_error", None)
            last_win32_error = getattr(result, "last_win32_error", None)
            send_attempts = getattr(result, "send_attempts", 0)
            zero_progress_retries = getattr(result, "zero_progress_retries", 0)
            send_duration_pure_us = getattr(result, "send_duration_pure_us", 0)
            bookkeeping_us = getattr(result, "bookkeeping_us", 0)
            dispatch_lateness_us = getattr(result, "dispatch_lateness_us", 0)
            head_of_line_delay_us = getattr(result, "head_of_line_delay_us", None)
            same_timestamp_release_before_down = getattr(
                result, "same_timestamp_release_before_down", None
            )
            packet_id = getattr(result, "packet_id", None)
            authored_us = getattr(result, "authored_us", None)
            wait_target_us = getattr(result, "wait_target_us", None)
            wake_us = getattr(result, "wake_us", None)
            wake_error_us = getattr(result, "wake_error_us", None)
            send_started_us = getattr(result, "send_started_us", None)
            send_completed_us = getattr(result, "send_completed_us", None)
            sender_completion_error_us = getattr(
                result, "sender_completion_error_us", None
            )
            authored_completion_error_ticks = getattr(
                result, "authored_completion_error_ticks", 0
            )
            send_operation_duration_us = getattr(
                result, "send_operation_duration_us", None
            )
            delivery_first_us = getattr(result, "delivery_first_us", None)
            delivery_last_us = getattr(result, "delivery_last_us", None)
            delivery_last_error_us = getattr(result, "delivery_last_error_us", None)
            intra_chord_delivery_spread_us = getattr(
                result, "intra_chord_delivery_spread_us", None
            )
            lead_components = getattr(result, "lead_components", None)

        assert event_index is not None
        assert kind is not None
        assert scheduled_us is not None
        assert actual_us is not None
        assert lateness_us is not None
        assert send_duration_us is not None
        assert scan_codes is not None
        assert reason is not None

        self._attempted_record_count += 1
        if len(self.records) >= self._telemetry_capacity:
            # Dispatch-thread cap path: constant-time bookkeeping only. Do not
            # construct a TelemetryRecord, slice the retained list, or export.
            self._dropped_count += 1
            self._truncated = True
            return

        self.records.append(
            TelemetryRecord(
                self.song_name,
                event_index,
                kind,
                scheduled_us,
                actual_us,
                lateness_us,
                send_duration_us,
                scan_codes,
                reason,
                dispatch_id,
                dispatch_completed_us,
                sent_scan_codes,
                skipped_scan_codes,
                generation_ids,
                runtime_outcome,
                deferred_by_us,
                pre_send_spin_us,
                idle_gap_us,
                visible_lateness_us,
                applied_lead_us,
                send_duration_pure_us,
                bookkeeping_us,
                dispatch_lateness_us,
                first_win32_error,
                last_win32_error,
                send_attempts,
                zero_progress_retries,
                head_of_line_delay_us,
                same_timestamp_release_before_down,
                packet_id,
                authored_us,
                wait_target_us,
                wake_us,
                wake_error_us,
                send_started_us,
                send_completed_us,
                sender_completion_error_us,
                delivery_first_us,
                delivery_last_us,
                delivery_last_error_us,
                intra_chord_delivery_spread_us,
                lead_components,
                send_operation_duration_us=send_operation_duration_us,
                authored_completion_error_ticks=authored_completion_error_ticks,
            )
        )
        self._accepted_record_count += 1

    def ingest_native_output(self, output: dict[str, Any]) -> None:
        """Ingest a terminal retain-first buffer produced by the Rust worker.

        Cross-validates the native envelope counters before any record is accepted:

        * ``attempted`` / ``accepted`` / ``dropped`` are non-negative integers;
        * ``truncated`` is a bool;
        * ``accepted == len(records)`` (the envelope's own self-report);
        * ``attempted >= accepted``;
        * ``dropped > 0`` implies ``truncated`` is True.

        Any violation raises ``ValueError`` rather than silently trusting the
        summary — the fail-closed seam: a truncated envelope cannot report a
        clean session.
        """
        if not self.enabled:
            return
        records = output.get("records")
        attempted = output.get("attempted")
        accepted_in = output.get("accepted")
        dropped = output.get("dropped")
        truncated = output.get("truncated")
        if not isinstance(records, list):
            raise ValueError("invalid native telemetry envelope: records is not a list")
        if isinstance(attempted, bool) or not isinstance(attempted, int) or attempted < 0:
            raise ValueError("invalid native telemetry envelope: attempted")
        if (
            isinstance(accepted_in, bool)
            or not isinstance(accepted_in, int)
            or accepted_in < 0
        ):
            raise ValueError("invalid native telemetry envelope: accepted")
        if isinstance(dropped, bool) or not isinstance(dropped, int) or dropped < 0:
            raise ValueError("invalid native telemetry envelope: dropped")
        if not isinstance(truncated, bool):
            raise ValueError("invalid native telemetry envelope: truncated")
        if accepted_in != len(records):
            raise ValueError(
                "invalid native telemetry envelope: accepted != len(records)"
            )
        if attempted < accepted_in:
            raise ValueError("invalid native telemetry envelope: attempted < accepted")
        if dropped > 0 and not truncated:
            raise ValueError(
                "invalid native telemetry envelope: dropped > 0 requires truncated"
            )

        self._attempted_record_count += attempted
        self._dropped_count += dropped
        native_truncated = truncated or dropped > 0
        for record in materialize_native_trace(output, song_name=self.song_name):
            if len(self.records) >= self._telemetry_capacity:
                self._dropped_count += 1
                native_truncated = True
                continue
            self.records.append(record)
            self._accepted_record_count += 1
        self._truncated = self._truncated or native_truncated

    def record_stats(self) -> dict[str, int]:
        """Return bounded session counters independent of the retained list.

        ``save()`` intentionally clears ``records`` in production.  Consumers
        that need lifecycle evidence must use these counters instead of taking
        the length of that post-save list.
        """
        return {
            "attempted": self._attempted_record_count,
            "accepted": self._accepted_record_count,
            "written": self._csv_events_written,
            "dropped": self._dropped_count,
            "retained": len(self.records),
        }

    def flush_if_large(self) -> bool:
        """Report a large buffer without mutating it on the dispatch thread.

        Naming debt (review of main@7c548527 §"Comment drift"): the legacy name suggests
        an I/O flush, but the implementation deliberately returns a boolean only. The
        dispatch thread must never own file I/O (an RT invariant — a CSV write mid-song
        is jitter that derailed note timing). ``save()`` owns the actual flush at the
        playback-lifecycle edge, and ``record_pause``/process_wait_states paths consult
        this probe to decide whether to trigger the off-dispatch write path. Returning
        ``True`` here means "the in-memory record buffer has crossed the soft threshold;
        consider flushing off-thread" — implementations may rotate it to ``save()`` at
        the next safe point.
        """
        return self.enabled and len(self.records) >= _TELEMETRY_FLUSH_CHUNK

    def _ensure_csv_open(self) -> None:
        """Lazily open the CSV at the current log_filepath (allows test path reassignment)."""
        if (
            self._csv_writer is not None
            or not self.enabled
            or self.log_filepath is None
        ):
            return
        self.log_filepath.parent.mkdir(parents=True, exist_ok=True)
        self._csv_file = self.log_filepath.open("w", newline="", encoding="utf-8")
        self._csv_writer = csv.DictWriter(self._csv_file, fieldnames=_CSV_FIELDS)
        self._csv_writer.writeheader()

    def _flush_records_to_csv(self, *, clear: bool = True) -> None:
        """Write unwritten records to the open CSV; optionally clear the in-memory list."""
        if not self.records or not self.enabled:
            return
        unwritten = self.records[self._records_written_offset :]
        if not unwritten:
            return
        self._ensure_csv_open()
        if self._csv_writer is None or self._csv_file is None:
            return
        with contextlib.suppress(Exception):
            self._csv_writer.writerows(r._materialize() for r in unwritten)
            self._csv_file.flush()
            self._csv_events_written += len(unwritten)
            self._records_written_offset += len(unwritten)
        if clear:
            self.records = []
            self._records_written_offset = 0
            self._cleared_mid_play = True

    def _close_csv(self) -> None:
        if self._csv_file is not None:
            with contextlib.suppress(Exception):
                self._csv_file.close()
        self._csv_file = None
        self._csv_writer = None

    def _read_csv_rows(self) -> list[dict[str, Any]]:
        """Re-read the on-disk CSV (after flush) as typed dicts for summary computation."""
        if self.log_filepath is None or not self.log_filepath.exists():
            return []
        if self._csv_file is not None:
            with contextlib.suppress(Exception):
                self._csv_file.flush()
        rows: list[dict[str, Any]] = []
        try:
            with self.log_filepath.open(newline="", encoding="utf-8") as f:
                reader = csv.DictReader(f)
                for raw in reader:
                    row: dict[str, Any] = dict(raw)
                    for field in _CSV_INT_FIELDS:
                        if field in row and row[field] not in (None, ""):
                            with contextlib.suppress(TypeError, ValueError):
                                row[field] = int(row[field])
                    rows.append(row)
        except Exception:
            return []
        return rows

    def _rows_for_summary(self) -> list[dict[str, Any]] | None:
        """Build the full row list for get_summary (memory and/or re-read CSV)."""
        # Complete in-memory set (never mid-cleared, or retain mode kept everything).
        if self.records and not self._cleared_mid_play:
            return [r._materialize() for r in self.records]
        if self._csv_events_written > 0:
            # Disk is authoritative after a mid-play clear; only append unwritten tail.
            rows = self._read_csv_rows()
            unwritten = self.records[self._records_written_offset :]
            if unwritten:
                rows.extend(r._materialize() for r in unwritten)
            return rows if rows else None
        if self.records:
            return [r._materialize() for r in self.records]
        return None

    def record_backend_health(self, health: BackendHealth) -> None:
        """Stores the backend health state at the end of playback."""
        self.backend_health = health

    def record_input_path_health(self, *, degraded: bool, warn_us: int) -> None:
        self.input_path_degraded = degraded
        self.input_path_warn_us = max(0, warn_us)

    def record_runtime_options(self, options: dict[str, object]) -> None:
        """Store runtime ablation/debug switches for the telemetry summary."""
        self.runtime_options = dict(options)

    def record_pause(self, reason: str, duration_us: int) -> None:
        if not self.enabled:
            return
        self.pause_durations_us.setdefault(reason, []).append(max(0, duration_us))
        # Playback is paused: safe to flush the soft buffer off the RT send path.
        self.flush_if_large()

    def record_abort(self, reason: str) -> None:
        """Tally one invocation of the unified abort helper (`abort_input_safe`).

        Safe to call from the dispatch thread (the abort caller) or from the engine
        main-thread pre-dispatch wait path — only an int bump under the GIL-free build.
        Unknown reasons are recorded verbatim so new callers do not require a schema
        change; the canonical reasons live in `abort_input_safe` callers.
        """
        key = reason
        self.abort_counts_by_reason[key] = self.abort_counts_by_reason.get(key, 0) + 1

    def record_abort_counts(self, counts: dict[str, int]) -> None:
        """Replace abort counters with a validated terminal native snapshot."""
        self.abort_counts_by_reason = {
            str(reason): max(0, int(count)) for reason, count in counts.items()
        }

    def record_release_outcome(self, outcome) -> None:
        """Stores the final release_all outcome at the end of playback."""
        self.release_outcome = outcome

    def record_generation_status_counts(self, counts: dict[str, int]) -> None:
        """Stores final runtime generation status counts for playback summary diagnostics."""
        self.generation_status_counts = {
            status: max(0, count) for status, count in counts.items()
        }

    def record_schedule_metadata(self, metadata) -> None:
        """Stores scheduler stress metrics for later calibration."""
        self.schedule_summary = {
            "compressed_holds": int(getattr(metadata, "compressed_holds", 0)),
            "impossible_same_key_repeats": int(
                getattr(metadata, "impossible_same_key_repeats", 0)
            ),
            "risky_same_key_repeats": int(
                getattr(metadata, "risky_same_key_repeats", 0)
            ),
            "deduplicated_note_count": int(
                getattr(metadata, "deduplicated_note_count", 0)
            ),
            "duplicate_note_count": int(getattr(metadata, "duplicate_note_count", 0)),
            "max_polyphony": int(getattr(metadata, "max_polyphony", 0)),
            "note_count": int(getattr(metadata, "note_count", 0)),
            "shortest_same_key_interval_us": getattr(
                metadata, "shortest_same_key_interval_us", None
            ),
            "min_same_key_up_gap_us": getattr(metadata, "min_same_key_up_gap_us", None),
            "sub_60fps_frame_notes": int(getattr(metadata, "sub_60fps_frame_notes", 0)),
        }

    def get_summary(self) -> dict | None:
        """Compute and return the stats dict in-memory (no file I/O beyond optional CSV re-read).

        Returns None when no summary is available (empty records AND no cached summary).

        After save() clears records (post-play hygiene), late callers — engine's
        _log_timing_summary, CLI report, tests — receive the previously-computed summary
        via _last_summary, so they keep working without re-pinning the (often multi-MB)
        per-record list in RSS. Read-only contract: callers read keys, never mutate.
        """
        rows = self._rows_for_summary()
        if rows is None:
            # Records were already persisted (save()) or never recorded: serve the cache.
            return self._last_summary

        def _scan_count(record: Mapping[str, object], field: str) -> int:
            return len([sc for sc in str(record.get(field, "")).split(";") if sc])

        def _record_count(
            record: Mapping[str, object], identity_field: str, native_field: str
        ) -> int:
            native_value = record.get(native_field)
            if isinstance(native_value, int) and not isinstance(native_value, bool):
                return native_value
            return _scan_count(record, identity_field)

        def _semantic_send_attempts(record: Mapping[str, object]) -> int:
            value = record.get("send_attempts", 0)
            return value if isinstance(value, int) and not isinstance(value, bool) else 0

        def _is_backend_dispatch(record: Mapping[str, object]) -> bool:
            return (
                _semantic_send_attempts(record) > 0
                or _record_count(record, "sent_scan_codes", "native_sent_count") > 0
                or _record_count(record, "skipped_scan_codes", "native_skipped_count")
                > 0
            )

        def _authored_request_count(
            record: Mapping[str, object], identity_field: str, native_field: str
        ) -> int:
            if record.get("runtime_outcome") in {
                "blocked_unfocused",
                "suppressed_stale_up",
            }:
                return 0
            return _record_count(record, identity_field, native_field)

        dispatch_records = [r for r in rows if _is_backend_dispatch(r)]
        noop_skipped_records = [
            r
            for r in dispatch_records
            if _record_count(r, "sent_scan_codes", "native_sent_count") == 0
            and _record_count(r, "skipped_scan_codes", "native_skipped_count") > 0
        ]
        noop_skipped_count = len(noop_skipped_records)
        noop_skipped_key_count = sum(
            _record_count(r, "skipped_scan_codes", "native_skipped_count")
            for r in noop_skipped_records
        )
        scheduler_dispatch_records = [
            r
            for r in dispatch_records
            if r.get("runtime_outcome") != "deferred_release"
        ]
        latenesses = [r["lateness_us"] for r in scheduler_dispatch_records]
        visible_latenesses = [
            r.get("visible_lateness_us", 0) for r in scheduler_dispatch_records
        ]
        send_durations = [r["send_duration_us"] for r in dispatch_records]
        send_durations_pure = [
            r.get("send_duration_pure_us", 0) for r in dispatch_records
        ]
        bookkeeping_durations = [r.get("bookkeeping_us", 0) for r in dispatch_records]
        dispatch_latenesses = [
            r.get("dispatch_lateness_us", 0) for r in scheduler_dispatch_records
        ]
        # Sender-warmup split: a send preceded by a long idle gap runs on a core that has likely
        # downclocked/parked, so we compare send_duration when "cold" vs "warm" to test whether
        # CPU coldness (caused by sleeping between notes) inflates send latency.
        SEND_COLD_THRESHOLD_US = 20_000
        cold_send_durations = [
            r["send_duration_us"]
            for r in dispatch_records
            if r.get("idle_gap_us", 0) > SEND_COLD_THRESHOLD_US
        ]
        warm_send_durations = [
            r["send_duration_us"]
            for r in dispatch_records
            if r.get("idle_gap_us", 0) <= SEND_COLD_THRESHOLD_US
        ]
        idle_gaps = [r.get("idle_gap_us", 0) for r in dispatch_records]
        pre_send_spins = [r.get("pre_send_spin_us", 0) for r in dispatch_records]
        sent_down_records = [
            record
            for record in dispatch_records
            if record["kind"] == "down"
            and _record_count(record, "sent_scan_codes", "native_sent_count") > 0
        ]
        down_timeline_drift_us = (
            sent_down_records[-1]["lateness_us"] - sent_down_records[0]["lateness_us"]
            if len(sent_down_records) >= 2
            else 0
        )

        # A late catch-up burst is a sequence of distinct authored down dispatches
        # that the runtime collapses into a <=1ms physical dispatch window.
        catch_up_bursts: list[list[dict]] = []
        current_burst: list[dict] = []
        for previous, current in itertools.pairwise(sent_down_records):
            actual_gap_us = current["actual_us"] - previous["actual_us"]
            authored_gap_us = current["scheduled_us"] - previous["scheduled_us"]
            collapsed = (
                0 <= actual_gap_us <= 1_000
                and authored_gap_us >= 2_000
                and current["lateness_us"] > 2_000
            )
            if collapsed:
                if not current_burst:
                    current_burst = [previous]
                current_burst.append(current)
            elif current_burst:
                catch_up_bursts.append(current_burst)
                current_burst = []
        if current_burst:
            catch_up_bursts.append(current_burst)

        hold_durations: list[int] = []
        confirmed_hold_lower_bounds: list[int] = []
        observed_holds: list[int] = []
        active_downs: dict[int, tuple[int, int]] = {}
        for r in rows:
            sent_codes = str(r.get("sent_scan_codes") or r["scan_codes"])
            codes = [int(sc) for sc in sent_codes.split(";") if sc]
            if r["kind"] == "down":
                for sc in codes:
                    active_downs[sc] = (
                        int(r["actual_us"]),
                        int(
                            r.get("dispatch_completed_us")
                            or (r["actual_us"] + r["send_duration_us"])
                        ),
                    )
            elif r["kind"] == "up":
                for sc in codes:
                    if sc in active_downs:
                        down_started_us, _down_completed_us = active_downs[sc]
                        hold_durations.append(r["actual_us"] - down_started_us)
                        observed_holds.append(
                            int(
                                r.get(
                                    "dispatch_completed_us",
                                    r["actual_us"] + r["send_duration_us"],
                                )
                                or 0
                            )
                            - _down_completed_us
                        )
                        # Compatibility metric from down dispatch start through up dispatch start;
                        # observed_hold_us is the completion-to-completion visibility metric.
                        confirmed_hold_lower_bounds.append(
                            int(
                                r.get(
                                    "dispatch_completed_us",
                                    r["actual_us"] + r["send_duration_us"],
                                )
                                or 0
                            )
                            - down_started_us
                        )
                        del active_downs[sc]

        def _pct(values: list[int], pct: float) -> float:
            if not values:
                return 0.0
            s = sorted(values)
            idx = round(pct * (len(s) - 1))
            return float(s[idx])

        def _stats(values: list[int], thresholds: bool = False) -> dict:
            if not values:
                base: dict = {
                    "min_us": 0.0,
                    "p50_us": 0.0,
                    "p95_us": 0.0,
                    "p99_us": 0.0,
                    "max_us": 0.0,
                    "avg_us": 0.0,
                }
                if thresholds:
                    base.update({"over_2ms": 0, "over_5ms": 0, "over_10ms": 0})
                return base
            res = {
                "min_us": float(min(values)),
                "p50_us": _pct(values, 0.50),
                "p95_us": _pct(values, 0.95),
                "p99_us": _pct(values, 0.99),
                "max_us": float(max(values)),
                "avg_us": (sum(values) / len(values)),
            }
            if thresholds:
                res.update(
                    {
                        "over_2ms": sum(1 for v in values if v > 2000),
                        "over_5ms": sum(1 for v in values if v > 5000),
                        "over_10ms": sum(1 for v in values if v > 10000),
                    }
                )
            return res

        backend_info: dict = {
            "panic_release_failures": 0,
            "failed_release_keys_final": [],
        }
        if self.backend_health is not None:
            backend_info["panic_release_failures"] = (
                self.backend_health.failed_release_count
            )
            backend_info["keys_dropped"] = self.backend_health.keys_dropped
            backend_info["chord_split_events"] = self.backend_health.chord_split_events
            backend_info["sendinput_partial_events"] = (
                self.backend_health.sendinput_partial_events
            )
            backend_info["sendinput_zero_progress_failures"] = (
                self.backend_health.sendinput_zero_progress_failures
            )
            backend_info["chords_rejected"] = self.backend_health.chords_rejected
            backend_info["authored_conflict_events"] = (
                self.backend_health.authored_conflict_events
            )
            backend_info["authored_chords_rejected"] = (
                self.backend_health.authored_chords_rejected
            )
            backend_info["authored_keys_rejected"] = (
                self.backend_health.authored_keys_rejected
            )

        if self.release_outcome is not None:
            backend_info["release_attempted"] = self.release_outcome.attempted
            backend_info["release_success"] = self.release_outcome.released_successfully
            backend_info["release_stuck_keys"] = self.release_outcome.stuck_keys
            backend_info["release_inconclusive"] = (
                self.release_outcome.verification_inconclusive
            )

        observed_hold_floor_us = (
            math.ceil(1_000_000 / self.fps)
            if self.fps is not None and self.fps > 0
            else self.min_hold_us
        )
        intended_down_count = sum(
            _authored_request_count(r, "scan_codes", "native_requested_count")
            for r in rows
            if r["kind"] == "down"
        )
        intended_up_count = sum(
            _authored_request_count(r, "scan_codes", "native_requested_count")
            for r in rows
            if r["kind"] == "up"
        )
        sent_down_count = sum(
            _record_count(r, "sent_scan_codes", "native_sent_count")
            for r in rows
            if r["kind"] == "down"
        )
        sent_up_count = sum(
            _record_count(r, "sent_scan_codes", "native_sent_count")
            for r in rows
            if r["kind"] == "up"
        )
        backend_skipped_down_count = sum(
            _record_count(r, "skipped_scan_codes", "native_skipped_count")
            for r in rows
            if r["kind"] == "down"
        )
        backend_skipped_up_count = sum(
            _record_count(r, "skipped_scan_codes", "native_skipped_count")
            for r in rows
            if r["kind"] == "up"
        )
        runtime_conflict_dropped_down_count = sum(
            _scan_count(r, "scan_codes")
            for r in rows
            if r.get("runtime_outcome") == "dropped_conflict"
        )
        expired_dropped_down_count = sum(
            _scan_count(r, "scan_codes")
            for r in rows
            if r.get("runtime_outcome") == "dropped_expired"
        )
        runtime_backend_dropped_down_count = sum(
            max(
                0,
                _record_count(r, "scan_codes", "native_requested_count")
                - _record_count(r, "sent_scan_codes", "native_sent_count"),
            )
            for r in rows
            if r["kind"] == "down"
            and r.get("runtime_outcome") != "blocked_unfocused"
            and r.get("runtime_outcome")
            not in {"dropped_conflict", "dropped_expired", "suppressed_stale_up"}
            and (
                not isinstance(r.get("native_requested_count"), int)
                or r.get("native_requested_count", 0) > 0
            )
        )
        before_send_missing_down_count = (
            runtime_conflict_dropped_down_count
            + expired_dropped_down_count
            + runtime_backend_dropped_down_count
        )
        sender_clean_known = not self._truncated
        sender_clean = (
            sender_clean_known
            and intended_down_count == sent_down_count
            and before_send_missing_down_count == 0
            and backend_skipped_down_count == 0
            and int(getattr(self.backend_health, "keys_dropped", 0)) == 0
        )
        summary = {
            "run_id": self.run_id,
            "song": self.song_name,
            "hold_frames": self.hold_frames,
            "hold_label": self.hold_label,
            "effective_hold_us": self.min_hold_us,
            "min_hold_us": self.min_hold_us,
            "fps": self.fps,
            "min_hold_margin_us": self.min_hold_margin_us,
            "min_hold_margin_source": self.min_hold_margin_source,
            "tempo_scale": self.tempo_scale,
            "total_events": len(rows),
            "telemetry_truncated": self._truncated,
            "telemetry_dropped_count": self._dropped_count,
            # One logical truncation marker for the exported summary. It
            # describes the retain-first policy without fabricating a CSV event.
            "telemetry_buffer": {
                "policy": "retain_first_n_stop_accepting",
                "capacity": self._telemetry_capacity,
                "attempted_count": self._attempted_record_count,
                "retained_count": len(rows),
                "dropped_count": self._dropped_count,
                "truncated": self._truncated,
                "marker_count": 1 if self._truncated else 0,
            },
            "timing_semantics": {
                "clock": "perf_counter_ns_us_quantized",
                "onset_definition": "sendinput_return",
                "visible_lateness_means": "send_completed_us - scheduled_us (sender proxy)",
                "game_phase_locked": False,
                "game_observed_available": False,
            },
            "evidence_boundaries": {
                "schedule": {
                    "intended_down_count": intended_down_count,
                    "intended_up_count": intended_up_count,
                },
                "runtime_dispatch": {
                    "attempted_dispatches": len(dispatch_records),
                    "runtime_conflict_dropped_down_count": runtime_conflict_dropped_down_count,
                    "expired_dropped_down_count": expired_dropped_down_count,
                    "runtime_backend_dropped_down_count": runtime_backend_dropped_down_count,
                    "before_send_missing_down_count": before_send_missing_down_count,
                },
                "sender_completion": {
                    "sent_down_count": sent_down_count,
                    "sent_up_count": sent_up_count,
                    "backend_skipped_down_count": backend_skipped_down_count,
                    "backend_skipped_up_count": backend_skipped_up_count,
                    "keys_dropped": int(getattr(self.backend_health, "keys_dropped", 0))
                    if self.backend_health is not None
                    else 0,
                    "chord_split_events": int(
                        getattr(self.backend_health, "chord_split_events", 0)
                    )
                    if self.backend_health is not None
                    else 0,
                    "sendinput_partial_events": int(
                        getattr(self.backend_health, "sendinput_partial_events", 0)
                    )
                    if self.backend_health is not None
                    else 0,
                    "sendinput_zero_progress_failures": int(
                        getattr(
                            self.backend_health, "sendinput_zero_progress_failures", 0
                        )
                    )
                    if self.backend_health is not None
                    else 0,
                    "chords_rejected": int(
                        getattr(self.backend_health, "chords_rejected", 0)
                    )
                    if self.backend_health is not None
                    else 0,
                    "authored_conflict_events": int(
                        getattr(self.backend_health, "authored_conflict_events", 0)
                    )
                    if self.backend_health is not None
                    else 0,
                    "authored_chords_rejected": int(
                        getattr(self.backend_health, "authored_chords_rejected", 0)
                    )
                    if self.backend_health is not None
                    else 0,
                    "authored_keys_rejected": int(
                        getattr(self.backend_health, "authored_keys_rejected", 0)
                    )
                    if self.backend_health is not None
                    else 0,
                    "sender_clean": sender_clean,
                    "sender_clean_known": sender_clean_known,
                },
                "game_observed": {
                    "available": False,
                    "game_acceptance_unknown": True,
                    "heard_onset_count": None,
                    "after_send_missing_count": None,
                    "note": (
                        "Telemetry stops at the SendInput side. Attach audio/onset evidence "
                        "before making game-acceptance claims."
                    ),
                },
            },
            "intended_down_count": intended_down_count,
            "intended_up_count": intended_up_count,
            "before_send_missing_down_count": before_send_missing_down_count,
            "sender_clean": sender_clean,
            "sender_clean_known": sender_clean_known,
            "game_acceptance_unknown": True,
            "game_observed_onset_count": None,
            "after_send_missing_count": None,
            "lateness_us": _stats(latenesses, thresholds=True),
            "visible_lateness_us": _stats(visible_latenesses, thresholds=True),
            "dispatch_lateness_us": _stats(dispatch_latenesses, thresholds=True),
            "send_duration_us": _stats(send_durations),
            "send_duration_pure_us": _stats(send_durations_pure),
            "bookkeeping_us": _stats(bookkeeping_durations),
            "send_warmup": {
                "cold_threshold_us": SEND_COLD_THRESHOLD_US,
                "cold_send_count": len(cold_send_durations),
                "warm_send_count": len(warm_send_durations),
                "send_duration_cold_us": _stats(cold_send_durations),
                "send_duration_warm_us": _stats(warm_send_durations),
                "idle_gap_us": _stats(idle_gaps),
                "pre_send_spin_us": _stats(pre_send_spins),
            },
            "note_hold_duration_us": _stats(hold_durations),
            "observed_hold_us": _stats(observed_holds),
            "observed_hold_below_frame_count": sum(
                1
                for hold_us in observed_holds
                if observed_hold_floor_us > 0 and hold_us < observed_hold_floor_us
            ),
            "confirmed_hold_lower_bound_us": _stats(confirmed_hold_lower_bounds),
            "confirmed_hold_shortfall_count": sum(
                1
                for hold_us in confirmed_hold_lower_bounds
                if self.min_hold_us > 0 and hold_us < self.min_hold_us
            ),
            "attempted_dispatches": len(dispatch_records),
            "noop_skipped_count": noop_skipped_count,
            "noop_skipped_key_count": noop_skipped_key_count,
            "successful_dispatches": sum(
                1
                for r in dispatch_records
                if _record_count(r, "sent_scan_codes", "native_sent_count") > 0
            ),
            "sent_down_count": sent_down_count,
            "sent_up_count": sent_up_count,
            "backend_skipped_down_count": backend_skipped_down_count,
            "backend_skipped_up_count": backend_skipped_up_count,
            "runtime_conflict_dropped_down_count": runtime_conflict_dropped_down_count,
            "runtime_backend_dropped_down_count": runtime_backend_dropped_down_count,
            "expired_dropped_down_count": expired_dropped_down_count,
            "catch_up_bursts": {
                "count": len(catch_up_bursts),
                "down_dispatch_count": sum(len(burst) for burst in catch_up_bursts),
                "max_collapsed_dispatches": max(
                    (len(burst) for burst in catch_up_bursts),
                    default=0,
                ),
                "max_authored_span_us": max(
                    (
                        burst[-1]["scheduled_us"] - burst[0]["scheduled_us"]
                        for burst in catch_up_bursts
                    ),
                    default=0,
                ),
            },
            # Use `rows` (already materialized) — avoid a second walk of self.records
            # that would double peak RSS at summary time.
            "deferred_release_count": sum(
                1 for r in rows if int(r.get("deferred_by_us", 0) or 0) > 0
            ),
            "release_deferral_us": _stats(
                [
                    int(r.get("deferred_by_us", 0) or 0)
                    for r in rows
                    if int(r.get("deferred_by_us", 0) or 0) > 0
                ]
            ),
            "down_timeline_drift_us": down_timeline_drift_us,
            "playback_pause": {
                reason: {
                    "count": len(durations),
                    "total_us": sum(durations),
                    "max_us": max(durations, default=0),
                }
                for reason, durations in self.pause_durations_us.items()
            },
            # Phase 0 of SendInput lifecycle plan: per-reason abort tally.
            "abort_counts_by_reason": dict(self.abort_counts_by_reason),
            # Count note-on dispatches whose native SendInput result broke
            # chord integrity. The older ``partial_note_on`` label is retained
            # only when reading historical telemetry records.
            "partial_note_on_count": sum(
                1
                for r in rows
                if r.get("kind") == "down"
                and r.get("runtime_outcome")
                in {"partial_note_on", "chord_integrity_lost"}
            ),
            "backend": backend_info,
            "input_path_degraded": self.input_path_degraded,
            "input_path_warn_us": self.input_path_warn_us,
            "runtime_options": self.runtime_options,
        }
        generation_counts = self.generation_status_counts
        summary.update(
            {
                "cancelled_generation_count": generation_counts.get("cancelled", 0),
                "dropped_conflict_count": generation_counts.get("dropped_conflict", 0),
                "dropped_backend_count": generation_counts.get("dropped_backend", 0),
                "released_count": generation_counts.get("released", 0),
            }
        )
        if self.schedule_summary is not None:
            summary["schedule"] = self.schedule_summary
        if TelemetryLogger.last_picker_cleanup is not None:
            summary["background"] = {
                "picker_cleanup": TelemetryLogger.last_picker_cleanup
            }
        # Cache the freshly computed summary before save() clears records so post-clear
        # callers (engine._log_timing_summary → get_summary()) read the cache instead of None.
        self._last_summary = summary
        return summary

    def release_summary(self) -> None:
        """Free the cached summary dict after all callers have read it.

        Called by the engine after ``_log_timing_summary()`` finishes, so the
        post-play summary dict (~100–500 KB) does not pin RSS until the engine
        object is GC'd.  Safe to call multiple times (idempotent).

        Skipped in ``retain_records_after_save=True`` mode (test-only) so test
        helpers that inspect ``telemetry`` after ``play()`` are not broken.
        """
        if self._retain_records_after_save:
            return  # test mode — keep everything intact
        self._last_summary = None  # MEM-4: allow GC of summary dict

    def save(self) -> None:
        if not self.enabled or not self.log_filepath:
            return
        if not self.records and self._csv_events_written == 0:
            return

        try:
            # 1. Flush any remaining in-memory records to the open CSV (or open fresh).
            self._flush_records_to_csv(clear=False)
            self._close_csv()

            # 2. Compute summary from full history (memory and/or re-read CSV).
            summary = self.get_summary()
            if summary is None:
                return
            summary["timestamp"] = time.strftime("%Y-%m-%d %H:%M:%S")

            # 3. Save companion summary JSON
            summary_path = self.log_filepath.with_suffix(".summary.json")
            with summary_path.open("w", encoding="utf-8") as f:
                json.dump(summary, f, indent=2)

        except Exception as e:
            sys.stderr.write(f"[telemetry] failed to save metrics: {e}\n")
            # On failure: keep records intact so a retry or debugger can inspect them.
            return

        # Reachable-memory hygiene: drop the per-event list once persisted to disk. get_summary()
        # callers after this point read the cached _last_summary (computed inside the try above),
        # so the records list is no longer needed in RSS.
        # Skipped when retain_records_after_save=True (test-only hook) so tests asserting on raw
        # engine.telemetry.records after play() keep working without moving onto CSV/summary.
        if not self._retain_records_after_save:
            self.records = []
            self._records_written_offset = 0

        # Clear process-lifetime attrs so stale data from this play does not pollute the next.
        TelemetryLogger.last_picker_cleanup = None
        TelemetryLogger.last_thread_census = None


def inspect_telemetry_report(target_path: str, recommend: bool = False) -> None:
    """Load and format a timing performance report from companion summary JSON telemetry files."""
    path = Path(target_path)
    summary_files = []

    if path.is_file():
        if path.suffix == ".json":
            summary_files.append(path)
        elif path.suffix == ".csv":
            summary_files.append(path.with_suffix(".summary.json"))
    elif path.is_dir():
        summary_files = list(path.glob("*.summary.json"))

    summary_files = [f for f in summary_files if f.exists()]
    if not summary_files:
        print(
            f"No valid telemetry summary files (.summary.json) found at {target_path}"
        )
        return

    print("\n==================================================")
    print(f" AGGREGATE TELEMETRY TIMING REPORT ({len(summary_files)} run(s))")
    print("==================================================")

    for f in summary_files:
        try:
            with f.open("r", encoding="utf-8") as file:
                data = json.load(file)

            print(
                f"\nPlayback: {data.get('song', 'Unknown')} at {data.get('timestamp', 'Unknown')} [Run ID: {data.get('run_id', 'N/A')}]"
            )
            print(
                f"  Hold: {data.get('hold_label', 'hold 1.00f')} | Tempo Scale: {data.get('tempo_scale', 1.0)}"
            )
            print(f"  Total Event Count: {data.get('total_events', 0)}")
            print(
                "  Evidence Boundary: "
                f"sender_clean={data.get('sender_clean', False)}, "
                f"before_send_missing_downs={data.get('before_send_missing_down_count', 0)}, "
                f"game_acceptance_unknown={data.get('game_acceptance_unknown', True)}"
            )

            lat = data.get("lateness_us", {})
            print("  Loop Lateness:")
            print(
                f"    * Average: {lat.get('avg_us', 0.0):.1f} us ({lat.get('avg_us', 0.0) / 1000:.3f} ms)"
            )
            print(f"    * Median (p50): {lat.get('p50_us', 0.0):.1f} us")
            print(f"    * 95th Percentile (p95): {lat.get('p95_us', 0.0):.1f} us")
            print(f"    * 99th Percentile (p99): {lat.get('p99_us', 0.0):.1f} us")
            print(f"    * Maximum: {lat.get('max_us', 0.0):.1f} us")
            print(
                f"    * Lateness Counts: >2ms={lat.get('over_2ms', 0)}, >5ms={lat.get('over_5ms', 0)}, >10ms={lat.get('over_10ms', 0)}"
            )

            dur = data.get("send_duration_us", {})
            print("  SendInput Execution Duration:")
            print(f"    * Average: {dur.get('avg_us', 0.0):.1f} us")
            print(f"    * p95: {dur.get('p95_us', 0.0):.1f} us")
            print(f"    * p99: {dur.get('p99_us', 0.0):.1f} us")

            catch_up = data.get("catch_up_bursts", {})
            print(
                "  Catch-up Bursts: "
                f"count={catch_up.get('count', 0)}, "
                f"down dispatches={catch_up.get('down_dispatch_count', 0)}, "
                f"max collapsed={catch_up.get('max_collapsed_dispatches', 0)}, "
                f"max authored span={catch_up.get('max_authored_span_us', 0)} us"
            )
            print(
                "  Runtime Down Drops: "
                f"conflict={data.get('runtime_conflict_dropped_down_count', 0)}, "
                f"backend={data.get('runtime_backend_dropped_down_count', 0)}"
            )

            hold = data.get("note_hold_duration_us", {})
            if hold:
                print("  Note Hold Durations:")
                print(
                    f"    * Average: {hold.get('avg_us', 0.0):.1f} us ({hold.get('avg_us', 0.0) / 1000:.1f} ms)"
                )
                print(f"    * p50: {hold.get('p50_us', 0.0):.1f} us")

            backend = data.get("backend", {})
            if backend.get("panic_release_failures", 0) > 0:
                print(
                    f"  [warning] Backend panic release failures count: {backend.get('panic_release_failures')}"
                )
            keys_dropped = int(backend.get("keys_dropped", 0))
            chord_splits = int(backend.get("chord_split_events", 0))
            if keys_dropped > 0:
                print(
                    f"  [warning] Note-on drops: {keys_dropped} key(s) not injected ({chord_splits} chord split(s))"
                )

            # Perform calibration recommendation if requested
            if recommend:
                from sky_music.orchestration.calibration import (
                    calibrate_timing,
                    calibration_input_from_summary,
                )

                inp = calibration_input_from_summary(data)
                rec = calibrate_timing(inp)

                print("\n  Calibration Recommendation:")
                print(f"    * Suggested Hold : {rec.hold_frames:.2f} frames")
                print(f"    * Suggested Tempo   : {rec.tempo_scale:.2f}x")
                print(
                    f"    * Effective Hold (us): {rec.recommended_hold_us} ({rec.recommended_hold_us / 1000:.1f} ms)"
                )
                print(f"    * Severity Level    : {rec.severity.upper()}")
                print(f"    * Reason            : {rec.reason}")
        except Exception as e:
            print(f"  [error] Failed to read summary file {f.name}: {e}")

    print("\n==================================================")
