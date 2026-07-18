from __future__ import annotations

import contextlib
import gc
import json
import threading
from collections.abc import Callable
from pathlib import Path
from typing import cast

from sky_music.config import RtPriorityMode
from sky_music.domain.domain import Song
from sky_music.domain.scheduler_types import ActionKind, KeyAction
from sky_music.infrastructure.backend import (
    InputBackend,
    ReleaseAllOutcome,
    WinSendInputBackend,
)
from sky_music.infrastructure.focus import (
    FocusGuard,
    NoopFocusGuard,
    Win32SkyFocusGuard,
)
from sky_music.infrastructure.realtime import (
    RealtimeProcessScope,
    _gil_enabled,
    create_realtime_sleeper,
)
from sky_music.infrastructure.timing import (
    Clock,
    PerfCounterClock,
    RealSleeper,
    Sleeper,
    SleepPolicy,
)
from sky_music.infrastructure.wait_strategy import HybridWaitStrategy, WaitStrategy

# Re-exports kept for compatibility: the decomposition (Phase 6) moved these out of engine.py but
# callers and tests still import them from here.
from sky_music.orchestration.dispatch_loop import (
    DispatchHealthMonitor as DispatchHealthMonitor,
)
from sky_music.orchestration.dispatch_loop import (
    DispatchLoop as DispatchLoop,
)
from sky_music.orchestration.dispatch_loop import (
    ExecutionResult as ExecutionResult,
)
from sky_music.orchestration.dispatch_loop import (
    PlaybackState as PlaybackState,
)
from sky_music.orchestration.dispatch_loop import (
    RuntimeSameKeyConflictError as RuntimeSameKeyConflictError,
)
from sky_music.orchestration.playback_supervisor import (
    PLAYBACK_FINISHED as PLAYBACK_FINISHED,
)
from sky_music.orchestration.playback_supervisor import (
    PLAYBACK_QUIT as PLAYBACK_QUIT,
)
from sky_music.orchestration.playback_supervisor import (
    PLAYBACK_SKIPPED as PLAYBACK_SKIPPED,
)
from sky_music.orchestration.playback_supervisor import (
    DirectCommandSource as DirectCommandSource,
)
from sky_music.orchestration.playback_supervisor import (
    DirectFocusSignal as DirectFocusSignal,
)
from sky_music.orchestration.playback_supervisor import (
    DirectProgressSink as DirectProgressSink,
)
from sky_music.orchestration.playback_supervisor import (
    PlaybackSupervisor as PlaybackSupervisor,
)
from sky_music.orchestration.runtime_dispatch import (
    RuntimeDispatchCoordinator,
    RuntimeSchedule,
    compile_runtime_intents,
)
from sky_music.orchestration.telemetry import TelemetryLogger


class SendLatencyEstimator:
    """Per-kind EMA of SendInput durations used to derive the adaptive dispatch lead.

    Down durations are bucketed by polyphony (number of scan-codes in the batch, clamped to
    [1, MAX_POLY]) so that each chord size gets its own EMA.  Fallback chain for an unseeded
    bucket: nearest seeded bucket ≤ N → total down EMA → 0.

    Up durations use a single scalar EMA (unchanged from the original design).

    The first N samples of each bucket yield lead 0 (cold estimates are worse than nothing); the
    Nth sample seeds the EMA with the average of all warm-up samples.

    Residual completion error (``update_completion_error``): after a lead is applied, any
    remaining ``visible_lateness`` is systematic prologue (spin overshoot + Python work before
    SendInput). An EMA of that residual is folded into ``get_lead_us`` (positive only, capped)
    so the next onsets pull earlier by the observed bias rather than leaving a constant late
    offset on every note.

    Honesty note: the residual bias is capped at ``_MAX_RESIDUAL_US`` (≈ ½ ms) and is positive-
    only. Completions land within ~``_MAX_RESIDUAL_US`` of schedule when residual warm; not exact (the cap absorbs
    prologue beyond the cap). It does NOT land exactly on schedule — by design, since for a
    frame-quantized target landing slightly early is strictly safer than late. A machine whose
    prologue routinely exceeds the cap will sit at the cap rather than chase it upward.
    """

    _SEED_SAMPLES = 5
    # Residual bias is a fine correction on top of pure-send lead — keep it small so a single
    # OS hitch cannot drag the lead into the multi-millisecond range.
    _MAX_RESIDUAL_US = 500

    __slots__ = (
        "_alpha",
        "_count_down",
        "_count_down_total",
        "_count_residual",
        "_count_up",
        "_ema_down",
        "_ema_down_total",
        "_ema_residual",
        "_ema_up",
        "_max_lead_us",
        "_sum_down",
        "_sum_down_total",
        "_sum_residual",
        "_sum_up",
        "_warm_down",
        "_warm_residual",
        "max_poly",
    )

    def __init__(
        self, alpha: float = 0.2, max_lead_us: int = 2_000, max_poly: int = 6
    ) -> None:
        self.max_poly = max_poly
        self._alpha: float = alpha
        self._max_lead_us: int = max_lead_us
        self._count_down: list[int] = [0] * (max_poly + 1)
        self._sum_down: list[int] = [0] * (max_poly + 1)
        self._ema_down: list[float] = [0.0] * (max_poly + 1)
        self._warm_down: list[bool] = [False] * (max_poly + 1)
        self._count_down_total: int = 0
        self._sum_down_total: int = 0
        self._ema_down_total: float = 0.0
        self._count_up: int = 0
        self._sum_up: int = 0
        self._ema_up: float = 0.0
        self._count_residual: int = 0
        self._sum_residual: int = 0
        self._ema_residual: float = 0.0
        self._warm_residual: bool = False

    def update(self, kind: ActionKind, duration_us: int, n_keys: int = 1) -> None:
        if kind == "down":
            n = max(1, min(self.max_poly, n_keys))
            self._count_down[n] += 1
            if self._warm_down[n]:
                self._ema_down[n] = (
                    self._alpha * duration_us + (1.0 - self._alpha) * self._ema_down[n]
                )
            else:
                self._sum_down[n] += duration_us
                if self._count_down[n] >= self._SEED_SAMPLES:
                    self._ema_down[n] = self._sum_down[n] / self._count_down[n]
                    self._warm_down[n] = True
            # Total fallback
            self._count_down_total += 1
            if self._count_down_total <= self._SEED_SAMPLES:
                self._sum_down_total += duration_us
                if self._count_down_total == self._SEED_SAMPLES:
                    self._ema_down_total = self._sum_down_total / self._SEED_SAMPLES
            else:
                self._ema_down_total = (
                    self._alpha * duration_us
                    + (1.0 - self._alpha) * self._ema_down_total
                )
        elif kind == "up":
            self._count_up += 1
            if self._count_up <= self._SEED_SAMPLES:
                self._sum_up += duration_us
                if self._count_up == self._SEED_SAMPLES:
                    self._ema_up = self._sum_up / self._SEED_SAMPLES
            else:
                self._ema_up = self._alpha * duration_us + (1.0 - self._alpha) * self._ema_up

    def update_completion_error(self, kind: ActionKind, error_us: int) -> None:
        """Fold residual completion error into the prologue bias EMA (downs only).

        ``error_us`` is ``visible_lateness_us`` after a lead was applied: positive means the
        note still completed late (prologue residual), negative means early. Spikes are
        hard-clamped so one OS hitch cannot dominate the bias.
        """
        if kind != "down":
            return
        sample = max(-self._MAX_RESIDUAL_US, min(self._MAX_RESIDUAL_US * 2, error_us))
        self._count_residual += 1
        if self._warm_residual:
            self._ema_residual = (
                self._alpha * sample + (1.0 - self._alpha) * self._ema_residual
            )
        else:
            self._sum_residual += sample
            if self._count_residual >= self._SEED_SAMPLES:
                self._ema_residual = self._sum_residual / self._count_residual
                self._warm_residual = True

    def _residual_bias_us(self) -> int:
        """Positive residual only — never shrink lead because of early completions."""
        if not self._warm_residual:
            return 0
        return max(0, min(self._MAX_RESIDUAL_US, round(self._ema_residual)))

    def get_lead_us(self, kind: ActionKind, n_keys: int = 1) -> int:
        residual = self._residual_bias_us()
        if kind == "down":
            n = max(1, min(self.max_poly, n_keys))
            # Exact bucket usable (seeded)?
            if self._warm_down[n]:
                return max(0, min(self._max_lead_us, round(self._ema_down[n]) + residual))
            # Nearest usable bucket ≤ n
            for b in range(n, 0, -1):
                if self._warm_down[b]:
                    return max(0, min(self._max_lead_us, round(self._ema_down[b]) + residual))
            # Total fallback
            if self._count_down_total >= self._SEED_SAMPLES:
                return max(0, min(self._max_lead_us, round(self._ema_down_total) + residual))
            return 0
        if kind == "up":
            if self._count_up < self._SEED_SAMPLES:
                return 0
            # Ups do not include residual prologue bias — residual is an onset (down) effect.
            return max(0, min(self._max_lead_us, round(self._ema_up)))
        return 0

    def export_state(self) -> dict[str, object]:
        return {
            "version": 2,
            "saved_at": __import__("datetime").datetime.now().isoformat(),
            "max_poly": self.max_poly,
            "ema_down": list(self._ema_down),
            "warm_down": list(self._warm_down),
            "count_down": list(self._count_down),
            "sum_down": list(self._sum_down),
            "ema_down_total": self._ema_down_total,
            "warm_down_total": self._count_down_total >= self._SEED_SAMPLES,
            "count_down_total": self._count_down_total,
            "sum_down_total": self._sum_down_total,
            "ema_up": self._ema_up,
            "warm_up": self._count_up >= self._SEED_SAMPLES,
            "count_up": self._count_up,
            "sum_up": self._sum_up,
            "ema_residual": self._ema_residual,
            "warm_residual": self._warm_residual,
            "count_residual": self._count_residual,
            "sum_residual": self._sum_residual,
        }

    def import_state(self, data: dict[str, object]) -> bool:
        try:
            if not isinstance(data, dict):
                return False
            if data.get("version") != 2:
                return False
            max_poly = data.get("max_poly", 0)
            if not isinstance(max_poly, int) or max_poly < 1 or max_poly > 32:
                return False
            ema_down = data.get("ema_down", [])
            if not isinstance(ema_down, list) or len(ema_down) != max_poly + 1:
                return False
            for v in ema_down:
                if not isinstance(v, (int, float)) or not (0 <= v <= self._max_lead_us):
                    return False
            warm_down = data.get("warm_down", [])
            if not isinstance(warm_down, list) or len(warm_down) != max_poly + 1:
                return False
            if not all(isinstance(v, bool) for v in warm_down):
                return False
            count_down = data.get("count_down", [])
            if not isinstance(count_down, list) or len(count_down) != max_poly + 1:
                return False
            sum_down = data.get("sum_down", [])
            if not isinstance(sum_down, list) or len(sum_down) != max_poly + 1:
                return False
            ema_down_total = data.get("ema_down_total", 0.0)
            if not isinstance(ema_down_total, (int, float)) or not (0 <= ema_down_total <= self._max_lead_us):
                return False
            ema_up = data.get("ema_up", 0.0)
            if not isinstance(ema_up, (int, float)) or not (0 <= ema_up <= self._max_lead_us):
                return False
            ema_residual = data.get("ema_residual", 0.0)
            if not isinstance(ema_residual, (int, float)) or not (0 <= ema_residual <= self._MAX_RESIDUAL_US):
                return False
            for key in ("warm_down_total", "warm_up", "warm_residual"):
                if not isinstance(data.get(key), bool):
                    return False
            for key in ("count_down_total", "count_up", "count_residual"):
                val = data.get(key, 0)
                if not isinstance(val, int) or val < 0:
                    return False
            # All valid — apply
            self.max_poly = max_poly
            self._ema_down = [float(v) for v in ema_down]
            self._warm_down = list(warm_down)
            self._count_down = list(count_down)
            self._sum_down = list(sum_down)
            self._ema_down_total = float(ema_down_total)
            self._count_down_total = cast(int, data["count_down_total"])
            self._sum_down_total = cast(int, data.get("sum_down_total", 0))
            self._ema_up = float(ema_up)
            self._count_up = cast(int, data["count_up"])
            self._sum_up = cast(int, data.get("sum_up", 0))
            self._ema_residual = float(ema_residual)
            self._warm_residual = cast(bool, data["warm_residual"])
            self._count_residual = cast(int, data["count_residual"])
            self._sum_residual = cast(int, data.get("sum_residual", 0))
            return True
        except (KeyError, TypeError, ValueError):
            return False


class PlaybackEngine:
    """Facade wiring schedule compilation, realtime context, DispatchLoop, and PlaybackSupervisor.

    Note on timing honesty: residual completion latencies inside the Windows kernel driver
    itself (after SendInput returns to us) are generally <0.5ms. Since 2026-07 they are
    covered by the constant ``min_hold_margin_us`` folded into the frame-model
    ``min_hold_us`` (docs/timing-principles.md §2) rather than left unaccounted.
    """

    def __init__(
        self,
        song: Song,
        actions: tuple[KeyAction, ...],
        backend: InputBackend,
        controls = None,
        renderer = None,
        telemetry_enabled: bool = False,
        require_focus: bool = True,
        clock: Clock | None = None,
        sleeper: Sleeper | None = None,
        sleep_policy: SleepPolicy | None = None,
        focus_guard: FocusGuard | None = None,
        profile_name: str = "balanced",
        tempo_scale: float = 1.0,
        focus_restore_grace_us: int = 100_000,
        fps: int | None = None,
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
        # Default True to match the production RuntimeState default: the supervisor rebases the
        # playback anchor on the dispatch thread as the final pre-run statement, so thread-spawn
        # and MMCSS-acquisition time (~165us p50 / ~1ms p99 measured) is not charged as lateness
        # against t=0 notes. Direct (non-threaded) mode never rebases regardless of this flag.
        enable_epoch_rebase: bool = True,
        wait_strategy: WaitStrategy | None = None,
        spin_floor_us: int = 700,
        retain_telemetry_records_after_save: bool = False,
        lead_cache_path: str | None = None,
        # Phase F.3: margin transparency — the applied device-delivery margin and its origin.
        # Passed by callers that resolve a FrameTimingPolicy so the summary is self-describing.
        min_hold_margin_us: int = 0,
        min_hold_margin_source: str = "default_500",
    ):
        self.song = song
        self.actions = actions
        self.lead_cache_path: str | None = lead_cache_path
        self._lead_cache_loaded: bool = False
        self.runtime_schedule: RuntimeSchedule | None = compile_runtime_intents(actions)
        self.total_time_us = max((int(action.at_us) for action in actions), default=0)
        self.backend = backend
        self.focus_restore_grace_us = focus_restore_grace_us
        self.min_hold_us = max(0, min_hold_us)
        self.same_key_conflict_policy = same_key_conflict_policy
        self.late_pulse_drop_threshold_us = (
            None
            if late_pulse_drop_threshold_us is None
            else max(0, late_pulse_drop_threshold_us)
        )
        self.use_dispatch_thread = use_dispatch_thread
        self.input_path_warn_us = max(0, input_path_warn_us)
        self.enable_timer_guard = enable_timer_guard
        self.enable_waitable_timer = enable_waitable_timer
        self.enable_gc_pause = enable_gc_pause
        self.enable_switch_interval_tuning = enable_switch_interval_tuning
        self.enable_adaptive_lead = enable_adaptive_lead
        self.enable_adaptive_spin = enable_adaptive_spin
        max_chord = max(
            (
                len(batch.intents)
                for batch in self.runtime_schedule.batches
                if batch.kind == "down"
            ),
            default=0,
        )
        self.estimator = SendLatencyEstimator(max_poly=max(6, max_chord))
        # Phase D: Load cross-session lead estimator state if cache available.
        self._lead_cache_loaded = False
        if self.lead_cache_path and enable_adaptive_lead:
            try:
                cache_file = Path(self.lead_cache_path)
                if cache_file.is_file() and cache_file.stat().st_size < 64 * 1024:
                    raw = cache_file.read_text(encoding="utf-8")
                    data = json.loads(raw)
                    if self.estimator.import_state(data):
                        self._lead_cache_loaded = True
            except Exception:
                pass  # Corrupt cache — silently fall back to cold start
        self.rt_priority_mode: RtPriorityMode = rt_priority_mode
        self.dispatch_lead_us = max(0, dispatch_lead_us)
        self.spin_floor_us = max(0, spin_floor_us)
        self.enable_event_wait = enable_event_wait
        self.enable_epoch_rebase = enable_epoch_rebase
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
            retain_records_after_save=retain_telemetry_records_after_save,
        )
        self.telemetry.record_runtime_options(
            {
                "min_hold_assumes_fps": fps,
                "min_hold_us": self.min_hold_us,
                "min_hold_margin_us": min_hold_margin_us,
                "min_hold_margin_source": min_hold_margin_source,
                "note": "min_hold is sized for the CONFIGURED fps; if the game runs slower, short notes may land within one real frame and not register.",
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
                "lead_cache_loaded": self._lead_cache_loaded,
                "lead_cache_path": self.lead_cache_path,
            }
        )
        self.require_focus = require_focus
        self.clock = clock if clock is not None else PerfCounterClock()
        self.sleeper = sleeper if sleeper is not None else RealSleeper()
        self.sleep_policy = sleep_policy if sleep_policy is not None else SleepPolicy()
        # Align WinSendInputBackend send_completed_us with the playback clock (A6a).
        # isinstance — not getattr — so the contract stays explicit until Phase 4 ports.
        if isinstance(self.backend, WinSendInputBackend):
            self.backend.set_clock(self.clock)

        # Inject standard FocusGuard depending on requirements
        if focus_guard is None:
            if self.require_focus:
                self.focus_guard: FocusGuard = Win32SkyFocusGuard()
            else:
                self.focus_guard = NoopFocusGuard()
        else:
            self.focus_guard = focus_guard

        self._runtime_coordinator: RuntimeDispatchCoordinator | None = None
        # Inject the cheap HWND-only foreground probe so the dispatch core stays platform-free
        # (Phase 4 §7.6). None → DispatchHealthMonitor degrades to focus_guard.is_active().
        # The closure looks the function up on the module at CALL time (late binding) so tests
        # monkeypatching ``inputs.is_foreground_cached_hwnd`` after construction still take effect.
        cheap_focus_probe: Callable[[], bool] | None = None
        with contextlib.suppress(Exception):
            from sky_music.platform.win32 import inputs as _inputs_focus

            def _probe_foreground() -> bool:
                return _inputs_focus.is_foreground_cached_hwnd()

            cheap_focus_probe = _probe_foreground
        self._health_monitor = DispatchHealthMonitor(
            backend=self.backend,
            clock=self.clock,
            focus_guard=self.focus_guard,
            require_focus=self.require_focus,
            input_path_warn_us=self.input_path_warn_us,
            cheap_focus_probe=cheap_focus_probe,
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

    def release_song_data(self) -> None:
        """Drop per-song schedule data after play() has returned.

        Call only when this engine will not be reused for a second play() on the
        same song. The Textual app calls it after play() returns (after any
        _log_timing_summary). After this call, ``self.actions == ()`` and
        ``runtime_schedule is None``. A subsequent play() on an empty actions
        tuple is a no-op timeline (safe empty iteration).
        """
        self.actions = ()
        self.runtime_schedule = None
        self._runtime_coordinator = None
        self._compat_loop = None
        from sky_music.orchestration.runtime_session import RUNTIME_STATE
        RUNTIME_STATE.clear_session()
        # MEM-3: Release dry-run history deque (~1-2 MB, maxlen=10_000) after playback.
        # No caller reads history after release_song_data(); history stays valid for the
        # duration of play() so post-play assertions in tests still work.
        from sky_music.infrastructure.backend import DryRunBackend
        if isinstance(self.backend, DryRunBackend):
            self.backend.history.clear()

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
        unfocused_hook: Callable[[], None] | None = None
        diagnostics_log: Callable[[str], None] | None = None
        # Cheap HWND-only foreground probe for the Phase-2 pre-down gate (§2.1). ONLY wired in
        # threaded mode: the gate reads a ``SharedFocusSignal`` sampled by the supervisor every
        # 20–50 ms there, so a fresh ``GetForegroundWindow()==sky`` recheck closes the alt-tab
        # race. In direct mode the gate's ``DirectFocusSignal`` already wraps
        # ``focus_guard.is_active()`` (the full, authoritative, fresh check every down), so the
        # probe would be redundant — and, against the mock DLL in tests where ``sky`` is None,
        # would spuriously block every down. Late-binding closure so tests monkeypatching
        # ``inputs.is_foreground_cached_hwnd`` after construction still take effect.
        cheap_foreground_probe: Callable[[], bool] | None = None
        with contextlib.suppress(Exception):
            from sky_music.platform.win32 import inputs as _inputs_unfocused

            unfocused_hook = _inputs_unfocused.note_send_while_unfocused
            diagnostics_log = _inputs_unfocused.debug_log

            if self._should_use_dispatch_thread():

                def _probe_foreground_hwnd() -> bool:
                    return _inputs_unfocused.is_foreground_cached_hwnd()

                cheap_foreground_probe = _probe_foreground_hwnd

        loop = DispatchLoop(
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
            enable_event_wait=self.enable_event_wait,
            dispatch_lead_us=self.dispatch_lead_us,
            estimator=self.estimator if self.enable_adaptive_lead else None,
            unfocused_send_hook=unfocused_hook,
            diagnostics_log=diagnostics_log,
            cheap_foreground_probe=cheap_foreground_probe,
        )
        # Phase E: wire idle-gap core warmup hook
        loop.core_warmup_hook = lambda max_us: self._spin_warmup(max_us)
        # Phase H: propagate reprobe kill switch and spin floor.
        loop.enable_spin_reprobe = self.enable_adaptive_spin
        loop._spin_floor_us = self.spin_floor_us
        return loop

    def _spin_warmup(self, max_us: int) -> None:
        """Busy-spin for up to max_us to warm the CPU core before sending after idle."""
        if max_us <= 0:
            return
        perf_counter_ns = __import__("time").perf_counter_ns
        target_ns = perf_counter_ns() + max_us * 1000
        while perf_counter_ns() < target_ns:
            pass

    def _probe_timer_wake_error(self, sleeper: Sleeper) -> int:
        """Measure this machine's sleeper wake error and derive the effective spin threshold.

        Runs strictly BEFORE the playback perf anchor (start_perf) is captured, like gc.collect():
        nothing may delay the dispatch start after the anchor or the first onsets compress. This is
        the single, one-shot adaptive-spin probe (``enable_adaptive_spin``); the former mid-play
        re-probe was removed as dead code (never wired into production).
        """
        wake_errors: list[int] = []
        for _ in range(30):
            t0 = self.clock.now_us()
            sleeper.sleep(0.002)
            t1 = self.clock.now_us()
            wake_errors.append((t1 - t0) - 2_000)

        wake_errors.sort()
        p90 = wake_errors[int(len(wake_errors) * 0.9)]
        threshold = max(self.spin_floor_us, min(3_000, p90 + 100))
        self.effective_spin_threshold_us = threshold

        self.telemetry.record_runtime_options(
            {
                **self.telemetry.runtime_options,
                "probe_wake_errors_us": wake_errors,
                "effective_spin_threshold_us": threshold,
                "enable_adaptive_spin": True,
            }
        )
        return threshold

    def play(self) -> str:
        # Reset partial-send diagnostics before any sending so the per-run counts (read in the
        # finally block) reflect only this playback. Single-writer: the dispatch thread is the sole
        # sender and has not started yet.
        try:
            from sky_music.platform.win32 import inputs as _inputs_diag

            _inputs_diag.reset_send_diagnostics()
            if self.telemetry.schedule_summary is not None:
                _inputs_diag.set_schedule_diagnostics(
                    min_gap=self.telemetry.schedule_summary.get("min_same_key_up_gap_us"),
                    impossible_repeats=self.telemetry.schedule_summary.get("impossible_same_key_repeats", 0),
                )
        except Exception:
            pass

        # Rebuild the RuntimeSchedule if a previous play() on this engine released it.
        # ``actions`` is the persistent source of truth; the compiled schedule (batches, intent
        # graph) is large and is dropped once playback ends so it does not pin RSS in any UI that
        # keeps the engine alive between songs. Re-compiling on a fresh play() is cheap compared
        # to a song's playback time. ``estimator`` was sized against this same ``actions`` in
        # ``__init__`` (max_poly = max(6, max_chord)), so it remains correctly sized.
        if self.runtime_schedule is None:
            self.runtime_schedule = compile_runtime_intents(self.actions)

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

        if self._should_use_dispatch_thread():
            try:
                from sky_music.platform.win32 import inputs

                shapes_to_prewarm: set[tuple[tuple[int, ...], bool]] = set()
                for batch in self.runtime_schedule.batches:
                    if batch.kind == "down":
                        shapes_to_prewarm.add((tuple(i.scan_code for i in batch.intents), False))
                distinct_keys = set()
                for action in self.actions:
                    distinct_keys.update(action.scan_codes)
                for sc in distinct_keys:
                    shapes_to_prewarm.add(((sc,), True))

                inputs.prewarm_input_arrays(shapes_to_prewarm)
            except Exception:
                pass

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
                # Phase H: persist reprobe telemetry. Dispatch thread has joined by this
                # point (supervisor.run() returned), so reading loop fields is race-free.
                self.telemetry.record_runtime_options(
                    {
                        **self.telemetry.runtime_options,
                        "reprobe_applied_thresholds": list(
                            dispatch_loop._reprobe_applied_thresholds
                        ),
                        "enable_spin_reprobe": dispatch_loop.enable_spin_reprobe,
                    }
                )
                if result == PLAYBACK_FINISHED:
                    self._log_timing_summary()
                    self.telemetry.release_summary()  # MEM-4: free ~100-500 KB summary dict
                return result
        finally:
            self._input_path_degraded = self._health_monitor.input_path_degraded
            # Partial-send diagnostics are logged from DispatchLoop.run's finally block (where the
            # dispatch thread reads its own counters — no data race). This block was intentionally
            # removed from here in the race-fix refactor; do not reintroduce a reader of the module-
            # level send-diagnostic counters from the main thread while the dispatcher may still be
            # writing them under free-threaded Python.
            if realtime_sleeper is not self.sleeper:
                close = getattr(realtime_sleeper, "close", None)
                if close is not None:
                    close()

            # Release the per-song data so a UI that keeps the engine instance alive between
            # playback sessions doesn't pin the compiled schedule + runtime coordinator in RSS.
            # ``self.actions`` is the persistent source of truth and stays; ``runtime_schedule``
            # is rebuilt on the next ``play()`` call (cheap vs. a song's playback time). The
            # dispatch thread holds its own local coordinator reference and has already cleaned
            # up its local telemetry / dispatch bookkeeping by the time this finally runs.
            self._runtime_coordinator = None
            self._compat_loop = None
            self.runtime_schedule = None
            # Drop the prebuilt INPUT-array cache so a session running many songs back-to-back
            # doesn't accumulate up to ~8192 cached chord-shaped ctypes arrays. Safe to clear
            # here because the dispatch thread has already joined (supervisor.run returned before
            # this finally); the next play() rebuilds the cache from prewarm_input_arrays before
            # the dispatch thread starts. _INPUT_CACHE (per-key structs) is intentionally kept
            # by clear_array_cache. Best-effort; non-Windows/test platforms must not abort teardown.
            with contextlib.suppress(Exception):
                from sky_music.platform.win32 import inputs as _inputs_cleanup

                _inputs_cleanup.clear_array_cache()
            # Force-collect once here (unconditional, not gated on enable_gc_pause) so the cyclic
            # GC sweep re-enabled by RealtimeProcessScope.__exit__ frees the unreachable garbage
            # the dispatch thread allocated during playback (ExecutionResult, batch tuples,
            # dispatch bookkeeping lists) plus the runtime_schedule edges dropped above. Note this
            # only reclaims cyclic/unreachable objects — the former big reachable holdout was
            # telemetry.records (kept alive via self.telemetry), which is now cleared inside
            # telemetry.save(). Windows Working Set and pymalloc arenas may still not release the
            # freed pages back to the OS promptly (sticky WS / arena reuse), so Task Manager RSS
            # can plateau even after this collection — that is platform behaviour, not an app leak.
            with contextlib.suppress(Exception):
                gc.collect()

            # Phase D: Persist lead estimator state for next session.
            if self.lead_cache_path and self.enable_adaptive_lead:
                try:
                    cache_file = Path(self.lead_cache_path)
                    cache_file.parent.mkdir(parents=True, exist_ok=True)
                    tmp = cache_file.with_suffix(".tmp")
                    tmp.write_text(
                        json.dumps(self.estimator.export_state(), indent=2),
                        encoding="utf-8",
                    )
                    tmp.replace(cache_file)
                except Exception:
                    pass

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
            # Legacy shim: only used by pre-decomposition tests that build the loop BEFORE
            # play(). If a consumer ever calls it after the engine has released its per-song
            # data (i.e. ``runtime_schedule is None``), rebuild on demand from ``actions`` —
            # mirrors the top-of-play() guard for the production path.
            schedule = self.runtime_schedule
            if schedule is None:
                schedule = compile_runtime_intents(self.actions)
                self.runtime_schedule = schedule
            coordinator = RuntimeDispatchCoordinator(schedule, self.min_hold_us)
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
    ) -> tuple[bool, str | None]:
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
        applied_lead_us: int = 0,
    ) -> ExecutionResult:
        return self._compat_dispatch_loop()._execute_action(
            idx=idx,
            action=action,
            state=state,
            generation_ids=generation_ids,
            runtime_outcome=runtime_outcome,
            applied_lead_us=applied_lead_us,
        )
