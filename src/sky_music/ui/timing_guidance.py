"""User-facing timing guidance strings (picker modals + advisories).

Security: copy must never instruct the user (or agents) to read game memory,
inject input outside SendInput, or bypass anti-cheat. FPS is user-declared only.
"""

from __future__ import annotations

HOLD_MODAL_INFO: str = (
    "[b]Hold is measured in game frames.[/b]\n\n"
    "[b]1.0 frame:[/b] Shortest supported hold and the default. Gives the most room for fast\n"
    "same-key repeats, but is most sensitive to FPS mismatch and sampling phase.\n\n"
    "[b]1.25 frames:[/b] Adds a moderate visibility cushion while keeping repeats relatively compact.\n\n"
    "[b]1.5 frames:[/b] Longest supported hold. Improves key-down visibility, but reduces the\n"
    "available interval for fast repeats on the same key.\n\n"
    "The actual duration is calculated from the FPS selected in this app."
)

FPS_MODAL_INFO: str = (
    "[b]Select the same FPS that you configured inside Sky.[/b]\n\n"
    "Sky Auto Player does not read or auto-detect the game's frame rate.\n"
    "If this value is higher than Sky's real FPS, generated holds may be shorter\n"
    "than one real game frame and notes may not register."
)


def fps_play_advisory(*, fps: int, short_note_count: int) -> str | None:
    """Non-blocking play-start advisory; None when no warning needed."""
    if fps <= 60 or short_note_count <= 0:
        return None
    return (
        f"Hold timing assumes {fps} FPS. {short_note_count} note(s) are shorter than one "
        "60 fps frame (~16.7 ms); if the game runs below the configured fps they may "
        "not register. Lower the configured FPS to match Sky or use a longer hold for visibility."
    )
