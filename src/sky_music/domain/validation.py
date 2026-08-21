from dataclasses import dataclass
from typing import Literal

from sky_music.domain.scheduler_types import FrameTimingPolicy, KeyAction

# Absolute hard floor for any scheduled key-down, used by the schedule validator as a
# backstop for the frame-aware minimum. It is a global constant, not a user
# timing selection. The selected frame-relative floor is min_hold_us.
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
        "same_key_release_gap",
        "excessive_polyphony",
    ]
    message: str
    at_us: int | None = None
    scan_code: int | None = None
    severity: Literal["info", "warning", "fatal"] = "fatal"


def validate_song_structure(song_dict: dict, filepath_str: str) -> None:
    """Strictly validates the high-level schema structure of a song dictionary."""
    if not isinstance(song_dict, dict):
        raise SongValidationError(
            f"[{filepath_str}] Invalid root element: expected JSON object, got {type(song_dict).__name__}"
        )

    if "songNotes" not in song_dict:
        raise SongValidationError(f"[{filepath_str}] Missing required key: 'songNotes'")

    song_notes = song_dict["songNotes"]
    if not isinstance(song_notes, list):
        raise SongValidationError(
            f"[{filepath_str}] Invalid 'songNotes': expected list, got {type(song_notes).__name__}"
        )


def validate_key_actions(
    actions: tuple[KeyAction, ...],
    policy: FrameTimingPolicy | None = None,
) -> tuple[ScheduleInvariantViolation, ...]:
    """
    Validates a sequence of KeyAction events to ensure correct input state transitions.
    Returns a tuple of ScheduleInvariantViolation objects describing any anomalies found.
    """
    if policy is None:
        policy = FrameTimingPolicy.from_hold_frames(1.0, 60)
    elif not isinstance(policy, FrameTimingPolicy):
        raise TypeError(
            "validate_key_actions requires FrameTimingPolicy; "
            "pass the same policy used to build the schedule."
        )

    min_release_gap_us = policy.min_release_gap_us
    if min_release_gap_us is None:
        raise ValueError("FrameTimingPolicy must materialize min_release_gap_us")

    violations = []
    active_keys = set()
    active_downs = {}  # scan_code -> (at_us, action_idx)
    last_ups: dict[int, int] = {}

    prev_at_us = -1
    for idx, action in enumerate(actions):
        # 1. Timeline sorted check
        if action.at_us < prev_at_us:
            violations.append(
                ScheduleInvariantViolation(
                    code="unsorted_timeline",
                    message=f"Action timeline is not sorted: index {idx} has at_us={action.at_us}us while previous was {prev_at_us}us",
                    at_us=action.at_us,
                    severity="fatal",
                )
            )
        prev_at_us = action.at_us

        # 2. Negative timestamp check
        if action.at_us < 0:
            violations.append(
                ScheduleInvariantViolation(
                    code="negative_timestamp",
                    message=f"Action at index {idx} has a negative timestamp: {action.at_us}us",
                    at_us=action.at_us,
                    severity="fatal",
                )
            )

        # 3. Empty scan codes check
        if not action.scan_codes:
            violations.append(
                ScheduleInvariantViolation(
                    code="empty_scan_codes",
                    message=f"Action at index {idx} at {action.at_us}us has no scan codes",
                    at_us=action.at_us,
                    severity="warning",
                )
            )

        # 4. Duplicate down & hold duration validation checks
        if action.kind == "down":
            for sc in action.scan_codes:
                if sc in last_ups:
                    release_gap = action.at_us - last_ups[sc]
                    if release_gap < int(min_release_gap_us):
                        violations.append(
                            ScheduleInvariantViolation(
                                code="same_key_release_gap",
                                message=(
                                    f"Scan code {sc} was released at {last_ups[sc]}us and "
                                    f"re-pressed at {action.at_us}us; release gap "
                                    f"{release_gap}us is below the required release "
                                    f"visibility gap ({int(min_release_gap_us)}us); base "
                                    f"game frame is {int(policy.frame_us)}us"
                                ),
                                at_us=action.at_us,
                                scan_code=sc,
                                severity="fatal",
                            )
                        )
                if sc in active_keys:
                    severity = (
                        "fatal"
                        if policy.same_key_conflict_policy == "strict"
                        else "warning"
                    )
                    violations.append(
                        ScheduleInvariantViolation(
                            code="duplicate_down",
                            message=f"Scan code {sc} pressed down at {action.at_us}us while already pressed",
                            at_us=action.at_us,
                            scan_code=sc,
                            severity=severity,
                        )
                    )
                active_keys.add(sc)
                active_downs[sc] = (action.at_us, idx)

        elif action.kind == "up":
            for sc in action.scan_codes:
                if sc not in active_keys:
                    violations.append(
                        ScheduleInvariantViolation(
                            code="unpaired_up",
                            message=f"Scan code {sc} released at {action.at_us}us but was not active",
                            at_us=action.at_us,
                            scan_code=sc,
                            severity="warning",
                        )
                    )
                else:
                    # Check hold duration against the resolved frame-relative floor. A hold
                    # equal to one frame is valid; shorter holds can fall between per-frame
                    # samples.
                    down_at, _down_idx = active_downs[sc]
                    hold = action.at_us - down_at
                    frame_floor = int(policy.frame_us)
                    min_req = max(ABSOLUTE_MIN_HOLD_US, frame_floor, int(policy.min_hold_us))
                    if hold < frame_floor:
                        violations.append(
                            ScheduleInvariantViolation(
                                code="insufficient_hold",
                                message=f"Hold duration for scan code {sc} is {hold}us, below 1-frame floor {frame_floor}us",
                                at_us=action.at_us,
                                scan_code=sc,
                                severity="fatal",
                            )
                        )
                    elif hold < min_req:
                        severity = (
                            "fatal"
                            if policy.same_key_conflict_policy == "strict"
                            else "warning"
                        )
                        violations.append(
                            ScheduleInvariantViolation(
                                code="insufficient_hold",
                                message=f"Hold duration for scan code {sc} is {hold}us, below required minimum {min_req}us",
                                at_us=action.at_us,
                                scan_code=sc,
                                severity=severity,
                            )
                        )

                    active_keys.discard(sc)
                    active_downs.pop(sc, None)
                    last_ups[sc] = action.at_us

    # 5. Stuck keys at the end of the song
    if active_keys:
        violations.extend(
            ScheduleInvariantViolation(
                code="stuck_keys",
                message=f"Scan code {sc} remains pressed after the end of the playback timeline",
                at_us=actions[-1].at_us if actions else 0,
                scan_code=sc,
                severity="fatal",
            )
            for sc in sorted(active_keys)
        )

    # 6. Max polyphony check
    active_keys_poly = set()
    for action in actions:
        if action.kind == "down":
            active_keys_poly.update(action.scan_codes)
            if len(active_keys_poly) > 15:
                violations.append(
                    ScheduleInvariantViolation(
                        code="excessive_polyphony",
                        message=f"Simultaneous polyphony of {len(active_keys_poly)} keys at {action.at_us}us exceeds threshold of 15 keys",
                        at_us=action.at_us,
                        severity="warning",
                    )
                )
        elif action.kind == "up":
            active_keys_poly.difference_update(action.scan_codes)
    return tuple(violations)
