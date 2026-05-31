from dataclasses import dataclass
from sky_music.domain.domain import Song, InstrumentProfile, NoteKey
from sky_music.layouts import NoteResolver, DefaultNoteResolver
from sky_music.domain.scheduler_types import TimingPolicy, KeyAction, ScheduleResult, Microseconds, FrameTimingPolicy, ScheduleDiagnostic
import math

class ScheduleBuildError(ValueError):
    """Raised when the schedule cannot be built due to strict conflict policies."""
    def __init__(self, message: str, recommended_tempo_scale: float | None = None, recommended_profile: str | None = None):
        super().__init__(message)
        self.recommended_tempo_scale = recommended_tempo_scale
        self.recommended_profile = recommended_profile

@dataclass(frozen=True, slots=True)
class ScheduledNoteDraft:
    source_time_us: int
    snapped_time_us: int
    shifted_time_us: int
    note_key: NoteKey
    scan_code: int
    source_index: int

def build_key_actions(
    song: Song,
    profile: InstrumentProfile | None = None,
    policy: TimingPolicy | FrameTimingPolicy = TimingPolicy(),
    scan_code_mode: str = "physical",
    resolver: NoteResolver | None = None,
    tempo_scale: float = 1.0
) -> ScheduleResult:
    """
    Builds a microsecond-accurate event timeline from a domain Song.
    Returns a ScheduleResult containing:
      - actions: tuple of sorted KeyAction objects
      - compressed_holds: count of note holds compressed due to dense scheduling
      - impossible_same_key_repeats: count of repeats scheduled too fast to meet min-hold
      - max_polyphony: maximum number of notes pressed simultaneously
      - note_count: total number of notes scheduled
    """
    if tempo_scale <= 0:
        raise ValueError("tempo_scale must be > 0")

    if not isinstance(policy, FrameTimingPolicy):
        policy = FrameTimingPolicy.from_timing_policy(policy)

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
        if k.startswith("1Key") or k.startswith("2Key"):
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
            note_key=k,
            scan_code=sc,
            source_index=idx
        ))
        
    # Sort chronologically by original source time
    drafts.sort(key=lambda d: d.source_time_us)

    # 2. Merge chords using tolerance window (chord_merge_window_us) based on physical scan_codes
    # Group notes of different keys within tolerance to snap to the same timestamp.
    # Same physical keys are NOT merged (they require separate cycles).
    merged_drafts = []
    current_groups = [] # list of tuples: (group_time_us, set_of_scan_codes_in_group)
    
    for draft in drafts:
        snapped = False
        target_snapped_time = draft.source_time_us
        
        if policy.chord_merge_window_us > 0:
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
            current_groups = [g for g in current_groups if draft.source_time_us <= g[0] + policy.chord_merge_window_us]
            
        # Recreate frozen draft with the computed snapped and shifted times
        shifted_time_us = max(0, target_snapped_time - policy.input_lead_us)
        merged_drafts.append(ScheduledNoteDraft(
            source_time_us=draft.source_time_us,
            snapped_time_us=target_snapped_time,
            shifted_time_us=shifted_time_us,
            note_key=draft.note_key,
            scan_code=draft.scan_code,
            source_index=draft.source_index
        ))
        
    # 3. Sort merged_drafts chronologically by shifted_time_us
    merged_drafts.sort(key=lambda d: d.shifted_time_us)
    
    # 4. Pre-calculate the next occurrence time of the same physical key
    next_same_key_time = {}
    last_seen_by_key = {}
    for idx in range(len(merged_drafts) - 1, -1, -1):
        draft = merged_drafts[idx]
        next_same_key_time[draft.source_index] = last_seen_by_key.get(draft.scan_code)
        last_seen_by_key[draft.scan_code] = (draft.shifted_time_us, draft.source_time_us)
        
    raw_events = [] # list of dicts: {"at_us": int, "sc": int, "kind": "down"|"up", "reason": str}
    diagnostics = []
    
    # 5. Schedule down/up bounds for each individual note
    for draft in merged_drafts:
        next_same_info = next_same_key_time[draft.source_index]
        sc = draft.scan_code
        shifted_us = draft.shifted_time_us
        orig_us = draft.source_time_us
        
        effective_delta_us = None
        max_hold = None
        risk = "ok"
        
        # Calculate maximum possible hold duration for this key
        if next_same_info is not None:
            next_shifted, next_orig = next_same_info
            effective_delta_us = next_shifted - shifted_us
            
            # Analyze interval delta from unshifted (original) timeline for report accuracy
            delta = next_orig - orig_us
            if shortest_same_key_interval_us is None or delta < shortest_same_key_interval_us:
                shortest_same_key_interval_us = delta
                
            max_hold = effective_delta_us - policy.repeat_release_gap_us
            required_interval_us = policy.hold_us + policy.repeat_release_gap_us
            
            # Strict/Adaptive same-key repeat policy check
            if effective_delta_us < required_interval_us:
                if policy.same_key_conflict_policy in ("strict", "adaptive"):
                    rec_tempo = round(tempo_scale * (delta / required_interval_us), 2)
                    if rec_tempo > 0.95:
                        rec_tempo = 0.92
                    if rec_tempo < 0.5:
                        rec_tempo = 0.5
                    
                    policy_str = policy.same_key_conflict_policy.capitalize()
                    
                    if effective_delta_us < policy.min_scheduled_hold_us + policy.repeat_release_gap_us:
                        msg = (
                            f"{policy_str} Policy Violation: Impossible same-key repeat detected. "
                            f"Effective delta between repeats is {effective_delta_us}us, which is below the minimum "
                            f"physical requirement of {policy.min_scheduled_hold_us + policy.repeat_release_gap_us}us."
                        )
                    else:
                        msg = (
                            f"{policy_str} Policy Violation: Impossible same-key repeat detected. "
                            f"Effective delta between repeats is {effective_delta_us}us, which is below the required "
                            f"interval of {required_interval_us}us (hold={policy.hold_us}us + gap={policy.repeat_release_gap_us}us)."
                        )
                        
                    raise ScheduleBuildError(
                        msg,
                        recommended_tempo_scale=rec_tempo,
                        recommended_profile="dense-safe"
                    )
            
            # Classify same-key repeat severity based on effective_delta_us for degraded mode
            if effective_delta_us < policy.min_scheduled_hold_us + policy.repeat_release_gap_us:
                risk = "impossible_repeat"
                if effective_delta_us <= 0:
                    impossible_same_key_repeats += 1
                    compressed_holds += 1
                    hold = 0
                    reason_up = "repeat_release"
                else:
                    impossible_same_key_repeats += 1
                    compressed_holds += 1
                    hold = min(policy.min_scheduled_hold_us, effective_delta_us)
                    reason_up = "repeat_release"
            elif effective_delta_us < policy.min_hold_us + policy.repeat_release_gap_us:
                risk = "risky_repeat"
                risky_same_key_repeats += 1
                compressed_holds += 1
                hold = max(policy.min_scheduled_hold_us, max_hold)
                reason_up = "repeat_release"
            elif max_hold < policy.hold_us:
                risk = "compressed"
                compressed_holds += 1
                hold = max(policy.min_scheduled_hold_us, max_hold)
                reason_up = "repeat_release"
            else:
                hold = policy.hold_us
                reason_up = "release"
        else:
            hold = policy.hold_us
            reason_up = "final_release"
            
        down_us = shifted_us
        up_us = shifted_us + hold
        
        diagnostics.append(ScheduleDiagnostic(
            source_index=draft.source_index,
            note_key=draft.note_key,
            scan_code=sc,
            source_time_us=Microseconds(orig_us),
            scheduled_down_us=Microseconds(down_us),
            scheduled_up_us=Microseconds(up_us),
            hold_us=Microseconds(hold),
            reason=reason_up,
            risk=risk
        ))
        
        raw_events.append({"at_us": down_us, "sc": sc, "kind": "down", "reason": "note"})
        raw_events.append({"at_us": up_us, "sc": sc, "kind": "up", "reason": reason_up})
        
    # 5.5 Delay normal releases that coincide with note down onsets to safety gap
    down_timestamps = {ev["at_us"] for ev in raw_events if ev["kind"] == "down"}
    for ev in raw_events:
        if ev["kind"] == "up" and ev["reason"] in ("release", "final_release"):
            if ev["at_us"] in down_timestamps:
                ev["at_us"] += policy.release_gap_us
        
    # 6. Group (coalesce) events at the exact same timestamp + kind
    grouped = {} # key: (at_us, kind, reason) -> list of scan codes
    for ev in raw_events:
        g_key = (ev["at_us"], ev["kind"], ev["reason"])
        if g_key not in grouped:
            grouped[g_key] = []
        grouped[g_key].append(ev["sc"])
        
    # Build a raw list of grouped KeyActions
    key_actions_list = []
    for (at_us, kind, reason), scs in grouped.items():
        unique_scs = tuple(dict.fromkeys(scs))
        key_actions_list.append(KeyAction(
            at_us=at_us,
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
    
    # 8. Calculate max polyphony (simultaneous active down keys)
    active_keys = set()
    max_polyphony = 0
    for action in key_actions_list:
        if action.kind == "down":
            active_keys.update(action.scan_codes)
            max_polyphony = max(max_polyphony, len(active_keys))
        else:
            active_keys.difference_update(action.scan_codes)
            
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

    return ScheduleResult(
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

