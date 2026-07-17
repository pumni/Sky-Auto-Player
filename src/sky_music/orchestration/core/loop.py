from __future__ import annotations

from collections import deque
from collections.abc import Callable
from dataclasses import dataclass, replace
from typing import TYPE_CHECKING, Any, Protocol

from sky_music.domain.scheduler_types import ActionKind, KeyAction, Microseconds
from sky_music.infrastructure.backend import (
    BackendHealth,
    InputBackend,
    ReleaseAllOutcome,
)
from sky_music.infrastructure.timing import Clock, Sleeper, SleepPolicy
from sky_music.infrastructure.wait_strategy import WaitStrategy
from sky_music.orchestration.core.coordinator import (
    PendingRelease,
    RuntimeActionBatch,
    RuntimeDispatchCoordinator,
    RuntimeKeyIntent,
)
from sky_music.orchestration.core.ports import (
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PLAYBACK_SKIPPED,
    CommandSource,
    FocusController,
    FocusSignal,
    LeadEstimator,
    PlaybackCommand,
    ProgressSink,
)
from sky_music.orchestration.core.state import PlaybackState as PlaybackState


class RuntimeSameKeyConflictError(RuntimeError):
    """Raised when confirmed runtime hold makes a strict same-key down infeasible."""


class OutcomeResolver(Protocol):
    """Protocol for a caller-supplied callback that labels a down dispatch's outcome AFTER
    the SendInput returned, based on the structured ``InputSendResult.success`` /
    ``sent`` prefix.

    Phase 3 of the SendInput lifecycle plan uses this to tag note-on dispatches whose
    SendInput landed a strict prefix as ``partial_note_on`` (distinct from the pre-send
    drops ``dropped_conflict`` / ``dropped_expired`` / ``blocked_unfocused``). The
    default ``runtime_outcome`` parameter of ``_execute_action`` is ``"sent"`` and the
    resolver may return it unchanged.
    """

    def __call__(
        self,
        action: KeyAction,
        send_result: object,
        default_outcome: str,
    ) -> str: ...


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


if TYPE_CHECKING:
    from sky_music.orchestration.telemetry import TelemetryLogger


class DispatchHealthMonitor:
    """Manages foreground window focus checking and input path degradation telemetry."""

    def __init__(
        self,
        backend: InputBackend,
        clock: Clock,
        focus_guard: FocusController,
        require_focus: bool,
        input_path_warn_us: int = 300,
        cheap_focus_probe: Callable[[], bool] | None = None,
    ) -> None:
        self.backend = backend
        self.clock = clock
        self.focus_guard = focus_guard
        self.require_focus = require_focus
        self.input_path_warn_us = max(0, input_path_warn_us)
        # Cheap HWND-only foreground probe (``inputs.is_foreground_cached_hwnd``), injected
        # by the engine so the core never imports the platform module (Phase 4 §7.6 boundary).
        # ``None`` in direct-mode fallback → degrade to ``focus_guard.is_active()``.
        self._cheap_focus_probe = cheap_focus_probe

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
        # Optional runtime FocusSignal (SharedFocusSignal under threaded dispatch).
        # Ownership: set once at DispatchLoop.run() entry; single-writer reference.
        # When present, focus_is_active() reads it instead of any focus_guard syscall.
        self._runtime_focus_signal: FocusSignal | None = None

    def set_runtime_signal(self, focus_signal: FocusSignal | None) -> None:
        """Wire the supervisor FocusSignal for cheap post-send focus diagnostics."""
        self._runtime_focus_signal = focus_signal

    def focus_is_active(self) -> bool:
        """Return True iff Sky is treated as focused for post-send diagnostics.

        Preference order (finding A4):
        1. Runtime ``FocusSignal`` when set (threaded: SharedFocusSignal sampled by
           the supervisor — zero syscalls on the dispatch thread).
        2. Cheap HWND-only check via the injected ``cheap_focus_probe``
           (``inputs.is_foreground_cached_hwnd``) with a 2 ms TTL (direct mode / no signal).
        3. Fall back to ``focus_guard.is_active()`` when no cheap probe was injected.

        The cheap probe is injected (not imported) so the core never depends on the
        platform module — Phase 4 §7.6 boundary rule.

        The Phase-2 pre-down gate does NOT use this method — it reads
        ``DispatchLoop._runtime_focus_signal`` directly. Full process-name validation
        remains on the supervisor / polled pause gate / pre-start wait only.
        """
        if self._runtime_focus_signal is not None:
            return self._runtime_focus_signal.is_active()
        now_us = self.clock.now_us()
        if now_us - self._focus_cache_at_us >= self._focus_cache_ttl_us:
            if self._cheap_focus_probe is not None:
                self._focus_active_cache = self._cheap_focus_probe()
            else:
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
        val = max(0, send_duration_us)
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
        onset_bias_us: int = 0,
        unfocused_send_hook: Callable[[], None] | None = None,
        diagnostics_log: Callable[[str], None] | None = None,
        cheap_foreground_probe: Callable[[], bool] | None = None,
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
        # Injected by engine → platform note_send_while_unfocused (no platform import here).
        self.unfocused_send_hook = unfocused_send_hook
        # Injected by engine → platform debug_log for teardown send-diagnostics (no platform
        # import here). ``None`` (test/DryRun) silences the diagnostic line. Phase 4 §7.6.
        self.diagnostics_log = diagnostics_log
        # Injected by engine → platform ``is_foreground_cached_hwnd`` (cheap HWND-only
        # foreground compare, no OpenProcess). Consulted by the Phase-2 pre-down gate as a
        # fresh recheck that closes the SharedFocusSignal sampling race (see
        # ``_dispatch_down_batch``). ``None`` (test/DryRun / no platform) → gate relies on
        # the runtime FocusSignal alone, preserving the pre-fix behaviour. Phase 4 §7.6 boundary.
        self.cheap_foreground_probe = cheap_foreground_probe

        self._next_dispatch_id = 0
        self._wait_spin_start_us = 0
        self._last_send_completed_us = 0
        # Phase 2 pre-down focus gate starts dormant — only after the first down dispatch
        # has actually fired does the loop accept the gate's strict no-send-while-unfocused
        # policy. Before that the polled focus-pause gate handles the pre-start unfocused
        # window (publishing "waiting_for_focus" via the renderer) — see the long comment in
        # ``_dispatch_down_batch``. Reset per ``run()`` invocation.
        self._first_down_dispatched = False
        # Runtime FocusSignal for the Phase 2 pre-down gate. Ownership: set at ``run()``
        # entry from the supervisor (SharedFocusSignal under threaded dispatch, DirectFocusSignal
        # in direct mode); dispatch-thread single-writer for the reference; reads of
        # ``is_active()`` may be cross-thread via the signal's own contract. Declared here
        # (not mid-``run()``) so the annotated assignment never overwrites a live signal.
        self._runtime_focus_signal: FocusSignal | None = None

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

    def _abort_input_safe(
        self,
        reason: str,
        *,
        full_instrument: bool = False,
    ) -> ReleaseAllOutcome:
        """Single unified input-abort helper for every interrupt path on the dispatch thread.

        Order is release-first then cancel — so the backend tracking sets still know which
        keys are held when ``release_all`` walks them, and ``coordinator.cancel_all`` then
        terminalizes the now-released generations to ``CANCELLED`` (idempotent on re-entry).

        ``reason`` is recorded into telemetry ``abort_counts_by_reason`` and propagates to the
        summary JSON. Canonical reasons: ``"manual_pause" | "focus_lost" | "panic" | "quit" |
        "finished" | "error"`` — callers MUST pass one. Unknown strings are still tallied
        verbatim (the counter is diagnostic, not a closed enum), but passing a stable string
        keeps summaries diffable across runs.

        ``full_instrument=True`` additionally issues a full Sky-15 KEYUP (identical scan-code
        set to the watchdog's ``panic_release_all``) on ``WinSendInputBackend`` sessions as a
        belt-and-braces failsafe against silently stuck keys after an asymmetric disaster
        (panic / process teardown). Test/DryRun backends inherit a ``_TrackedKeyState`` default
        that degrades to ``release_all`` (same outcome, no extra OS call) — so callers needing
        the failsafe can request it unconditionally without per-backend type-switching. The
        Phase 4 §7.2 contract promotion means the loop just calls the Protocol method directly;
        no ``getattr`` probe is needed. The watchdog remains the last-resort hard-kill
        regardless of this flag.
        """
        outcome = (
            self.backend.release_all_full_instrument()
            if full_instrument
            else self.backend.release_all()
        )
        self.coordinator.cancel_all()
        self.telemetry.record_abort(reason)
        return outcome

    def _intent_generation_ids(self, intents: tuple[RuntimeKeyIntent, ...]) -> tuple[int, ...]:
        if not self.telemetry.enabled:
            return ()
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
        outcome_resolver: OutcomeResolver | None = None,
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

        # Anchor onset / hold to the SendInput RETURN (pure completion), not the post-syscall
        # bookkeeping tail: health/focus/telemetry after the call must not push the "note landed"
        # timestamp later than the OS inject moment. NOTE: SendInput-return is a sender-side
        # PROXY, not the game-perceived onset — the call only enqueues to the Raw Input Thread;
        # actual delivery + the game's per-frame poll happen later by an unmeasured, variable
        # delta. Every telemetry metric derived from this is injection-relative (the summary is
        # explicit: game_observed.available=false until audio/onset evidence is attached — see
        # tests/measure_stutter*.py). Do not treat this timestamp as ground-truth game onset.
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
        if not self.health_monitor.focus_is_active() and self.unfocused_send_hook is not None:
            self.unfocused_send_hook()

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
        # Phase 3 outcome resolver: let the caller (e.g. _dispatch_down_batch) override
        # the runtime_outcome label AFTER the send returned, based on send_result.success
        # / sent-prefix. The record is frozen, so we use ``replace`` to construct a new one
        # BEFORE ``telemetry.record`` consumes it — this keeps the dispatch_id / lead /
        # completion timestamp intact alongside the relabelled outcome.
        if outcome_resolver is not None:
            resolved_outcome = outcome_resolver(action, send_result, runtime_outcome)
            if resolved_outcome != runtime_outcome:
                result = replace(result, runtime_outcome=resolved_outcome)

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

    @staticmethod
    def _resolve_down_outcome(
        action: KeyAction,
        send_result: object,
        default_outcome: str,
    ) -> str:
        # Phase 3 partial-send outcome hygiene: relabel note-on dispatches whose
        # SendInput landed a strict prefix (or nothing) as ``partial_note_on``.
        # G5 musical no-retry keeps us from finishing the remainder late; the
        # coordinator promotes unsent gens to DROPPED_BACKEND in ``activate_sent_downs``.
        # ``partial_note_on`` makes the sender-side atomicity break first-class in
        # CSV/telemetry, distinct from pre-send drops.
        sent = getattr(send_result, "sent", ())
        if not sent and len(action.scan_codes) > 0:
            return "partial_note_on"
        if (
            default_outcome == "sent"
            and action.scan_codes
            and len(sent) < len(action.scan_codes)
        ):
            return "partial_note_on"
        return default_outcome

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

        # Phase 2 fresh focus recheck gate — close the check-vs-send race that the polled
        # focus-pause gate still leaves open between deadline-wake and SendInput. We use the
        # ``FocusSignal`` cached at ``run()`` entry (the same one the polled gate consults)
        # rather than the health_monitor's raw ``focus_guard`` — under threaded dispatch the
        # signal is a ``SharedFocusSignal`` updated by the control thread's periodic sample,
        # so reading it does NOT block the dispatch thread's hot note-on path with a slow
        # GetForegroundWindow. In direct mode the signal wraps ``focus_guard.is_active()``
        # directly, which is what the plan §2.1 "force refresh" intention captures. The gate
        # only fires after the first down batch has actually been dispatched (see the
        # ``_first_down_dispatched`` flag) — before that the polled ``_process_wait_states``
        # gate handles the pre-start unfocused window (publishing "waiting_for_focus" via
        # the renderer, which existing tests rely on). Subsequent race-window slips mid-song
        # are caught here: we DROP the down (mark every gen ``blocked_unfocused``), call
        # ``_abort_input_safe`` to clear held keys, and let the polled gate take over the
        # visible "focus_lost" status + pause anchor on the next iteration.
        if (
            self._first_down_dispatched
            and self.health_monitor.require_focus
            and self._runtime_focus_signal is not None
            and (
                not self._runtime_focus_signal.is_active()
                # Fresh HWND-only recheck (§2.1): the SharedFocusSignal is sampled by the
                # supervisor only every focus_poll_s (20–50 ms), so on alt-tab it can lag and
                # still read active while a *newer* window already owns the foreground —
                # a window into which the down below would inject. ``GetForegroundWindow() ==
                # sky`` (no OpenProcess) is a ~sub-µs shared-state read, so this closes the
                # race on the hot note-on path with no process lookup. Short-circuits: the
                # syscall runs only when the (cheap) signal says active. ``None`` probe
                # (tests / no platform) leaves the gate exactly as it was.
                or (
                    self.cheap_foreground_probe is not None
                    and not self.cheap_foreground_probe()
                )
            )
        ):
            self._abort_input_safe("focus_lost")
            self.coordinator.drop_expired_downs(batch.intents)
            self._record_without_dispatch(
                idx=batch.source_action_index,
                kind="down",
                scheduled_us=batch.scheduled_us,
                scan_codes=tuple(intent.scan_code for intent in batch.intents),
                generation_ids=self._intent_generation_ids(batch.intents),
                reason=batch.reason,
                runtime_outcome="blocked_unfocused",
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
            outcome_resolver=self._resolve_down_outcome,
        )
        # Lead tracks pure SendInput duration (not post-syscall bookkeeping) so completions
        # land on schedule rather than overshooting by the telemetry/health tail.
        # Skip no-op sends (all keys already held → sent empty): zero-duration samples
        # drag the down-lead EMA toward 0 (finding A6b).
        if result.sent_scan_codes:
            self.estimator.update(
                ActionKind.DOWN, result.send_duration_pure_us, n_keys=len(playable)
            )
            if lead_down > 0:
                # Residual completion error after lead was applied — systematic prologue bias
                # (spin overshoot + Python work before SendInput) folds into the next lead.
                self.estimator.update_completion_error(
                    ActionKind.DOWN, result.visible_lateness_us
                )
        self.coordinator.activate_sent_downs(
            playable,
            tuple(scan_code for scan_code in result.sent_scan_codes),
            dispatch_started_us=result.actual_us,
            dispatch_completed_us=result.dispatch_completed_us,
        )
        # Phase 2 resume signal: this down batch reached the OS — the pre-down focus gate
        # (if installed) is now armed for subsequent down batches. We deliberately set
        # the flag on the FIRST *attempted* down send, not the first successful one, so a
        # cold-start that loses focus between t=0 dispatch and the next dispatch still
        # benefits from the gate. The flag is reset on each new run() invocation above.
        self._first_down_dispatched = True
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
            if result.sent_scan_codes:
                self.estimator.update(ActionKind.UP, result.send_duration_pure_us)
            self.coordinator.complete_releases(
                releases,
                tuple(sc for sc in result.sent_scan_codes),
                tuple(sc for sc in result.skipped_scan_codes),
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
        if result.sent_scan_codes:
            self.estimator.update(ActionKind.UP, result.send_duration_pure_us)
        self.coordinator.complete_releases(
            releases,
            tuple(sc for sc in result.sent_scan_codes),
            tuple(sc for sc in result.skipped_scan_codes),
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
        # PlaybackCommand is a StrEnum: raw-string command sources still compare equal to
        # the members, so this migration (§7.3) is non-breaking for legacy producers.
        if command == PlaybackCommand.QUIT:
            progress_sink.finish(f"Stopped: {self.telemetry.song_name}")
            return PLAYBACK_QUIT
        if command == PlaybackCommand.SKIP:
            progress_sink.finish(f"Skipped: {self.telemetry.song_name}")
            return PLAYBACK_SKIPPED
        if command == PlaybackCommand.REFOCUS:
            self.health_monitor.focus_guard.focus()
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status="refocus",
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
                force=True,
            )
        if command == PlaybackCommand.PANIC:
            self._abort_input_safe("panic", full_instrument=True)
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status="panic",
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
                force=True,
            )
        if command == PlaybackCommand.PAUSE:
            now_us = self.clock.now_us()
            if not state.has_pause_reason("manual"):
                self._abort_input_safe("manual_pause")
                state.enter_pause("manual", now_us)
                progress_sink.publish(
                    elapsed_us=state.get_elapsed_us(self.clock),
                    total_us=total_time_us,
                    status="paused",
                    health=self.health_monitor.get_backend_health_snapshot(force=True),
                    input_path_degraded=self.health_monitor.input_path_degraded,
                    force=True,
                )
            else:
                closed = state.exit_pause("manual", now_us)
                if closed is not None:
                    duration_us, attribution = closed
                    self.telemetry.record_pause(attribution, duration_us)
                status = "paused" if state.is_paused() else "playing"
                progress_sink.publish(
                    elapsed_us=state.get_elapsed_us(self.clock),
                    total_us=total_time_us,
                    status=status,
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
        if state.has_pause_reason("manual"):
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status="paused",
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
            )
            # Off hot path: soft-flush telemetry while idle-polling (finding A3).
            self.telemetry.flush_if_large()
            self.sleeper.sleep(self.sleep_policy.poll_s)
            return True, None

        if self.health_monitor.require_focus and not focus_signal.is_active():
            if not state.has_pause_reason("focus"):
                # Phase 1 dual-release: release the OS keyboard state NOW on focus loss
                # (not only on regain). Without this, held scan codes remain injected
                # into whatever window the user alt-tabbed into, and the game's logical
                # hold may persist. A second idempotent KEYUP still fires on regain below
                # to clear any game-side half-holds Sky may not have observed going up.
                self._abort_input_safe("focus_lost")
                state.enter_pause("focus", self.clock.now_us())
            status_val = "waiting_for_focus" if not first_action_executed else "focus_lost"
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status=status_val,
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
            )
            # Off hot path: soft-flush telemetry while idle-polling (finding A3).
            self.telemetry.flush_if_large()
            self.sleeper.sleep(self.sleep_policy.poll_s)
            return True, None

        if state.has_pause_reason("focus"):
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
                if early_cmd in (PlaybackCommand.PAUSE, PlaybackCommand.PANIC):
                    break

            self.backend.release_all()
            # Phase 1 dual-release: this is the SECOND KEYUP (idempotent). The first fired
            # on focus LOSS above; this one clears game-side half-holds Sky sampled while it
            # was still foreground and might not have observed going up. Generations were
            # cancelled on loss, so no cancel_all here — release_all alone is enough.

            closed = state.exit_pause("focus", self.clock.now_us())
            if closed is not None:
                duration_us, attribution = closed
                self.telemetry.record_pause(attribution, duration_us)
            status = "paused" if state.is_paused() else "playing"
            progress_sink.publish(
                elapsed_us=state.get_elapsed_us(self.clock),
                total_us=total_time_us,
                status=status,
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
                force=True,
            )
            if state.has_pause_reason("manual"):
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
                state.is_paused()
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
                return None, last_runtime_poll_us, last_render_time_us, first_action_executed

            remaining_us = target_elapsed_us - elapsed_us
            target_system_us = state.epoch_us + target_elapsed_us
            if remaining_us <= self.spin_threshold_us:
                self._wait_spin_start_us = elapsed_us
                self.wait_strategy.spin_until_us(target_system_us, self.clock)
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

            # Event-mode spin happens inside wait_until_us; record the nominal spin start so
            # pre_send_spin_us reflects the guard spin (the high-res sleeper wakes within the guard
            # of target, so actual spin start is within tens of µs of this).
            self._wait_spin_start_us = target_elapsed_us - self.spin_threshold_us
            
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
        # Phase 2 pre-down gate: cache the runtime FocusSignal so ``_dispatch_down_batch``
        # can re-check focus without going through the blocking ``focus_guard`` in threaded
        # mode (where the supervisor owns the focus sample cadence via SharedFocusSignal).
        self._runtime_focus_signal = focus_signal
        # Same signal for post-send diagnostic focus checks (A4): threaded mode then
        # never calls focus_guard / OpenProcess from the dispatch thread.
        self.health_monitor.set_runtime_signal(focus_signal)
        # Per-run reset of the Phase 2 pre-down gate arming flag.
        self._first_down_dispatched = False

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

        final_abort_reason = "error"  # default for any exception path not explicitly classified
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

            progress_sink.publish(
                elapsed_us=total_time_us,
                total_us=total_time_us,
                status="done",
                health=self.health_monitor.get_backend_health_snapshot(force=True),
                input_path_degraded=self.health_monitor.input_path_degraded,
                force=True,
            )
            progress_sink.finish(f"Finished playing {self.telemetry.song_name}")

            final_abort_reason = "finished"
            return PLAYBACK_FINISHED
        except RuntimeSameKeyConflictError:
            progress_sink.finish(
                f"Stopped: runtime same-key conflict in {self.telemetry.song_name}"
            )
            final_abort_reason = "quit"
            return PLAYBACK_QUIT
        finally:
            outcome = self._abort_input_safe(final_abort_reason)
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
            # Phase 4 §7.2: get_send_diagnostics is in the InputBackend Protocol (with a default on
            # _TrackedKeyState), so call directly. The legacy ``getattr`` probe is removed.
            send_diag = self.backend.get_send_diagnostics()
            self.telemetry.record_runtime_options(
                {
                    **self.telemetry.runtime_options,
                    "send_diagnostics": send_diag,
                }
            )
            if send_diag.get("partial_send_events") and self.diagnostics_log is not None:
                # Injected platform debug_log (no platform import on the core boundary).
                self.diagnostics_log(
                    "[input] SEND DIAGNOSTICS (this run): "
                    f"chord_splits={send_diag.get('chord_split_events', 0)}, "
                    f"partial_send_events={send_diag.get('partial_send_events', 0)}, "
                    f"keys_deferred={send_diag.get('keys_deferred', 0)}, "
                    f"keys_dropped={send_diag.get('keys_dropped', 0)}, "
                    f"keys_retried={send_diag.get('keys_retried', 0)}, "
                    f"zero_progress_retries={send_diag.get('zero_progress_retries', 0)}"
                )
            self.telemetry.save()
