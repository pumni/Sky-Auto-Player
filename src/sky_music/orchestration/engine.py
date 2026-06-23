from __future__ import annotations

import threading
from typing import Optional, Tuple

from sky_music.config import RtPriorityMode
from sky_music.domain.domain import Song
from sky_music.domain.scheduler_types import KeyAction
from sky_music.infrastructure.backend import InputBackend, ReleaseAllOutcome
from sky_music.infrastructure.focus import FocusGuard, NoopFocusGuard, Win32SkyFocusGuard
from sky_music.infrastructure.realtime import (
    RealtimeProcessScope,
    _gil_enabled,
    create_realtime_sleeper,
)
from sky_music.infrastructure.timing import Clock, PerfCounterClock, RealSleeper, Sleeper, SleepPolicy
from sky_music.infrastructure.wait_strategy import HybridWaitStrategy, WaitStrategy
from sky_music.orchestration.runtime_dispatch import (
    RuntimeDispatchCoordinator,
    compile_runtime_intents,
)
from sky_music.orchestration.telemetry import TelemetryLogger

# Re-exports kept for compatibility: the decomposition (Phase 6) moved these out of engine.py but
# callers and tests still import them from here.
from sky_music.orchestration.dispatch_loop import (
    DispatchHealthMonitor as DispatchHealthMonitor,
    DispatchLoop as DispatchLoop,
    ExecutionResult as ExecutionResult,
    PlaybackState as PlaybackState,
    RuntimeSameKeyConflictError as RuntimeSameKeyConflictError,
)
from sky_music.orchestration.playback_supervisor import (
    DirectCommandSource as DirectCommandSource,
    DirectFocusSignal as DirectFocusSignal,
    DirectProgressSink as DirectProgressSink,
    PlaybackSupervisor as PlaybackSupervisor,
    PLAYBACK_FINISHED as PLAYBACK_FINISHED,
    PLAYBACK_SKIPPED as PLAYBACK_SKIPPED,
    PLAYBACK_QUIT as PLAYBACK_QUIT,
)


class SendLatencyEstimator:
    """Per-kind EMA of SendInput durations used to derive the adaptive dispatch lead.

    The first N samples of each kind yield lead 0 (cold estimates are worse than nothing); the
    Nth sample seeds the EMA with the average of all warm-up samples.
    """

    _SEED_SAMPLES = 5

    __slots__ = (
        "_ema_down",
        "_ema_up",
        "_count_down",
        "_count_up",
        "_sum_down",
        "_sum_up",
        "_alpha",
        "_max_lead_us",
    )

    def __init__(self, alpha: float = 0.2, max_lead_us: int = 2_000) -> None:
        self._ema_down: float = 0.0
        self._ema_up: float = 0.0
        self._count_down: int = 0
        self._count_up: int = 0
        self._sum_down: int = 0
        self._sum_up: int = 0
        self._alpha: float = alpha
        self._max_lead_us: int = max_lead_us

    def update(self, kind: str, duration_us: int) -> None:
        if kind == "down":
            self._count_down += 1
            if self._count_down <= self._SEED_SAMPLES:
                self._sum_down += duration_us
                if self._count_down == self._SEED_SAMPLES:
                    self._ema_down = self._sum_down / self._SEED_SAMPLES
            else:
                self._ema_down = self._alpha * duration_us + (1.0 - self._alpha) * self._ema_down
        elif kind == "up":
            self._count_up += 1
            if self._count_up <= self._SEED_SAMPLES:
                self._sum_up += duration_us
                if self._count_up == self._SEED_SAMPLES:
                    self._ema_up = self._sum_up / self._SEED_SAMPLES
            else:
                self._ema_up = self._alpha * duration_us + (1.0 - self._alpha) * self._ema_up

    def get_lead_us(self, kind: str) -> int:
        if kind == "down":
            if self._count_down < self._SEED_SAMPLES:
                return 0
            return max(0, min(self._max_lead_us, round(self._ema_down)))
        if kind == "up":
            if self._count_up < self._SEED_SAMPLES:
                return 0
            return max(0, min(self._max_lead_us, round(self._ema_up)))
        return 0


class PlaybackEngine:
    """Facade wiring schedule compilation, realtime context, DispatchLoop, and PlaybackSupervisor."""

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
        enable_timer_guard: bool = True,
        enable_waitable_timer: bool = True,
        enable_gc_pause: bool = True,
        enable_switch_interval_tuning: bool = True,
        enable_adaptive_lead: bool = False,
        enable_adaptive_spin: bool = False,
        rt_priority_mode: RtPriorityMode = "auto",
        dispatch_lead_us: int = 0,
        enable_event_wait: bool = False,
        enable_epoch_rebase: bool = False,
        wait_strategy: Optional[WaitStrategy] = None,
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
        self.enable_timer_guard = bool(enable_timer_guard)
        self.enable_waitable_timer = bool(enable_waitable_timer)
        self.enable_gc_pause = bool(enable_gc_pause)
        self.enable_switch_interval_tuning = bool(enable_switch_interval_tuning)
        self.enable_adaptive_lead = bool(enable_adaptive_lead)
        self.enable_adaptive_spin = bool(enable_adaptive_spin)
        self.estimator = SendLatencyEstimator()
        self.rt_priority_mode: RtPriorityMode = rt_priority_mode
        self.dispatch_lead_us = max(0, int(dispatch_lead_us))
        self.enable_event_wait = bool(enable_event_wait)
        self.enable_epoch_rebase = bool(enable_epoch_rebase)
        # Test seam: deterministic tests inject a strategy whose spin advances their fake clock.
        self._wait_strategy: WaitStrategy = (
            wait_strategy
            if wait_strategy is not None
            else HybridWaitStrategy(enable_event_wait=self.enable_event_wait)
        )
        self.effective_spin_threshold_us: int | None = None
        self._input_path_degraded = False
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
        self.telemetry.record_runtime_options(
            {
                "use_dispatch_thread": self.use_dispatch_thread,
                "timer_guard": self.enable_timer_guard,
                "waitable_timer": self.enable_waitable_timer,
                "gc_pause": self.enable_gc_pause,
                "switch_interval_tuning": self.enable_switch_interval_tuning,
                "gil_enabled": _gil_enabled(),
                "adaptive_lead": self.enable_adaptive_lead,
                "rt_priority_mode": self.rt_priority_mode,
                "enable_event_wait": self.enable_event_wait,
                "epoch_rebase": self.enable_epoch_rebase,
            }
        )
        self.require_focus = require_focus
        self.clock = clock if clock is not None else PerfCounterClock()
        self.sleeper = sleeper if sleeper is not None else RealSleeper()
        self.sleep_policy = sleep_policy

        # Inject standard FocusGuard depending on requirements
        if focus_guard is None:
            if self.require_focus:
                self.focus_guard: FocusGuard = Win32SkyFocusGuard()
            else:
                self.focus_guard = NoopFocusGuard()
        else:
            self.focus_guard = focus_guard

        self._runtime_coordinator: RuntimeDispatchCoordinator | None = None
        self._health_monitor = DispatchHealthMonitor(
            backend=self.backend,
            clock=self.clock,
            focus_guard=self.focus_guard,
            require_focus=self.require_focus,
            input_path_warn_us=self.input_path_warn_us,
        )
        # Compatibility shim for legacy engine-level tests (_execute_action/_process_wait_states):
        # one cached loop so dispatch ids keep incrementing across calls. Lazily built.
        self._compat_loop: DispatchLoop | None = None

    @property
    def input_path_degraded(self) -> bool:
        return self._input_path_degraded

    @property
    def _focus_cache_ttl_us(self) -> int:
        return self._health_monitor._focus_cache_ttl_us

    @property
    def current_spin_threshold_us(self) -> int:
        if self.enable_adaptive_spin and self.effective_spin_threshold_us is not None:
            return self.effective_spin_threshold_us
        return self.sleep_policy.spin_threshold_us

    def _should_use_dispatch_thread(self) -> bool:
        return (
            self.use_dispatch_thread
            and isinstance(self.clock, PerfCounterClock)
            and isinstance(self.sleeper, RealSleeper)
            and self.backend.__class__.__name__ != "DryRunBackend"
        )

    def _release_all_and_cancel_runtime(self) -> ReleaseAllOutcome:
        outcome = self.backend.release_all()
        if self._runtime_coordinator is not None:
            self._runtime_coordinator.cancel_all()
        return outcome

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

    def _build_dispatch_loop(
        self,
        coordinator: RuntimeDispatchCoordinator,
        sleeper: Sleeper,
    ) -> DispatchLoop:
        return DispatchLoop(
            coordinator=coordinator,
            clock=self.clock,
            sleeper=sleeper,
            wait_strategy=self._wait_strategy,
            backend=self.backend,
            telemetry=self.telemetry,
            sleep_policy=self.sleep_policy,
            health_monitor=self._health_monitor,
            min_hold_us=self.min_hold_us,
            spin_threshold_us=self.current_spin_threshold_us,
            focus_restore_grace_us=self.focus_restore_grace_us,
            late_pulse_drop_threshold_us=self.late_pulse_drop_threshold_us,
            same_key_conflict_policy=self.same_key_conflict_policy,
            enable_adaptive_lead=self.enable_adaptive_lead,
            enable_event_wait=self.enable_event_wait,
            dispatch_lead_us=self.dispatch_lead_us,
            estimator=self.estimator,
        )

    def _probe_timer_wake_error(self, sleeper: Sleeper) -> None:
        """Measure this machine's sleeper wake error and derive the effective spin threshold.

        Runs strictly BEFORE the playback perf anchor (start_perf) is captured, like gc.collect():
        nothing may delay the dispatch start after the anchor or the first onsets compress.
        """
        wake_errors: list[int] = []
        for _ in range(10):
            t0 = self.clock.now_us()
            sleeper.sleep(0.002)
            t1 = self.clock.now_us()
            wake_errors.append((t1 - t0) - 2_000)

        p_max = max(wake_errors)
        self.effective_spin_threshold_us = max(300, min(3_000, p_max + 200))

        self.telemetry.record_runtime_options(
            {
                **self.telemetry.runtime_options,
                "probe_wake_errors_us": wake_errors,
                "effective_spin_threshold_us": self.effective_spin_threshold_us,
                "enable_adaptive_spin": True,
            }
        )

    def play(self) -> str:
        # Re-resolve the live game window per run
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

        use_dispatch_thread = self._should_use_dispatch_thread()
        realtime_sleeper = (
            create_realtime_sleeper(self.sleeper)
            if (self.enable_waitable_timer and use_dispatch_thread)
            else self.sleeper
        )

        try:
            if self.enable_adaptive_spin:
                self._probe_timer_wake_error(realtime_sleeper)
            else:
                self.telemetry.record_runtime_options(
                    {
                        **self.telemetry.runtime_options,
                        "enable_adaptive_spin": False,
                    }
                )

            with RealtimeProcessScope(
                enabled=self.enable_gc_pause,
                enable_switch_interval_tuning=self.enable_switch_interval_tuning,
            ):
                state = PlaybackState(start_perf=self.clock.now_us())

                dispatch_loop = self._build_dispatch_loop(coordinator, realtime_sleeper)

                supervisor = PlaybackSupervisor(
                    controls=self.controls,
                    focus_guard=self.focus_guard,
                    require_focus=self.require_focus,
                    renderer=self.renderer,
                    telemetry=self.telemetry,
                    sleep_policy=self.sleep_policy,
                    clock=self.clock,
                    sleeper=self.sleeper,
                    song_name=self.song.name,
                    rt_priority_mode=self.rt_priority_mode,
                    enable_timer_guard=self.enable_timer_guard,
                    enable_event_wait=self.enable_event_wait,
                    enable_epoch_rebase=self.enable_epoch_rebase,
                )

                result = supervisor.run(
                    dispatch_loop=dispatch_loop,
                    coordinator=coordinator,
                    state=state,
                    total_time_us=self.total_time_us,
                    use_dispatch_thread=use_dispatch_thread,
                )
                if result == PLAYBACK_FINISHED:
                    self._log_timing_summary()
                return result
        finally:
            self._input_path_degraded = self._health_monitor.input_path_degraded
            if realtime_sleeper is not self.sleeper:
                close = getattr(realtime_sleeper, "close", None)
                if close is not None:
                    close()

    # Picker workers use these thread-name prefixes; none may be alive once playback starts.
    _PICKER_WORKER_THREAD_PREFIXES = (
        "sky-metadata-coord",
        "sky-picker-meta",
        "sky-picker-cache",
    )

    def _record_thread_census(self) -> str | None:
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

    def _focus_is_active(self) -> bool:
        """Return memoised focus state, refreshing the heavy Win32 check after its short TTL."""
        return self._health_monitor.focus_is_active()

    # ------------------------------------------------------------------
    # Legacy compatibility shims: pre-decomposition tests drive these engine-level entry points.
    # They share one cached DispatchLoop so dispatch ids stay monotonic across calls.
    # ------------------------------------------------------------------

    def _compat_dispatch_loop(self) -> DispatchLoop:
        if self._compat_loop is None:
            coordinator = RuntimeDispatchCoordinator(self.runtime_schedule, self.min_hold_us)
            self._compat_loop = self._build_dispatch_loop(coordinator, self.sleeper)
        return self._compat_loop

    def _process_wait_states(
        self,
        state: PlaybackState,
        first_action_executed: bool,
        total_time_us: int | float,
        command_source = None,
        focus_signal = None,
        progress_sink = None,
    ) -> Tuple[bool, Optional[str]]:
        resolved_total_time_us = (
            int(total_time_us * 1_000_000)
            if isinstance(total_time_us, float)
            else total_time_us
        )
        command_source = command_source or DirectCommandSource(self.controls)
        focus_signal = focus_signal or DirectFocusSignal(self._focus_is_active)
        progress_sink = progress_sink or DirectProgressSink(self.renderer, self.song.name)

        return self._compat_dispatch_loop()._process_wait_states(
            state=state,
            first_action_executed=first_action_executed,
            total_time_us=resolved_total_time_us,
            command_source=command_source,
            focus_signal=focus_signal,
            progress_sink=progress_sink,
        )

    def _execute_action(
        self,
        idx: int,
        action: KeyAction,
        state: PlaybackState,
        *,
        generation_ids: tuple[int, ...] = (),
        runtime_outcome: str = "sent",
        deferred_by_us: int = 0,
        applied_lead_us: int = 0,
    ) -> ExecutionResult:
        return self._compat_dispatch_loop()._execute_action(
            idx=idx,
            action=action,
            state=state,
            generation_ids=generation_ids,
            runtime_outcome=runtime_outcome,
            deferred_by_us=deferred_by_us,
            applied_lead_us=applied_lead_us,
        )
