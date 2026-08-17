import math
from dataclasses import dataclass
from typing import Literal

from sky_music.domain.domain import InstrumentProfile, Microseconds, NoteKey, Song
from sky_music.domain.scheduler_types import (
    ActionKind,
    FrameTimingPolicy,
    KeyAction,
    ScheduleDiagnostic,
    ScheduleMetadata,
)
from sky_music.layouts import DefaultNoteResolver, NoteResolver


class ScheduleBuildError(ValueError):
    """Raised when the schedule cannot be built due to strict conflict policies."""
    def __init__(self, message: str, recommended_tempo_scale: float | None = None, recommended_hold_frames: float | None = None):
        super().__init__(message)
        self.recommended_tempo_scale = recommended_tempo_scale
        self.recommended_hold_frames = recommended_hold_frames


def _recommended_tempo_scale_for_repeats(
    shortest_interval_us: int | None,
    policy: FrameTimingPolicy,
    tempo_scale: float,
) -> float | None:
    if shortest_interval_us is None or shortest_interval_us <= 0:
        return None
    min_cycle_us = int(policy.min_hold_us)
    if shortest_interval_us >= min_cycle_us:
        return None
    suggested = tempo_scale * shortest_interval_us / min_cycle_us
    return max(0.1, min(1.0, round(suggested, 2)))

@dataclass(frozen=True, slots=True)
class ScheduledNoteDraft:
    at_us: int
    note_key: NoteKey
    scan_code: int
    source_index: int


@dataclass(frozen=True, slots=True)
class PlannedKeyHold:
    hold_us: int
    risk: Literal["ok", "moderate", "severe"]
    effective_delta_us: int | None = None
    compressed: bool = False


@dataclass(frozen=True, slots=True)
class RawKeyEvent:
    at_us: int
    scan_code: int
    kind: ActionKind
    reason: str


def normalise_note_drafts(drafts: list[ScheduledNoteDraft]) -> tuple[ScheduledNoteDraft, ...]:
    """Remove exact same-key timestamp duplicates while preserving intentional chords."""
    sorted_drafts = sorted(drafts, key=lambda draft: draft.at_us)
    deduped_drafts: list[ScheduledNoteDraft] = []
    seen_note_slots: set[tuple[int, int]] = set()
    for draft in sorted_drafts:
        slot = (draft.at_us, draft.scan_code)
        if slot in seen_note_slots:
            continue
        seen_note_slots.add(slot)
        deduped_drafts.append(draft)
    return tuple(deduped_drafts)


def next_same_key_times(drafts: tuple[ScheduledNoteDraft, ...]) -> dict[int, int | None]:
    next_time_by_source_index: dict[int, int | None] = {}
    last_seen_by_key: dict[int, int] = {}
    for idx in range(len(drafts) - 1, -1, -1):
        draft = drafts[idx]
        next_time_by_source_index[draft.source_index] = last_seen_by_key.get(draft.scan_code)
        last_seen_by_key[draft.scan_code] = draft.at_us
    return next_time_by_source_index


def plan_same_key_hold(
    *,
    target_hold_us: int,
    min_hold_us: int,
    effective_delta_us: int | None,
) -> PlannedKeyHold:
    if effective_delta_us is None:
        return PlannedKeyHold(hold_us=target_hold_us, risk="ok")

    max_hold_us = effective_delta_us
    # Feasibility floor is exactly the authored min_hold. A same-key repeat
    # whose interval is below that hold is rejected for real playback during
    # preparation; the runtime never derives a release target from completion.
    feasibility_floor_us = min_hold_us
    if max_hold_us < feasibility_floor_us:
        return PlannedKeyHold(
            hold_us=min_hold_us,
            risk="severe",
            effective_delta_us=effective_delta_us,
            compressed=min_hold_us < target_hold_us,
        )

    if max_hold_us < target_hold_us:
        return PlannedKeyHold(
            hold_us=max_hold_us,
            risk="moderate",
            effective_delta_us=effective_delta_us,
            compressed=True,
        )

    return PlannedKeyHold(
        hold_us=target_hold_us,
        risk="ok",
        effective_delta_us=effective_delta_us,
    )


def build_key_actions(
    song: Song,
    profile: InstrumentProfile | None = None,
    policy: FrameTimingPolicy | None = None,
    scan_code_mode: str = "physical",
    resolver: NoteResolver | None = None,
    tempo_scale: float = 1.0
) -> ScheduleMetadata:
    """
    Builds a microsecond-accurate event timeline from a domain Song.
    Returns a ScheduleMetadata containing:
      - actions: tuple of sorted KeyAction objects

    Caller must pass a resolved FrameTimingPolicy (e.g. via PlaybackSessionContext.resolve_effective_policy).

    Same-key conflict policies are retained for dry-run diagnostics and
    persisted configuration compatibility. Real playback rejects an
    infeasible same-key repeat during preparation; native dispatch never
    drops a chord or performs a degraded runtime retry.
    """
    if tempo_scale <= 0:
        raise ValueError("tempo_scale must be > 0")

    if policy is None:
        policy = FrameTimingPolicy.from_hold_frames(1.0, 60)
    elif not isinstance(policy, FrameTimingPolicy):
        raise TypeError(
            "build_key_actions requires FrameTimingPolicy; "
            "resolve via FrameTimingPolicy.from_timing_policy() or "
            "PlaybackSessionContext.resolve_effective_policy() before calling."
        )

    if profile is None:
        from sky_music.layouts import SKY_15_KEY_PROFILE
        profile = SKY_15_KEY_PROFILE

    if resolver is None:
        resolver = DefaultNoteResolver(profile)

    compressed_holds = 0
    impossible_same_key_repeats = 0
    risky_same_key_repeats = 0
    sub_60fps_frame_notes = 0
    gap_below_frame_repeats = 0
    shortest_same_key_interval_us = None
    min_same_key_up_gap_us = None
    note_count = len(song.notes)
    
    # Stage 1: normalise note intents after tempo scaling and scan-code resolution.
    drafts: list[ScheduledNoteDraft] = []
    for idx, note in enumerate(song.notes):
        k = note.key
        if k.startswith("1Key") or k.startswith("2Key") or k.startswith("3Key"):
            k = NoteKey("Key" + k[4:])
        
        # Scale to microseconds
        source_time_us = round(note.time_ms * 1000 / tempo_scale)
        
        # Resolve scan code
        sc = resolver.resolve_scan_code(k, scan_code_mode)
        if sc <= 0:
            raise ValueError(
                f"Cannot map note key {k!r} to a scan code "
                f"(scan_code_mode={scan_code_mode!r}, profile={profile.name!r})"
            )
            
        drafts.append(ScheduledNoteDraft(
            at_us=source_time_us,
            note_key=k,
            scan_code=sc,
            source_index=idx
        ))
        
    raw_draft_count = len(drafts)
    drafts = normalise_note_drafts(drafts)  # type: ignore[assignment]
    deduplicated_note_count = len(drafts)
    duplicate_note_count = raw_draft_count - deduplicated_note_count
    
    # Stage 2: plan each physical-key lane and emit typed raw key events.
    next_same_key_time = next_same_key_times(tuple(drafts))

    raw_events: list[RawKeyEvent] = []
    diagnostics: list[ScheduleDiagnostic] = []
    
    logical_bounds: list[tuple[int, int, int]] = [] # (time, kind(0=down, 1=up), scan_code)

    for draft in drafts:
        next_same_info = next_same_key_time[draft.source_index]
        sc = draft.scan_code
        down_at_us = draft.at_us
        
        effective_delta_us = None
        if next_same_info is not None:
            effective_delta_us = next_same_info - down_at_us
            
            if shortest_same_key_interval_us is None or effective_delta_us < shortest_same_key_interval_us:
                shortest_same_key_interval_us = effective_delta_us

        planned_hold = plan_same_key_hold(
            target_hold_us=int(policy.hold_us),
            min_hold_us=int(policy.min_hold_us),
            effective_delta_us=effective_delta_us,
        )

        if planned_hold.risk == "severe":
            impossible_same_key_repeats += 1
            assert effective_delta_us is not None
            diagnostics.append(ScheduleDiagnostic(
                source_index=draft.source_index,
                note_key=draft.note_key,
                scan_code=draft.scan_code,
                code="impossible_repeat",
                message=(
                    f"Repeat too fast: {effective_delta_us / 1000:.1f}ms interval < "
                    f"{int(policy.min_hold_us) / 1000:.1f}ms min_hold; preserving min_hold means "
                    "the next same-key down occurs before the previous release."
                )
            ))
            if policy.same_key_conflict_policy == "strict":
                raise ScheduleBuildError(
                    f"Cannot build schedule under strict policy: same-key repeat interval "
                    f"{effective_delta_us / 1000:.1f}ms is below min_hold "
                    f"{int(policy.min_hold_us) / 1000:.1f}ms.",
                    recommended_tempo_scale=_recommended_tempo_scale_for_repeats(
                        effective_delta_us, policy, tempo_scale
                    ),
                    recommended_hold_frames=1.0,
                )
        elif planned_hold.risk == "moderate":
            risky_same_key_repeats += 1

        actual_hold = planned_hold.hold_us
        if actual_hold < math.ceil(1_000_000 / 60):
            sub_60fps_frame_notes += 1

        if planned_hold.compressed:
            compressed_holds += 1

        if effective_delta_us is not None:
            same_key_up_gap_us = effective_delta_us - actual_hold
            if min_same_key_up_gap_us is None or same_key_up_gap_us < min_same_key_up_gap_us:
                min_same_key_up_gap_us = same_key_up_gap_us

            if (
                int(policy.frame_us) > 0
                and planned_hold.risk != "severe"
                and same_key_up_gap_us < int(policy.frame_us)
            ):
                gap_below_frame_repeats += 1
                diagnostics.append(ScheduleDiagnostic(
                    source_index=draft.source_index,
                    note_key=draft.note_key,
                    scan_code=draft.scan_code,
                    code="gap_below_frame",
                    message=(
                        f"Release-to-repress gap {same_key_up_gap_us / 1000:.1f}ms is below one frame "
                        f"({int(policy.frame_us) / 1000:.1f}ms); the game may sample the key as continuously "
                        "held and miss the repeat."
                    ),
                ))
            
        up_at_us = down_at_us + actual_hold
        
        raw_events.append(RawKeyEvent(
            at_us=down_at_us,
            scan_code=sc,
            kind=ActionKind.DOWN,
            reason="onset",
        ))
        raw_events.append(RawKeyEvent(
            at_us=up_at_us,
            scan_code=sc,
            kind=ActionKind.UP,
            reason="repeat_release" if next_same_info is not None else "release",
        ))
        
        logical_bounds.append((down_at_us, 0, sc))
        logical_bounds.append((up_at_us, 1, sc))

    # Stage 3: group simultaneous events by (time, kind).  Reason is dropped from the key so
    # releases with different origins (e.g. "release" vs "repeat_release") at the same
    # microsecond collapse into a single KeyAction — fewer backend calls, no behavioural change.
    grouped: dict[tuple[int, ActionKind], list[tuple[int, str]]] = {}
    for ev in raw_events:
        g_key = (ev.at_us, ev.kind)
        if g_key not in grouped:
            grouped[g_key] = []
        grouped[g_key].append((ev.scan_code, ev.reason))

    key_actions_list: list[KeyAction] = []
    for (at_us, kind), items in grouped.items():
        scs = [sc for sc, _ in items]
        reasons = {r for _, r in items}
        unique_scs = tuple(dict.fromkeys(scs))
        reason = reasons.pop() if len(reasons) == 1 else "mixed"
        key_actions_list.append(KeyAction(
            at_us=Microseconds(at_us),
            scan_codes=unique_scs,  # type: ignore[arg-type]
            kind=kind,
            reason=reason
        ))
        
    # Stage 4: sort the executable timeline with up-before-down priority.
    key_actions_list.sort(key=lambda a: (a.at_us, a.kind == "down"))
    
    # Stage 5: derive metrics from the executable timeline.
    # Derive logical chord size from the authored executable timeline.
    logical_bounds.sort(key=lambda b: (b[0], b[1]))
    logical_active: set[int] = set()
    max_polyphony = 0
    for _t, kind, sc in logical_bounds:
        if kind == 0:
            logical_active.add(sc)
            max_polyphony = max(max_polyphony, len(logical_active))
        else:
            logical_active.discard(sc)
            
    warnings = []
    if impossible_same_key_repeats > 0:
        warnings.append(
            f"Detected {impossible_same_key_repeats} infeasible same-key repeat(s): "
            "authored interval is below min_hold; the next same-key down "
            "preserves min_hold and overlaps the previous release."
        )
    if risky_same_key_repeats > 0:
        warnings.append(
            f"Compressed {risky_same_key_repeats} same-key hold(s) to release before the next down."
        )
    if compressed_holds > 0:
        warnings.append(f"Compressed {compressed_holds} note hold(s) due to same-key scheduling pressure.")
    if sub_60fps_frame_notes > 0 and policy.fps > 60:
        warnings.append(
            f"This hold selection assumes {policy.fps} FPS. {sub_60fps_frame_notes} short note(s) are shorter "
            f"than one 60 fps frame; if your game runs below {policy.fps} fps they may not register. "
            "Lower the configured FPS to match Sky or use a longer hold for visibility."
        )
    if gap_below_frame_repeats > 0:
        warnings.append(
            f"Detected {gap_below_frame_repeats} same-key repeat(s) whose release-to-repress gap is "
            f"below one game frame ({int(policy.frame_us) / 1000:.1f}ms). The game may miss these repeats; "
            "consider lowering the tempo scale or accepting probabilistic repeat registration."
        )

    duration_us = Microseconds(key_actions_list[-1].at_us) if key_actions_list else Microseconds(0)
    playback_duration_us = duration_us
    source_duration_us = Microseconds(max((d.at_us for d in drafts), default=0) + policy.hold_us)

    rec_tempo_scale = None
    rec_hold_frames = None
    if impossible_same_key_repeats > 0 and shortest_same_key_interval_us is not None:
        rec_tempo_scale = _recommended_tempo_scale_for_repeats(
            shortest_same_key_interval_us, policy, tempo_scale
        )
        rec_hold_frames = 1.0

    return ScheduleMetadata(
        actions=tuple(key_actions_list),
        compressed_holds=compressed_holds,
        impossible_same_key_repeats=impossible_same_key_repeats,
        max_polyphony=max_polyphony,
        note_count=note_count,
        deduplicated_note_count=deduplicated_note_count,
        duplicate_note_count=duplicate_note_count,
        duration_us=duration_us,
        warnings=tuple(warnings),
        risky_same_key_repeats=risky_same_key_repeats,
        shortest_same_key_interval_us=shortest_same_key_interval_us,
        min_same_key_up_gap_us=min_same_key_up_gap_us,
        source_duration_us=source_duration_us,
        playback_duration_us=playback_duration_us,
        diagnostics=tuple(diagnostics),
        recommended_hold_frames=rec_hold_frames,
        recommended_tempo_scale=rec_tempo_scale,
        sub_60fps_frame_notes=sub_60fps_frame_notes,
        gap_below_frame_repeats=gap_below_frame_repeats,
    )
