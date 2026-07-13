from __future__ import annotations

from collections import defaultdict, deque
from collections.abc import Callable
from dataclasses import dataclass
from enum import StrEnum

from sky_music.domain.scheduler_types import ActionKind, KeyAction


class GenerationStatus(StrEnum):
    SCHEDULED = "scheduled"
    ACTIVE = "active"
    RELEASE_PENDING = "release_pending"
    RELEASED = "released"
    DROPPED_CONFLICT = "dropped_conflict"
    DROPPED_BACKEND = "dropped_backend"
    DROPPED_EXPIRED = "dropped_expired"
    CANCELLED = "cancelled"


GENERATION_STATUSES: tuple[GenerationStatus, ...] = tuple(GenerationStatus)


@dataclass(frozen=True, slots=True)
class RuntimeKeyIntent:
    source_action_index: int
    generation_id: int | None
    kind: ActionKind
    scan_code: int
    scheduled_us: int
    reason: str


@dataclass(frozen=True, slots=True)
class RuntimeActionBatch:
    source_action_index: int
    kind: ActionKind
    scheduled_us: int
    reason: str
    intents: tuple[RuntimeKeyIntent, ...]


@dataclass(frozen=True, slots=True)
class RuntimeSchedule:
    batches: tuple[RuntimeActionBatch, ...]
    generation_count: int


@dataclass(frozen=True, slots=True)
class ActiveKeyGeneration:
    generation_id: int
    scan_code: int
    source_action_index: int
    scheduled_down_us: int
    down_dispatch_started_us: int
    down_dispatch_completed_us: int
    release_not_before_us: int


@dataclass(frozen=True, slots=True)
class PendingRelease:
    generation_id: int
    scan_code: int
    source_action_index: int
    scheduled_release_us: int
    down_dispatch_started_us: int
    release_not_before_us: int
    reason: str

    @property
    def effective_release_us(self) -> int:
        return max(self.scheduled_release_us, self.release_not_before_us)

    def get_effective_release_us(self, lead_up: int = 0) -> int:
        return max(self.scheduled_release_us - lead_up, self.release_not_before_us)


def compile_runtime_intents(actions: tuple[KeyAction, ...]) -> RuntimeSchedule:
    """Attach stable per-key generations to an already-built scheduler timeline."""
    next_generation_id = 0
    unmatched_downs: dict[int, deque[int]] = defaultdict(deque)
    batches: list[RuntimeActionBatch] = []

    for source_action_index, action in enumerate(actions):
        intents: list[RuntimeKeyIntent] = []
        for scan_code in action.scan_codes:
            generation_id: int | None
            if action.kind == "down":
                generation_id = next_generation_id
                next_generation_id += 1
                unmatched_downs[scan_code].append(generation_id)
            else:
                queue = unmatched_downs[scan_code]
                generation_id = queue.popleft() if queue else None

            intents.append(
                RuntimeKeyIntent(
                    source_action_index=source_action_index,
                    generation_id=generation_id,
                    kind=action.kind,
                    scan_code=scan_code,
                    scheduled_us=int(action.at_us),
                    reason=action.reason,
                )
            )

        batches.append(
            RuntimeActionBatch(
                source_action_index=source_action_index,
                kind=action.kind,
                scheduled_us=int(action.at_us),
                reason=action.reason,
                intents=tuple(intents),
            )
        )

    return RuntimeSchedule(
        batches=tuple(batches),
        generation_count=next_generation_id,
    )


class RuntimeDispatchCoordinator:
    """Owns per-key runtime generations and release eligibility."""

    def __init__(self, schedule: RuntimeSchedule, min_hold_us: int) -> None:
        self.schedule = schedule
        self.min_hold_us = max(0, int(min_hold_us))
        self.cursor = 0
        self.active_by_scan_code: dict[int, ActiveKeyGeneration] = {}
        # Live status dict: holds ONLY generations in a non-terminal state
        # (ACTIVE / RELEASE_PENDING), so its size is bounded by live polyphony —
        # a few dozen entries — not by the song length. SCHEDULED is the implicit
        # default for any generation_id < generation_count that has no entry.
        # Terminal states (RELEASED / DROPPED_* / CANCELLED) are folded into O(1)
        # running counters the instant a generation reaches them (see _terminalize),
        # so per-note state is never retained for the whole song. This is the
        # difference between O(note_count) and O(polyphony) resident memory during
        # playback — the former made RSS climb steadily and never drop for dense songs.
        self.status_by_generation: dict[int, GenerationStatus] = {}
        self._terminal_counts: dict[GenerationStatus, int] = {}
        self._generation_count: int = schedule.generation_count
        self.pending_by_generation: dict[int, PendingRelease] = {}
        self.pending_scan_codes: set[int] = set()

    def _terminalize(self, generation_id: int, status: GenerationStatus) -> None:
        """Fold a generation into its terminal-status counter and drop its live entry.

        Terminal states are final (a generation never transitions out of RELEASED /
        DROPPED_* / CANCELLED), so the per-generation entry carries no further
        information once reached — counting it and dropping it keeps the live dict
        bounded by polyphony while the aggregate counts stay exact.
        """
        self.status_by_generation.pop(generation_id, None)
        self._terminal_counts[status] = self._terminal_counts.get(status, 0) + 1

    def _early_pop_blocked(self, batch: RuntimeActionBatch) -> bool:
        """No-early-conflict guard predicate: a down batch may not be popped BEFORE its authored
        time while any of its scan codes is still active or pending release — an early pop would
        turn dispatch lead into a dropped_conflict (a lost note)."""
        if batch.kind != "down":
            return False
        active = self.active_by_scan_code
        pending = self.pending_scan_codes
        if not active and not pending:
            return False
        return any(
            intent.scan_code in active or intent.scan_code in pending
            for intent in batch.intents
        )

    def next_authored_us(
        self,
        dispatch_lead_us: int = 0,
        *,
        lead_for_batch: Callable[[RuntimeActionBatch], int] | None = None,
    ) -> int | None:
        if self.cursor >= len(self.schedule.batches):
            return None
        batch = self.schedule.batches[self.cursor]
        lead = dispatch_lead_us
        if lead_for_batch is not None:
            lead = lead_for_batch(batch)
        # Guard-aware deadline: while the early pop is blocked, the batch only becomes poppable at
        # its authored time, so report that instead of scheduled - lead. Otherwise the engine
        # would wake at the led deadline, drain nothing, and busy-loop until the blocking release
        # fires (and a fake-clock test would hang forever).
        if lead > 0 and self._early_pop_blocked(batch):
            return batch.scheduled_us
        return max(0, batch.scheduled_us - lead)

    def next_pending_release_us(self, lead_up: int = 0) -> int | None:
        if not self.pending_by_generation:
            return None
        return min(pending.get_effective_release_us(lead_up) for pending in self.pending_by_generation.values())

    def next_deadline_us(
        self,
        dispatch_lead_us: int = 0,
        lead_up: int = 0,
        *,
        lead_for_batch: Callable[[RuntimeActionBatch], int] | None = None,
    ) -> int | None:
        authored = self.next_authored_us(dispatch_lead_us, lead_for_batch=lead_for_batch)
        pending = self.next_pending_release_us(lead_up)
        if authored is None:
            return pending
        if pending is None:
            return authored
        return authored if authored < pending else pending

    def is_finished(self) -> bool:
        return self.cursor >= len(self.schedule.batches) and not self.pending_by_generation

    def generation_status_counts(self) -> dict[str, int]:
        """Return counts for every runtime generation terminal/intermediate status."""
        # Terminal states live in the O(1) counters; the live dict holds only the
        # non-terminal (ACTIVE / RELEASE_PENDING) generations. The remainder
        # (generation_count minus terminals minus live non-terminals) are still
        # implicitly SCHEDULED. This yields the same distribution the old
        # per-generation dict produced, at O(polyphony) instead of O(note_count).
        counts: dict[GenerationStatus, int] = dict(self._terminal_counts)
        nonterminal = 0
        for status in self.status_by_generation.values():
            counts[status] = counts.get(status, 0) + 1
            nonterminal += 1
        terminal_total = sum(self._terminal_counts.values())
        implicit_scheduled = self._generation_count - terminal_total - nonterminal
        if implicit_scheduled > 0:
            counts[GenerationStatus.SCHEDULED] = counts.get(GenerationStatus.SCHEDULED, 0) + implicit_scheduled
        return {status.value: counts.get(status, 0) for status in GENERATION_STATUSES}

    def pop_due_pending(self, now_us: int, lead_up: int = 0) -> tuple[PendingRelease, ...]:
        if not self.pending_by_generation:
            return ()
        due = [
            pending
            for pending in self.pending_by_generation.values()
            if pending.get_effective_release_us(lead_up) <= now_us
        ]
        if not due:
            return ()
        due.sort(
            key=lambda pending: (
                pending.get_effective_release_us(lead_up),
                pending.source_action_index,
                pending.scan_code,
            ),
        )
        for pending in due:
            self.pending_by_generation.pop(pending.generation_id, None)
            self.pending_scan_codes.discard(pending.scan_code)
        return tuple(due)

    def pop_due_authored(
        self,
        now_us: int,
        dispatch_lead_us: int = 0,
        *,
        lead_for_batch: Callable[[RuntimeActionBatch], int] | None = None,
    ) -> tuple[RuntimeActionBatch, ...]:
        due: list[RuntimeActionBatch] = []
        while self.cursor < len(self.schedule.batches):
            batch = self.schedule.batches[self.cursor]
            lead = dispatch_lead_us
            if lead_for_batch is not None:
                lead = lead_for_batch(batch)
            if batch.scheduled_us > now_us + lead:
                break
            if batch.scheduled_us > now_us and self._early_pop_blocked(batch):
                # Cannot pop early; stop popping to preserve timeline order. The batch pops
                # normally once now_us reaches its authored time (degraded conflict handling in
                # split_down_intents then applies as before lead existed).
                break

            due.append(batch)
            self.cursor += 1
        return tuple(due)

    def activate_sent_downs(
        self,
        intents: tuple[RuntimeKeyIntent, ...],
        sent_scan_codes: tuple[int, ...],
        *,
        dispatch_started_us: int,
        dispatch_completed_us: int,
    ) -> None:
        # Single-key/codes fast path: skip the set allocation. The dispatch loop already collapses
        # small chords through this path; avoiding the per-down set allocation removes one of the
        # ~note-rate heap allocations that made resident RSS tick up under long sessions.
        if len(sent_scan_codes) == 1:
            only_sent = sent_scan_codes[0]
            for intent in intents:
                generation_id = intent.generation_id
                if generation_id is None:
                    continue
                if intent.scan_code != only_sent:
                    self._terminalize(generation_id, GenerationStatus.DROPPED_BACKEND)
                    continue
                self.active_by_scan_code[intent.scan_code] = ActiveKeyGeneration(
                    generation_id=generation_id,
                    scan_code=intent.scan_code,
                    source_action_index=intent.source_action_index,
                    scheduled_down_us=intent.scheduled_us,
                    down_dispatch_started_us=dispatch_started_us,
                    down_dispatch_completed_us=dispatch_completed_us,
                    release_not_before_us=dispatch_completed_us + self.min_hold_us,
                )
                self.status_by_generation[generation_id] = GenerationStatus.ACTIVE
            return
        sent = set(sent_scan_codes)
        for intent in intents:
            if intent.generation_id is None:
                continue
            if intent.scan_code not in sent:
                self._terminalize(intent.generation_id, GenerationStatus.DROPPED_BACKEND)
                continue
            self.active_by_scan_code[intent.scan_code] = ActiveKeyGeneration(
                generation_id=intent.generation_id,
                scan_code=intent.scan_code,
                source_action_index=intent.source_action_index,
                scheduled_down_us=intent.scheduled_us,
                down_dispatch_started_us=dispatch_started_us,
                down_dispatch_completed_us=dispatch_completed_us,
                # Anchor the visibility floor to the down DISPATCH COMPLETION.
                #
                # Telemetry shows the game-observed hold follows completion-to-completion: a
                # start-anchored floor subtracts the down SendInput latency from every note, leaving
                # roughly half of 1-frame local_precise notes below visibility at 144fps. With the
                # completion anchor, observed hold is min_hold plus the up SendInput duration.
                #
                # Same-key feasibility is kept honest in the scheduler by requiring
                # interval >= min_hold; the anchor itself is only the visibility rule.
                # See docs/timing-principles.md §7.
                release_not_before_us=dispatch_completed_us + self.min_hold_us,
            )
            self.status_by_generation[intent.generation_id] = GenerationStatus.ACTIVE

    def split_down_intents(
        self,
        intents: tuple[RuntimeKeyIntent, ...],
    ) -> tuple[tuple[RuntimeKeyIntent, ...], tuple[RuntimeKeyIntent, ...]]:
        active = self.active_by_scan_code
        # Fast path: nothing is held, so no down can conflict — the whole (already-immutable) batch
        # is playable. Skips two list allocations + a per-intent membership scan on the hot path.
        if not active:
            return intents, ()
        playable: list[RuntimeKeyIntent] = []
        conflicts: list[RuntimeKeyIntent] = []
        for intent in intents:
            if intent.scan_code in active:
                conflicts.append(intent)
                if intent.generation_id is not None:
                    self._terminalize(intent.generation_id, GenerationStatus.DROPPED_CONFLICT)
            else:
                playable.append(intent)
        return tuple(playable), tuple(conflicts)

    def drop_expired_downs(self, intents: tuple[RuntimeKeyIntent, ...]) -> None:
        for intent in intents:
            if intent.generation_id is not None:
                self._terminalize(intent.generation_id, GenerationStatus.DROPPED_EXPIRED)

    def request_releases(
        self,
        intents: tuple[RuntimeKeyIntent, ...],
    ) -> tuple[tuple[PendingRelease, ...], tuple[RuntimeKeyIntent, ...]]:
        # Hot path: the common case is a single key-up with a known active generation. Skip the
        # list allocations and the add-set work by detuning the codepath to a scalar return shape
        # that matches the dispatcher's typical release batch.
        if len(intents) == 1:
            intent = intents[0]
            generation_id = intent.generation_id
            if generation_id is None:
                return (), (intent,)
            active = self.active_by_scan_code.get(intent.scan_code)
            if active is None or active.generation_id != generation_id:
                return (), (intent,)
            pending = PendingRelease(
                generation_id=generation_id,
                scan_code=intent.scan_code,
                source_action_index=intent.source_action_index,
                scheduled_release_us=intent.scheduled_us,
                down_dispatch_started_us=active.down_dispatch_started_us,
                release_not_before_us=active.release_not_before_us,
                reason=intent.reason,
            )
            self.pending_by_generation[generation_id] = pending
            self.pending_scan_codes.add(intent.scan_code)
            self.status_by_generation[generation_id] = GenerationStatus.RELEASE_PENDING
            return (pending,), ()
        requested: list[PendingRelease] = []
        suppressed: list[RuntimeKeyIntent] = []
        for intent in intents:
            generation_id = intent.generation_id
            if generation_id is None:
                suppressed.append(intent)
                continue
            active = self.active_by_scan_code.get(intent.scan_code)
            if active is None or active.generation_id != generation_id:
                suppressed.append(intent)
                continue
            pending = PendingRelease(
                generation_id=generation_id,
                scan_code=intent.scan_code,
                source_action_index=intent.source_action_index,
                scheduled_release_us=intent.scheduled_us,
                down_dispatch_started_us=active.down_dispatch_started_us,
                release_not_before_us=active.release_not_before_us,
                reason=intent.reason,
            )
            self.pending_by_generation[generation_id] = pending
            self.pending_scan_codes.add(intent.scan_code)
            self.status_by_generation[generation_id] = GenerationStatus.RELEASE_PENDING
            requested.append(pending)
        return tuple(requested), tuple(suppressed)

    def complete_releases(
        self,
        releases: tuple[PendingRelease, ...],
        sent_scan_codes: tuple[int, ...],
        skipped_scan_codes: tuple[int, ...] = (),
    ) -> None:
        # Hot path: a single-release dispatch is the dominant case in real songs. Avoid the two
        # set allocations and use direct tuple scans instead — set() on a 1-tuple costs more than
        # an `in` over the sent/skipped tuples.
        if len(releases) == 1:
            pending = releases[0]
            if (
                pending.scan_code in sent_scan_codes
                or pending.scan_code in skipped_scan_codes
            ):
                active = self.active_by_scan_code.get(pending.scan_code)
                if active is not None and active.generation_id == pending.generation_id:
                    self.active_by_scan_code.pop(pending.scan_code, None)
                self._terminalize(
                    pending.generation_id,
                    GenerationStatus.RELEASED if pending.scan_code in sent_scan_codes else GenerationStatus.DROPPED_BACKEND,
                )
            return
        sent = set(sent_scan_codes)
        skipped = set(skipped_scan_codes)
        for pending in releases:
            if pending.scan_code not in sent and pending.scan_code not in skipped:
                continue
            active = self.active_by_scan_code.get(pending.scan_code)
            if active is not None and active.generation_id == pending.generation_id:
                self.active_by_scan_code.pop(pending.scan_code, None)
            self._terminalize(
                pending.generation_id,
                GenerationStatus.RELEASED if pending.scan_code in sent else GenerationStatus.DROPPED_BACKEND,
            )

    def cancel_all(self) -> tuple[int, ...]:
        active_gen_ids = {active.generation_id for active in self.active_by_scan_code.values()}
        cancelled = tuple(sorted(active_gen_ids))
        # A RELEASE_PENDING generation is still present in active_by_scan_code (its down was
        # never popped), so the two sets overlap — union so each cancelled generation is
        # counted exactly once (matching the old dict, where the second write was idempotent).
        for generation_id in active_gen_ids | self.pending_by_generation.keys():
            self._terminalize(generation_id, GenerationStatus.CANCELLED)
        self.active_by_scan_code.clear()
        self.pending_by_generation.clear()
        self.pending_scan_codes.clear()
        return cancelled
