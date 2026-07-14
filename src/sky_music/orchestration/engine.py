from __future__ import annotations

import contextlib
import gc
import json
import statistics
import threading
from pathlib import Path

from sky_music.config import RtPriorityMode
from sky_music.domain.domain import Song
from sky_music.domain.scheduler_types import ActionKind, KeyAction
from sky_music.infrastructure.backend import InputBackend, ReleaseAllOutcome
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
    compile_runtime_intents,
)
from sky_music.orchestration.telemetry import TelemetryLogger


class SendLatencyEstimator:
    """Per-kind EMA of SendInput durations used to derive the adaptive dispatch lead.

    Down durations are bucketed by polyphony (number of scan-codes in the batch, clamped to
    [1, MAX_POLY]) so that each chord size gets its own EMA.  Fallback chain for an unseeded
    bucket: online linear model (send ≈ a + b·N, fit across all down samples) → nearest seeded
    bucket ≤ N → total down EMA → 0.  The linear model lets a rarely-seen or first-of-its-size
    chord be led correctly (extrapolated) instead of borrowing a smaller chord's smaller lead.

    The linear fit is recursive least squares with an EXPONENTIAL FORGETTING factor (``lin_forget``):
    each new sample decays the prior weighted sums by ``lin_forget`` before accumulating, giving an
    effective window of ~1/(1-lin_forget) samples.  This (a) lets the backbone track slow per-machine
    send-latency drift instead of being a frozen lifetime average, and (b) bounds the accumulators so
    they cannot grow without limit across long sessions.  The raw integer sample count is kept
    separate from the decayed weight so the warm-up availability guard is unaffected by forgetting.

    Warm-start: once the linear model is available, a bucket's EMA is seeded from the linear
    prediction on its FIRST sample (then refined per-bucket via EMA), so a rare large chord is led
    well from its first occurrence instead of needing a full cold warm-up.

    Up durations use a single scalar EMA (unchanged from the original design).

    The first N samples of each bucket yield lead 0 (cold estimates are worse than nothing); the
    Nth sample seeds the EMA with the average of all warm-up samples.

    Residual completion error (``update_completion_error``): after a lead is applied, any
    remaining ``visible_lateness`` is systematic prologue (spin overshoot + Python work before
    SendInput). An EMA of that residual is folded into ``get_lead_us`` (positive only, capped)
    so the next onsets pull earlier by the observed bias rather than leaving a constant late
    offset on every note.
    """

    _SEED_SAMPLES = 5
    # Residual bias is a fine correction on top of pure-send lead — keep it small so a single
    # OS hitch cannot drag the lead into the multi-millisecond range.
    _MAX_RESIDUAL_US = 500

    # State fields (natural sort for RUF023). Semantics:
    # - down buckets by n_keys; total fallback; up scalar EMA
    # - residual prologue EMA; RLS linear model with exponential forgetting
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
        "_lin_count",
        "_lin_forget",
        "_lin_sx",
        "_lin_sxx",
        "_lin_sxy",
        "_lin_sy",
        "_lin_w",
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
        self, alpha: float = 0.2, max_lead_us: int = 2_000, lin_forget: float = 0.999, max_poly: int = 6
    ) -> None:
        self.max_poly = max_poly
        self._alpha: float = alpha
        self._max_lead_us: int = max_lead_us
        # Forgetting factor in (0, 1]; 1.0 reproduces the old lifetime-sum behaviour. 0.999 ≈ a
        # ~1000-sample effective window (about a song's worth of sends).
        self._lin_forget: float = lin_forget
        self._count_down: list[int] = [0] * (max_poly + 1)
        self._sum_down: list[int] = [0] * (max_poly + 1)
        self._ema_down: list[float] = [0.0] * (max_poly + 1)
        self._warm_down: list[bool] = [False] * (max_poly + 1)
        self._count_down_total: int = 0
        self._sum_down_total: int = 0
        self._ema_down_total: float = 0.0
        self._lin_count: int = 0
        self._lin_w: float = 0.0
        self._lin_sx: float = 0.0
        self._lin_sxx: float = 0.0
        self._lin_sy: float = 0.0
        self._lin_sxy: float = 0.0
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
                warm_base = self._predict_linear(n)
                if warm_base is not None:
                    # Warm-start this bucket from the linear model (built on prior samples),
                    # then fold in the current sample so it is usable from the first occurrence.
                    self._ema_down[n] = (
                        self._alpha * duration_us + (1.0 - self._alpha) * warm_base
                    )
                    self._warm_down[n] = True
                else:
                    # Linear model not ready yet: classic accumulate-then-seed.
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
            # RLS accumulators with exponential forgetting (x = polyphony, y = duration). Decay the
            # prior weighted sums, then fold in the new sample at unit weight.
            lam = self._lin_forget
            self._lin_count += 1
            self._lin_w = self._lin_w * lam + 1.0
            self._lin_sx = self._lin_sx * lam + n
            self._lin_sxx = self._lin_sxx * lam + n * n
            self._lin_sy = self._lin_sy * lam + duration_us
            self._lin_sxy = self._lin_sxy * lam + n * duration_us
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
        sample = max(-self._MAX_RESIDUAL_US, min(self._MAX_RESIDUAL_US * 2, int(error_us)))
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

    def _predict_linear(self, n: int) -> int | None:
        """Predict send duration for polyphony n via online least-squares (a + b·N).

        Returns None until there are enough samples spanning ≥2 distinct polyphony values
        (otherwise the slope is undefined). Lets an unseen/rare chord size be extrapolated
        instead of borrowing a smaller chord's smaller lead.
        """
        if self._lin_count < self._SEED_SAMPLES:
            return None
        w = self._lin_w
        denom = w * self._lin_sxx - self._lin_sx * self._lin_sx
        if denom <= 0:  # all samples share one polyphony → slope undefined
            return None
        slope = (w * self._lin_sxy - self._lin_sx * self._lin_sy) / denom
        intercept = (self._lin_sy - slope * self._lin_sx) / w
        return max(0, min(self._max_lead_us, round(intercept + slope * n)))

    def get_lead_us(self, kind: ActionKind, n_keys: int = 1) -> int:
        residual = self._residual_bias_us()
        if kind == "down":
            n = max(1, min(self.max_poly, n_keys))
            # Exact bucket usable (seeded or warm-started)?
            if self._warm_down[n]:
                return max(0, min(self._max_lead_us, round(self._ema_down[n]) + residual))
            # Linear extrapolation (best for an unseen/rare chord size)
            predicted = self._predict_linear(n)
            if predicted is not None:
                return max(0, min(self._max_lead_us, predicted + residual))
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

    def export_state(self) -> dict:
        """Serializable warm state for per-machine cross-session persistence.

        Only WARM buckets/scalars are exported; cold seeding state is intentionally dropped
        (it would just re-seed identically). Linear accumulators are exported so warm-start
        for a not-yet-seen chord size also survives a restart. The send/prologue latency this
        captures is a property of the machine (CPU + Python build), independent of song/profile,
        so a single global cache is correct.
        """
        return {
            "version": 1,
            "max_poly": self.max_poly,
            "ema_down": {
                str(n): self._ema_down[n]
                for n in range(1, self.max_poly + 1)
                if self._warm_down[n]
            },
            "ema_down_total": (
                self._ema_down_total
                if self._count_down_total >= self._SEED_SAMPLES
                else None
            ),
            "ema_up": self._ema_up if self._count_up >= self._SEED_SAMPLES else None,
            "ema_residual": self._ema_residual if self._warm_residual else None,
            "lin": (
                {
                    "count": self._lin_count,
                    "w": self._lin_w,
                    "sx": self._lin_sx,
                    "sxx": self._lin_sxx,
                    "sy": self._lin_sy,
                    "sxy": self._lin_sxy,
                }
                if self._lin_count >= self._SEED_SAMPLES
                else None
            ),
        }

    def import_state(self, state: object) -> None:
        """Seed warm EMAs from a previously exported state (best-effort, validated).

        A bucket seeded this way is marked warm, so its lead is used from the FIRST event of a
        run (no cold-start lateness), then refined by the normal EMA as real samples arrive. All
        values are range-checked; a corrupt cache is ignored rather than allowed to inject an
        absurd lead (get_lead_us also clamps to max_lead_us on read as a second guard).
        """
        if not isinstance(state, dict) or state.get("version") != 1:
            return
        sane_max = float(self._max_lead_us) * 4.0
        ema_down = state.get("ema_down")
        if isinstance(ema_down, dict):
            for raw_key, raw_val in ema_down.items():
                try:
                    n = int(raw_key)
                    val = float(raw_val)
                except (TypeError, ValueError):
                    continue
                if 1 <= n <= self.max_poly and 0.0 <= val <= sane_max:
                    self._ema_down[n] = val
                    self._warm_down[n] = True
                    if self._count_down[n] < self._SEED_SAMPLES:
                        self._count_down[n] = self._SEED_SAMPLES
        total = state.get("ema_down_total")
        if isinstance(total, (int, float)) and 0.0 <= float(total) <= sane_max:
            self._ema_down_total = float(total)
            if self._count_down_total < self._SEED_SAMPLES:
                self._count_down_total = self._SEED_SAMPLES
        up = state.get("ema_up")
        if isinstance(up, (int, float)) and 0.0 <= float(up) <= sane_max:
            self._ema_up = float(up)
            if self._count_up < self._SEED_SAMPLES:
                self._count_up = self._SEED_SAMPLES
        residual = state.get("ema_residual")
        if isinstance(residual, (int, float)):
            val = float(residual)
            if 0.0 <= val <= float(self._MAX_RESIDUAL_US) * 2.0:
                self._ema_residual = val
                self._warm_residual = True
                if self._count_residual < self._SEED_SAMPLES:
                    self._count_residual = self._SEED_SAMPLES
        lin = state.get("lin")
        if isinstance(lin, dict):
            try:
                if "count" in lin:
                    lin_count = int(lin["count"])
                    lin_w = float(lin["w"])
                else:
                    # Legacy cache (pre-forgetting): "n" was the undecayed count and also served as
                    # the weight, so reuse it for both — the restored fit matches what was saved.
                    lin_count = int(lin["n"])
                    lin_w = float(lin["n"])
                lin_sx = float(lin["sx"])
                lin_sxx = float(lin["sxx"])
                lin_sy = float(lin["sy"])
                lin_sxy = float(lin["sxy"])
            except (KeyError, TypeError, ValueError):
                return
            if lin_count >= 0:
                self._lin_count = lin_count
                self._lin_w = lin_w
                self._lin_sx = lin_sx
                self._lin_sxx = lin_sxx
                self._lin_sy = lin_sy
                self._lin_sxy = lin_sxy


_LEAD_CACHE_PATH = Path(__file__).resolve().parents[3] / ".cache" / "lead_estimator.json"


def load_lead_cache(path: Path) -> dict | None:
    """Read a persisted estimator state, or None on any error (missing/corrupt cache)."""
    try:
        with path.open("r", encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else None
    except Exception:
        return None


def save_lead_cache(path: Path, state: dict) -> None:
    """Persist estimator state atomically. Best-effort: never raise into playback."""
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(".json.tmp")
        with tmp.open("w", encoding="utf-8") as f:
            json.dump(state, f)
        tmp.replace(path)
    except Exception:
        pass


class PlaybackEngine:
    """Facade wiring schedule compilation, realtime context, DispatchLoop, and PlaybackSupervisor."""

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
        enable_epoch_rebase: bool = False,
        wait_strategy: WaitStrategy | None = None,
        enable_reprobe: bool = False,
        onset_bias_us: int = 0,
        lead_cache_path: Path | None = None,
        retain_telemetry_records_after_save: bool = False,
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
        max_chord = max(
            (
                len(batch.intents)
                for batch in self.runtime_schedule.batches
                if batch.kind == "down"
            ),
            default=0,
        )
        self.estimator = SendLatencyEstimator(max_poly=max(6, max_chord))
        # Per-machine warm-start: seed the estimator from a prior session so the first notes of
        # each chord size are led correctly instead of paying full cold-start lateness (the first
        # _SEED_SAMPLES events of every bucket would otherwise dispatch with lead 0). Best-effort.
        self.lead_cache_path = lead_cache_path
        if self._lead_cache_enabled:
            cached = load_lead_cache(self.lead_cache_path)  # type: ignore[arg-type]
            if cached is not None:
                self.estimator.import_state(cached)
        self.rt_priority_mode: RtPriorityMode = rt_priority_mode
        self.dispatch_lead_us = max(0, int(dispatch_lead_us))
        self.onset_bias_us = max(0, int(onset_bias_us))
        self.enable_event_wait = bool(enable_event_wait)
        self.enable_epoch_rebase = bool(enable_epoch_rebase)
        self.enable_reprobe = bool(enable_reprobe)
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
        self.sleep_policy = sleep_policy if sleep_policy is not None else SleepPolicy()

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
    def _lead_cache_enabled(self) -> bool:
        # Only a real-backend, adaptive-lead run may read/write the per-machine cache. DryRunBackend
        # sends never hit SendInput, so their ~0 durations would poison the cache for real sessions.
        return (
            self.enable_adaptive_lead
            and self.lead_cache_path is not None
            and self.backend.__class__.__name__ != "DryRunBackend"
        )

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
        import weakref
        
        engine_ref = weakref.ref(self)
        def weak_probe(s: Sleeper) -> int:
            engine = engine_ref()
            if engine is not None:
                return engine.probe_spin_threshold(s)
            return 700

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
            enable_event_wait=self.enable_event_wait,
            dispatch_lead_us=self.dispatch_lead_us,
            estimator=self.estimator if self.enable_adaptive_lead else None,
            onset_bias_us=self.onset_bias_us,
            enable_reprobe=self.enable_reprobe,
            probe_callback=weak_probe,
        )

    def _measure_spin_threshold(self, sleeper: Sleeper, *, prefix: str) -> int:
        wake_errors: list[int] = []
        for _ in range(10):
            t0 = self.clock.now_us()
            sleeper.sleep(0.002)
            t1 = self.clock.now_us()
            wake_errors.append((t1 - t0) - 2_000)

        # Use mean + 3σ rather than raw max: a single scheduler hiccup during the probe
        # would inflate max by 2–5ms and push the threshold into territory where nearly
        # every inter-note gap triggers a sleep, defeating the purpose of the spin floor.
        mean = statistics.fmean(wake_errors)
        stdev = statistics.pstdev(wake_errors)
        threshold = max(700, min(3_000, int(mean + 3 * stdev) + 100))
        self.effective_spin_threshold_us = threshold

        if prefix == "reprobe":
            self.telemetry.record_runtime_options(
                {
                    **self.telemetry.runtime_options,
                    "reprobe_wake_errors_us": wake_errors,
                    "reprobe_effective_spin_threshold_us": threshold,
                    "reprobe_trigger": "focus_restore",
                }
            )
        else:
            self.telemetry.record_runtime_options(
                {
                    **self.telemetry.runtime_options,
                    "probe_wake_errors_us": wake_errors,
                    "effective_spin_threshold_us": threshold,
                    "enable_adaptive_spin": True,
                }
            )
        return threshold

    def probe_spin_threshold(self, sleeper: Sleeper) -> int:
        """Measure this machine's sleeper wake error and derive the effective spin threshold.

        Called mid-play (after focus restore). Records reprobe_* telemetry keys so the
        initial probe_* keys from _probe_timer_wake_error remain intact for comparison.
        """
        return self._measure_spin_threshold(sleeper, prefix="reprobe")

    def _probe_timer_wake_error(self, sleeper: Sleeper) -> None:
        """Measure this machine's sleeper wake error and derive the effective spin threshold.

        Runs strictly BEFORE the playback perf anchor (start_perf) is captured, like gc.collect():
        nothing may delay the dispatch start after the anchor or the first onsets compress.
        Uses the original probe_* telemetry keys (not reprobe_*) so tests can distinguish
        initial probing from mid-play re-probing.
        """
        self._measure_spin_threshold(sleeper, prefix="probe")

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
                if result == PLAYBACK_FINISHED:
                    self._log_timing_summary()
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
            # Persist the warmed estimator (any exit path) so the next run starts warm. Outside the
            # RT scope's timing window — playback is already done — and fully best-effort.
            if self._lead_cache_enabled:
                save_lead_cache(self.lead_cache_path, self.estimator.export_state())  # type: ignore[arg-type]
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
