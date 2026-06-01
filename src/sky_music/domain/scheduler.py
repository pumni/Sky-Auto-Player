from dataclasses import dataclass
from sky_music.domain.domain import Song, InstrumentProfile, NoteKey
from sky_music.layouts import NoteResolver, DefaultNoteResolver
from sky_music.domain.scheduler_types import KeyAction, ScheduleMetadata, Microseconds, FrameTimingPolicy, ScheduleDiagnostic, align_frame_down_us

class ScheduleBuildError(ValueError):
    """Raised when the schedule cannot be built due to strict conflict policies."""
    def __init__(self, message: str, recommended_tempo_scale: float | None = None, recommended_profile: str | None = None):
        super().__init__(message)
        self.recommended_tempo_scale = recommended_tempo_scale
        self.recommended_profile = recommended_profile


def _recommended_tempo_scale_for_repeats(
    shortest_interval_us: int | None,
    policy: FrameTimingPolicy,
    tempo_scale: float,
) -> float | None:
    if shortest_interval_us is None or shortest_interval_us <= 0:
        return None
    min_cycle_us = policy.min_hold_us + policy.repeat_release_gap_us
    if shortest_interval_us >= min_cycle_us:
        return None
    suggested = tempo_scale * shortest_interval_us / min_cycle_us
    return max(0.1, min(1.0, round(suggested, 2)))

@dataclass(frozen=True, slots=True)
class ScheduledNoteDraft:
    source_time_us: int
    snapped_time_us: int
    shifted_time_us: int
    down_time_us: int
    note_key: NoteKey
    scan_code: int
    source_index: int

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
    """
    if tempo_scale <= 0:
        raise ValueError("tempo_scale must be > 0")

    if policy is None:
        policy = FrameTimingPolicy.balanced()
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
    shortest_same_key_interval_us = None
    note_count = len(song.notes)
    
    # 1. Normalize and resolve NoteKeys to physical scan codes & Apply tempo scaling
    drafts = []
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
            source_time_us=source_time_us,
            snapped_time_us=source_time_us,
            shifted_time_us=source_time_us,
            down_time_us=source_time_us,
            note_key=k,
            scan_code=sc,
            source_index=idx
        ))
        
    # Sort chronologically by original source time
    drafts.sort(key=lambda d: d.source_time_us)

    # 2. Merge chords using tolerance window (chord_merge_window_us) based on physical scan_codes
    merged_drafts = []
    current_groups = [] # list of tuples: (group_time_us, set_of_scan_codes_in_group)
    
    for draft in drafts:
        snapped = False
        target_snapped_time = draft.source_time_us

        if policy.chord_merge_window_us > 0:
            # Keep the active chord-group window small even when many notes snap
            # into existing groups. Previously stale groups were only pruned when
            # creating a new group, which made dense songs do extra linear scans.
            current_groups = [
                group
                for group in current_groups
                if draft.source_time_us <= group[0] + policy.chord_merge_window_us
            ]
            for g_time_us, scan_codes_in_group in current_groups:
                if draft.source_time_us <= g_time_us + policy.chord_merge_window_us:
                    if draft.scan_code not in scan_codes_in_group:
                        target_snapped_time = g_time_us
                        scan_codes_in_group.add(draft.scan_code)
                        snapped = True
                        break

        if not snapped:
            g_time_us = draft.source_time_us
            target_snapped_time = g_time_us
            current_groups.append((g_time_us, {draft.scan_code}))
            
        # Recreate frozen draft with the computed snapped and shifted times
        shifted_time_us = max(0, target_snapped_time - policy.input_lead_us)
        merged_drafts.append(ScheduledNoteDraft(
            source_time_us=draft.source_time_us,
            snapped_time_us=target_snapped_time,
            shifted_time_us=shifted_time_us,
            down_time_us=shifted_time_us,
            note_key=draft.note_key,
            scan_code=draft.scan_code,
            source_index=draft.source_index
        ))
        
    # 3. Apply optional frame alignment to the effective key-down timestamps.
    # Same-key repeat constraints must use this final down time, not the pre-align time.
    aligned_drafts = []
    for draft in merged_drafts:
        down_time_us = draft.shifted_time_us
        if policy.frame_align == "down_only" and policy.frame_us > 0:
            down_time_us = align_frame_down_us(down_time_us, policy.frame_us, policy.frame_align)
        aligned_drafts.append(ScheduledNoteDraft(
            source_time_us=draft.source_time_us,
            snapped_time_us=draft.snapped_time_us,
            shifted_time_us=draft.shifted_time_us,
            down_time_us=down_time_us,
            note_key=draft.note_key,
            scan_code=draft.scan_code,
            source_index=draft.source_index
        ))

    # 4. Sort merged_drafts chronologically by final key-down time
    merged_drafts = sorted(aligned_drafts, key=lambda d: d.down_time_us)
    
    # 5. Pre-calculate the next occurrence time of the same physical key
    next_same_key_time = {}
    last_seen_by_key = {}
    for idx in range(len(merged_drafts) - 1, -1, -1):
        draft = merged_drafts[idx]
        next_same_key_time[draft.source_index] = last_seen_by_key.get(draft.scan_code)
        last_seen_by_key[draft.scan_code] = (draft.down_time_us, draft.source_time_us)
        
    raw_events = [] # list of dicts: {"at_us": int, "sc": int, "kind": "down"|"up", "reason": str}
    diagnostics = []
    
    # 6. Schedule down/up bounds for each individual note
    for draft in merged_drafts:
        next_same_info = next_same_key_time[draft.source_index]
        sc = draft.scan_code
        shifted_us = draft.down_time_us
        orig_us = draft.source_time_us
        
        effective_delta_us = None
        max_hold = None
        risk = "ok"
        
        if next_same_info is not None:
            next_shifted_us, next_orig_us = next_same_info
            effective_delta_us = next_shifted_us - shifted_us
            
            if shortest_same_key_interval_us is None or effective_delta_us < shortest_same_key_interval_us:
                shortest_same_key_interval_us = effective_delta_us
            
            # Constraint: Must release phím TRƯỚC KHI bấm lại phím đó lần sau ít nhất policy.repeat_release_gap_us
            max_hold = effective_delta_us - policy.repeat_release_gap_us
            
            if max_hold < policy.min_hold_us:
                impossible_same_key_repeats += 1
                risk = "severe"
                diagnostics.append(ScheduleDiagnostic(
                    source_index=draft.source_index,
                    note_key=draft.note_key,
                    scan_code=draft.scan_code,
                    code="impossible_repeat",
                    message=f"Repeat too fast: {effective_delta_us/1000:.1f}ms interval < { (policy.min_hold_us + policy.repeat_release_gap_us)/1000:.1f}ms minimum cycle"
                ))
                if policy.same_key_conflict_policy == "strict":
                    raise ScheduleBuildError(
                        f"Cannot build schedule under strict policy: same-key repeat interval "
                        f"{effective_delta_us / 1000:.1f}ms is below minimum cycle "
                        f"{(policy.min_hold_us + policy.repeat_release_gap_us) / 1000:.1f}ms.",
                        recommended_tempo_scale=_recommended_tempo_scale_for_repeats(
                            effective_delta_us, policy, tempo_scale
                        ),
                        recommended_profile="dense-safe",
                    )
            elif max_hold < policy.hold_us:
                risky_same_key_repeats += 1
                risk = "moderate"
                
        # Determine actual hold duration
        if max_hold is not None:
            actual_hold = max(policy.min_hold_us, min(policy.hold_us, max_hold))
            if actual_hold < policy.hold_us:
                compressed_holds += 1
        else:
            actual_hold = policy.hold_us
            
        up_at_us = shifted_us + actual_hold
        
        # Add events
        raw_events.append({"at_us": shifted_us, "sc": sc, "kind": "down", "reason": "onset"})
        raw_events.append({"at_us": up_at_us, "sc": sc, "kind": "up", "reason": "repeat_release" if next_same_info else "release"})

    # 5.5 Release collision delay: when a standard release coincides with another key's down,
    # defer release so the new down is not swallowed by release ordering.
    downs_at_us: dict[int, set[int]] = {}
    for ev in raw_events:
        if ev["kind"] == "down":
            downs_at_us.setdefault(ev["at_us"], set()).add(ev["sc"])

    for ev in raw_events:
        if ev["kind"] != "up" or ev["reason"] == "repeat_release":
            continue
        conflicting = downs_at_us.get(ev["at_us"], set()) - {ev["sc"]}
        if conflicting:
            ev["at_us"] += policy.release_gap_us
        
    # 6. Group simultaneous events into single KeyAction objects
    grouped = {} # key: (at_us, kind, reason) -> list of scan codes
    for ev in raw_events:
        g_key = (ev["at_us"], ev["kind"], ev["reason"])
        if g_key not in grouped:
            grouped[g_key] = []
        grouped[g_key].append(ev["sc"])
        
    key_actions_list = []
    for (at_us, kind, reason), scs in grouped.items():
        unique_scs = tuple(dict.fromkeys(scs))
        key_actions_list.append(KeyAction(
            at_us=Microseconds(at_us),
            scan_codes=unique_scs,
            kind=kind,
            reason=reason
        ))
        
    # 7. Sort final timeline with strict microsecond accuracy & kind prioritization
    def action_priority(action: KeyAction) -> int:
        if action.kind == "up":
            if action.reason == "repeat_release":
                return 0 # Release repeat keys first!
            elif action.reason == "release":
                return 2 # Safe release after down
            else:
                return 3 # Final release last
        else:
            return 1 # Down note onset has priority over standard releases
            
    key_actions_list.sort(key=lambda a: (a.at_us, action_priority(a)))
    
    # 8. Calculate max polyphony
    active_keys = set()
    max_polyphony = 0
    for action in key_actions_list:
        if action.kind == "down":
            active_keys.update(action.scan_codes)
            max_polyphony = max(max_polyphony, len(active_keys))
        else:
            for sc in action.scan_codes:
                active_keys.discard(sc)
            
    warnings = []
    if impossible_same_key_repeats > 0:
        warnings.append(f"Detected {impossible_same_key_repeats} impossible same-key repeat(s) scheduled too fast.")
    if risky_same_key_repeats > 0:
        warnings.append(f"Detected {risky_same_key_repeats} risky same-key repeat(s) close to min hold.")
    if compressed_holds > 0:
        warnings.append(f"Compressed {compressed_holds} note hold(s) due to dense scheduling.")

    duration_us = Microseconds(key_actions_list[-1].at_us) if key_actions_list else Microseconds(0)
    playback_duration_us = duration_us
    source_duration_us = Microseconds(max((d.source_time_us for d in merged_drafts), default=0) + policy.hold_us)

    return ScheduleMetadata(
        actions=tuple(key_actions_list),
        compressed_holds=compressed_holds,
        impossible_same_key_repeats=impossible_same_key_repeats,
        max_polyphony=max_polyphony,
        note_count=note_count,
        duration_us=duration_us,
        warnings=tuple(warnings),
        risky_same_key_repeats=risky_same_key_repeats,
        shortest_same_key_interval_us=shortest_same_key_interval_us,
        source_duration_us=source_duration_us,
        playback_duration_us=playback_duration_us,
        diagnostics=tuple(diagnostics),
        frame_us=policy.frame_us,
        fps=policy.fps
    )
