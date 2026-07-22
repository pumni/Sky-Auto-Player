"""User-facing timing guidance strings (picker modals + advisories).

Security: copy must never instruct the user (or agents) to read game memory,
inject input outside SendInput, or bypass anti-cheat. FPS is user-declared only.
"""

from __future__ import annotations

FPS_MODAL_INFO: str = (
    "[b]Match the FPS you set in Sky[/b]\n"
    "\n"
    "Sky Auto Player schedules note holds from this FPS. It does [b]not[/b] read the game\n"
    "or auto-detect frame rate (by design — no game-process access).\n"
    "\n"
    "[b]If this value is higher than the game's real FPS[/b], short notes may never\n"
    "register. If lower, holds are longer (safer, less sharp).\n"
    "\n"
    "Tip: open Sky settings, note your FPS cap / limit, then pick the same value here.\n"
    "60 FPS is the safe default for mixed local + online play."
)

PROFILE_MODAL_INFO: str = (
    "[b]Timing profile = how long keys stay held[/b]\n"
    "\n"
    "\u2022 [b]local-precise[/b] \u2014 shortest holds (\u2248 1 game frame). Sharpest local feel;\n"
    "  highest miss risk if FPS is wrong or for remote listeners.\n"
    "\u2022 [b]balanced[/b] \u2014 default; small cushion over one frame.\n"
    "\u2022 [b]audience-safe[/b] \u2014 longer holds for online rooms / slower remote clients.\n"
    "\n"
    "Holds scale with the FPS you selected. Wrong FPS + local-precise is the most\n"
    "common cause of \"missing notes\" that is [b]not[/b] a sender bug."
)


def fps_play_advisory(*, fps: int, short_note_count: int) -> str | None:
    """Non-blocking play-start advisory; None when no warning needed."""
    if fps <= 60 or short_note_count <= 0:
        return None
    return (
        f"Profile assumes {fps} fps. {short_note_count} note(s) are shorter than one "
        "60 fps frame (~16.7 ms); if the game runs below the configured fps they may "
        "not register. Lower fps here or use a safer profile (audience-safe / balanced)."
    )