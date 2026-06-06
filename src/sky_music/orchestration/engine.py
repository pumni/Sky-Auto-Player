from dataclasses import dataclass
from collections import deque
import queue
import threading
import time
from typing import Any, Protocol, Tuple, Optional
from sky_music.domain.domain import Song
from sky_music.domain.scheduler_types import KeyAction, Microseconds, ScanCode
from sky_music.infrastructure.backend import BackendHealth, InputBackend, ReleaseAllOutcome
from sky_music.infrastructure.realtime import (
    MmcssRegistration,
    RealtimeProcessScope,
    create_realtime_sleeper,
)
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


class CommandSource(Protocol):
    def poll(self) -> str | None: ...


class FocusSignal(Protocol):
    def is_active(self) -> bool: ...


class ProgressSink(Protocol):
    def publish(
        self,
        *,
        elapsed_us: int,
        total_us: int,
        status: str,
        lateness_us: int | None = None,
        health: BackendHealth | None = None,
        input_path_degraded: bool = False,
        force: bool = False,
    ) -> None: ...

    def finish(self, message: str) -> None: ...

    def update_counters(self, lateness_us: int) -> None: ...


@dataclass(frozen=True, slots=True)
class DirectCommandSource:
    controls: Any

    def poll(self) -> str | None:
        if self.controls is None:
            return None
        return self.controls.poll()


@dataclass(frozen=True, slots=True)
class DirectFocusSignal:
    engine: "PlaybackEngine"

    def is_active(self) -> bool:
        return self.engine._focus_is_active()


@dataclass(frozen=True, slots=True)
class DirectProgressSink:
    renderer: Any
    song_name: str

    def publish(
        self,
        *,
        elapsed_us: int,
        total_us: int,
        status: str,
        lateness_us: int | None = None,
        health: BackendHealth | None = None,
        input_path_degraded: bool = False,
        force: bool = False,
    ) -> None:
        if self.renderer is None:
            return
        self.renderer.render(
            elapsed_us / 1_000_000,
            total_us / 1_000_000,
            self.song_name,
            status=status,
            force=force,
            input_path_degraded=input_path_degraded,
            backend_health=health,
        )

    def finish(self, message: str) -> None:
        if self.renderer is not None:
            self.renderer.finish(message)

    def update_counters(self, lateness_us: int) -> None:
        if self.renderer is not None and hasattr(self.renderer, "update_counters"):
            self.renderer.update_counters(lateness_us)


@dataclass(frozen=True, slots=True)
class ProgressSnapshot:
    elapsed_us: int
    total_us: int
    status: str
    lateness_us: int | None = None
    health: BackendHealth | None = None
    input_path_degraded: bool = False
    force: bool = False


class QueueCommandSource:
    def __init__(self, commands: "queue.Queue[str]") -> None:
        self._commands = commands

    def poll(self) -> str | None:
        try:
            return self._commands.get_nowait()
        except queue.Empty:
            return None


class SharedFocusSignal:
    def __init__(self, active: bool = True) -> None:
        self._active = active
        self._lock = threading.Lock()

    def set_active(self, active: bool) -> None:
        with self._lock:
            self._active = active

    def is_active(self) -> bool:
        with self._lock:
            return self._active


class SnapshotProgressSink:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._snapshot: ProgressSnapshot | None = None
        self._version = 0
        self._finish_message: str | None = None
        self._counter_updates: deque[int] = deque()

    def publish(
        self,
        *,
        elapsed_us: int,
        total_us: int,
        status: str,
        lateness_us: int | None = None,
        health: BackendHealth | None = None,
        input_path_degraded: bool = False,
        force: bool = False,
    ) -> None:
        with self._lock:
            self._snapshot = ProgressSnapshot(
                elapsed_us=elapsed_us,
                total_us=total_us,
                status=status,
                lateness_us=lateness_us,
                health=health,
                input_path_degraded=input_path_degraded,
                force=force,
            )
            self._version += 1

    def finish(self, message: str) -> None:
        with self._lock:
            self._finish_message = message

    def update_counters(self, lateness_us: int) -> None:
        with self._lock:
            self._counter_updates.append(lateness_us)

    def consume(
        self,
        last_version: int,
    ) -> tuple[int, ProgressSnapshot | None, tuple[int, ...], str | None]:
        with self._lock:
            snapshot = self._snapshot if self._version != last_version else None
            version = self._version
            counters = tuple(self._counter_updates)
            self._counter_updates.clear()
            finish_message = self._finish_message
            self._finish_message = None
        return version, snapshot, counters, finish_message


@dataclass(slots=True)
class DispatchThreadResult:
    result: str | None = None
    error: BaseException | None = None


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


@dataclass(slots=True)
class LoopState:
    last_runtime_poll_us: int
    last_render_time_us: int
    first_action_executed: bool = False


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
        late_pulse_drop_threshold_us: int | None = None,
        use_dispatch_thread: bool = True,
        input_path_warn_us: int = 300,
    ):
        self.song = song
        self.actions = actions
        self.runtime_schedule = compile_runtime_intents(actions)
        self.total_time_us = max((int(action.at_us) for action in actions), default=0)
        self.backend = backend
        self.focus_restore_grace_us = focus_restore_grace_us
        self.min_hold_us = max(0, int(min_hold_us))
        self.same_key_conflict_policy = same_key_conflict_policy
        self.late_pulse_drop_threshold_us = (
            None
            if late_pulse_drop_threshold_us is None
            else max(0, int(late_pulse_drop_threshold_us))
        )
        self.use_dispatch_thread = use_dispatch_thread
        self.input_path_warn_us = max(0, int(input_path_warn_us))
        self._send_duration_window: deque[int] = deque(maxlen=64)
        self._input_path_degraded = False
        self._input_path_warn_started_us: int | None = None
        self._backend_health_snapshot_interval_us = 100_000
        self._backend_health_snapshot_at_us = -self._backend_health_snapshot_interval_us - 1
        self._backend_health_snapshot_value: BackendHealth | None = None
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
        # Sender-warmup instrumentation (observe-only, never affects timing): the start of the
        # busy-spin before the current deadline, and the completion time of the previous send.
        # Together they yield per-send pre_send_spin_us (warm-up the core got) and idle_gap_us
        # (how long the thread slept/idled before that spin — a proxy for CPU coldness).
        self._wait_spin_start_us = 0
        self._last_send_completed_us = 0

    @property
    def input_path_degraded(self) -> bool:
        return self._input_path_degraded

    def _should_use_dispatch_thread(self) -> bool:
        return (
            self.use_dispatch_thread
            and type(self.clock) is PerfCounterClock
            and type(self.sleeper) is RealSleeper
            and self.backend.__class__.__name__ != "DryRunBackend"
        )

    def _release_all_and_cancel_runtime(self) -> ReleaseAllOutcome:
        outcome = self.backend.release_all()
        if self._runtime_coordinator is not None:
            self._runtime_coordinator.cancel_all()
        return outcome

    def _backend_health_snapshot(self, *, force: bool = False) -> BackendHealth:
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

    def _record_input_path_send_duration(
        self,
        send_duration_us: int,
        elapsed_us: int,
    ) -> None:
        if self.input_path_warn_us <= 0:
            return
        self._send_duration_window.append(max(0, int(send_duration_us)))
        sorted_window = sorted(self._send_duration_window)
        p95_idx = int(round(0.95 * (len(sorted_window) - 1)))
        p95_us = sorted_window[p95_idx]
        if p95_us <= self.input_path_warn_us:
            self._input_path_warn_started_us = None
            return
        if self._input_path_warn_started_us is None:
            self._input_path_warn_started_us = elapsed_us
            return
        if elapsed_us - self._input_path_warn_started_us >= 1_000_000:
            self._input_path_degraded = True

    def _handle_commands(
        self,
        command: Optional[str],
        state: PlaybackState,
        total_time_us: int,
        progress_sink: ProgressSink,
    ) -> Optional[str]:
        """Handles playback commands like pause, skip, quit, etc."""
        if command == "quit":
            progress_sink.finish(f"Stopped: {self.song.name}")
            return PLAYBACK_QUIT
        if command == "skip":
            progress_sink.finish(f"Skipped: {self.song.name}")
            return PLAYBACK_SKIPPED
        if command == "refocus":
            self.focus_guard.focus()
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status="refocus",
                health=self._backend_health_snapshot(force=True),
                input_path_degraded=self._input_path_degraded,
                force=True,
            )
        if command == "panic":
            self._release_all_and_cancel_runtime()
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status="panic",
                health=self._backend_health_snapshot(force=True),
                input_path_degraded=self._input_path_degraded,
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
                    health=self._backend_health_snapshot(force=True),
                    input_path_degraded=self._input_path_degraded,
                    force=True,
                )
            else:
                pause_duration_us = self.clock.now_us() - state.manual_pause_started_us
                state.pause_time_us += pause_duration_us
                self.telemetry.record_pause("manual", pause_duration_us)
                state.manual_pause_started_us = None
                progress_sink.publish(
                    elapsed_us=state.get_elapsed_us(self.clock),
                    total_us=total_time_us,
                    status="playing",
                    health=self._backend_health_snapshot(force=True),
                    input_path_degraded=self._input_path_degraded,
                    force=True,
                )
        return None

    def _focus_is_active(self) -> bool:
        """Return memoised focus state, refreshing the heavy Win32 check after its short TTL."""
        now_us = self.clock.now_us()
        if now_us - self._focus_cache_at_us >= self._focus_cache_ttl_us:
            self._focus_active_cache = self.focus_guard.is_active()
            self._focus_cache_at_us = now_us
        return self._focus_active_cache

    def _process_wait_states(
        self,
        state: PlaybackState,
        first_action_executed: bool,
        total_time_us: int | float,
        command_source: CommandSource | None = None,
        focus_signal: FocusSignal | None = None,
        progress_sink: ProgressSink | None = None,
    ) -> Tuple[bool, Optional[str]]:
        """Handles focus lost and manual pause wait states."""
        resolved_total_time_us = (
            int(total_time_us * 1_000_000)
            if isinstance(total_time_us, float)
            else total_time_us
        )
        command_source = command_source or DirectCommandSource(self.controls)
        focus_signal = focus_signal or DirectFocusSignal(self)
        progress_sink = progress_sink or DirectProgressSink(self.renderer, self.song.name)

        if state.manual_pause_started_us is not None:
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=resolved_total_time_us,
                status="paused",
                health=self._backend_health_snapshot(force=True),
                input_path_degraded=self._input_path_degraded,
            )
            self.sleeper.sleep(self.sleep_policy.poll_s)
            return True, None

        if self.require_focus and not focus_signal.is_active():
            if state.focus_pause_started_us is None:
                self._release_all_and_cancel_runtime()
                state.focus_pause_started_us = self.clock.now_us()
            status_val = "waiting_for_focus" if not first_action_executed else "focus_lost"
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=resolved_total_time_us,
                status=status_val,
                health=self._backend_health_snapshot(force=True),
                input_path_degraded=self._input_path_degraded,
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
                    resolved_total_time_us,
                    progress_sink,
                )
                if cmd_res:
                    return True, cmd_res
                if early_cmd in ("pause", "panic"):
                    break

            pause_duration_us = self.clock.now_us() - state.focus_pause_started_us
            state.pause_time_us += pause_duration_us
            self.telemetry.record_pause("focus", pause_duration_us)
            state.focus_pause_started_us = None
            status = "paused" if state.manual_pause_started_us is not None else "playing"
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=resolved_total_time_us,
                status=status,
                health=self._backend_health_snapshot(force=True),
                input_path_degraded=self._input_path_degraded,
                force=True,
            )
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
        self._record_input_path_send_duration(send_duration_us, send_end_us)

        # Sender-warmup instrumentation (pure arithmetic on already-captured timestamps):
        #   pre_send_spin_us = how long the core busy-spun right before this send (warm-up window)
        #   idle_gap_us      = how long the thread idled/slept before that spin (CPU coldness proxy)
        pre_send_spin_us = max(0, send_start_us - self._wait_spin_start_us)
        idle_gap_us = max(0, self._wait_spin_start_us - self._last_send_completed_us)
        self._last_send_completed_us = send_end_us

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
            pre_send_spin_us=pre_send_spin_us,
            idle_gap_us=idle_gap_us,
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
        if self.late_pulse_drop_threshold_us is not None:
            now_us = state.get_elapsed_us(self.clock)
            if now_us - batch.scheduled_us > self.late_pulse_drop_threshold_us:
                coordinator.drop_expired_downs(batch.intents)
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
            max(
                release.down_dispatch_started_us + self.min_hold_us - release.scheduled_release_us
                for release in releases
            ),
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
        loop_state: LoopState,
        total_time_us: int,
        command_source: CommandSource,
        focus_signal: FocusSignal,
        progress_sink: ProgressSink,
    ) -> str | None:
        while True:
            elapsed_us = state.get_elapsed_us(self.clock)
            if elapsed_us >= target_elapsed_us:
                # Already due (we were late): no warm-up spin happened before the send.
                self._wait_spin_start_us = elapsed_us
                return None

            remaining_us = target_elapsed_us - elapsed_us
            target_system_us = state.start_perf + state.pause_time_us + target_elapsed_us
            if remaining_us <= self.sleep_policy.spin_threshold_us:
                # Mark when the final busy-spin began; _execute_action uses it to report how long
                # the core was spinning (warm-up) before SendInput, and how long it idled before that.
                self._wait_spin_start_us = elapsed_us
                self.precise_sleeper.spin_until_us(target_system_us, self.clock)
                return None

            now_us = self.clock.now_us()
            if now_us - loop_state.last_runtime_poll_us >= self._runtime_poll_interval_us:
                loop_state.last_runtime_poll_us = now_us
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
                    loop_state.first_action_executed,
                    total_time_us,
                    command_source,
                    focus_signal,
                    progress_sink,
                )
                if wait_res:
                    if wait_cmd:
                        return wait_cmd
                    continue

                elapsed_us = state.get_elapsed_us(self.clock)
                remaining_us = target_elapsed_us - elapsed_us
                if remaining_us >= 5_000:
                    now_render_us = self.clock.now_us()
                    if now_render_us - loop_state.last_render_time_us >= 33_000:
                        loop_state.last_render_time_us = now_render_us
                        progress_sink.publish(
                            elapsed_us=elapsed_us,
                            total_us=total_time_us,
                            status="playing",
                            health=self._backend_health_snapshot(),
                            input_path_degraded=self._input_path_degraded,
                        )

            target_system_us = state.start_perf + state.pause_time_us + target_elapsed_us
            self.precise_sleeper.sleep_step_towards_us(
                target_system_us,
                self.clock,
                self.sleeper,
                self.sleep_policy.spin_threshold_us,
            )

    def _drain_due(
        self,
        now_us: int,
        state: PlaybackState,
        coordinator: RuntimeDispatchCoordinator,
        loop_state: LoopState,
    ) -> tuple[ExecutionResult | None, ...]:
        results: list[ExecutionResult | None] = []

        pending = coordinator.pop_due_pending(now_us)
        if pending:
            loop_state.first_action_executed = True
            results.append(self._dispatch_pending_releases(pending, state, coordinator))

        # Focus is intentionally checked in the wait/poll phase, not between every due batch here.
        # A mid-burst focus loss is cleaned up on the next poll via release_all(); keeping this hot
        # path free of extra Win32 focus calls preserves dispatch timing for dense due bursts.
        for batch in coordinator.pop_due_authored(now_us):
            loop_state.first_action_executed = True
            if batch.kind == "up":
                self._request_up_batch(batch, state, coordinator)
                newly_due = coordinator.pop_due_pending(state.get_elapsed_us(self.clock))
                results.append(
                    self._dispatch_pending_releases(newly_due, state, coordinator)
                )
            else:
                results.append(self._dispatch_down_batch(batch, state, coordinator))

        return tuple(results)

    def _log_timing_summary(self) -> None:
        from sky_music.platform.win32 import inputs

        if not (hasattr(inputs, "PLAYBACK_DEBUG") and inputs.PLAYBACK_DEBUG):
            return

        summary = self.telemetry.get_summary() or {}
        lateness = summary.get("lateness_us", {})
        inputs.debug_log(
            f"Timing summary (Microsecond Engine): "
            f"late events over 2ms={lateness.get('over_2ms', 0)}, "
            f"late events over 5ms={lateness.get('over_5ms', 0)}, "
            f"late events over 10ms={lateness.get('over_10ms', 0)}, "
            f"max lateness={lateness.get('max_us', 0.0) / 1_000_000:.6f}s"
        )

    def _run_dispatch(
        self,
        coordinator: RuntimeDispatchCoordinator,
        state: PlaybackState,
        *,
        command_source: CommandSource,
        focus_signal: FocusSignal,
        progress_sink: ProgressSink,
    ) -> str:
        loop_state = LoopState(
            last_runtime_poll_us=-self._runtime_poll_interval_us,
            last_render_time_us=0,
        )
        total_time_us = self.total_time_us

        def observe_result(exec_result: ExecutionResult | None) -> None:
            if exec_result is None:
                return
            if exec_result.runtime_outcome != "deferred_release":
                progress_sink.update_counters(max(0, exec_result.lateness_us))

        try:
            while not coordinator.is_finished():
                deadline_us = coordinator.next_deadline_us()
                if deadline_us is None:
                    break
                command_result = self._wait_until_runtime_deadline(
                    deadline_us,
                    state,
                    loop_state,
                    total_time_us,
                    command_source,
                    focus_signal,
                    progress_sink,
                )
                if command_result:
                    return command_result

                now_us = state.get_elapsed_us(self.clock)
                for result in self._drain_due(now_us, state, coordinator, loop_state):
                    observe_result(result)

            progress_sink.publish(
                elapsed_us=total_time_us,
                total_us=total_time_us,
                status="done",
                health=self._backend_health_snapshot(force=True),
                input_path_degraded=self._input_path_degraded,
                force=True,
            )
            progress_sink.finish(f"Finished playing {self.song.name}")

            self._log_timing_summary()

            return PLAYBACK_FINISHED
        except RuntimeSameKeyConflictError:
            progress_sink.finish(
                f"Stopped: runtime same-key conflict in {self.song.name}"
            )
            return PLAYBACK_QUIT
        finally:
            outcome = self._release_all_and_cancel_runtime()
            if self._runtime_coordinator is not None:
                self.telemetry.record_generation_status_counts(
                    self._runtime_coordinator.generation_status_counts()
                )
            self.telemetry.record_release_outcome(outcome)
            self.telemetry.record_backend_health(self.backend.get_health())
            self.telemetry.record_input_path_health(
                degraded=self._input_path_degraded,
                warn_us=self.input_path_warn_us,
            )
            self.telemetry.save()

    def _render_progress_snapshot(self, snapshot: ProgressSnapshot) -> None:
        if self.renderer is None:
            return
        self.renderer.render(
            snapshot.elapsed_us / 1_000_000,
            snapshot.total_us / 1_000_000,
            self.song.name,
            status=snapshot.status,
            force=snapshot.force,
            input_path_degraded=snapshot.input_path_degraded,
            backend_health=snapshot.health,
        )

    def _consume_progress_updates(
        self,
        progress_sink: SnapshotProgressSink,
        last_version: int,
    ) -> int:
        version, snapshot, counters, finish_message = progress_sink.consume(last_version)
        if self.renderer is not None and hasattr(self.renderer, "update_counters"):
            for lateness_us in counters:
                self.renderer.update_counters(lateness_us)
        if snapshot is not None:
            self._render_progress_snapshot(snapshot)
        if finish_message is not None and self.renderer is not None:
            self.renderer.finish(finish_message)
        return version

    def _run_threaded_dispatch(
        self,
        coordinator: RuntimeDispatchCoordinator,
        state: PlaybackState,
    ) -> str:
        command_queue: queue.Queue[str] = queue.Queue()
        command_source = QueueCommandSource(command_queue)
        focus_signal = SharedFocusSignal(True)
        progress_sink = SnapshotProgressSink()
        dispatch_result = DispatchThreadResult()

        def dispatch_target() -> None:
            from sky_music.platform.win32 import inputs

            original_sleeper = self.sleeper
            realtime_sleeper = create_realtime_sleeper(original_sleeper)
            self.sleeper = realtime_sleeper
            try:
                # high_resolution_timer_scope() is the defensive 1ms-timer safety net for the
                # RealSleeper fallback; MMCSS raises the dispatch thread's scheduling class. Both are
                # re-asserted per run so no volatile session state can leave dispatch coarse.
                with inputs.high_resolution_timer_scope(), MmcssRegistration():
                    dispatch_result.result = self._run_dispatch(
                        coordinator,
                        state,
                        command_source=command_source,
                        focus_signal=focus_signal,
                        progress_sink=progress_sink,
                    )
            except BaseException as exc:
                dispatch_result.error = exc
            finally:
                self.sleeper = original_sleeper
                if realtime_sleeper is not original_sleeper:
                    close = getattr(realtime_sleeper, "close", None)
                    if close is not None:
                        close()

        dispatch_thread = threading.Thread(
            target=dispatch_target,
            name="sky-music-dispatch",
        )
        dispatch_thread.start()

        last_snapshot_version = 0
        next_control_poll_s = 0.0
        next_focus_check_s = 0.0
        control_poll_s = min(max(self.sleep_policy.poll_s, 0.005), 0.010)
        focus_poll_s = min(max(self.sleep_policy.poll_s, 0.010), 0.025)
        control_sleep_s = min(control_poll_s, 0.005)

        while dispatch_thread.is_alive():
            now_s = time.perf_counter()
            if now_s >= next_control_poll_s:
                command = self.controls.poll() if self.controls is not None else None
                if command in ("pause", "skip", "quit", "panic"):
                    command_queue.put(command)
                elif command == "refocus":
                    self.focus_guard.focus()
                next_control_poll_s = now_s + control_poll_s

            if now_s >= next_focus_check_s:
                focus_signal.set_active(
                    True if not self.require_focus else self.focus_guard.is_active()
                )
                next_focus_check_s = now_s + focus_poll_s

            last_snapshot_version = self._consume_progress_updates(
                progress_sink,
                last_snapshot_version,
            )
            time.sleep(control_sleep_s)

        dispatch_thread.join()
        last_snapshot_version = self._consume_progress_updates(
            progress_sink,
            last_snapshot_version,
        )

        if dispatch_result.error is not None:
            raise dispatch_result.error
        return dispatch_result.result or PLAYBACK_FINISHED

    def play(self) -> str:
        # Re-resolve the live game window per run so a HWND that went stale during the session
        # (game restarted / window re-created / focus juggled) cannot carry a bad target into this
        # playback — the volatile-state fault class that otherwise only clears on a player restart.
        if self.require_focus:
            try:
                from sky_music.platform.win32 import inputs

                inputs.reset_window_cache()
                if getattr(inputs, "PLAYBACK_DEBUG", False):
                    inputs.debug_log(
                        f"[play] start {self.song.name!r}: {inputs.describe_input_target()}, "
                        f"min_hold_us={self.min_hold_us}"
                    )
            except Exception:
                pass

        # Verify no picker-phase worker thread survived into playback.  The picker scope is closed
        # with wait=True before this point; this is the runtime backstop for that contract, catching
        # drift the compile-time static guard cannot (e.g. a future feature spawning a worker).
        self._record_thread_census()

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

        # Pause cyclic GC for the whole dispatch so a mid-send GC pause cannot stall the precise
        # SendInput loop (thread scheduling is handled by MMCSS, not a process priority bump).
        # Re-enabled on exit.  The schedule's perf anchor (start_perf) is captured INSIDE the scope,
        # after the one-shot gc.collect(), so pre-playback collection cannot delay the dispatch
        # thread start and compress the first onsets.
        with RealtimeProcessScope():
            state = PlaybackState(start_perf=self.clock.now_us())
            if self._should_use_dispatch_thread():
                return self._run_threaded_dispatch(coordinator, state)

            command_source = DirectCommandSource(self.controls)
            focus_signal = DirectFocusSignal(self)
            progress_sink = DirectProgressSink(self.renderer, self.song.name)

            return self._run_dispatch(
                coordinator,
                state,
                command_source=command_source,
                focus_signal=focus_signal,
                progress_sink=progress_sink,
            )

    # Picker workers use these thread-name prefixes; none may be alive once playback starts.
    _PICKER_WORKER_THREAD_PREFIXES = (
        "sky-metadata-coord",
        "sky-picker-meta",
        "sky-picker-cache",
    )

    def _record_thread_census(self) -> str | None:
        """Log/record any picker-phase worker threads still alive at playback start.

        Returns the comma-joined offending thread names (or None when clean).  Records structured
        evidence on ``TelemetryLogger`` alongside the picker-cleanup snapshot.  Diagnostic only:
        a leak is surfaced loudly but does not abort an otherwise-ready playback.
        """
        leaked = [
            t.name
            for t in threading.enumerate()
            if t.is_alive()
            and any(t.name.startswith(prefix) for prefix in self._PICKER_WORKER_THREAD_PREFIXES)
        ]
        from sky_music.orchestration.telemetry import TelemetryLogger

        TelemetryLogger.last_thread_census = {
            "clean": not leaked,
            "leaked_threads": leaked,
        }
        if leaked:
            from sky_music.platform.win32 import inputs

            inputs.debug_log(
                f"[background] WARNING picker worker thread(s) still alive at [play] start: {leaked}"
            )
            return ", ".join(leaked)
        return None
