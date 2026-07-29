"""Prewarm admission order regression tests (review of main@7c548527 §3).

Mandatory singleton-up INPUT arrays (one per distinct Sky scan code) MUST be reserved in
the cache BEFORE authored batch shapes are admitted up to the cap. The legacy code added
authored shapes first, then tried to add singleton-ups in a second pass; if authored shapes
already saturated the cap, freshly-needed singleton-up releases hit the lazy build path
under _CACHE_LOCK on the RT path — exactly the work prewarm exists to eliminate.

We monkey-patch ``inputs.ARRAY_CACHE_MAX`` to a tiny value to force the cap pressure on a
small synthetic schedule (without spending 8 192 cache slots), then assert that for every
distinct scan code referenced by the actions, ``((sc,), True)`` is in the prewarm set even
when authored shapes would otherwise fill the cap.
"""
from __future__ import annotations

from unittest.mock import MagicMock, patch

from sky_music.domain.domain import Microseconds, ScanCode
from sky_music.domain.scheduler_types import ActionKind, KeyAction
from sky_music.infrastructure.timing import SleepPolicy
from sky_music.orchestration.engine import PLAYBACK_QUIT, PlaybackEngine


def test_mandatory_singleton_ups_reserved_before_authored_when_cap_is_tight() -> None:
    """Even when authored shapes exceed the cap, every distinct scan code's singleton-up
    pattern survives in the prewarm set, while authored shapes are admitted by frequency.
    """
    # Three distinct keys (16, 17, 18) — singleton-ups for each MUST be reserved regardless.
    # Plus three authored batch shapes — ((16,), False), ((17, 18), False), ((16, 17, 18), True).
    actions = (
        KeyAction(
            kind=ActionKind.DOWN,
            scan_codes=(ScanCode(16),),
            at_us=Microseconds(0),
        ),
        KeyAction(
            kind=ActionKind.DOWN,
            scan_codes=(ScanCode(17), ScanCode(18)),
            at_us=Microseconds(0),
        ),
        KeyAction(
            kind=ActionKind.UP,
            scan_codes=(ScanCode(16), ScanCode(17), ScanCode(18)),
            at_us=Microseconds(100_000),
        ),
    )

    engine = PlaybackEngine(
        song=MagicMock(name="TestSong"),
        actions=actions,
        backend=MagicMock(),
        controls=None,
        sleep_policy=SleepPolicy(poll_s=0.01),
        min_hold_us=50_000,
    )

    with (
        patch("sky_music.platform.win32.inputs.ARRAY_CACHE_MAX", 3),
        patch("sky_music.platform.win32.inputs.prewarm_input_arrays") as mock_prewarm,
        patch.object(engine, "_should_use_dispatch_thread", return_value=True),
    ):
        # Trigger an early return after prewarm so play() exits cleanly.
        engine.require_focus = True
        engine.focus_guard = MagicMock()
        engine.focus_guard.is_active.return_value = False
        controls = MagicMock()
        controls.poll.return_value = "quit"
        engine.controls = controls

        result = engine.play()
        assert result == PLAYBACK_QUIT

    mock_prewarm.assert_called_once()
    shapes = set(mock_prewarm.call_args[0][0])

    # Mandatory: singleton-up for every distinct scan code (16/17/18) — three entries.
    # These MUST all be present even though the cap is 3 (the authored batches must not
    # elbow them out).
    for sc in (16, 17, 18):
        assert ((sc,), True) in shapes, (
            f"mandatory singleton-up shape {((sc,), True)} missing from prewarm set {shapes}"
        )


def test_authored_high_frequency_shape_admitted_before_low_frequency_at_cap_pressure() -> None:
    """Authored batches are admitted by descending frequency, so a hot shape that does
    appear as authored (and would be a frequent cache hit) wins a slot over a once-only
    batch even when both fight for the same remaining cap slack.
    """
    # ((16,), False) appears 2× (two down actions), ((17,), False) appears 1×.
    actions = (
        KeyAction(
            kind=ActionKind.DOWN,
            scan_codes=(ScanCode(16),),
            at_us=Microseconds(0),
        ),
        KeyAction(
            kind=ActionKind.UP,
            scan_codes=(ScanCode(16),),
            at_us=Microseconds(50_000),
        ),
        KeyAction(
            kind=ActionKind.DOWN,
            scan_codes=(ScanCode(16),),
            at_us=Microseconds(100_000),
        ),
        KeyAction(
            kind=ActionKind.UP,
            scan_codes=(ScanCode(16),),
            at_us=Microseconds(150_000),
        ),
        KeyAction(
            kind=ActionKind.DOWN,
            scan_codes=(ScanCode(17),),
            at_us=Microseconds(200_000),
        ),
        KeyAction(
            kind=ActionKind.UP,
            scan_codes=(ScanCode(17),),
            at_us=Microseconds(250_000),
        ),
    )

    engine = PlaybackEngine(
        song=MagicMock(name="TestSong"),
        actions=actions,
        backend=MagicMock(),
        controls=None,
        sleep_policy=SleepPolicy(poll_s=0.01),
        min_hold_us=50_000,
    )

    # Singleton-up for {16, 17} takes 2 of the 4-slot cap. The remaining 2 must go to the
    # three authored shapes (two (16,) down (2× freq), one (17,) down (1× freq)). The
    # high-frequency one — ((16,), False) — must be admitted before ((17,), False).
    with (
        patch("sky_music.platform.win32.inputs.ARRAY_CACHE_MAX", 4),
        patch("sky_music.platform.win32.inputs.prewarm_input_arrays") as mock_prewarm,
        patch.object(engine, "_should_use_dispatch_thread", return_value=True),
    ):
        engine.require_focus = True
        engine.focus_guard = MagicMock()
        engine.focus_guard.is_active.return_value = False
        controls = MagicMock()
        controls.poll.return_value = "quit"
        engine.controls = controls

        engine.play()

    shapes = set(mock_prewarm.call_args[0][0])
    # Mandatory singleton-ups for both keys.
    assert ((16,), True) in shapes
    assert ((17,), True) in shapes
    # The high-frequency authored shape must NOT be crowded out.
    assert ((16,), False) in shapes, (
        f"high-frequency ((16,), False) must be admitted at cap pressure; got {shapes}"
    )
