import sys
from unittest.mock import patch, MagicMock

from sky_music.infrastructure.realtime import RealtimeProcessScope, DISPATCH_SWITCH_INTERVAL_S, _gil_enabled


def test_realtime_scope_saves_and_restores_switch_interval() -> None:
    initial_interval = sys.getswitchinterval()
    # Ensure our target interval is different from initial to make the test meaningful
    if abs(initial_interval - DISPATCH_SWITCH_INTERVAL_S) < 1e-7:
        temp_interval = 0.005
        sys.setswitchinterval(temp_interval)
        initial_interval = temp_interval

    scope = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=True)
    with scope:
        assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7

    assert abs(sys.getswitchinterval() - initial_interval) < 1e-7


def test_realtime_scope_ablation_flag_respected() -> None:
    initial_interval = sys.getswitchinterval()
    # Ensure it's not the tuned value
    if abs(initial_interval - DISPATCH_SWITCH_INTERVAL_S) < 1e-7:
        sys.setswitchinterval(0.005)
        initial_interval = 0.005

    scope = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=False)
    with scope:
        assert abs(sys.getswitchinterval() - initial_interval) < 1e-7

    assert abs(sys.getswitchinterval() - initial_interval) < 1e-7


def test_realtime_scope_restores_on_exception() -> None:
    initial_interval = sys.getswitchinterval()
    if abs(initial_interval - DISPATCH_SWITCH_INTERVAL_S) < 1e-7:
        sys.setswitchinterval(0.005)
        initial_interval = 0.005

    try:
        with RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=True):
            assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7
            raise ValueError("Intentional exception")
    except ValueError:
        pass

    assert abs(sys.getswitchinterval() - initial_interval) < 1e-7


def test_realtime_scope_nested_idempotence() -> None:
    initial_interval = sys.getswitchinterval()
    if abs(initial_interval - DISPATCH_SWITCH_INTERVAL_S) < 1e-7:
        sys.setswitchinterval(0.005)
        initial_interval = 0.005

    scope1 = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=True)
    scope2 = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=True)

    with scope1:
        assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7
        with scope2:
            assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7
        assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7

    assert abs(sys.getswitchinterval() - initial_interval) < 1e-7


def test_realtime_scope_gc_pause_enabled() -> None:
    with patch("gc.isenabled", return_value=True) as mock_isenabled, \
         patch("gc.collect") as mock_collect, \
         patch("gc.disable") as mock_disable, \
         patch("gc.enable") as mock_enable:
        
        scope = RealtimeProcessScope(enabled=True, enable_switch_interval_tuning=False)
        with scope:
            mock_isenabled.assert_called_once()
            mock_collect.assert_called_once()
            mock_disable.assert_called_once()
            mock_enable.assert_not_called()

        mock_enable.assert_called_once()


def test_realtime_scope_gc_pause_disabled() -> None:
    with patch("gc.isenabled") as mock_isenabled, \
         patch("gc.collect") as mock_collect, \
         patch("gc.disable") as mock_disable, \
         patch("gc.enable") as mock_enable:
        
        scope = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=False)
        with scope:
            mock_isenabled.assert_not_called()
            mock_collect.assert_not_called()
            mock_disable.assert_not_called()

        mock_enable.assert_not_called()


# ---------------------------------------------------------------------------
# A4 — GIL-awareness gate tests (monkeypatched; do not depend on real build)
# ---------------------------------------------------------------------------

def test_gil_enabled_helper_no_probe() -> None:
    """Without sys._is_gil_enabled (Python < 3.13), GIL is assumed present."""
    with patch.object(sys, "_is_gil_enabled", None, create=True):
        # getattr fallback path: attribute is None, not callable → treat as absent
        probe_backup = getattr(sys, "_is_gil_enabled", "MISSING")
        try:
            # Simulate absence by removing the attribute if present
            if hasattr(sys, "_is_gil_enabled"):
                delattr(sys, "_is_gil_enabled")  # type: ignore[misc]
            assert _gil_enabled() is True
        finally:
            if probe_backup != "MISSING":
                sys._is_gil_enabled = probe_backup  # type: ignore[attr-defined]


def test_gil_enabled_helper_with_probe_true() -> None:
    """When sys._is_gil_enabled() returns True, helper returns True."""
    mock_probe = MagicMock(return_value=True)
    with patch.object(sys, "_is_gil_enabled", mock_probe, create=True):
        assert _gil_enabled() is True
        mock_probe.assert_called_once()


def test_gil_enabled_helper_with_probe_false() -> None:
    """When sys._is_gil_enabled() returns False (free-threaded), helper returns False."""
    mock_probe = MagicMock(return_value=False)
    with patch.object(sys, "_is_gil_enabled", mock_probe, create=True):
        assert _gil_enabled() is False
        mock_probe.assert_called_once()


def test_realtime_scope_free_threaded_skips_setswitchinterval() -> None:
    """On a free-threaded build (GIL disabled), setswitchinterval is NOT called
    and _old_switch_interval stays None so __exit__ performs no revert."""
    initial_interval = sys.getswitchinterval()

    with patch("sky_music.infrastructure.realtime._gil_enabled", return_value=False):
        with patch("sys.setswitchinterval") as mock_set, \
             patch("sys.getswitchinterval", return_value=initial_interval):
            scope = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=True)
            with scope:
                mock_set.assert_not_called()
                # _old_switch_interval must be None (no save happened)
                assert scope._old_switch_interval is None
            # __exit__ must not have called setswitchinterval either
            mock_set.assert_not_called()


def test_realtime_scope_gil_enabled_tunes_normally() -> None:
    """When GIL is active, switch-interval tuning proceeds as before."""
    initial_interval = sys.getswitchinterval()
    # Ensure initial != target
    if abs(initial_interval - DISPATCH_SWITCH_INTERVAL_S) < 1e-7:
        sys.setswitchinterval(0.005)
        initial_interval = 0.005

    with patch("sky_music.infrastructure.realtime._gil_enabled", return_value=True):
        scope = RealtimeProcessScope(enabled=False, enable_switch_interval_tuning=True)
        with scope:
            assert abs(sys.getswitchinterval() - DISPATCH_SWITCH_INTERVAL_S) < 1e-7

    assert abs(sys.getswitchinterval() - initial_interval) < 1e-7
