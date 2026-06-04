from dataclasses import dataclass
from typing import Tuple, Optional, Any
from sky_music.domain.domain import Song
from sky_music.domain.scheduler_types import KeyAction
from sky_music.infrastructure.backend import InputBackend
from sky_music.orchestration.telemetry import TelemetryLogger
from sky_music.infrastructure.timing import Clock, Sleeper, PerfCounterClock, RealSleeper, SleepPolicy, PreciseSleeper
from sky_music.infrastructure.focus import FocusGuard, NoopFocusGuard, Win32SkyFocusGuard

# We use standard outputs from UI and main
PLAYBACK_FINISHED = "finished"
PLAYBACK_SKIPPED = "skipped"
PLAYBACK_QUIT = "quit"


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


@dataclass
class PlaybackConfig:
    """Configuration and optional dependencies for PlaybackEngine."""
    telemetry_enabled: bool = False
    require_focus: bool = True
    clock: Optional[Clock] = None
    sleeper: Optional[Sleeper] = None
    sleep_policy: SleepPolicy = SleepPolicy()
    focus_guard: Optional[FocusGuard] = None
    profile_name: str = "balanced"
    tempo_scale: float = 1.0
    focus_restore_grace_us: int = 100_000
    fps: Optional[int] = None
    controls: Any = None
    renderer: Any = None


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
        fps: Optional[int] = None
    ):
        self.song = song
        self.actions = actions
        self.backend = backend
        
        # Internal configuration from parameters
        self.config = PlaybackConfig(
            telemetry_enabled=telemetry_enabled,
            require_focus=require_focus,
            clock=clock,
            sleeper=sleeper,
            sleep_policy=sleep_policy,
            focus_guard=focus_guard,
            profile_name=profile_name,
            tempo_scale=tempo_scale,
            focus_restore_grace_us=focus_restore_grace_us,
            fps=fps,
            controls=controls,
            renderer=renderer
        )
        
        self.focus_restore_grace_us = self.config.focus_restore_grace_us
        self.controls = self.config.controls
        self.renderer = self.config.renderer
        self.telemetry = TelemetryLogger(
            song.name,
            enabled=self.config.telemetry_enabled,
            profile_name=self.config.profile_name,
            tempo_scale=self.config.tempo_scale,
            fps=self.config.fps
        )
        self.require_focus = self.config.require_focus
        self.clock = self.config.clock if self.config.clock is not None else PerfCounterClock()
        self.sleeper = self.config.sleeper if self.config.sleeper is not None else RealSleeper()
        self.sleep_policy = self.config.sleep_policy
        self.precise_sleeper = PreciseSleeper()
        if self.renderer is not None:
            self.renderer.backend = self.backend
        
        # Inject standard FocusGuard depending on requirements
        if self.config.focus_guard is None:
            if self.require_focus:
                self.focus_guard: FocusGuard = Win32SkyFocusGuard()
            else:
                self.focus_guard = NoopFocusGuard()
        else:
            self.focus_guard = self.config.focus_guard

        # Focus-check cache. is_active() is a heavy Win32 chain (GetForegroundWindow +
        # OpenProcess/QueryFullProcessImageName/CloseHandle for process validation). The playback
        # loop polls it once per iteration, and the final spin phase before each event iterates
        # every few microseconds — re-running that chain hundreds of times right inside the timing-
        # critical window burns CPU and adds jitter. Focus state does not need microsecond freshness
        # (the OS itself changes foreground on a far coarser scale, and pause polling already runs at
        # poll_s ~ 25 ms), so we memoise it for a short TTL. This collapses the per-event spin burst
        # to at most one real check per TTL while still pausing within a couple of milliseconds of an
        # alt-tab.
        self._focus_cache_ttl_us: int = 2_000
        self._focus_active_cache: bool = True
        self._focus_cache_at_us: int = -self._focus_cache_ttl_us - 1

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
            self.backend.release_all()
            if self.renderer:
                self.renderer.render(state.get_elapsed_us(self.clock) / 1_000_000, total_time_seconds, self.song.name, status="panic", force=True)
        if command == "pause":
            if state.manual_pause_started_us is None:
                self.backend.release_all()
                state.manual_pause_started_us = self.clock.now_us()
                if self.renderer:
                    self.renderer.render(state.get_elapsed_us(self.clock) / 1_000_000, total_time_seconds, self.song.name, status="paused", force=True)
            else:
                state.pause_time_us += (self.clock.now_us() - state.manual_pause_started_us)
                state.manual_pause_started_us = None
                if self.renderer:
                    self.renderer.render(state.get_elapsed_us(self.clock) / 1_000_000, total_time_seconds, self.song.name, status="playing", force=True)
        return None

    def _focus_is_active(self) -> bool:
        """Focus state with short-TTL memoisation to keep the heavy is_active() call out of the
        hot spin loop. Returns the cached value unless the TTL has elapsed."""
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
                self.backend.release_all()
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

            state.pause_time_us += (self.clock.now_us() - state.focus_pause_started_us)
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
    ) -> ExecutionResult:
        """Dispatch a single KeyAction to the backend and record precise timing metrics."""
        send_start_us = state.get_elapsed_us(self.clock)
        if action.kind == "down":
            self.backend.key_down(action.scan_codes)
        else:
            self.backend.key_up(action.scan_codes)
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
        )

        self.telemetry.record(
            event_index=idx,
            kind=action.kind,
            scheduled_us=action.at_us,
            actual_us=send_start_us,
            lateness_us=lateness_us,
            send_duration_us=send_duration_us,
            scan_codes=action.scan_codes,
            reason=action.reason,
        )
        return result

    def play(self) -> str:
        # Wait for initial focus if required to prevent "Focus lost" showing immediately at start
        if self.require_focus and not self.focus_guard.is_active():
            self.backend.release_all()
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
                    self.backend.release_all()
                self.sleeper.sleep(self.sleep_policy.poll_s)

        state = PlaybackState(start_perf=self.clock.now_us())
        
        first_action_executed = False
        last_render_time_us = 0

        total_time_us = max(a.at_us for a in self.actions) if self.actions else 0
        total_time_seconds = total_time_us / 1_000_000

        # Telemetry diagnostic counters
        late_events_over_2ms = 0
        late_events_over_5ms = 0
        late_events_over_10ms = 0
        max_lateness_us = 0

        # Main execution loop
        try:
            for idx, action in enumerate(self.actions):
                while True:
                    command = self.controls.poll() if self.controls is not None else None
                    
                    cmd_res = self._handle_commands(command, state, total_time_seconds)
                    if cmd_res:
                        return cmd_res

                    wait_res, wait_cmd = self._process_wait_states(state, first_action_executed, total_time_seconds)
                    if wait_res:
                        if wait_cmd:
                            return wait_cmd
                        continue

                    elapsed_us = state.get_elapsed_us(self.clock)
                    if elapsed_us >= action.at_us:
                        break

                    remaining_us = action.at_us - elapsed_us

                    # Throttle rendering: limit to max 30 FPS (~33ms) and skip if within 5ms of an action
                    should_render = False
                    if self.renderer and remaining_us >= 5_000:
                        now_render_us = self.clock.now_us()
                        if now_render_us - last_render_time_us >= 33_000:
                            should_render = True
                            last_render_time_us = now_render_us

                    if should_render:
                        self.renderer.render(elapsed_us / 1_000_000, total_time_seconds, self.song.name, status="playing")

                    target_system_us = state.start_perf + state.pause_time_us + action.at_us
                    self.precise_sleeper.sleep_step_towards_us(target_system_us, self.clock, self.sleeper, self.sleep_policy.spin_threshold_us)

                # Execute action via extracted method (telemetry + late tracking inside)
                first_action_executed = True
                exec_result = self._execute_action(idx, action, state)

                lateness_us = exec_result.lateness_us
                if exec_result.is_late:
                    max_lateness_us = max(max_lateness_us, lateness_us)
                    if lateness_us > 2_000:
                        late_events_over_2ms += 1
                    if lateness_us > 5_000:
                        late_events_over_5ms += 1
                    if exec_result.is_critically_late:
                        late_events_over_10ms += 1

                if self.renderer and hasattr(self.renderer, 'update_counters'):
                    self.renderer.update_counters(max(0, lateness_us))
                
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
            
        finally:
            outcome = self.backend.release_all()
            self.telemetry.record_release_outcome(outcome)
            self.telemetry.record_backend_health(self.backend.get_health())
            self.telemetry.save()
