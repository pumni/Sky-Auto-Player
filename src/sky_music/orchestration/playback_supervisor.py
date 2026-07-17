from __future__ import annotations

import contextlib
import queue
import threading
import time
from collections import deque
from contextlib import nullcontext
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

from sky_music.config import RtPriorityMode
from sky_music.infrastructure.backend import BackendHealth
from sky_music.infrastructure.focus import FocusGuard
from sky_music.infrastructure.timing import Clock, Sleeper, SleepPolicy

if TYPE_CHECKING:
    from sky_music.orchestration.dispatch_loop import DispatchLoop
    from sky_music.orchestration.engine import PlaybackState
    from sky_music.orchestration.runtime_dispatch import RuntimeDispatchCoordinator
    from sky_music.orchestration.telemetry import TelemetryLogger

# Protocol re-exports — single source of truth lives in ``core.ports`` so the
# core boundary test can verify the seam. Tests still import these names
# from orchestration.playback_supervisor; the symbols are kept alive here.
from sky_music.orchestration.core.ports import (
    PLAYBACK_FINISHED as PLAYBACK_FINISHED,
)
from sky_music.orchestration.core.ports import (
    PLAYBACK_QUIT as PLAYBACK_QUIT,
)
from sky_music.orchestration.core.ports import (
    PLAYBACK_SKIPPED as PLAYBACK_SKIPPED,
)
from sky_music.orchestration.core.ports import (
    CommandSource as CommandSource,
)
from sky_music.orchestration.core.ports import (
    FocusSignal as FocusSignal,
)
from sky_music.orchestration.core.ports import (
    ProgressSink as ProgressSink,
)


@dataclass(frozen=True, slots=True)
class DirectCommandSource:
    controls: Any

    def poll(self) -> str | None:
        if self.controls is None:
            return None
        return self.controls.poll()


@dataclass(frozen=True, slots=True)
class DirectFocusSignal:
    is_active_fn: Any

    def is_active(self) -> bool:
        return self.is_active_fn()


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
        lateness_us: int | None = None,  # noqa: ARG002
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

    def update_counters(self, lateness_us: int, kind: str = "down") -> None:
        if self.renderer is not None and hasattr(self.renderer, "update_counters"):
            self.renderer.update_counters(lateness_us, kind=kind)


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
    def __init__(self, commands: queue.Queue[str]) -> None:
        self._commands = commands

    def poll(self) -> str | None:
        if self._commands.empty():
            return None
        try:
            return self._commands.get_nowait()
        except queue.Empty:
            return None


class SharedFocusSignal:
    """Lock-protected focus flag shared between the supervisor and dispatch thread.

    ``is_active`` lives on the hot read path of the dispatch loop; the writer only
    flips it on focus transitions. We avoid the lock on the unchanged case so a
    steady-state dispatch loop pays no lock acquire under free-threaded Python.
    """

    def __init__(self, active: bool = True) -> None:
        self._active = active
        self._lock = threading.Lock()

    def set_active(self, active: bool) -> None:
        with self._lock:
            if active != self._active:
                self._active = active

    def is_active(self) -> bool:
        # Fast path: read the live bool. If two set_active calls race against this
        # read the worst outcome is one extra dispatch loop iteration using the
        # previous state, then an immediate re-read sees the fresh value — no missed
        # transition is possible because set_active signals the command event after
        # flipping, so the loop re-checks is_active() on the next wake.
        return self._active


class SnapshotProgressSink:
    """Lock-protected snapshot handoff from dispatch thread to supervisor/UI thread.

    Memory bounds: the counter ring buffer is capped (``_max_counters``) so a UI thread
    that briefly stalls cannot let counter tuples accumulate without bound — bounded
    deque eviction is O(1) and never triggers a freeing collection on the hot path.
    """

    def __init__(self, max_counters: int = 512) -> None:
        self._lock = threading.Lock()
        self._snapshot: ProgressSnapshot | None = None
        self._version = 0
        self._finish_message: str | None = None
        # Bounded deque — a UI thread hiccup can drop counters but must never grow memory.
        self._counter_updates: deque[tuple[int, str]] = deque(maxlen=max_counters)

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

    def update_counters(self, lateness_us: int, kind: str = "down") -> None:
        with self._lock:
            self._counter_updates.append((lateness_us, kind))

    def consume(
        self,
        last_version: int,
    ) -> tuple[int, ProgressSnapshot | None, tuple[tuple[int, str], ...], str | None]:
        # Fast path: nothing changed since last consume — avoids taking a lock and any
        # deque materialisation on the supervisor's 200Hz tick.
        with self._lock:
            if self._version == last_version and not self._counter_updates and self._finish_message is None:
                return last_version, None, (), None
            snapshot = self._snapshot if self._version != last_version else None
            version = self._version
            # tuple(deque) is a fresh allocation; only do it when there are counters.
            counters = tuple(self._counter_updates) if self._counter_updates else ()
            self._counter_updates.clear()
            finish_message = self._finish_message
            self._finish_message = None
        return version, snapshot, counters, finish_message


@dataclass(slots=True)
class DispatchThreadResult:
    result: str | None = None
    error: BaseException | None = None


class PlaybackSupervisor:
    """Coordinates thread execution, Windows focus checks, and input/output routing."""

    def __init__(
        self,
        controls: Any,
        focus_guard: FocusGuard,
        require_focus: bool,
        renderer: Any,
        telemetry: TelemetryLogger,
        sleep_policy: SleepPolicy,
        clock: Clock,
        sleeper: Sleeper,
        song_name: str,
        rt_priority_mode: RtPriorityMode = "auto",
        enable_timer_guard: bool = True,
        enable_event_wait: bool = False,
        enable_epoch_rebase: bool = False,
    ) -> None:
        self.controls = controls
        self.focus_guard = focus_guard
        self.require_focus = require_focus
        self.renderer = renderer
        self.telemetry = telemetry
        self.sleep_policy = sleep_policy
        self.clock = clock
        self.sleeper = sleeper
        self.song_name = song_name
        self.rt_priority_mode: RtPriorityMode = rt_priority_mode
        self.enable_timer_guard = enable_timer_guard
        self.enable_event_wait = enable_event_wait
        self.enable_epoch_rebase = enable_epoch_rebase
        # Set by _run_threaded when enable_epoch_rebase is True; read by the post-run
        # telemetry flush. Initialized to None so pyright can track it as int | None.
        self._epoch_rebase_us: int | None = None

    def run(
        self,
        dispatch_loop: DispatchLoop,
        coordinator: RuntimeDispatchCoordinator,
        state: PlaybackState,
        total_time_us: int,
        use_dispatch_thread: bool,
    ) -> str:
        if use_dispatch_thread:
            return self._run_threaded(dispatch_loop, coordinator, state, total_time_us)
        return self._run_direct(dispatch_loop, coordinator, state, total_time_us)

    def _run_direct(
        self,
        dispatch_loop: DispatchLoop,
        coordinator: RuntimeDispatchCoordinator,  # noqa: ARG002
        state: PlaybackState,
        total_time_us: int,
    ) -> str:
        command_source = DirectCommandSource(self.controls)
        def get_focus_active() -> bool:
            return True if not self.require_focus else self.focus_guard.is_active()
        focus_signal = DirectFocusSignal(get_focus_active)
        progress_sink = DirectProgressSink(self.renderer, self.song_name)

        # Event-driven waits need a supervisor thread to signal the event; in direct mode nobody
        # would, so commands could never interrupt a sleep. Always run direct mode polled
        # (command_event=None drives the loop's polling behaviour).
        return dispatch_loop.run(
            state=state,
            command_source=command_source,
            focus_signal=focus_signal,
            progress_sink=progress_sink,
            total_time_us=total_time_us,
            command_event=None,
        )

    def _run_threaded(
        self,
        dispatch_loop: DispatchLoop,
        coordinator: RuntimeDispatchCoordinator,  # noqa: ARG002
        state: PlaybackState,
        total_time_us: int,
    ) -> str:
        from sky_music.platform.win32 import inputs

        command_queue: queue.Queue[str] = queue.Queue()
        command_source = QueueCommandSource(command_queue)
        focus_signal = SharedFocusSignal(True)
        progress_sink = SnapshotProgressSink()
        dispatch_result = DispatchThreadResult()
        self._epoch_rebase_us = None  # reset per-run; set below iff enable_epoch_rebase

        # The supervisor owns the command event and creates it BEFORE the dispatch thread starts,
        # so a command enqueued during thread startup can never lose its wake-up signal (in event
        # mode the loop only polls the queue on event wake-ups).
        command_event_handle: int | None = None
        if self.enable_event_wait:
            command_event_handle = inputs.create_auto_reset_event()
            if command_event_handle is None:
                inputs.debug_log("[realtime] command event unavailable; falling back to polled waits")

        def dispatch_target() -> None:
            try:
                sleeper_is_high_res = getattr(dispatch_loop.sleeper, "is_high_resolution", False)
                if not sleeper_is_high_res:
                    inputs.debug_log("[realtime] high-resolution waitable timer disabled")
                # The 1 ms timer-resolution guard is now a FALLBACK-ONLY safety net. When the
                # dispatch sleeper is the high-resolution waitable timer
                # (CREATE_WAITABLE_TIMER_HIGH_RESOLUTION), it wakes with sub-millisecond accuracy
                # WITHOUT a global timeBeginPeriod(1) — measured on Win11/py3.14t: wake p99 ≈
                # 0.57 ms period-OFF vs 0.57 ms period-ON, absorbed by the ~0.8 ms spin guard. So
                # the process-wide 1 ms period (retired from main.py) buys no accuracy there and
                # only raises the system-wide timer-interrupt rate (power). We install the guard
                # only on the fallback path (old Windows / high-res timer unavailable), where the
                # RealSleeper (time.sleep) may still be coarse and needs the 1 ms floor.
                use_timer_guard = self.enable_timer_guard and not sleeper_is_high_res
                timer_scope = (
                    inputs.high_resolution_timer_scope()
                    if use_timer_guard
                    else nullcontext()
                )
                if not self.enable_timer_guard:
                    inputs.debug_log("[realtime] timer guard disabled")
                elif not use_timer_guard:
                    inputs.debug_log("[realtime] timer guard skipped (high-res sleeper needs no 1ms period)")

                from sky_music.infrastructure.rt_priority import (
                    DispatchThreadPriorityScope,
                )
                priority_scope = DispatchThreadPriorityScope(self.rt_priority_mode)

                with timer_scope, priority_scope:
                    if priority_scope.outcome is not None:
                        self.telemetry.record_runtime_options(
                            {
                                **self.telemetry.runtime_options,
                                "rt_priority_acquired": priority_scope.outcome.acquired,
                                "power_throttling_disabled": priority_scope.power_throttling_disabled,
                            }
                        )
                        inputs.debug_log(
                            f"[rt_priority] Requested: {priority_scope.outcome.requested_mode}, "
                            f"Acquired: {priority_scope.outcome.acquired}, "
                            f"power_throttling_disabled={priority_scope.power_throttling_disabled}, "
                            f"Detail: {priority_scope.outcome.detail}"
                        )

                    if self.enable_epoch_rebase:
                        self.telemetry.record_runtime_options(
                            {
                                **self.telemetry.runtime_options,
                                "epoch_rebase": True,
                            }
                        )
                        # Keep this as the final pre-run statement; nothing after it may be
                        # charged against t=0 notes.
                        rebase_us = state.rebase_epoch(self.clock.now_us())
                        self._epoch_rebase_us = rebase_us

                    dispatch_result.result = dispatch_loop.run(
                        state=state,
                        command_source=command_source,
                        focus_signal=focus_signal,
                        progress_sink=progress_sink,
                        total_time_us=total_time_us,
                        command_event=command_event_handle,
                    )
            except BaseException as exc:
                dispatch_result.error = exc

        dispatch_thread = threading.Thread(
            target=dispatch_target,
            name="sky-music-dispatch",
        )
        dispatch_thread.start()

        last_snapshot_version = 0
        next_control_poll_s = 0.0
        next_focus_check_s = 0.0
        next_full_focus_check_s = 0.0
        next_progress_publish_s = 0.0
        # 5 ms control poll fires ~200 Hz and spends the whole tick calling
        # GetAsyncKeyState ×5 (≈12 pollhammer calls/sec) — the user-visible CPU
        # footprint of "playing a song". 10 ms halves that rate without anyone
        # noticing: even a fast human needs ~150 ms to react to a hotkey prompt,
        # so 10 ms sup approves quit/pause commands well below the perceptible
        # floor while the dispatch loop itself still services real-time notes.
        control_poll_s = min(max(self.sleep_policy.poll_s, 0.010), 0.020)
        focus_poll_s = min(max(self.sleep_policy.poll_s, 0.020), 0.050)
        control_sleep_s = min(control_poll_s, 0.010)
        progress_publish_s = 0.033

        last_active_state = True

        try:
            while dispatch_thread.is_alive():
                now_s = time.perf_counter()
                if now_s >= next_control_poll_s:
                    command = self.controls.poll() if self.controls is not None else None
                    if command in ("pause", "skip", "quit", "panic"):
                        command_queue.put(command)
                        if command_event_handle is not None:
                            inputs.set_event(command_event_handle)
                    elif command == "refocus":
                        focused = self.focus_guard.focus()
                        active = True if not self.require_focus else (self.focus_guard.is_active() or focused)
                        last_active_state = active
                        focus_signal.set_active(active)
                        progress_sink.publish(
                            elapsed_us=state.elapsed_snapshot_us(self.clock)[0],
                            total_us=total_time_us,
                            status="refocus",
                            health=None,
                            input_path_degraded=dispatch_loop.health_monitor.input_path_degraded,
                            force=True,
                        )
                        if command_event_handle is not None:
                            inputs.set_event(command_event_handle)
                    next_control_poll_s = now_s + control_poll_s

                if now_s >= next_focus_check_s:
                    if not self.require_focus:
                        active = True
                    elif now_s >= next_full_focus_check_s:
                        active = self.focus_guard.is_active()
                        next_full_focus_check_s = now_s + 1.0
                    else:
                        active = inputs.is_foreground_cached_hwnd()
                        
                    if active != last_active_state:
                        last_active_state = active
                        focus_signal.set_active(active)
                        if command_event_handle is not None:
                            inputs.set_event(command_event_handle)
                    next_focus_check_s = now_s + focus_poll_s

                # In event mode the dispatch loop sleeps whole inter-note gaps without iterating,
                # so the periodic "playing" progress is published here instead. Pause/focus states
                # are published by the loop itself (it is awake and polling in those states).
                # Phase 4 §7.4: cross-thread read uses the atomic snapshot rather than the live
                # pause fields, so concurrent dispatch-thread pause transitions cannot tear the
                # display read (an in-progress pause-window change just looks like the latest
                # contiguous interval — acceptable for display-only progress).
                if command_event_handle is not None and now_s >= next_progress_publish_s:
                    _display_elapsed_us, _is_paused = state.elapsed_snapshot_us(self.clock)
                    if not _is_paused:
                        # health=None on purpose: backend state is dispatch-thread-owned and must
                        # not be read from the control thread (see
                        # test_threaded_dispatch_keeps_all_backend_calls_on_dispatch_thread).
                        progress_sink.publish(
                            elapsed_us=_display_elapsed_us,
                            total_us=total_time_us,
                            status="playing",
                            health=None,
                            input_path_degraded=dispatch_loop.health_monitor.input_path_degraded,
                        )
                    next_progress_publish_s = now_s + progress_publish_s

                last_snapshot_version = self._consume_progress_updates(
                    progress_sink,
                    last_snapshot_version,
                )
                time.sleep(control_sleep_s)

            dispatch_thread.join()
        finally:
            # Always close: a stuck dispatcher after join timeout must not leak the event handle.
            if command_event_handle is not None:
                with contextlib.suppress(Exception):
                    inputs.close_handle(command_event_handle)
                command_event_handle = None

            if self._epoch_rebase_us is not None:
                self.telemetry.record_runtime_options(
                    {
                        **self.telemetry.runtime_options,
                        "epoch_rebase_us": self._epoch_rebase_us,
                    }
                )

        last_snapshot_version = self._consume_progress_updates(
            progress_sink,
            last_snapshot_version,
        )

        if dispatch_result.error is not None:
            raise dispatch_result.error
        return dispatch_result.result or PLAYBACK_FINISHED

    def _render_progress_snapshot(self, snapshot: ProgressSnapshot) -> None:
        if self.renderer is None:
            return
        self.renderer.render(
            snapshot.elapsed_us / 1_000_000,
            snapshot.total_us / 1_000_000,
            self.song_name,
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
            for lateness_us, kind in counters:
                self.renderer.update_counters(lateness_us, kind=kind)
        if snapshot is not None:
            self._render_progress_snapshot(snapshot)
        if finish_message is not None and self.renderer is not None:
            self.renderer.finish(finish_message)
        return version
