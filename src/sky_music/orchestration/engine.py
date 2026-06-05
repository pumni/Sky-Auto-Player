from dataclasses import dataclass
from typing import Tuple, Optional
from sky_music.domain.domain import Song
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import InputBackend, ReleaseAllOutcome
from sky_music.orchestration.telemetry import TelemetryLogger
from sky_music.infrastructure.timing import Clock, Sleeper, PerfCounterClock, RealSleeper, SleepPolicy, PreciseSleeper
from sky_music.infrastructure.focus import FocusGuard, NoopFocusGuard, Win32SkyFocusGuard
from sky_music.orchestration.runtime_dispatch import (
    PendingRelease,
    RuntimeActionBatch,
    RuntimeDispatchCoordinator,
    RuntimeKeyIntent,
    compile_runtime_intents,
)

# We use standard outputs from UI and main
PLAYBACK_FINISHED = "finished"
PLAYBACK_SKIPPED = "skipped"
PLAYBACK_QUIT = "quit"


class RuntimeSameKeyConflictError(RuntimeError):
    """Raised when confirmed runtime hold makes a strict same-key down infeasible."""


@dataclass(frozen=True, slots=True)
class ExecutionResult:
    """Result of executing a single KeyAction — used for telemetry and late compensation."""
    event_index: int
    scheduled_us: int
    actual_us: int
    lateness_us: int           # actual_us - scheduled_us; negative means early (should not happen)
    send_duration_us: int      # wall-clock time the backend call took
    is_late: bool              # True when lateness_us > 0
    is_critically_late: bool   # True when lateness_us > 10_000 (10 ms)
    dispatch_completed_us: int = 0
    sent_scan_codes: tuple[int, ...] = ()
    skipped_scan_codes: tuple[int, ...] = ()
    runtime_outcome: str = "sent"


@dataclass
class PlaybackState:
    """Manages the runtime state of the playback loop."""
    start_perf: int
    pause_time_us: int = 0
    manual_pause_started_us: Optional[int] = None
    focus_pause_started_us: Optional[int] = None

    def is_paused(self) -> bool:
        return self.manual_pause_started_us is not None or self.focus_pause_started_us is not None

    def get_elapsed_us(self, clock: Clock) -> int:
        """Compute elapsed playback time in microseconds, accounting for pauses."""
        now_us = clock.now_us()
        elapsed = now_us - self.start_perf - self.pause_time_us
        if self.manual_pause_started_us is not None:
            elapsed -= (now_us - self.manual_pause_started_us)
        if self.focus_pause_started_us is not None:
            elapsed -= (now_us - self.focus_pause_started_us)
        return max(0, elapsed)


class PlaybackEngine:
    """Manages the real-time execution loop of the scheduled KeyActions timeline."""
    _runtime_poll_interval_us = 1_000

    def __init__(
        self,
        song: Song,
        actions: Tuple[KeyAction, ...],
        backend: InputBackend,
        controls = None,
        renderer = None,
        telemetry_enabled: bool = False,
        require_focus: bool = True,
        clock: Optional[Clock] = None,
        sleeper: Optional[Sleeper] = None,
        sleep_policy: SleepPolicy = SleepPolicy(),
        focus_guard: Optional[FocusGuard] = None,
        profile_name: str = "balanced",
        tempo_scale: float = 1.0,
        focus_restore_grace_us: int = 100_000,
        fps: Optional[int] = None,
        min_hold_us: int = 0,
        same_key_conflict_policy: str = "degraded",
    ):
        self.song = song
        self.actions = actions
        self.runtime_schedule = compile_runtime_intents(actions)
        self.total_time_us = max((int(action.at_us) for action in actions), default=0)
        self.backend = backend
        self.focus_restore_grace_us = focus_restore_grace_us
        self.min_hold_us = max(0, int(min_hold_us))
        self.same_key_conflict_policy = same_key_conflict_policy
        self.controls = controls
        self.renderer = renderer
        self.telemetry = TelemetryLogger(
            song.name,
            enabled=telemetry_enabled,
            profile_name=profile_name,
            tempo_scale=tempo_scale,
            fps=fps,
            min_hold_us=self.min_hold_us,
        )
        self.require_focus = require_focus
        self.clock = clock if clock is not None else PerfCounterClock()
        self.sleeper = sleeper if sleeper is not None else RealSleeper()
        self.sleep_policy = sleep_policy
        self.precise_sleeper = PreciseSleeper()
        if self.renderer is not None:
            self.renderer.backend = self.backend
        
        # Inject standard FocusGuard depending on requirements
        if focus_guard is None:
            if self.require_focus:
                self.focus_guard: FocusGuard = Win32SkyFocusGuard()
            else:
                self.focus_guard = NoopFocusGuard()
        else:
            self.focus_guard = focus_guard

        # is_active() is a heavy Win32 chain (GetForegroundWindow +
        # OpenProcess/QueryFullProcessImageName/CloseHandle for process validation). The approach
        # phase checks focus at most once per runtime-poll interval and the final spin bypasses it
        # entirely. A short TTL further reduces the heavy calls without making alt-tab detection
        # perceptibly slower.
        self._focus_cache_ttl_us: int = 2_000
        self._focus_active_cache: bool = True
        self._focus_cache_at_us: int = -self._focus_cache_ttl_us - 1
        self._runtime_coordinator: RuntimeDispatchCoordinator | None = None
        self._next_dispatch_id = 0

    def _release_all_and_cancel_runtime(self) -> ReleaseAllOutcome:
        outcome = self.backend.release_all()
        if self._runtime_coordinator is not None:
            self._runtime_coordinator.cancel_all()
        return outcome

    def _handle_commands(self, command: Optional[str], state: PlaybackState, total_time_seconds: float) -> Optional[str]:
        """Handles playback commands like pause, skip, quit, etc."""
        if command == "quit":
            if self.renderer:
                self.renderer.finish(f"Stopped: {self.song.name}")
            return PLAYBACK_QUIT
        if command == "skip":
            if self.renderer:
                self.renderer.finish(f"Skipped: {self.song.name}")
            return PLAYBACK_SKIPPED
        if command == "refocus":
            self.focus_guard.focus()
            if self.renderer:
                self.renderer.render(state.get_elapsed_us(self.clock) / 1_000_000, total_time_seconds, self.song.name, status="refocus", force=True)
        if command == "panic":
            self._release_all_and_cancel_runtime()
            if self.renderer:
                self.renderer.render(state.get_elapsed_us(self.clock) / 1_000_000, total_time_seconds, self.song.name, status="panic", force=True)
        if command == "pause":
            if state.manual_pause_started_us is None:
                self._release_all_and_cancel_runtime()
                state.manual_pause_started_us = self.clock.now_us()
                if self.renderer:
                    self.renderer.render(state.get_elapsed_us(self.clock) / 1_000_000, total_time_seconds, self.song.name, status="paused", force=True)
            else:
                pause_duration_us = self.clock.now_us() - state.manual_pause_started_us
                state.pause_time_us += pause_duration_us
                self.telemetry.record_pause("manual", pause_duration_us)
                state.manual_pause_started_us = None
                if self.renderer:
                    self.renderer.render(state.get_elapsed_us(self.clock) / 1_000_000, total_time_seconds, self.song.name, status="playing", force=True)
        return None

    def _focus_is_active(self) -> bool:
        """Return memoised focus state, refreshing the heavy Win32 check after its short TTL."""
        now_us = self.clock.now_us()
        if now_us - self._focus_cache_at_us >= self._focus_cache_ttl_us:
            self._focus_active_cache = self.focus_guard.is_active()
            self._focus_cache_at_us = now_us
        return self._focus_active_cache

    def _process_wait_states(self, state: PlaybackState, first_action_executed: bool, total_time_seconds: float) -> Tuple[bool, Optional[str]]:
        """Handles focus lost and manual pause wait states."""
        if state.manual_pause_started_us is not None:
            if self.renderer:
                self.renderer.render(state.get_elapsed_us(self.clock) / 1_000_000, total_time_seconds, self.song.name, status="paused")
            self.sleeper.sleep(self.sleep_policy.poll_s)
            return True, None

        if self.require_focus and not self._focus_is_active():
            if state.focus_pause_started_us is None:
                self._release_all_and_cancel_runtime()
                state.focus_pause_started_us = self.clock.now_us()
            if self.renderer:
                status_val = "waiting_for_focus" if not first_action_executed else "focus_lost"
                self.renderer.render(state.get_elapsed_us(self.clock) / 1_000_000, total_time_seconds, self.song.name, status=status_val)
            self.sleeper.sleep(self.sleep_policy.poll_s)
            return True, None

        if state.focus_pause_started_us is not None:
            grace_us = self.focus_restore_grace_us
            grace_start_us = self.clock.now_us()
            while self.clock.now_us() - grace_start_us < grace_us:
                self.sleeper.sleep(0.005)
                if self.controls is not None:
                    early_cmd = self.controls.poll()
                    cmd_res = self._handle_commands(early_cmd, state, total_time_seconds)
                    if cmd_res:
                        return True, cmd_res
                    if early_cmd in ("pause", "panic"):
                        break

            pause_duration_us = self.clock.now_us() - state.focus_pause_started_us
            state.pause_time_us += pause_duration_us
            self.telemetry.record_pause("focus", pause_duration_us)
            state.focus_pause_started_us = None
            if self.renderer:
                status = "paused" if state.manual_pause_started_us is not None else "playing"
                self.renderer.render(state.get_elapsed_us(self.clock) / 1_000_000, total_time_seconds, self.song.name, status=status, force=True)
            if state.manual_pause_started_us is not None:
                return True, None
        return False, None

    def _execute_action(
        self,
        idx: int,
        action: KeyAction,
        state: PlaybackState,
        *,
        generation_ids: tuple[int, ...] = (),
        runtime_outcome: str = "sent",
        deferred_by_us: int = 0,
    ) -> ExecutionResult:
        """Dispatch a single KeyAction to the backend and record precise timing metrics."""
        send_start_us = state.get_elapsed_us(self.clock)
        if action.kind == "down":
            send_result = self.backend.key_down(action.scan_codes)
        else:
            send_result = self.backend.key_up(action.scan_codes)
        send_end_us = state.get_elapsed_us(self.clock)
        send_duration_us = send_end_us - send_start_us
        lateness_us = send_start_us - action.at_us

        result = ExecutionResult(
            event_index=idx,
            scheduled_us=action.at_us,
            actual_us=send_start_us,
            lateness_us=lateness_us,
            send_duration_us=send_duration_us,
            is_late=lateness_us > 0,
            is_critically_late=lateness_us > 10_000,
            dispatch_completed_us=send_end_us,
            sent_scan_codes=send_result.sent,
            skipped_scan_codes=send_result.skipped_duplicates,
            runtime_outcome=runtime_outcome,
        )

        dispatch_id = self._next_dispatch_id
        self._next_dispatch_id += 1
        self.telemetry.record(
            event_index=idx,
            kind=action.kind,
            scheduled_us=action.at_us,
            actual_us=send_start_us,
            lateness_us=lateness_us,
            send_duration_us=send_duration_us,
            scan_codes=action.scan_codes,
            reason=action.reason,
            dispatch_id=dispatch_id,
            dispatch_completed_us=send_end_us,
            sent_scan_codes=send_result.sent,
            skipped_scan_codes=send_result.skipped_duplicates,
            generation_ids=generation_ids,
            runtime_outcome=runtime_outcome,
            deferred_by_us=deferred_by_us,
        )
        return result

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

    @staticmethod
    def _intent_generation_ids(intents: tuple[RuntimeKeyIntent, ...]) -> tuple[int, ...]:
        return tuple(
            intent.generation_id
            for intent in intents
            if intent.generation_id is not None
        )

    def _dispatch_down_batch(
        self,
        batch: RuntimeActionBatch,
        state: PlaybackState,
        coordinator: RuntimeDispatchCoordinator,
    ) -> ExecutionResult | None:
        playable, conflicts = coordinator.split_down_intents(batch.intents)
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
            kind="down",
            scan_codes=tuple(ScanCode(intent.scan_code) for intent in playable),
            at_us=Microseconds(batch.scheduled_us),
            reason=batch.reason,
        )
        result = self._execute_action(
            batch.source_action_index,
            action,
            state,
            generation_ids=self._intent_generation_ids(playable),
        )
        coordinator.activate_sent_downs(
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
        coordinator: RuntimeDispatchCoordinator,
    ) -> ExecutionResult | None:
        if not releases:
            return None
        representative = min(
            releases,
            key=lambda release: (
                release.effective_release_us,
                release.source_action_index,
                release.scan_code,
            ),
        )
        scheduled_us = min(release.scheduled_release_us for release in releases)
        deferred_by_us = max(
            0,
            max(release.effective_release_us - release.scheduled_release_us for release in releases),
        )
        source_action_indices = {release.source_action_index for release in releases}
        reasons = {release.reason for release in releases}
        reason = (
            representative.reason
            if len(source_action_indices) == 1 and len(reasons) == 1
            else "mixed_deferred_release"
        )
        action = KeyAction(
            kind="up",
            scan_codes=tuple(ScanCode(release.scan_code) for release in releases),
            at_us=Microseconds(scheduled_us),
            reason=reason,
        )
        result = self._execute_action(
            representative.source_action_index,
            action,
            state,
            generation_ids=tuple(release.generation_id for release in releases),
            runtime_outcome="deferred_release" if deferred_by_us > 0 else "sent",
            deferred_by_us=deferred_by_us,
        )
        coordinator.complete_releases(
            releases,
            tuple(int(scan_code) for scan_code in result.sent_scan_codes),
            tuple(int(scan_code) for scan_code in result.skipped_scan_codes),
        )
        return result

    def _request_up_batch(
        self,
        batch: RuntimeActionBatch,
        state: PlaybackState,
        coordinator: RuntimeDispatchCoordinator,
    ) -> None:
        _, suppressed = coordinator.request_releases(batch.intents)
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

    def _wait_until_runtime_deadline(
        self,
        target_elapsed_us: int,
        state: PlaybackState,
        first_action_executed: bool,
        total_time_seconds: float,
        last_runtime_poll_us: int,
        last_render_time_us: int,
    ) -> tuple[str | None, int, int]:
        while True:
            elapsed_us = state.get_elapsed_us(self.clock)
            if elapsed_us >= target_elapsed_us:
                return None, last_runtime_poll_us, last_render_time_us

            remaining_us = target_elapsed_us - elapsed_us
            target_system_us = state.start_perf + state.pause_time_us + target_elapsed_us
            if remaining_us <= self.sleep_policy.spin_threshold_us:
                self.precise_sleeper.spin_until_us(target_system_us, self.clock)
                return None, last_runtime_poll_us, last_render_time_us

            now_us = self.clock.now_us()
            if now_us - last_runtime_poll_us >= self._runtime_poll_interval_us:
                last_runtime_poll_us = now_us
                command = self.controls.poll() if self.controls is not None else None
                cmd_res = self._handle_commands(command, state, total_time_seconds)
                if cmd_res:
                    return cmd_res, last_runtime_poll_us, last_render_time_us

                wait_res, wait_cmd = self._process_wait_states(
                    state, first_action_executed, total_time_seconds
                )
                if wait_res:
                    if wait_cmd:
                        return wait_cmd, last_runtime_poll_us, last_render_time_us
                    continue

                elapsed_us = state.get_elapsed_us(self.clock)
                remaining_us = target_elapsed_us - elapsed_us
                if self.renderer and remaining_us >= 5_000:
                    now_render_us = self.clock.now_us()
                    if now_render_us - last_render_time_us >= 33_000:
                        last_render_time_us = now_render_us
                        self.renderer.render(
                            elapsed_us / 1_000_000,
                            total_time_seconds,
                            self.song.name,
                            status="playing",
                        )

            target_system_us = state.start_perf + state.pause_time_us + target_elapsed_us
            self.precise_sleeper.sleep_step_towards_us(
                target_system_us,
                self.clock,
                self.sleeper,
                self.sleep_policy.spin_threshold_us,
            )

    def play(self) -> str:
        # Wait for initial focus if required to prevent "Focus lost" showing immediately at start
        if self.require_focus and not self.focus_guard.is_active():
            self._release_all_and_cancel_runtime()
            if self.renderer:
                self.renderer.render(0.0, 0.001, self.song.name, status="waiting_for_focus", force=True)
            while self.require_focus and not self.focus_guard.is_active():
                command = self.controls.poll() if self.controls is not None else None
                if command == "quit":
                    if self.renderer:
                        self.renderer.finish(f"Stopped: {self.song.name}")
                    return PLAYBACK_QUIT
                if command == "refocus":
                    self.focus_guard.focus()
                if command == "panic":
                    self._release_all_and_cancel_runtime()
                self.sleeper.sleep(self.sleep_policy.poll_s)

        coordinator = RuntimeDispatchCoordinator(self.runtime_schedule, self.min_hold_us)
        self._runtime_coordinator = coordinator
        state = PlaybackState(start_perf=self.clock.now_us())
        
        first_action_executed = False
        last_render_time_us = 0
        last_runtime_poll_us = -self._runtime_poll_interval_us

        total_time_seconds = self.total_time_us / 1_000_000

        # Telemetry diagnostic counters
        late_events_over_2ms = 0
        late_events_over_5ms = 0
        late_events_over_10ms = 0
        max_lateness_us = 0
        def observe_result(exec_result: ExecutionResult | None) -> None:
            nonlocal late_events_over_2ms
            nonlocal late_events_over_5ms
            nonlocal late_events_over_10ms
            nonlocal max_lateness_us
            if exec_result is None:
                return
            lateness_us = exec_result.lateness_us
            if exec_result.is_late and exec_result.runtime_outcome != "deferred_release":
                max_lateness_us = max(max_lateness_us, lateness_us)
                if lateness_us > 2_000:
                    late_events_over_2ms += 1
                if lateness_us > 5_000:
                    late_events_over_5ms += 1
                if exec_result.is_critically_late:
                    late_events_over_10ms += 1
            if (
                self.renderer
                and hasattr(self.renderer, "update_counters")
                and exec_result.runtime_outcome != "deferred_release"
            ):
                self.renderer.update_counters(max(0, lateness_us))

        try:
            while not coordinator.is_finished():
                deadline_us = coordinator.next_deadline_us()
                if deadline_us is None:
                    break
                command_result, last_runtime_poll_us, last_render_time_us = (
                    self._wait_until_runtime_deadline(
                        deadline_us,
                        state,
                        first_action_executed,
                        total_time_seconds,
                        last_runtime_poll_us,
                        last_render_time_us,
                    )
                )
                if command_result:
                    return command_result

                now_us = state.get_elapsed_us(self.clock)

                pending = coordinator.pop_due_pending(now_us)
                if pending:
                    first_action_executed = True
                    observe_result(self._dispatch_pending_releases(pending, state, coordinator))

                for batch in coordinator.pop_due_authored(now_us):
                    first_action_executed = True
                    if batch.kind == "up":
                        self._request_up_batch(batch, state, coordinator)
                        newly_due = coordinator.pop_due_pending(state.get_elapsed_us(self.clock))
                        observe_result(
                            self._dispatch_pending_releases(newly_due, state, coordinator)
                        )
                    else:
                        observe_result(self._dispatch_down_batch(batch, state, coordinator))
                
            if self.renderer:
                self.renderer.render(total_time_seconds, total_time_seconds, self.song.name, status="done", force=True)
                self.renderer.finish(f"Finished playing {self.song.name}")
                
            # Log summary diagnostic metrics
            from sky_music.platform.win32 import inputs
            if hasattr(inputs, "PLAYBACK_DEBUG") and inputs.PLAYBACK_DEBUG:
                inputs.debug_log(
                    f"Timing summary (Microsecond Engine): "
                    f"late events over 2ms={late_events_over_2ms}, "
                    f"late events over 5ms={late_events_over_5ms}, "
                    f"late events over 10ms={late_events_over_10ms}, "
                    f"max lateness={max_lateness_us / 1_000_000:.6f}s"
                )
                
            return PLAYBACK_FINISHED
        except RuntimeSameKeyConflictError:
            if self.renderer:
                self.renderer.finish(
                    f"Stopped: runtime same-key conflict in {self.song.name}"
                )
            return PLAYBACK_QUIT
            
        finally:
            outcome = self._release_all_and_cancel_runtime()
            self.telemetry.record_release_outcome(outcome)
            self.telemetry.record_backend_health(self.backend.get_health())
            self.telemetry.save()
