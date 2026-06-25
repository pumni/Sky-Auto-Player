import csv
import json
from typing import Any
import math
from pathlib import Path
import sys
import time
import random
from sky_music.infrastructure.backend import BackendHealth

class TelemetryRecord:
    __slots__ = (
        "_dict",
        "song_name",
        "event_index",
        "kind",
        "scheduled_us",
        "actual_us",
        "lateness_us",
        "send_duration_us",
        "scan_codes",
        "reason",
        "dispatch_id",
        "dispatch_completed_us",
        "sent_scan_codes",
        "skipped_scan_codes",
        "generation_ids",
        "runtime_outcome",
        "deferred_by_us",
        "pre_send_spin_us",
        "idle_gap_us",
        "visible_lateness_us",
        "applied_lead_us",
        "send_duration_pure_us",
        "bookkeeping_us",
        "dispatch_lateness_us",
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
    ) -> None:
        self._dict = None
        self.song_name = song_name
        self.event_index = event_index
        self.kind = kind
        self.scheduled_us = scheduled_us
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

    def _materialize(self) -> dict:
        if self._dict is None:
            scan_codes_str = ";".join(str(sc) for sc in self.scan_codes)
            sent_scan_codes = self.scan_codes if self.sent_scan_codes is None else self.sent_scan_codes
            visible_lat = (
                self.visible_lateness_us
                if self.visible_lateness_us is not None
                else (self.dispatch_completed_us - self.scheduled_us if self.dispatch_completed_us is not None else (self.actual_us + self.send_duration_us - self.scheduled_us))
            )
            self._dict = {
                "song": self.song_name,
                "event_index": self.event_index,
                "dispatch_id": self.event_index if self.dispatch_id is None else self.dispatch_id,
                "evidence_scope": "sendinput_side",
                "kind": self.kind,
                "scheduled_us": self.scheduled_us,
                "actual_us": self.actual_us,
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
                "skipped_scan_codes": ";".join(str(sc) for sc in self.skipped_scan_codes),
                "generation_ids": ";".join(str(generation_id) for generation_id in self.generation_ids),
                "runtime_outcome": self.runtime_outcome,
                "deferred_by_us": self.deferred_by_us,
                "pre_send_spin_us": self.pre_send_spin_us,
                "idle_gap_us": self.idle_gap_us,
                "reason": self.reason,
                "applied_lead_us": self.applied_lead_us,
                "send_duration_pure_us": self.send_duration_pure_us,
                "bookkeeping_us": self.bookkeeping_us,
                "dispatch_lateness_us": self.dispatch_lateness_us,
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


class TelemetryLogger:
    """Records precise microsecond timing metrics into clean CSV and companion summary JSON files for calibration."""
    last_picker_cleanup: dict | None = None
    last_thread_census: dict | None = None

    def __init__(
        self,
        song_name: str,
        enabled: bool = False,
        profile_name: str = "balanced",
        tempo_scale: float = 1.0,
        run_id: str | None = None,
        fps: int | None = None,
        min_hold_us: int = 0,
    ):
        self.song_name = song_name
        self.enabled = enabled
        self.profile_name = profile_name
        self.tempo_scale = tempo_scale
        self.fps = fps
        self.min_hold_us = max(0, int(min_hold_us))
        self.records = []
        self.log_filepath = None
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
        
        # Unique run ID generation
        if run_id is None:
            self.run_id = f"{time.strftime('%Y%m%d-%H%M%S')}-{random.randint(1000, 9999)}"
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
            send_duration_pure_us = getattr(result, "send_duration_pure_us", 0)
            bookkeeping_us = getattr(result, "bookkeeping_us", 0)
            dispatch_lateness_us = getattr(result, "dispatch_lateness_us", 0)

        assert event_index is not None
        assert kind is not None
        assert scheduled_us is not None
        assert actual_us is not None
        assert lateness_us is not None
        assert send_duration_us is not None
        assert scan_codes is not None
        assert reason is not None

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
            )
        )

        
    def record_backend_health(self, health: BackendHealth) -> None:
        """Stores the backend health state at the end of playback."""
        self.backend_health = health

    def record_input_path_health(self, *, degraded: bool, warn_us: int) -> None:
        self.input_path_degraded = bool(degraded)
        self.input_path_warn_us = max(0, int(warn_us))

    def record_runtime_options(self, options: dict[str, object]) -> None:
        """Store runtime ablation/debug switches for the telemetry summary."""
        self.runtime_options = dict(options)

    def record_pause(self, reason: str, duration_us: int) -> None:
        if not self.enabled:
            return
        self.pause_durations_us.setdefault(reason, []).append(max(0, int(duration_us)))

    def record_release_outcome(self, outcome) -> None:
        """Stores the final release_all outcome at the end of playback."""
        self.release_outcome = outcome

    def record_generation_status_counts(self, counts: dict[str, int]) -> None:
        """Stores final runtime generation status counts for playback summary diagnostics."""
        self.generation_status_counts = {
            str(status): max(0, int(count))
            for status, count in counts.items()
        }

    def record_schedule_metadata(self, metadata) -> None:
        """Stores scheduler stress metrics for later calibration."""
        self.schedule_summary = {
            "compressed_holds": int(getattr(metadata, "compressed_holds", 0)),
            "impossible_same_key_repeats": int(getattr(metadata, "impossible_same_key_repeats", 0)),
            "risky_same_key_repeats": int(getattr(metadata, "risky_same_key_repeats", 0)),
            "deduplicated_note_count": int(getattr(metadata, "deduplicated_note_count", 0)),
            "duplicate_note_count": int(getattr(metadata, "duplicate_note_count", 0)),
            "same_key_compressed_holds": int(getattr(metadata, "same_key_compressed_holds", 0)),
            "infeasible_same_key_repeats": int(getattr(metadata, "infeasible_same_key_repeats", 0)),
            "max_polyphony": int(getattr(metadata, "max_polyphony", 0)),
            "note_count": int(getattr(metadata, "note_count", 0)),
            "shortest_same_key_interval_us": getattr(metadata, "shortest_same_key_interval_us", None),
            "min_same_key_up_gap_us": getattr(metadata, "min_same_key_up_gap_us", None),
            "frame_us": getattr(metadata, "frame_us", None),
            "fps": getattr(metadata, "fps", None),
        }

    def get_summary(self) -> dict | None:
        """Compute and return the stats dict in-memory (no file I/O).

        Returns None if there are no records (e.g. telemetry disabled and
        no events recorded).  Callers should guard against None.
        """
        if not self.records:
            return None

        # Materialize once — all later list comprehensions work from `rows`
        rows = [r._materialize() for r in self.records]

        # dispatch_records: only records where SendInput was actually called
        # (sent_scan_codes is non-empty string). No-op release skips are excluded.
        dispatch_records = [r for r in rows if r.get("sent_scan_codes")]
        noop_skipped_count = sum(
            1 for r in rows
            if not r.get("sent_scan_codes") and r.get("skipped_scan_codes")
        )
        scheduler_dispatch_records = [
            r for r in dispatch_records
            if r.get("runtime_outcome") != "deferred_release"
        ]
        latenesses = [r["lateness_us"] for r in scheduler_dispatch_records]
        visible_latenesses = [r.get("visible_lateness_us", 0) for r in scheduler_dispatch_records]
        send_durations = [r["send_duration_us"] for r in dispatch_records]
        send_durations_pure = [r.get("send_duration_pure_us", 0) for r in dispatch_records]
        bookkeeping_durations = [r.get("bookkeeping_us", 0) for r in dispatch_records]
        dispatch_latenesses = [r.get("dispatch_lateness_us", 0) for r in scheduler_dispatch_records]
        # Sender-warmup split: a send preceded by a long idle gap runs on a core that has likely
        # downclocked/parked, so we compare send_duration when "cold" vs "warm" to test whether
        # CPU coldness (caused by sleeping between notes) inflates send latency.
        SEND_COLD_THRESHOLD_US = 20_000
        cold_send_durations = [
            r["send_duration_us"] for r in dispatch_records if r.get("idle_gap_us", 0) > SEND_COLD_THRESHOLD_US
        ]
        warm_send_durations = [
            r["send_duration_us"] for r in dispatch_records if r.get("idle_gap_us", 0) <= SEND_COLD_THRESHOLD_US
        ]
        idle_gaps = [r.get("idle_gap_us", 0) for r in dispatch_records]
        pre_send_spins = [r.get("pre_send_spin_us", 0) for r in dispatch_records]
        sent_down_records = [
            record
            for record in dispatch_records
            if record["kind"] == "down" and record.get("sent_scan_codes", "")
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
        for previous, current in zip(sent_down_records, sent_down_records[1:]):
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
            sent_codes = r.get("sent_scan_codes", r["scan_codes"])
            codes = [int(sc) for sc in sent_codes.split(";") if sc]
            if r["kind"] == "down":
                for sc in codes:
                    active_downs[sc] = (
                        r["actual_us"],
                        r.get("dispatch_completed_us", r["actual_us"] + r["send_duration_us"]),
                    )
            elif r["kind"] == "up":
                for sc in codes:
                    if sc in active_downs:
                        down_started_us, _down_completed_us = active_downs[sc]
                        hold_durations.append(r["actual_us"] - down_started_us)
                        observed_holds.append(
                            r.get(
                                "dispatch_completed_us",
                                r["actual_us"] + r["send_duration_us"],
                            )
                            - _down_completed_us
                        )
                        # Compatibility metric from down dispatch start through up dispatch start;
                        # observed_hold_us is the completion-to-completion visibility metric.
                        confirmed_hold_lower_bounds.append(
                            r.get(
                                "dispatch_completed_us",
                                r["actual_us"] + r["send_duration_us"],
                            )
                            - down_started_us
                        )
                        del active_downs[sc]

        def _pct(values: list[int], pct: float) -> float:
            if not values:
                return 0.0
            s = sorted(values)
            idx = int(round(pct * (len(s) - 1)))
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
                "avg_us": float(sum(values) / len(values)),
            }
            if thresholds:
                res.update({
                    "over_2ms": sum(1 for v in values if v > 2000),
                    "over_5ms": sum(1 for v in values if v > 5000),
                    "over_10ms": sum(1 for v in values if v > 10000),
                })
            return res

        def _scan_count(record: dict, field: str) -> int:
            return len([sc for sc in str(record.get(field, "")).split(";") if sc])

        backend_info: dict = {"panic_release_failures": 0, "failed_release_keys_final": []}
        if self.backend_health is not None:
            backend_info["panic_release_failures"] = self.backend_health.failed_release_count
            
        if self.release_outcome is not None:
            backend_info["release_attempted"] = self.release_outcome.attempted
            backend_info["release_success"] = self.release_outcome.released_successfully
            backend_info["release_stuck_keys"] = self.release_outcome.stuck_keys
            backend_info["release_inconclusive"] = self.release_outcome.verification_inconclusive

        observed_hold_floor_us = (
            math.ceil(1_000_000 / self.fps)
            if self.fps is not None and self.fps > 0
            else self.min_hold_us
        )
        intended_down_count = sum(
            _scan_count(r, "scan_codes") for r in rows if r["kind"] == "down"
        )
        intended_up_count = sum(
            _scan_count(r, "scan_codes") for r in rows if r["kind"] == "up"
        )
        sent_down_count = sum(
            _scan_count(r, "sent_scan_codes") for r in rows if r["kind"] == "down"
        )
        sent_up_count = sum(
            _scan_count(r, "sent_scan_codes") for r in rows if r["kind"] == "up"
        )
        backend_skipped_down_count = sum(
            _scan_count(r, "skipped_scan_codes") for r in rows if r["kind"] == "down"
        )
        backend_skipped_up_count = sum(
            _scan_count(r, "skipped_scan_codes") for r in rows if r["kind"] == "up"
        )
        runtime_conflict_dropped_down_count = sum(
            _scan_count(r, "scan_codes") for r in rows
            if r.get("runtime_outcome") == "dropped_conflict"
        )
        expired_dropped_down_count = sum(
            _scan_count(r, "scan_codes") for r in rows
            if r.get("runtime_outcome") == "dropped_expired"
        )
        runtime_backend_dropped_down_count = sum(
            max(
                0,
                _scan_count(r, "scan_codes") - _scan_count(r, "sent_scan_codes"),
            )
            for r in rows
            if r["kind"] == "down"
            and r.get("runtime_outcome")
            not in {"dropped_conflict", "dropped_expired", "suppressed_stale_up"}
        )
        before_send_missing_down_count = (
            runtime_conflict_dropped_down_count
            + expired_dropped_down_count
            + runtime_backend_dropped_down_count
        )
        sender_clean = (
            intended_down_count == sent_down_count
            and before_send_missing_down_count == 0
            and backend_skipped_down_count == 0
        )
        summary = {
            "run_id": self.run_id,
            "song": self.song_name,
            "profile": self.profile_name,
            "fps": self.fps,
            "tempo_scale": self.tempo_scale,
            "total_events": len(rows),
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
                "sendinput_side": {
                    "sent_down_count": sent_down_count,
                    "sent_up_count": sent_up_count,
                    "backend_skipped_down_count": backend_skipped_down_count,
                    "backend_skipped_up_count": backend_skipped_up_count,
                    "sender_clean": sender_clean,
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
            "successful_dispatches": len(dispatch_records),  # already filtered to sent-only
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
            "deferred_release_count": sum(
                1 for record in self.records if int(record.get("deferred_by_us", 0)) > 0
            ),
            "release_deferral_us": _stats(
                [
                    int(record.get("deferred_by_us", 0))
                    for record in self.records
                    if int(record.get("deferred_by_us", 0)) > 0
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
        return summary
        
    def save(self) -> None:
        if not self.enabled or not self.log_filepath or not self.records:
            return

        try:
            # 1. Save standard raw CSV records
            fields = [
                "song",
                "event_index",
                "dispatch_id",
                "kind",
                "scheduled_us",
                "actual_us",
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
            ]
            with self.log_filepath.open("w", newline="", encoding="utf-8") as f:
                writer = csv.DictWriter(f, fieldnames=fields)
                writer.writeheader()
                writer.writerows(self.records)

            # 2. Reuse get_summary() — augment with timestamp for the persisted JSON
            summary = self.get_summary()
            if summary is None:
                return
            summary["timestamp"] = time.strftime('%Y-%m-%d %H:%M:%S')

            # 3. Save companion summary JSON
            summary_path = self.log_filepath.with_suffix(".summary.json")
            with summary_path.open("w", encoding="utf-8") as f:
                json.dump(summary, f, indent=2)

        except Exception as e:
            sys.stderr.write(f"[telemetry] failed to save metrics: {e}\n")

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
        print(f"No valid telemetry summary files (.summary.json) found at {target_path}")
        return
        
    print("\n==================================================")
    print(f" AGGREGATE TELEMETRY TIMING REPORT ({len(summary_files)} run(s))")
    print("==================================================")
    
    for f in summary_files:
        try:
            with f.open("r", encoding="utf-8") as file:
                data = json.load(file)
                
            print(f"\nPlayback: {data.get('song', 'Unknown')} at {data.get('timestamp', 'Unknown')} [Run ID: {data.get('run_id', 'N/A')}]")
            print(f"  Profile: {data.get('profile', 'balanced')} | Tempo Scale: {data.get('tempo_scale', 1.0)}")
            print(f"  Total Event Count: {data.get('total_events', 0)}")
            print(
                "  Evidence Boundary: "
                f"sender_clean={data.get('sender_clean', False)}, "
                f"before_send_missing_downs={data.get('before_send_missing_down_count', 0)}, "
                f"game_acceptance_unknown={data.get('game_acceptance_unknown', True)}"
            )
            
            lat = data.get("lateness_us", {})
            print("  Loop Lateness:")
            print(f"    * Average: {lat.get('avg_us', 0.0):.1f} us ({lat.get('avg_us', 0.0)/1000:.3f} ms)")
            print(f"    * Median (p50): {lat.get('p50_us', 0.0):.1f} us")
            print(f"    * 95th Percentile (p95): {lat.get('p95_us', 0.0):.1f} us")
            print(f"    * 99th Percentile (p99): {lat.get('p99_us', 0.0):.1f} us")
            print(f"    * Maximum: {lat.get('max_us', 0.0):.1f} us")
            print(f"    * Lateness Counts: >2ms={lat.get('over_2ms', 0)}, >5ms={lat.get('over_5ms', 0)}, >10ms={lat.get('over_10ms', 0)}")
            
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
                print(f"    * Average: {hold.get('avg_us', 0.0):.1f} us ({hold.get('avg_us', 0.0)/1000:.1f} ms)")
                print(f"    * p50: {hold.get('p50_us', 0.0):.1f} us")
                
            backend = data.get("backend", {})
            if backend.get("panic_release_failures", 0) > 0:
                print(f"  [warning] Backend panic release failures count: {backend.get('panic_release_failures')}")
                
            # Perform calibration recommendation if requested
            if recommend:
                from sky_music.orchestration.calibration import calibrate_profile, calibration_input_from_summary
                inp = calibration_input_from_summary(data)
                rec = calibrate_profile(inp)
                
                print("\n  Calibration Recommendation:")
                print(f"    * Suggested Profile : {rec.profile_name}")
                print(f"    * Suggested Tempo   : {rec.tempo_scale:.2f}x")
                print(f"    * Hold Duration (us): {rec.hold_us} ({rec.hold_us/1000:.1f} ms)")
                print(f"    * Severity Level    : {rec.severity.upper()}")
                print(f"    * Reason            : {rec.reason}")
        except Exception as e:
            print(f"  [error] Failed to read summary file {f.name}: {e}")
            
    print("\n==================================================")
