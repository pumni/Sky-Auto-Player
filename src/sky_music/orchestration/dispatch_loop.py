from __future__ import annotations

import statistics
from collections import deque
from collections.abc import Callable
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, Protocol

from sky_music.domain.scheduler_types import ActionKind, KeyAction, Microseconds
from sky_music.infrastructure.backend import (
    BackendHealth,
    InputBackend,
    ReleaseAllOutcome,
)
from sky_music.infrastructure.timing import Clock, Sleeper, SleepPolicy
from sky_music.infrastructure.wait_strategy import WaitStrategy
from sky_music.orchestration.playback_supervisor import (
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PLAYBACK_SKIPPED,
    CommandSource,
    FocusSignal,
    ProgressSink,
)
from sky_music.orchestration.runtime_dispatch import (
    PendingRelease,
    RuntimeActionBatch,
    RuntimeDispatchCoordinator,
    RuntimeKeyIntent,
)


class RuntimeSameKeyConflictError(RuntimeError):
    """Raised when confirmed runtime hold makes a strict same-key down infeasible."""


@dataclass(frozen=True, slots=True)
class ExecutionResult:
    """Result of executing a single KeyAction — used for telemetry and late compensation."""
    event_index: int
    scheduled_us: int
    actual_us: int
    lateness_us: int           # actual_us - scheduled_us; negative means early (expected when
                               # dispatch lead is applied — visible_lateness_us is the on-time metric)
    send_duration_us: int      # wall-clock time the backend call took (including bookkeeping)
    is_late: bool              # True when lateness_us > 0
    is_critically_late: bool   # True when lateness_us > 10_000 (10 ms)
    kind: str = "down"         # "down" (onset) or "up" (release) — used to separate counters
    dispatch_completed_us: int = 0
    deferred_by_us: int = 0
    visible_lateness_us: int = 0
    sent_scan_codes: tuple[int, ...] = ()
    skipped_scan_codes: tuple[int, ...] = ()
    runtime_outcome: str = "sent"
    applied_lead_us: int = 0
    send_duration_pure_us: int = 0  # time from backend call start to SendInput return (no bookkeeping)
    bookkeeping_us: int = 0        # time from SendInput return to backend call end
    dispatch_lateness_us: int = 0  # send_completed_us - action.at_us; player-side completion lateness


@dataclass(slots=True)
class PlaybackState:
    """Manages the runtime state of the playback loop."""
    start_perf: int
    pause_time_us: int = 0
    manual_pause_started_us: int | None = None
    focus_pause_started_us: int | None = None
    epoch_us: int = 0

    def __post_init__(self) -> None:
        self.epoch_us = self.start_perf + self.pause_time_us

    def is_paused(self) -> bool:
        return self.manual_pause_started_us is not None or self.focus_pause_started_us is not None

    def update_pause_time(self, duration_us: int) -> None:
        self.pause_time_us += duration_us
        self.epoch_us = self.start_perf + self.pause_time_us

    def rebase_epoch(self, now_us: int) -> int:
        """Move the playback anchor to now and return the old-to-new delta."""
        old_start_perf = self.start_perf
        self.start_perf = now_us
        self.epoch_us = self.start_perf + self.pause_time_us
        return now_us - old_start_perf

    def get_elapsed_us(self, clock: Clock, now_us: int | None = None) -> int:
        """Compute elapsed playback time in microseconds, accounting for pauses."""
        if now_us is None:
            now_us = clock.now_us()
        if self.manual_pause_started_us is not None:
            elapsed = self.manual_pause_started_us - self.epoch_us
            if self.focus_pause_started_us is not None:
                elapsed -= (now_us - self.focus_pause_started_us)
            return max(0, elapsed)
        if self.focus_pause_started_us is not None:
            return max(0, self.focus_pause_started_us - self.epoch_us)
        return max(0, now_us - self.epoch_us)

if TYPE_CHECKING:
    from sky_music.infrastructure.focus import FocusGuard
    from sky_music.orchestration.telemetry import TelemetryLogger


class DispatchHealthMonitor:
    """Manages foreground window focus checking and input path degradation telemetry."""

    def __init__(
        self,
        backend: InputBackend,
        clock: Clock,
        focus_guard: FocusGuard,
        require_focus: bool,
        input_path_warn_us: int = 300,
    ) -> None:
        self.backend = backend
        self.clock = clock
        self.focus_guard = focus_guard
        self.require_focus = require_focus
        self.input_path_warn_us = max(0, int(input_path_warn_us))

        self._send_duration_window: deque[int] = deque(maxlen=64)
        self._send_over_warn_count = 0
        self._input_path_degraded = False
        self._input_path_warn_started_us: int | None = None

        self._backend_health_snapshot_interval_us = 100_000
        self._backend_health_snapshot_at_us = -self._backend_health_snapshot_interval_us - 1
        self._backend_health_snapshot_value: BackendHealth | None = None

        self._focus_cache_ttl_us = 2_000
        self._focus_active_cache = True
        self._focus_cache_at_us = -self._focus_cache_ttl_us - 1

    def focus_is_active(self) -> bool:
        now_us = self.clock.now_us()
        if now_us - self._focus_cache_at_us >= self._focus_cache_ttl_us:
            self._focus_active_cache = self.focus_guard.is_active()
            self._focus_cache_at_us = now_us
        return self._focus_active_cache

    def get_backend_health_snapshot(self, force: bool = False) -> BackendHealth:
        now_us = self.clock.now_us()
        if (
            force
            or self._backend_health_snapshot_value is None
            or now_us - self._backend_health_snapshot_at_us
            >= self._backend_health_snapshot_interval_us
        ):
            self._backend_health_snapshot_value = self.backend.get_health()
            self._backend_health_snapshot_at_us = now_us
        return self._backend_health_snapshot_value

    def record_input_path_send_duration(self, send_duration_us: int, elapsed_us: int) -> None:
        if self.input_path_warn_us <= 0:
            return

        window = self._send_duration_window
        # Evict before append so we read the outgoing element once (deque[0]) only when full.
        evicted: int | None = window[0] if len(window) == window.maxlen else None
        val = max(0, int(send_duration_us))
        window.append(val)

        if evicted is not None and evicted > self.input_path_warn_us:
            self._send_over_warn_count -= 1
        if val > self.input_path_warn_us:
            self._send_over_warn_count += 1

        L = len(window)
        if self._send_over_warn_count <= L - 1 - round(0.95 * (L - 1)):
            self._input_path_warn_started_us = None
            return
        if self._input_path_warn_started_us is None:
            self._input_path_warn_started_us = elapsed_us
            return
        if elapsed_us - self._input_path_warn_started_us >= 1_000_000:
            self._input_path_degraded = True

    @property
    def input_path_degraded(self) -> bool:
        return self._input_path_degraded


class LeadEstimator(Protocol):
    def get_lead_us(self, kind: ActionKind, n_keys: int = 1) -> int: ...
    def update(self, kind: ActionKind, duration_us: int, n_keys: int = 1) -> None: ...
    def update_completion_error(self, kind: ActionKind, error_us: int) -> None: ...


class _NullEstimator:
    """Null-object estimator that always returns zero lead and accepts updates as no-ops.

    Replaces all ``if enable_adaptive_lead and estimator is not None`` branches on the hot
    path so the dispatch loop never needs to guess whether the estimator is active.
    """

    __slots__ = ()

    @staticmethod
    def get_lead_us(kind: str = "down", n_keys: int = 1) -> int:  # noqa: ARG004
        return 0

    @staticmethod
    def update(kind: str, duration_us: int, n_keys: int = 1) -> None:  # noqa: ARG004
        return

    @staticmethod
    def update_completion_error(kind: str, error_us: int) -> None:  # noqa: ARG004
        return


class DispatchLoop:
    """Core real-time dispatch loop implementation (wait -> drain -> execute)."""

    def __init__(
        self,
        coordinator: RuntimeDispatchCoordinator,
        clock: Clock,
        sleeper: Sleeper,
        wait_strategy: WaitStrategy,
        backend: InputBackend,
        telemetry: TelemetryLogger,
        sleep_policy: SleepPolicy,
        health_monitor: DispatchHealthMonitor,
        min_hold_us: int,
        spin_threshold_us: int,
        focus_restore_grace_us: int = 100_000,
        late_pulse_drop_threshold_us: int | None = None,
        same_key_conflict_policy: str = "degraded",
        enable_event_wait: bool = False,
        dispatch_lead_us: int = 0,
        estimator: LeadEstimator | None = None,
        enable_reprobe: bool = False,
        probe_callback: Callable[[Sleeper], int] | None = None,
        onset_bias_us: int = 0,
    ) -> None:
        self.coordinator = coordinator
        self.clock = clock
        self.sleeper = sleeper
        self.wait_strategy = wait_strategy
        self.backend = backend
        self.telemetry = telemetry
        self.onset_bias_us = onset_bias_us
        self.sleep_policy = sleep_policy
        self.health_monitor = health_monitor
        self.min_hold_us = min_hold_us
        self.spin_threshold_us = spin_threshold_us
        self.focus_restore_grace_us = focus_restore_grace_us
        self.late_pulse_drop_threshold_us = late_pulse_drop_threshold_us
        self.same_key_conflict_policy = same_key_conflict_policy
        self.enable_event_wait = enable_event_wait
        self.dispatch_lead_us = dispatch_lead_us
        self.estimator: LeadEstimator = estimator if estimator is not None else _NullEstimator()
        self.enable_reprobe = enable_reprobe
        self.probe_callback = probe_callback

        self._next_dispatch_id = 0
        self._wait_spin_start_us = 0
        self._last_send_completed_us = 0

        # Periodic reprobe state — track actual wake overshoot to adapt spin threshold under load
        self._overshoot_samples: deque[int] = deque(maxlen=200)
        self._last_reprobe_us: int = 0
        self._reprobe_interval_us: int = 5_000_000  # 5 seconds of elapsed playback time

    def _current_lead_up(self) -> int:
        if self.dispatch_lead_us > 0:
            return self.dispatch_lead_us
        return self.estimator.get_lead_us(ActionKind.UP)

    def get_current_leads(self) -> tuple[int, int]:
        if self.dispatch_lead_us > 0:
            lead_down = self.dispatch_lead_us
        else:
            lead_down = self.estimator.get_lead_us(ActionKind.DOWN)
        lead_down += self.onset_bias_us
        return lead_down, self._current_lead_up()

    def _down_lead_for_batch(self, batch: RuntimeActionBatch) -> int:
        # onset_bias_us is an onset-only (key-down) knob — never added to releases.
        if batch.kind == "up":
            if self.dispatch_lead_us > 0:
                return self.dispatch_lead_us
            return self.estimator.get_lead_us(ActionKind.UP)
        if self.dispatch_lead_us > 0:
            return self.dispatch_lead_us + self.onset_bias_us
        return self.estimator.get_lead_us(ActionKind.DOWN, len(batch.intents)) + self.onset_bias_us

    def _record_overshoot(self, elapsed_us: int, target_elapsed_us: int) -> None:
        overshoot_us = elapsed_us - target_elapsed_us
        if overshoot_us > 0:
            self._overshoot_samples.append(overshoot_us)

    def _recompute_spin_threshold_from_overshoot(self) -> int:
        if len(self._overshoot_samples) < 10:
            return self.spin_threshold_us
        mean = statistics.fmean(self._overshoot_samples)
        stdev = statistics.pstdev(self._overshoot_samples)
        return max(700, min(3_000, int(mean + 3 * stdev) + 100))

    def _release_all_and_cancel_runtime(self) -> ReleaseAllOutcome:
        outcome = self.backend.release_all()
        self.coordinator.cancel_all()
        return outcome

    @staticmethod
    def _intent_generation_ids(intents: tuple[RuntimeKeyIntent, ...]) -> tuple[int, ...]:
        return tuple(
            intent.generation_id
            for intent in intents
            if intent.generation_id is not None
        )

    def _record_without_dispatch(
        self,
        *,
        idx: int,
        kind: str,
        scheduled_us: int,
        scan_codes: tuple[int, ...],
        generation_ids: tuple[int, ...],
        reason: str,
        runtime_outcome: str,
        state: PlaybackState,
    ) -> None:
        now_us = state.get_elapsed_us(self.clock)
        dispatch_id = self._next_dispatch_id
        self._next_dispatch_id += 1
        self.telemetry.record(
            event_index=idx,
            kind=kind,
            scheduled_us=scheduled_us,
            actual_us=now_us,
            lateness_us=now_us - scheduled_us,
            send_duration_us=0,
            scan_codes=scan_codes,
            reason=reason,
            dispatch_id=dispatch_id,
            dispatch_completed_us=now_us,
            sent_scan_codes=(),
            skipped_scan_codes=(),
            generation_ids=generation_ids,
            runtime_outcome=runtime_outcome,
        )

    def _execute_action(
        self,
        idx: int,
        action: KeyAction,
        state: PlaybackState,
        *,
        generation_ids: tuple[int, ...] = (),
        runtime_outcome: str = "sent",
        applied_lead_us: int = 0,
        deferred_by_us: int = 0,
    ) -> ExecutionResult:
        send_start_raw = self.clock.now_us()
        send_start_us = state.get_elapsed_us(self.clock, send_start_raw)
        if action.kind == "down":
            send_result = self.backend.key_down(action.scan_codes)
        else:
            send_result = self.backend.key_up(action.scan_codes)
        send_end_raw = self.clock.now_us()
        send_end_us = state.get_elapsed_us(self.clock, send_end_raw)
        send_duration_us = send_end_us - send_start_us
        lateness_us = send_start_us - action.at_us

        # Prefer pure SendInput completion for onset / hold anchoring. Bookkeeping after the
        # syscall (health, focus, telemetry) must not push the "note landed" timestamp later
        # than the OS inject moment — that is the true game-facing onset.
        if send_result.send_completed_us is not None:
            send_duration_pure_us = send_result.send_completed_us - send_start_raw
            bookkeeping_us = send_end_raw - send_result.send_completed_us
            completion_us = state.get_elapsed_us(self.clock, send_result.send_completed_us)
        else:
            send_duration_pure_us = send_duration_us
            bookkeeping_us = 0
            completion_us = send_end_us

        pre_send_spin_us = max(0, send_start_us - self._wait_spin_start_us)
        idle_gap_us = max(0, self._wait_spin_start_us - self._last_send_completed_us)
        self._last_send_completed_us = completion_us
        visible_lateness_us = completion_us - action.at_us
        dispatch_lateness_us = lateness_us + send_duration_pure_us

        # Non-critical path: input-path health + unfocused diagnostics (never on the
        # completion-timestamp critical section above).
        self.health_monitor.record_input_path_send_duration(send_duration_us, send_end_us)
        if not self.health_monitor.focus_is_active():
            try:
                from sky_music.platform.win32 import inputs
                inputs.note_send_while_unfocused()
            except ImportError:
                pass

        result = ExecutionResult(
            event_index=idx,
            scheduled_us=action.at_us,
            actual_us=send_start_us,
            lateness_us=lateness_us,
            send_duration_us=send_duration_us,
            send_duration_pure_us=send_duration_pure_us,
            bookkeeping_us=bookkeeping_us,
            dispatch_lateness_us=dispatch_lateness_us,
            is_late=lateness_us > 0,
            is_critically_late=lateness_us > 10_000,
            kind=action.kind,
            dispatch_completed_us=completion_us,
            deferred_by_us=deferred_by_us,
            visible_lateness_us=visible_lateness_us,
            sent_scan_codes=send_result.sent,
            skipped_scan_codes=send_result.skipped_duplicates,
            runtime_outcome=runtime_outcome,
            applied_lead_us=applied_lead_us,
        )

        dispatch_id = self._next_dispatch_id
        self._next_dispatch_id += 1
        self.telemetry.record(
            result=result,
            kind=action.kind,
            scan_codes=action.scan_codes,
            reason=action.reason,
            dispatch_id=dispatch_id,
            pre_send_spin_us=pre_send_spin_us,
            idle_gap_us=idle_gap_us,
            generation_ids=generation_ids,
        )
        return result

    def _dispatch_down_batch(
        self,
        batch: RuntimeActionBatch,
        state: PlaybackState,
        *,
        lead_down: int,
        now_us: int | None = None,
    ) -> ExecutionResult | None:
        if now_us is None:
            now_us = state.get_elapsed_us(self.clock)

        if self.late_pulse_drop_threshold_us is not None and now_us - batch.scheduled_us > self.late_pulse_drop_threshold_us:
                self.coordinator.drop_expired_downs(batch.intents)
                self._record_without_dispatch(
                    idx=batch.source_action_index,
                    kind="down",
                    scheduled_us=batch.scheduled_us,
                    scan_codes=tuple(intent.scan_code for intent in batch.intents),
                    generation_ids=self._intent_generation_ids(batch.intents),
                    reason=batch.reason,
                    runtime_outcome="dropped_expired",
                    state=state,
                )
                return None

        playable, conflicts = self.coordinator.split_down_intents(batch.intents)
        if conflicts:
            self._record_without_dispatch(
                idx=batch.source_action_index,
                kind="down",
                scheduled_us=batch.scheduled_us,
                scan_codes=tuple(intent.scan_code for intent in conflicts),
                generation_ids=self._intent_generation_ids(conflicts),
                reason=batch.reason,
                runtime_outcome="dropped_conflict",
                state=state,
            )
            if self.same_key_conflict_policy == "strict":
                raise RuntimeSameKeyConflictError(
                    "Runtime same-key conflict under strict policy"
                )

        if not playable:
            return None

        action = KeyAction(
            kind=ActionKind.DOWN,
            scan_codes=tuple(intent.scan_code for intent in playable),  # type: ignore[arg-type]
            at_us=Microseconds(batch.scheduled_us),
            reason=batch.reason,
        )
        result = self._execute_action(
            batch.source_action_index,
            action,
            state,
            generation_ids=self._intent_generation_ids(playable),
            applied_lead_us=lead_down,
        )
        # Lead tracks pure SendInput duration (not post-syscall bookkeeping) so completions
        # land on schedule rather than overshooting by the telemetry/health tail.
        self.estimator.update(ActionKind.DOWN, result.send_duration_pure_us, n_keys=len(playable))
        if lead_down > 0:
            # Residual completion error after lead was applied — systematic prologue bias
            # (spin overshoot + Python work before SendInput) folds into the next lead.
            self.estimator.update_completion_error(ActionKind.DOWN, result.visible_lateness_us)
        self.coordinator.activate_sent_downs(
            playable,
            tuple(int(scan_code) for scan_code in result.sent_scan_codes),
            dispatch_started_us=result.actual_us,
            dispatch_completed_us=result.dispatch_completed_us,
        )
        return result

    def _dispatch_pending_releases(
        self,
        releases: tuple[PendingRelease, ...],
        state: PlaybackState,
        *,
        lead_up: int,
    ) -> ExecutionResult | None:
        if not releases:
            return None

        # Fast path: single release is dominant in real songs. Avoids every set/list allocation.
        if len(releases) == 1:
            only = releases[0]
            # Deferral is vs the completion-anchored floor (release_not_before), not the
            # start-anchored estimate — matches when the coordinator actually holds the up.
            deferred_by_us = max(0, only.release_not_before_us - only.scheduled_release_us)
            action = KeyAction(
                kind=ActionKind.UP,
                scan_codes=(only.scan_code,),  # type: ignore[arg-type]
                at_us=Microseconds(only.scheduled_release_us),
                reason=only.reason,
            )
            result = self._execute_action(
                only.source_action_index,
                action,
                state,
                generation_ids=(only.generation_id,),
                runtime_outcome="deferred_release" if deferred_by_us > 0 else "sent",
                applied_lead_us=lead_up,
                deferred_by_us=deferred_by_us,
            )
            self.estimator.update(ActionKind.UP, result.send_duration_pure_us)
            self.coordinator.complete_releases(
                releases,
                tuple(int(sc) for sc in result.sent_scan_codes),
                tuple(int(sc) for sc in result.skipped_scan_codes),
            )
            return result

        # Multi-release path (chords / deferred batches): single pass over releases.
        best = releases[0]
        best_key = (
            best.effective_release_us,
            best.source_action_index,
            best.scan_code,
        )
        scheduled_us = best.scheduled_release_us
        max_deferral = 0
        first_source_idx = best.source_action_index
        first_reason = best.reason
        all_same_source = True
        scan_codes_list: list[int] = []
        gen_ids_list: list[int] = []
        for release in releases:
            eff = release.effective_release_us
            key = (eff, release.source_action_index, release.scan_code)
            if key < best_key:
                best = release
                best_key = key
            if release.scheduled_release_us < scheduled_us:
                scheduled_us = release.scheduled_release_us
            deferral = release.release_not_before_us - release.scheduled_release_us
            if deferral > max_deferral:
                max_deferral = deferral
            if release.source_action_index != first_source_idx or release.reason != first_reason:
                all_same_source = False
            scan_codes_list.append(release.scan_code)
            gen_ids_list.append(release.generation_id)
        if len(scan_codes_list) != len(set(scan_codes_list)):
            raise RuntimeError(
                f"duplicate scan codes in pending releases: {scan_codes_list}"
            )
        deferred_by_us = max(0, max_deferral)
        reason = best.reason if all_same_source else "mixed_deferred_release"
        action = KeyAction(
            kind=ActionKind.UP,
            scan_codes=tuple(scan_codes_list),  # type: ignore[arg-type]
            at_us=Microseconds(scheduled_us),
            reason=reason,
        )
        result = self._execute_action(
            best.source_action_index,
            action,
            state,
            generation_ids=tuple(gen_ids_list),
            runtime_outcome="deferred_release" if deferred_by_us > 0 else "sent",
            applied_lead_us=lead_up,
            deferred_by_us=deferred_by_us,
        )
        self.estimator.update(ActionKind.UP, result.send_duration_pure_us)
        self.coordinator.complete_releases(
            releases,
            tuple(int(sc) for sc in result.sent_scan_codes),
            tuple(int(sc) for sc in result.skipped_scan_codes),
        )
        return result

    def _request_up_batch(
        self,
        batch: RuntimeActionBatch,
        state: PlaybackState,
    ) -> None:
        _, suppressed = self.coordinator.request_releases(batch.intents)
        if suppressed:
            self._record_without_dispatch(
                idx=batch.source_action_index,
                kind="up",
                scheduled_us=batch.scheduled_us,
                scan_codes=tuple(intent.scan_code for intent in suppressed),
                generation_ids=self._intent_generation_ids(suppressed),
                reason=batch.reason,
                runtime_outcome="suppressed_stale_up",
                state=state,
            )

    def _handle_commands(
        self,
        command: str | None,
        state: PlaybackState,
        total_time_us: int,
        progress_sink: ProgressSink,
    ) -> str | None:
        if command == "quit":
            progress_sink.finish(f"Stopped: {self.telemetry.song_name}")
            return PLAYBACK_QUIT
        if command == "skip":
            progress_sink.finish(f"Skipped: {self.telemetry.song_name}")
            return PLAYBACK_SKIPPED
        if command == "refocus":
            self.health_monitor.focus_guard.focus()
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status="refocus",
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
                force=True,
            )
        if command == "panic":
            self._release_all_and_cancel_runtime()
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status="panic",
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
                force=True,
            )
        if command == "pause":
            if state.manual_pause_started_us is None:
                self._release_all_and_cancel_runtime()
                state.manual_pause_started_us = self.clock.now_us()
                progress_sink.publish(
                    elapsed_us=state.get_elapsed_us(self.clock),
                    total_us=total_time_us,
                    status="paused",
                    health=self.health_monitor.get_backend_health_snapshot(force=True),
                    input_path_degraded=self.health_monitor.input_path_degraded,
                    force=True,
                )
            else:
                pause_duration_us = self.clock.now_us() - state.manual_pause_started_us
                state.update_pause_time(pause_duration_us)
                self.telemetry.record_pause("manual", pause_duration_us)
                state.manual_pause_started_us = None
                progress_sink.publish(
                    elapsed_us=state.get_elapsed_us(self.clock),
                    total_us=total_time_us,
                    status="playing",
                    health=self.health_monitor.get_backend_health_snapshot(force=True),
                    input_path_degraded=self.health_monitor.input_path_degraded,
                    force=True,
                )
        return None

    def _process_wait_states(
        self,
        state: PlaybackState,
        first_action_executed: bool,
        total_time_us: int,
        command_source: CommandSource,
        focus_signal: FocusSignal,
        progress_sink: ProgressSink,
    ) -> tuple[bool, str | None]:
        if state.manual_pause_started_us is not None:
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status="paused",
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
            )
            self.sleeper.sleep(self.sleep_policy.poll_s)
            return True, None

        if self.health_monitor.require_focus and not focus_signal.is_active():
            if state.focus_pause_started_us is None:
                self.coordinator.cancel_all()
                state.focus_pause_started_us = self.clock.now_us()
            status_val = "waiting_for_focus" if not first_action_executed else "focus_lost"
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status=status_val,
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
            )
            self.sleeper.sleep(self.sleep_policy.poll_s)
            return True, None

        if state.focus_pause_started_us is not None:
            grace_us = self.focus_restore_grace_us
            grace_start_us = self.clock.now_us()
            while self.clock.now_us() - grace_start_us < grace_us:
                self.sleeper.sleep(0.005)
                early_cmd = command_source.poll()
                cmd_res = self._handle_commands(
                    early_cmd,
                    state,
                    total_time_us,
                    progress_sink,
                )
                if cmd_res:
                    return True, cmd_res
                if early_cmd in ("pause", "panic"):
                    break

            if self.enable_reprobe and self.probe_callback is not None and self.sleeper is not None:
                new_threshold = self.probe_callback(self.sleeper)
                self.spin_threshold_us = new_threshold

            self.backend.release_all()

            pause_duration_us = self.clock.now_us() - state.focus_pause_started_us
            state.update_pause_time(pause_duration_us)
            self.telemetry.record_pause("focus", pause_duration_us)
            state.focus_pause_started_us = None
            status = "paused" if state.manual_pause_started_us is not None else "playing"
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status=status,
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
                force=True,
            )
            if state.manual_pause_started_us is not None:
                return True, None
        return False, None

    def _service_control_state(
        self,
        state: PlaybackState,
        first_action_executed: bool,
        total_time_us: int,
        command_source: CommandSource,
        focus_signal: FocusSignal,
        progress_sink: ProgressSink,
        *,
        check_focus_signal: bool,
    ) -> str | None:
        while True:
            needs_service = (
                state.manual_pause_started_us is not None
                or state.focus_pause_started_us is not None
                or (
                    check_focus_signal
                    and self.health_monitor.require_focus
                    and not focus_signal.is_active()
                )
            )
            if not needs_service:
                return None

            command = command_source.poll()
            cmd_res = self._handle_commands(
                command,
                state,
                total_time_us,
                progress_sink,
            )
            if cmd_res:
                return cmd_res

            wait_res, wait_cmd = self._process_wait_states(
                state,
                first_action_executed,
                total_time_us,
                command_source,
                focus_signal,
                progress_sink,
            )
            if wait_cmd:
                return wait_cmd
            if not wait_res:
                return None

    def _wait_until_runtime_deadline(
        self,
        target_elapsed_us: int,
        state: PlaybackState,
        last_runtime_poll_us: int,
        last_render_time_us: int,
        first_action_executed: bool,
        total_time_us: int,
        command_source: CommandSource,
        focus_signal: FocusSignal,
        progress_sink: ProgressSink,
        command_event: int | None = None,
    ) -> Any:
        # Polling cadence for command/focus housekeeping in non-event (polled) mode.
        # 1 ms cost ~1000 wake-ups/second even when the dispatch thread sits idle in
        # long inter-note gaps. 2 ms halves this at the cost of <= 2 ms extra command
        # acknowledgement latency — commands are human-rate (≈200 ms reaction floor),
        # so 2 ms is well below the noise floor of any user interaction.
        poll_interval_us = 2_000

        while True:
            if state.is_paused():
                service_result = self._service_control_state(
                    state,
                    first_action_executed,
                    total_time_us,
                    command_source,
                    focus_signal,
                    progress_sink,
                    check_focus_signal=False,
                )
                if service_result:
                    return service_result, last_runtime_poll_us, last_render_time_us, first_action_executed

            elapsed_us = state.get_elapsed_us(self.clock)
            if elapsed_us >= target_elapsed_us:
                self._wait_spin_start_us = elapsed_us
                if self.enable_reprobe:
                    self._record_overshoot(elapsed_us, target_elapsed_us)
                return None, last_runtime_poll_us, last_render_time_us, first_action_executed

            remaining_us = target_elapsed_us - elapsed_us
            target_system_us = state.epoch_us + target_elapsed_us
            if remaining_us <= self.spin_threshold_us:
                self._wait_spin_start_us = elapsed_us
                self.wait_strategy.spin_until_us(target_system_us, self.clock)
                if self.enable_reprobe:
                    after_elapsed = state.get_elapsed_us(self.clock)
                    self._record_overshoot(after_elapsed, target_elapsed_us)
                return None, last_runtime_poll_us, last_render_time_us, first_action_executed

            service_result = self._service_control_state(
                state,
                first_action_executed,
                total_time_us,
                command_source,
                focus_signal,
                progress_sink,
                check_focus_signal=True,
            )
            if service_result:
                return service_result, last_runtime_poll_us, last_render_time_us, first_action_executed

            woken_by_event = self.wait_strategy.wait_until_us(
                target_system_us=target_system_us,
                clock=self.clock,
                sleeper=self.sleeper,
                spin_threshold_us=self.spin_threshold_us,
                policy=self.sleep_policy,
                command_event=command_event,
            )

            now_us = self.clock.now_us()
            # Polling is governed by the PRESENCE of a command event, not the enable flag: in
            # event mode (handle provided by the supervisor) commands arrive via event wake-ups
            # only; without a handle (direct/non-threaded paths) the loop must keep polling.
            if woken_by_event or (command_event is None and (now_us - last_runtime_poll_us >= poll_interval_us)):
                last_runtime_poll_us = now_us
                command = command_source.poll()
                cmd_res = self._handle_commands(
                    command,
                    state,
                    total_time_us,
                    progress_sink,
                )
                if cmd_res:
                    return cmd_res, last_runtime_poll_us, last_render_time_us, first_action_executed

                wait_res, wait_cmd = self._process_wait_states(
                    state,
                    first_action_executed,
                    total_time_us,
                    command_source,
                    focus_signal,
                    progress_sink,
                )
                if wait_res:
                    if wait_cmd:
                        return wait_cmd, last_runtime_poll_us, last_render_time_us, first_action_executed
                    continue

                elapsed_us = state.get_elapsed_us(self.clock)
                remaining_us = target_elapsed_us - elapsed_us
                # In event mode the supervisor publishes periodic progress; the loop only
                # publishes from its polled path.
                if command_event is None and remaining_us >= 5_000:
                    now_render_us = self.clock.now_us()
                    if now_render_us - last_render_time_us >= 33_000:
                        last_render_time_us = now_render_us
                        progress_sink.publish(
                            elapsed_us=elapsed_us,
                            total_us=total_time_us,
                            status="playing",
                            health=self.health_monitor.get_backend_health_snapshot(),
                            input_path_degraded=self.health_monitor.input_path_degraded,
                        )

    def _drain_due(
        self,
        now_us: int,
        state: PlaybackState,
        lead_up: int,
        observe: object = None,
    ) -> None:
        """Drain all due actions at *now_us* and immediately observe each result.

        Accepts an optional ``observe`` callable (ExecutionResult | None) -> None so the
        caller avoids building a tuple of results just to iterate over it.  The default
        ``None`` sentinel means "skip the observe call" (used internally when the result
        is not needed, e.g. the run() loop provides its own closure).

        The per-batch down lead comes solely from lead_for_batch (_down_lead_for_batch);
        a scalar down lead would be overridden inside pop_due_authored anyway.
        """
        pending = self.coordinator.pop_due_pending(now_us, lead_up)
        if pending:
            result = self._dispatch_pending_releases(pending, state, lead_up=lead_up)
            if observe is not None:
                observe(result)  # type: ignore[operator]

        for batch in self.coordinator.pop_due_authored(
            now_us, lead_for_batch=self._down_lead_for_batch
        ):
            if batch.kind == "up":
                self._request_up_batch(batch, state)
                newly_due = self.coordinator.pop_due_pending(state.get_elapsed_us(self.clock), lead_up)
                result = self._dispatch_pending_releases(newly_due, state, lead_up=lead_up)
            else:
                down_lead = self._down_lead_for_batch(batch)
                result = self._dispatch_down_batch(batch, state, lead_down=down_lead, now_us=now_us)
            if observe is not None:
                observe(result)  # type: ignore[operator]

    def run(
        self,
        state: PlaybackState,
        command_source: CommandSource,
        focus_signal: FocusSignal,
        progress_sink: ProgressSink,
        total_time_us: int,
        command_event: int | None = None,
    ) -> str:
        last_runtime_poll_us = -1000
        last_render_time_us = 0
        first_action_executed = False

        def observe_result(exec_result: ExecutionResult | None) -> None:
            if exec_result is None:
                return
            if exec_result.runtime_outcome != "deferred_release":
                progress_sink.update_counters(exec_result.lateness_us, exec_result.kind)

        # Defined once (not per loop iteration) so the hot dispatch window never allocates a
        # fresh closure per note. ``first_action_executed`` is captured by reference, so the
        # loop's own reassignment of it (from the wait-deadline return) and this closure's
        # nonlocal write target the same variable — identical behaviour to an inline def.
        def _observe(result: ExecutionResult | None) -> None:
            nonlocal first_action_executed
            if result is not None:
                first_action_executed = True
            observe_result(result)

        try:
            while not self.coordinator.is_finished():
                # The per-batch down lead (_down_lead_for_batch) overrides the scalar dispatch_lead_us
                # arg inside next_authored_us, so only lead_up is consumed at the loop level. Computing
                # the discarded down lead here would be a wasted estimator read in the hot window.
                lead_up = self._current_lead_up()
                deadline_us = self.coordinator.next_deadline_us(
                    0, lead_up, lead_for_batch=self._down_lead_for_batch
                )
                if deadline_us is None:
                    break
                command_result, last_runtime_poll_us, last_render_time_us, first_action_executed = self._wait_until_runtime_deadline(
                    deadline_us,
                    state,
                    last_runtime_poll_us,
                    last_render_time_us,
                    first_action_executed,
                    total_time_us,
                    command_source,
                    focus_signal,
                    progress_sink,
                    command_event=command_event,
                )
                if command_result:
                    return command_result

                now_us = state.get_elapsed_us(self.clock)
                self._drain_due(now_us, state, lead_up, observe=_observe)

                if self.enable_reprobe and now_us - self._last_reprobe_us >= self._reprobe_interval_us:
                    new_threshold = self._recompute_spin_threshold_from_overshoot()
                    if new_threshold > self.spin_threshold_us:
                        self.spin_threshold_us = new_threshold
                    self._last_reprobe_us = now_us

            progress_sink.publish(
                elapsed_us=total_time_us,
                total_us=total_time_us,
                status="done",
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
                force=True,
            )
            progress_sink.finish(f"Finished playing {self.telemetry.song_name}")

            return PLAYBACK_FINISHED
        except RuntimeSameKeyConflictError:
            progress_sink.finish(
                f"Stopped: runtime same-key conflict in {self.telemetry.song_name}"
            )
            return PLAYBACK_QUIT
        finally:
            outcome = self._release_all_and_cancel_runtime()
            self.telemetry.record_generation_status_counts(
                self.coordinator.generation_status_counts()
            )
            self.telemetry.record_release_outcome(outcome)
            self.telemetry.record_backend_health(self.backend.get_health())
            self.telemetry.record_input_path_health(
                degraded=self.health_monitor.input_path_degraded,
                warn_us=self.health_monitor.input_path_warn_us,
            )
            # Partial-send diagnostics: recorded BEFORE save() so they reach the summary. The
            # dispatch thread is the sole sender and is finishing here, so the counters are final.
            # Pure instrumentation — never let a backend without it (test fakes) break teardown.
            # The debug_log lives here on the dispatch thread (not in engine.py's finally) so
            # reading the counters never races with the writer — critical under free-threaded builds.
            get_send_diag = getattr(self.backend, "get_send_diagnostics", None)
            if get_send_diag is not None:
                send_diag = get_send_diag()
                self.telemetry.record_runtime_options(
                    {
                        **self.telemetry.runtime_options,
                        "send_diagnostics": send_diag,
                    }
                )
                if send_diag.get("partial_send_events"):
                    from sky_music.platform.win32 import inputs as _inputs_diag
                    _inputs_diag.debug_log(
                        "[input] SEND DIAGNOSTICS (this run): "
                        f"chord_splits={send_diag.get('chord_split_events', 0)}, "
                        f"partial_send_events={send_diag.get('partial_send_events', 0)}, "
                        f"keys_deferred={send_diag.get('keys_deferred', 0)}, "
                        f"keys_dropped={send_diag.get('keys_dropped', 0)}, "
                        f"keys_retried={send_diag.get('keys_retried', 0)}, "
                        f"zero_progress_retries={send_diag.get('zero_progress_retries', 0)}"
                    )
            self.telemetry.save()
