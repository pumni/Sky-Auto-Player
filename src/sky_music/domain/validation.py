import math
from dataclasses import dataclass
from typing import Literal

from sky_music.domain.scheduler_types import FrameTimingPolicy, KeyAction

# Absolute hard floor for any scheduled key-down, used by the schedule validator as a
# backstop when frame-aware sizing is unavailable. This was previously a per-profile
# field (min_scheduled_hold_us, identical = 500 everywhere); it is a global constant, not
# a tuning knob. The real per-profile floor is min_hold_us (frame-aware).
ABSOLUTE_MIN_HOLD_US: int = 500

class SongParseError(Exception):
    """Raised when the file format is corrupt, unparseable, or invalid JSON."""
    pass

class SongValidationError(Exception):
    """Raised when the sheet data does not conform to the required layout/schema specifications."""
    pass

@dataclass(frozen=True, slots=True)
class ScheduleInvariantViolation:
    code: Literal[
        "negative_timestamp",
        "duplicate_down",
        "empty_scan_codes",
        "stuck_keys",
        "unsorted_timeline",
        "unpaired_up",
        "insufficient_hold",
        "excessive_polyphony"
    ]
    message: str
    at_us: int | None = None
    scan_code: int | None = None
    severity: Literal["info", "warning", "fatal"] = "fatal"

def validate_song_structure(song_dict: dict, filepath_str: str) -> None:
    """Strictly validates the high-level schema structure of a song dictionary."""
    if not isinstance(song_dict, dict):
        raise SongValidationError(f"[{filepath_str}] Invalid root element: expected JSON object, got {type(song_dict).__name__}")
        
    if "songNotes" not in song_dict:
        raise SongValidationError(f"[{filepath_str}] Missing required key: 'songNotes'")
        
    song_notes = song_dict["songNotes"]
    if not isinstance(song_notes, list):
        raise SongValidationError(f"[{filepath_str}] Invalid 'songNotes': expected list, got {type(song_notes).__name__}")

def validate_key_actions(
    actions: tuple[KeyAction, ...],
    policy: FrameTimingPolicy | None = None,
) -> tuple[ScheduleInvariantViolation, ...]:
    """
    Validates a sequence of KeyAction events to ensure correct input state transitions.
    Returns a tuple of ScheduleInvariantViolation objects describing any anomalies found.
    """
    if policy is None:
        policy = FrameTimingPolicy.balanced()
    elif not isinstance(policy, FrameTimingPolicy):
        raise TypeError(
            "validate_key_actions requires FrameTimingPolicy; "
            "pass the same policy used to build the schedule."
        )

    violations = []
    active_keys = set()
    active_downs = {} # scan_code -> (at_us, action_idx)
    
    prev_at_us = -1
    for idx, action in enumerate(actions):
        # 1. Timeline sorted check
        if action.at_us < prev_at_us:
            violations.append(ScheduleInvariantViolation(
                code="unsorted_timeline",
                message=f"Action timeline is not sorted: index {idx} has at_us={action.at_us}us while previous was {prev_at_us}us",
                at_us=action.at_us,
                severity="fatal"
            ))
        prev_at_us = action.at_us

        # 2. Negative timestamp check
        if action.at_us < 0:
            violations.append(ScheduleInvariantViolation(
                code="negative_timestamp",
                message=f"Action at index {idx} has a negative timestamp: {action.at_us}us",
                at_us=action.at_us,
                severity="fatal"
            ))
            
        # 3. Empty scan codes check
        if not action.scan_codes:
            violations.append(ScheduleInvariantViolation(
                code="empty_scan_codes",
                message=f"Action at index {idx} at {action.at_us}us has no scan codes",
                at_us=action.at_us,
                severity="warning"
            ))
            
        # 4. Duplicate down & hold duration validation checks
        if action.kind == "down":
            for sc in action.scan_codes:
                if sc in active_keys:
                    severity = "fatal" if policy.same_key_conflict_policy == "strict" else "warning"
                    violations.append(ScheduleInvariantViolation(
                        code="duplicate_down",
                        message=f"Scan code {sc} pressed down at {action.at_us}us while already pressed",
                        at_us=action.at_us,
                        scan_code=sc,
                        severity=severity,
                    ))
                active_keys.add(sc)
                active_downs[sc] = (action.at_us, idx)
                
        elif action.kind == "up":
            for sc in action.scan_codes:
                if sc not in active_keys:
                    violations.append(ScheduleInvariantViolation(
                        code="unpaired_up",
                        message=f"Scan code {sc} released at {action.at_us}us but was not active",
                        at_us=action.at_us,
                        scan_code=sc,
                        severity="warning"
                    ))
                else:
                    # Check hold duration against the visibility floor (Appendix A): a hold
                    # shorter than one frame can fall between the game's per-frame input
                    # samples. When frame-aware, require >= one frame; otherwise fall back to
                    # the absolute scheduled-hold floor.
                    down_at, _down_idx = active_downs[sc]
                    hold = action.at_us - down_at
                    frame_floor = int(policy.frame_us) if getattr(policy, "frame_us", 0) else 0
                    policy_min_hold = int(policy.min_hold_us) if getattr(policy, "min_hold_us", 0) else 0
                    min_req = max(ABSOLUTE_MIN_HOLD_US, frame_floor, policy_min_hold)
                    if frame_floor > 0 and hold < frame_floor:
                        violations.append(ScheduleInvariantViolation(
                            code="insufficient_hold",
                            message=f"Hold duration for scan code {sc} is {hold}us, below 1-frame floor {frame_floor}us",
                            at_us=action.at_us,
                            scan_code=sc,
                            severity="fatal"
                        ))
                    elif hold < min_req:
                        severity = "fatal" if policy.same_key_conflict_policy == "strict" else "warning"
                        violations.append(ScheduleInvariantViolation(
                            code="insufficient_hold",
                            message=f"Hold duration for scan code {sc} is {hold}us, below required minimum {min_req}us",
                            at_us=action.at_us,
                            scan_code=sc,
                            severity=severity
                        ))
                    
                    active_keys.discard(sc)
                    active_downs.pop(sc, None)

    # 5. Stuck keys at the end of the song
    if active_keys:
        violations.extend(
            ScheduleInvariantViolation(
                code="stuck_keys",
                message=f"Scan code {sc} remains pressed after the end of the playback timeline",
                at_us=actions[-1].at_us if actions else 0,
                scan_code=sc,
                severity="fatal"
            )
            for sc in sorted(active_keys)
        )

    # 6. Max polyphony check
    active_keys_poly = set()
    for action in actions:
        if action.kind == "down":
            active_keys_poly.update(action.scan_codes)
            if len(active_keys_poly) > 15:
                violations.append(ScheduleInvariantViolation(
                    code="excessive_polyphony",
                    message=f"Simultaneous polyphony of {len(active_keys_poly)} keys at {action.at_us}us exceeds threshold of 15 keys",
                    at_us=action.at_us,
                    severity="warning"
                ))
        elif action.kind == "up":
            active_keys_poly.difference_update(action.scan_codes)
            
    return tuple(violations)


def _has_frame_model(profile: dict, stem: str) -> bool:
    return f"{stem}_frames" in profile


def _frame_coupled_us(
    profile: dict,
    *,
    stem: str,
    legacy_key: str,
    default_frames: float,
    fps: int,
) -> int:
    if legacy_key in profile and _has_frame_model(profile, stem):
        return int(profile[legacy_key])
    if _has_frame_model(profile, stem):
        from sky_music.domain.scheduler_types import FrameTimingPolicy
        frame_us = math.ceil(1_000_000 / fps)
        frames = float(profile.get(f"{stem}_frames", default_frames))
        # Device-delivery margin — mirror of FrameTimingPolicy.from_timing_policy: added only in
        # the frame-model branch (explicit legacy/override values above win verbatim; both hold
        # and min_hold stems get it so the ordering invariant is preserved). Keep the two
        # computations identical.
        margin_us = max(0, int(profile.get("min_hold_margin_us", 500)))
        return int(FrameTimingPolicy.materialise_frame_us(frames, frame_us)) + margin_us
    return int(profile[legacy_key])


def _hold_us(profile: dict, *, fps: int = 60) -> int | None:
    if "hold_us" not in profile and not _has_frame_model(profile, "hold"):
        return None
    return _frame_coupled_us(
        profile,
        stem="hold",
        legacy_key="hold_us",
        default_frames=1.25,
        fps=fps,
    )


def _min_hold_us(profile: dict, *, fps: int = 60) -> int | None:
    if "min_hold_us" not in profile and not _has_frame_model(profile, "min_hold"):
        return None
    return _frame_coupled_us(
        profile,
        stem="min_hold",
        legacy_key="min_hold_us",
        default_frames=1.25,
        fps=fps,
    )


def validate_hold_ordering(profile: dict[str, int]) -> None:
    """Single source of truth for the hold-duration ordering invariant.

    Enforces ``0 < min_hold_us <= hold_us`` for whichever of those keys are present.
    ``hold_us`` is the preferred (ceiling) duration and must never sit below
    ``min_hold_us`` (the visible-down floor); previously this relationship was assumed
    everywhere but never validated, so a profile could silently ship ``hold_us`` below
    its own floor (e.g. ``hold_us: 1``). The absolute lower bound for any scheduled hold
    is the module constant ``ABSOLUTE_MIN_HOLD_US`` (no longer a per-profile field).
    """
    min_hold = _min_hold_us(profile)
    hold = _hold_us(profile)

    if min_hold is not None and min_hold <= 0:
        raise ValueError("min_hold_us must be > 0")
    if hold is not None and min_hold is not None and hold < min_hold:
        raise ValueError(
            f"hold_us ({hold}us) must be >= min_hold_us ({min_hold}us)"
        )


def validate_timing_profile(profile: dict[str, int], *, fps: int = 60) -> None:
    frame_us = 1_000_000 / fps

    hold_frames = profile.get("hold_frames")
    min_hold_frames = profile.get("min_hold_frames")
    
    if hold_frames is not None:
        if isinstance(hold_frames, bool):
            raise ValueError("hold_frames must be numeric")
        f_hold = float(hold_frames)
        if not math.isfinite(f_hold) or f_hold <= 0:
            raise ValueError("hold_frames must be > 0 and finite")
            
    if min_hold_frames is not None:
        if isinstance(min_hold_frames, bool):
            raise ValueError("min_hold_frames must be numeric")
        f_min = float(min_hold_frames)
        if not math.isfinite(f_min) or f_min < 1.0:
            raise ValueError("min_hold_frames must be >= 1.0 and finite")
        if hold_frames is not None and f_min > float(hold_frames):
            raise ValueError("min_hold_frames must be <= hold_frames")

    validate_hold_ordering(profile)

    min_hold_us = _min_hold_us(profile, fps=fps)
    if min_hold_us is None:
        raise ValueError("min_hold_us must be present")

    if min_hold_us <= frame_us:
        raise ValueError(
            f"Unsafe min_hold_us: {min_hold_us:.0f}us <= one frame {frame_us:.0f}us"
        )

    frame_model_min_hold = _has_frame_model(profile, "min_hold") and "min_hold_us" not in profile
    if min_hold_us < 10_000 and not frame_model_min_hold:
        raise ValueError("min_hold_us below 10000us is not allowed for built-ins")

def validate_builtin_timing_profile(
    _name: str,
    profile: dict[str, int],
    *,
    selected_fps: int = 60,
) -> None:
    validate_timing_profile(profile, fps=selected_fps if selected_fps > 0 else 60)
