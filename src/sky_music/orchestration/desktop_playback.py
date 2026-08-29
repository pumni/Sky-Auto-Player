"""Desktop playback admission and session orchestration.

This module is intentionally a thin application boundary around the existing
``prepare_playback`` and ``PlaybackEngine`` implementations.  It owns opaque
desktop identities and legal session transitions; it never schedules notes or
emits input itself.
"""

from __future__ import annotations

import hashlib
import json
import math
import queue
import threading
import uuid
from collections import OrderedDict
from collections.abc import Callable, Mapping
from contextlib import suppress
from dataclasses import asdict, dataclass
from typing import Any

from sky_music.config import AppConfig
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.orchestration.catalog_service import (
    CatalogGenerationError,
    CatalogLookupError,
)
from sky_music.orchestration.desktop_models import (
    PlaybackConfigDto,
    PlaybackDecisionAcceptanceDto,
    PlaybackFinishedDto,
    PlaybackRecommendationDto,
    PlaybackSessionDto,
    PlaybackSnapshotDto,
    PlaybackState,
    PreparedPlaybackDto,
    RiskDecisionDto,
    RiskSummaryDto,
    SongDetailDto,
)
from sky_music.orchestration.engine import PlaybackEngine
from sky_music.orchestration.native_models import (
    PLAYBACK_FINISHED,
    PLAYBACK_QUIT,
    PLAYBACK_SHUTDOWN_TIMEOUT,
    PLAYBACK_SKIPPED,
    NativeDispatchError,
)
from sky_music.orchestration.playback_controller import (
    PlaybackError,
    PlaybackPlan,
    prepare_playback,
)

MAX_PREPARED_PLANS = 64
MAX_EVENT_TEXT_BYTES = 4096
MAX_DECISION_COUNT = 8


class DesktopPlaybackError(ValueError):
    """A bounded, user-facing playback contract error."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


class DesktopPlaybackControls:
    """Thread-safe command mailbox polled by ``RustDispatchRuntime``."""

    def __init__(self) -> None:
        self._commands: queue.Queue[str] = queue.Queue(maxsize=32)
        self._closed = threading.Event()

    def push(self, command: str) -> None:
        if self._closed.is_set():
            return
        try:
            self._commands.put_nowait(command)
        except queue.Full:
            # Stop is the only command that must not be lost.  A full mailbox
            # means repeated UI clicks; collapse the stale queue and enqueue it.
            if command == "quit":
                while True:
                    try:
                        self._commands.get_nowait()
                    except queue.Empty:
                        break
                with suppress(queue.Full):
                    self._commands.put_nowait(command)

    def poll(self) -> str | None:
        try:
            return self._commands.get_nowait()
        except queue.Empty:
            return None

    def start(self) -> None:
        return None

    def close(self) -> None:
        self._closed.set()


@dataclass(frozen=True, slots=True)
class _PreparedRecord:
    prepared_id: str
    song_id: str
    catalog_generation: int
    settings_fingerprint: str
    plan: PlaybackPlan
    dto: PreparedPlaybackDto


@dataclass(slots=True)
class _ActiveSession:
    session_id: str
    prepared_id: str
    song_id: str
    plan: PlaybackPlan
    dry_run: bool
    controls: DesktopPlaybackControls
    worker: threading.Thread | None = None
    state: PlaybackState = "starting"


_LEGAL_TRANSITIONS: dict[PlaybackState, frozenset[PlaybackState]] = {
    "starting": frozenset({"playing", "stopping", "finished", "failed"}),
    "playing": frozenset({"paused", "stopping", "finished", "failed"}),
    "paused": frozenset({"playing", "stopping", "finished", "failed"}),
    "stopping": frozenset({"finished", "failed"}),
    "finished": frozenset(),
    "cancelled": frozenset(),
    "failed": frozenset(),
    "idle": frozenset({"starting"}),
    "ready": frozenset({"starting"}),
    "awaiting_confirmation": frozenset({"starting"}),
    "preparing": frozenset({"starting"}),
    "countdown": frozenset({"playing", "stopping", "failed"}),
    "focus_lost": frozenset({"playing", "paused", "stopping", "failed"}),
    "error": frozenset(),
}


class _SnapshotRenderer:
    """Adapter from the existing renderer callback to bounded Core events."""

    def __init__(
        self,
        publish: Callable[[str, Mapping[str, object]], None],
        session_id: str,
        song_id: str,
        title: str,
        on_state: Callable[[PlaybackState], None] | None = None,
    ) -> None:
        self._publish = publish
        self._session_id = session_id
        self._song_id = song_id
        self._title = title
        self._on_state = on_state
        self._seq = 0

    def update_counters_batch(self, _counters: object) -> None:
        # Native telemetry remains owned by the existing engine.  The desktop
        # snapshot is deliberately the small bounded progress view below.
        return None

    def render(self, current_seconds: float, total_seconds: float, _song_name: str, *, status: str, pre_roll_remaining_us: int = 0, input_path_degraded: bool = False, backend_health: object | None = None, **_kwargs: object) -> None:
        self._seq += 1
        current_us = max(0, round(current_seconds * 1_000_000))
        total_us = max(0, round(total_seconds * 1_000_000))
        state: PlaybackState = {
            "countdown": "starting",
            "waiting_for_focus": "starting",
            "focus_lost": "paused",
            "paused": "paused",
            "playing": "playing",
        }.get(status, "playing")  # type: ignore[assignment]
        focus_state = "unfocused" if status == "focus_lost" else "waiting" if status == "waiting_for_focus" else "focused"
        health = "degraded" if input_path_degraded else "healthy"
        if backend_health is not None and getattr(backend_health, "last_error", None):
            health = "error"
        snapshot = PlaybackSnapshotDto(
            seq=self._seq,
            state=state,
            song_id=self._song_id,
            title=self._title,
            current_us=current_us,
            total_us=total_us,
            pre_roll_remaining_us=max(0, int(pre_roll_remaining_us)),
            focus_state=focus_state,  # type: ignore[arg-type]
            health=health,  # type: ignore[arg-type]
            input_path_degraded=bool(input_path_degraded),
            message=None,
        )
        if self._on_state is not None:
            self._on_state(state)
        self._publish("playback.snapshot", asdict(snapshot) | {"session_id": self._session_id})

    def finish(self, _message: str) -> None:
        return None


class DesktopPlaybackService:
    """Own prepared plans and the single active desktop playback session."""

    def __init__(
        self,
        *,
        settings_service: Any,
        catalog_service: Any,
        publish_event: Callable[[str, Mapping[str, object]], None],
    ) -> None:
        self._settings_service = settings_service
        self._catalog_service = catalog_service
        self._publish_event = publish_event
        self._lock = threading.RLock()
        self._prepared: OrderedDict[str, _PreparedRecord] = OrderedDict()
        self._active: _ActiveSession | None = None
        self._last_terminal: tuple[str, PlaybackState] | None = None

    @staticmethod
    def _opaque_id() -> str:
        return uuid.uuid4().hex

    @staticmethod
    def _settings_fingerprint(cfg: AppConfig) -> str:
        payload = {
            "hold": cfg.default_hold_frames,
            "tempo": cfg.default_tempo_scale,
            "fps": cfg.game_fps,
            "theme": cfg.theme,
            "telemetry": cfg.telemetry_enabled_by_default,
        }
        return hashlib.sha256(json.dumps(payload, sort_keys=True).encode()).hexdigest()

    @staticmethod
    def _risk(plan: PlaybackPlan) -> RiskSummaryDto:
        report = plan.risk_report
        headline = {
            "low": "Low timing risk",
            "medium": "Medium timing risk",
            "high": "High timing risk",
        }[report.severity]
        return RiskSummaryDto(
            level=report.severity,
            headline=headline,
            reasons=(report.reason,) if report.reason else (),
            recommendations=tuple(report.recommendations),
        )

    @staticmethod
    def _song_detail(plan: PlaybackPlan, risk: RiskSummaryDto) -> SongDetailDto:
        recommendation = PlaybackRecommendationDto(
            recommended_hold_frames=plan.risk_report.suggested_hold_frames,
            recommended_tempo_scale=plan.risk_report.suggested_tempo_scale,
            summary=(plan.risk_report.recommendations[0] if plan.risk_report.recommendations else "Keep the selected settings."),
        )
        return SongDetailDto(
            song_id="",
            title=plan.song.name,
            duration_us=int(plan.sched_meta.source_duration_us),
            note_count=len(plan.song.notes),
            format_label="SHEET",
            risk=risk,
            recommendation=recommendation,
        )

    @staticmethod
    def _fingerprint(song_id: str, plan: PlaybackPlan, config: PlaybackConfigDto) -> str:
        actions = [
            [str(action.kind), int(action.at_us), [int(code) for code in action.scan_codes]]
            for action in plan.actions
        ]
        payload = {
            "song_id": song_id,
            "config": asdict(config),
            "policy": {
                "fps": plan.active_policy.fps,
                "min_hold_us": plan.active_policy.min_hold_us,
                "min_release_gap_us": plan.active_policy.min_release_gap_us,
            },
            "actions": actions,
        }
        return hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()).hexdigest()

    def invalidate_catalog(self, generation: int) -> None:
        with self._lock:
            for key, record in tuple(self._prepared.items()):
                if record.catalog_generation != generation:
                    self._prepared.pop(key, None)

    def invalidate_settings(self) -> None:
        with self._lock:
            self._prepared.clear()

    def prepare(
        self,
        *,
        song_id: str,
        generation: int,
        config: Mapping[str, object],
        resolve_path: Callable[..., Any],
    ) -> dict[str, object]:
        if type(song_id) is not str or len(song_id) != 32 or any(c not in "0123456789abcdef" for c in song_id):
            raise DesktopPlaybackError("invalid_params", "song_id must be a canonical opaque ID")
        if type(generation) is not int or generation < 0:
            raise DesktopPlaybackError("invalid_params", "generation must be a non-negative integer")
        allowed = {"hold_frames", "tempo_scale", "fps", "dry_run"}
        if set(config) - allowed or set(config) != allowed:
            raise DesktopPlaybackError("invalid_params", "playback config must contain exactly hold_frames, tempo_scale, fps, and dry_run")
        values = dict(config)
        if type(values["dry_run"]) is not bool:
            raise DesktopPlaybackError("invalid_params", "dry_run must be boolean")
        if isinstance(values["hold_frames"], bool) or not isinstance(values["hold_frames"], (int, float)):
            raise DesktopPlaybackError("invalid_params", "hold_frames must be numeric")
        if isinstance(values["tempo_scale"], bool) or not isinstance(values["tempo_scale"], (int, float)):
            raise DesktopPlaybackError("invalid_params", "tempo_scale must be numeric")
        if type(values["fps"]) is not int:
            raise DesktopPlaybackError("invalid_params", "fps must be an integer")
        playback_config = PlaybackConfigDto(
            hold_frames=float(values["hold_frames"]),
            tempo_scale=float(values["tempo_scale"]),
            fps=int(values["fps"]),
            dry_run=bool(values["dry_run"]),
        )
        try:
            path = resolve_path(song_id, generation=generation)
            if not math.isfinite(playback_config.hold_frames) or playback_config.hold_frames <= 0:
                raise DesktopPlaybackError("invalid_params", "hold_frames must be finite and positive")
            if not math.isfinite(playback_config.tempo_scale) or playback_config.tempo_scale <= 0:
                raise DesktopPlaybackError("invalid_params", "tempo_scale must be finite and positive")
            session = PlaybackSessionContext(
                hold_frames=playback_config.hold_frames,
                tempo_scale=playback_config.tempo_scale,
                fps=playback_config.fps,
            )
            cfg = self._settings_service.config_snapshot()
            plan_result = prepare_playback(path, session, cfg, is_dry_run=playback_config.dry_run)
        except (CatalogGenerationError, CatalogLookupError):
            raise
        except DesktopPlaybackError:
            raise
        except Exception as exc:
            raise DesktopPlaybackError("prepare_failed", str(exc)) from exc

        if isinstance(plan_result, PlaybackError):
            risk = RiskSummaryDto(
                level="high",
                headline="Playback blocked",
                reasons=(plan_result.message,),
                recommendations=tuple(
                    recommendation
                    for recommendation in (
                        (
                            f"Try tempo {plan_result.recommended_tempo_scale:.2f}×"
                            if plan_result.recommended_tempo_scale
                            else None
                        ),
                        (
                            f"Try hold {plan_result.recommended_hold_frames:.2f} frames"
                            if plan_result.recommended_hold_frames
                            else None
                        ),
                    )
                    if recommendation is not None
                ),
            )
            blocked_song = SongDetailDto(
                song_id=song_id,
                title=song_id,
                duration_us=0,
                note_count=0,
                format_label="UNKNOWN",
                risk=risk,
                recommendation=None,
            )
            blocked = PreparedPlaybackDto(
                prepared_id=None,
                song=blocked_song,
                config=playback_config,
                admission="blocked",
                risk=risk,
                decisions=(),
                plan_fingerprint=None,
                error_code=plan_result.code,
                error_message=plan_result.message,
            )
            return asdict(blocked)

        plan = plan_result
        risk = self._risk(plan)
        decisions = () if risk.level == "low" else (
            RiskDecisionDto("proceed", "Proceed with current settings"),
            RiskDecisionDto("use_recommended", "Use recommended settings"),
            RiskDecisionDto("dry_run", "Run a dry-run first"),
        )
        prepared_id = self._opaque_id()
        detail = self._song_detail(plan, risk)
        detail = SongDetailDto(
            song_id=song_id,
            title=detail.title,
            duration_us=detail.duration_us,
            note_count=detail.note_count,
            format_label=detail.format_label,
            risk=detail.risk,
            recommendation=detail.recommendation,
        )
        dto = PreparedPlaybackDto(
            prepared_id=prepared_id,
            song=detail,
            config=playback_config,
            admission="ready" if not decisions else "confirmation_required",
            risk=risk,
            decisions=decisions,
            plan_fingerprint=self._fingerprint(song_id, plan, playback_config),
        )
        record = _PreparedRecord(
            prepared_id=prepared_id,
            song_id=song_id,
            catalog_generation=generation,
            settings_fingerprint=self._settings_fingerprint(cfg),
            plan=plan,
            dto=dto,
        )
        with self._lock:
            self._prepared[prepared_id] = record
            while len(self._prepared) > MAX_PREPARED_PLANS:
                self._prepared.popitem(last=False)
        return asdict(dto)

    def _emit_state(
        self,
        active: _ActiveSession,
        state: PlaybackState,
        *,
        message: str | None = None,
        outcome: str | None = None,
    ) -> None:
        with self._lock:
            if self._active is not active:
                return
            if state != active.state and state not in _LEGAL_TRANSITIONS.get(active.state, frozenset()):
                raise DesktopPlaybackError(
                    "illegal_transition",
                    f"cannot transition playback from {active.state} to {state}",
                )
            active.state = state
        payload: dict[str, object] = {
            "session_id": active.session_id,
            "song_id": active.song_id,
            "state": state,
            "physical": not active.dry_run,
            "message": message,
            "outcome": outcome,
        }
        self._publish_event("playback.state_changed", payload)

    def _on_native_state(self, active: _ActiveSession, state: PlaybackState) -> None:
        with self._lock:
            if self._active is not active or active.state == state:
                return
            # A stop request owns the terminal transition. A late native
            # progress frame must not move the session out of Stopping.
            if active.state == "stopping":
                return
        self._emit_state(active, state)

    def _run(self, active: _ActiveSession) -> None:
        plan = active.plan
        try:
            engine = PlaybackEngine(
                song=plan.song,
                actions=plan.actions,
                dry_run=active.dry_run,
                controls=active.controls,
                renderer=_SnapshotRenderer(
                    self._publish_event,
                    active.session_id,
                    active.song_id,
                    plan.song.name,
                    on_state=lambda state: self._on_native_state(active, state),
                ),
                telemetry_enabled=bool(self._settings_service.snapshot().telemetry_enabled or active.dry_run),
                require_focus=not active.dry_run,
                hold_label=plan.session.display_hold_label(),
                hold_frames=plan.session.hold_frames,
                game_fps=int(plan.active_policy.fps),
                tempo_scale=plan.session.tempo_scale,
                focus_restore_grace_us=int(plan.active_policy.focus_restore_grace_us),
                min_hold_us=int(plan.active_policy.min_hold_us),
                min_release_gap_us=int(plan.active_policy.min_release_gap_us or 0),
                min_hold_margin_us=int(plan.active_policy.min_hold_margin_us),
                min_hold_margin_source=plan.active_policy.min_hold_margin_source,
                down_late_grace_us=int(plan.active_policy.down_late_grace_us),
                pre_roll_us=0,
            )
            if not active.dry_run and not engine.prepare_focus_for_playback():
                raise DesktopPlaybackError("focus_rejected", "The validated Sky window could not be focused")
            self._emit_state(active, "playing")
            outcome = str(engine.play())
            if outcome in (PLAYBACK_QUIT, PLAYBACK_SKIPPED):
                self._emit_state(active, "finished", outcome=outcome)
            elif outcome == PLAYBACK_SHUTDOWN_TIMEOUT:
                self._emit_state(active, "failed", message="native playback shutdown timed out", outcome=outcome)
            else:
                self._emit_state(active, "finished", outcome=outcome or PLAYBACK_FINISHED)
            self._publish_event(
                "playback.finished",
                asdict(
                    PlaybackFinishedDto(
                        session_id=active.session_id,
                        song_id=active.song_id,
                        outcome=outcome,
                        total_us=int(plan.sched_meta.playback_duration_us),
                        message="Playback finished" if outcome == PLAYBACK_FINISHED else f"Playback {outcome}",
                    )
                ),
            )
        except DesktopPlaybackError as exc:
            self._emit_state(active, "failed", message=exc.message)
            self._publish_event("playback.failed", {"session_id": active.session_id, "song_id": active.song_id, "code": exc.code, "message": exc.message})
        except (ImportError, NativeDispatchError, RuntimeError, ValueError) as exc:
            message = str(exc)[:MAX_EVENT_TEXT_BYTES]
            self._emit_state(active, "failed", message=message)
            self._publish_event("playback.failed", {"session_id": active.session_id, "song_id": active.song_id, "code": "native_error", "message": message})
        finally:
            with self._lock:
                if self._active is active:
                    self._last_terminal = (active.session_id, active.state)
                    self._active = None

    def start(self, *, prepared_id: str, decisions: list[Mapping[str, object]]) -> dict[str, object]:
        if type(prepared_id) is not str or len(prepared_id) != 32 or any(c not in "0123456789abcdef" for c in prepared_id):
            raise DesktopPlaybackError("invalid_params", "prepared_id is invalid")
        if not isinstance(decisions, list) or len(decisions) > MAX_DECISION_COUNT:
            raise DesktopPlaybackError("invalid_params", "decisions must be a bounded array")
        accepted: list[PlaybackDecisionAcceptanceDto] = []
        for item in decisions:
            if not isinstance(item, Mapping) or set(item) != {"decision", "accepted"} or type(item.get("decision")) is not str or type(item.get("accepted")) is not bool:
                raise DesktopPlaybackError("invalid_params", "each decision must contain decision and accepted")
            accepted.append(PlaybackDecisionAcceptanceDto(str(item["decision"]), bool(item["accepted"])))
        with self._lock:
            if self._active is not None:
                raise DesktopPlaybackError("session_active", "another playback session is active")
            record = self._prepared.get(prepared_id)
            if record is None:
                raise DesktopPlaybackError("prepared_not_found", "prepared playback is stale or already consumed")
            required = {item.decision for item in record.dto.decisions}
            selected = [item.decision for item in accepted if item.accepted]
            if record.dto.admission == "blocked":
                raise DesktopPlaybackError("playback_blocked", record.dto.error_message or "playback is blocked")
            if (
                record.dto.admission == "confirmation_required"
                and (
                    len(accepted) != 1
                    or len(selected) != 1
                    or accepted[0].decision not in required
                    or not accepted[0].accepted
                )
            ):
                raise DesktopPlaybackError("confirmation_required", "an exact risk decision is required")
            if record.dto.admission == "ready" and accepted:
                raise DesktopPlaybackError("invalid_confirmation", "ready playback accepts no risk decisions")
            selected_decision = selected[0] if selected else None
            effective_plan = record.plan
            if selected_decision == "use_recommended":
                effective_plan = prepare_playback(
                    record.plan.song,
                    record.plan.session.with_hold_frames(record.plan.risk_report.suggested_hold_frames).with_tempo(
                        record.plan.risk_report.suggested_tempo_scale
                    ),
                    record.plan.cfg,
                    is_dry_run=record.dto.config.dry_run,
                )
                if isinstance(effective_plan, PlaybackError):
                    raise DesktopPlaybackError("recommendation_failed", effective_plan.message)
            # Consume only after all admission checks and any recommendation
            # rebuild succeed. A rejected confirmation can therefore be
            # retried with the same immutable prepared plan.
            self._prepared.pop(prepared_id, None)
            session_id = self._opaque_id()
            active = _ActiveSession(
                session_id=session_id,
                prepared_id=prepared_id,
                song_id=record.song_id,
                plan=effective_plan,
                dry_run=record.dto.config.dry_run or (selected_decision == "dry_run"),
                controls=DesktopPlaybackControls(),
            )
            self._active = active
            self._emit_state(active, "starting")
            worker = threading.Thread(target=self._run, args=(active,), name="desktop-playback", daemon=True)
            active.worker = worker
            worker.start()
        return asdict(PlaybackSessionDto(session_id=session_id, prepared_id=prepared_id, song_id=record.song_id, state="starting"))

    def _require_active(self, session_id: str) -> _ActiveSession:
        if type(session_id) is not str or len(session_id) != 32:
            raise DesktopPlaybackError("invalid_params", "session_id is invalid")
        with self._lock:
            active = self._active
            if active is None:
                raise DesktopPlaybackError("no_active_session", "there is no active playback session")
            if active.session_id != session_id:
                raise DesktopPlaybackError("stale_session", "session_id is stale or foreign")
            return active

    def command(self, *, session_id: str, command: str) -> dict[str, object]:
        with self._lock:
            if (
                self._active is None
                and command == "stop"
                and self._last_terminal is not None
                and self._last_terminal[0] == session_id
            ):
                return {"accepted": True, "session_id": session_id, "state": self._last_terminal[1]}
        active = self._require_active(session_id)
        if command == "stop":
            if active.state not in {"starting", "playing", "paused"}:
                return {"accepted": True, "session_id": session_id, "state": active.state}
            self._emit_state(active, "stopping")
            active.controls.push("quit")
        elif command == "pause":
            if active.state != "playing":
                raise DesktopPlaybackError("illegal_transition", "pause requires a playing session")
            active.controls.push("pause")
        elif command == "resume":
            if active.state != "paused":
                raise DesktopPlaybackError("illegal_transition", "resume requires a paused session")
            active.controls.push("pause")
        elif command == "skip":
            if active.state not in {"starting", "playing", "paused"}:
                raise DesktopPlaybackError("illegal_transition", "skip requires an active session")
            active.controls.push("skip")
        else:
            raise DesktopPlaybackError("unknown_command", "unsupported playback command")
        return {"accepted": True, "session_id": session_id, "state": active.state}

    def shutdown(self, timeout: float = 5.0) -> bool:
        with self._lock:
            active = self._active
        if active is None:
            return True
        if active.state in {"starting", "playing", "paused"}:
            self._emit_state(active, "stopping")
            active.controls.push("quit")
        if active.worker is not None:
            active.worker.join(timeout=max(0.0, timeout))
            return not active.worker.is_alive()
        return True


__all__ = [
    "MAX_PREPARED_PLANS",
    "DesktopPlaybackControls",
    "DesktopPlaybackError",
    "DesktopPlaybackService",
]
