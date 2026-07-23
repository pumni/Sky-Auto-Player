import json
from unittest.mock import MagicMock, patch
from pathlib import Path
import pytest
from sky_music.orchestration.engine import PlaybackEngine, PLAYBACK_QUIT
from sky_music.domain.scheduler_types import KeyAction, ActionKind
from sky_music.infrastructure.timing import SleepPolicy

# 1. Test spin_floor_us=0 not coerced to 700
def test_spin_floor_us_zero_not_coerced():
    engine = PlaybackEngine(
        song=MagicMock(name="TestSong"),
        actions=[],
        backend=MagicMock(),
        controls=None,
        sleep_policy=SleepPolicy(poll_s=0.01),
        min_hold_us=50_000,
        spin_floor_us=0,
    )
    assert engine.spin_floor_us == 0, "spin_floor_us=0 should be preserved, not coerced to 700"

# 2. Test multi-key up prewarm populates cache
@patch("sky_music.platform.win32.inputs.prewarm_input_arrays")
def test_multi_key_up_prewarm(mock_prewarm):
    actions = [
        KeyAction(kind=ActionKind.DOWN, scan_codes=(16, 17, 18), at_us=0),
        KeyAction(kind=ActionKind.UP, scan_codes=(16, 17, 18), at_us=100_000),
    ]
    engine = PlaybackEngine(
        song=MagicMock(name="TestSong"),
        actions=actions,
        backend=MagicMock(),
        controls=None,
        sleep_policy=SleepPolicy(poll_s=0.01),
        min_hold_us=50_000,
    )
    # Require dispatch thread to trigger prewarm
    with patch.object(engine, "_should_use_dispatch_thread", return_value=True):
        # We need a quick return after prewarm to check what was passed.
        # Injecting a focus failure will cause an early quit
        engine.require_focus = True
        engine.focus_guard = MagicMock()
        engine.focus_guard.is_active.return_value = False
        
        # Make controls return quit to avoid infinite loop
        controls = MagicMock()
        controls.poll.return_value = "quit"
        engine.controls = controls
        
        result = engine.play()
        assert result == PLAYBACK_QUIT
        
        # Verify prewarm was called
        mock_prewarm.assert_called_once()
        shapes = mock_prewarm.call_args[0][0]
        
        # We should find the exact multi-key up shape: ((16, 17, 18), True)
        assert ((16, 17, 18), True) in shapes, "Multi-key up shape not found in prewarm set"
        # We should find the exact multi-key down shape: ((16, 17, 18), False)
        assert ((16, 17, 18), False) in shapes, "Multi-key down shape not found in prewarm set"

# 3. Test early-quit clears schedule/cache
def test_early_quit_cleans_resources():
    engine = PlaybackEngine(
        song=MagicMock(name="TestSong"),
        actions=[],
        backend=MagicMock(),
        controls=MagicMock(),
        sleep_policy=SleepPolicy(poll_s=0.01),
        min_hold_us=50_000,
    )
    engine.require_focus = True
    engine.focus_guard = MagicMock()
    engine.focus_guard.is_active.return_value = False
    engine.controls.poll.return_value = "quit"
    
    with patch("sky_music.platform.win32.inputs.clear_array_cache") as mock_clear:
        result = engine.play()
        assert result == PLAYBACK_QUIT
        # The schedule and coordinator should be cleared
        assert engine.runtime_schedule is None
        assert engine._runtime_coordinator is None
        
        # The array cache should be cleared
        mock_clear.assert_called_once()
